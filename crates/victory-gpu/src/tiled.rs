//! Unified Tiled Physical-Page Pool (`VICTORY_TILED_POOL=1`).
//!
//! Xbox consoles map 64 KiB physical pages (`xmem`) in scattered orders, with
//! unmapped holes and multi-VA aliasing. D3D12 `OpenExistingHeapFromAddress1`
//! requires a single contiguous OS allocation, which turns down ~85% of GTA VI's
//! windows and buffers.
//!
//! Hardware-verified on AMD Radeon RX 6600 (`TiledResourcesTier = 4`):
//! - Each physical 64 KiB page `(section_handle, page_index)` touched by the GPU
//!   is backed ONCE in a shared pool of 64 MiB `D3D12_HEAP_TYPE_CUSTOM`
//!   (`MEMORY_POOL_L0`, `CPU_PAGE_PROPERTY_WRITE_BACK`) heaps with a persistent
//!   CPU `Map()` pointer.
//! - Each scattered/sparse window or buffer becomes a `CreateReservedResource`
//!   whose 64 KiB tiles map directly to those pool tiles via `UpdateTileMappings`
//!   (in arbitrary scattered order, with `D3D12_TILE_RANGE_FLAG_NULL` for holes
//!   — 0 bytes allocated! — and shared tiles for aliased VAs — 100% GPU-coherent
//!   in hardware with zero duplication!).
//! - When `xmem` streams/remaps pages (`inplace::view_epoch()`), only the changed
//!   tiles call `UpdateTileMappings` (~0.2 ms for 64 tiles) in-place without
//!   recreating descriptors or draining the GPU.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC};

const K64: usize = 64 << 10;
const CHUNK_TILES: usize = 1024; // 64 MiB per heap chunk
const CHUNK_BYTES: u64 = (CHUNK_TILES * K64) as u64;

/// Physical page identity matching `inplace::views()` (`xmem` section handle +
/// 64 KiB page index inside that section).
pub(crate) type PhysPage = (usize, u32);

/// Whether the Tiled Physical-Page Pool is enabled (`VICTORY_TILED_POOL=1`).
pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var_os("VICTORY_TILED_POOL").is_some_and(|v| v != "0" && !v.is_empty());
        if on {
            tracing::info!(
                budget_mb = budget_bytes() >> 20,
                "unified tiled physical-page pool enabled (VICTORY_TILED_POOL=1)"
            );
        }
        on
    })
}

fn budget_bytes() -> u64 {
    static MB: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *MB.get_or_init(|| {
        std::env::var("VICTORY_TILED_POOL_MB")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(2560)
            .clamp(128, 6144)
            << 20
    })
}

/// Telemetry counters:
/// 0: reserved resources alive (windows)
/// 1: reserved virtual bytes alive (windows)
/// 2: reserved resources alive (buffers)
/// 3: reserved virtual bytes alive (buffers)
/// 4: UpdateTileMappings calls
/// 5: UpdateTileMappings total microseconds
/// 6: tiles mapped (non-null) across live resources
/// 7: null hole tiles saved across live resources
/// 8: deduplicated page uploads skipped (already fresh in pool!)
/// 9: turned down (non-view / mixed private / over budget)
static STATS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];

/// A single 64 KiB physical tile inside a `HeapChunk`.
#[derive(Clone, Copy)]
pub(crate) struct TileSlot {
    pub(crate) chunk: u32,
    pub(crate) tile: u32,
    /// Persistent CPU Write-Back pointer to the 64 KiB tile (`L0` RAM).
    pub(crate) ptr: *mut u8,
    pub(crate) page: PhysPage,
}

// SAFETY: `ptr` points to persistently mapped `L0` heap memory kept alive for
// the lifetime of the chunk (never unmapped while any `Shadow` or GPU flight
// can reference it).
unsafe impl Send for TileSlot {}
unsafe impl Sync for TileSlot {}

struct HeapChunk {
    heap: ID3D12Heap,
    /// Placed buffer covering the full 64 MiB chunk so the CPU has a persistent
    /// Write-Back mapping (`ptr`) to every tile in `heap`.
    _cpu_buf: ID3D12Resource,
    cpu_ptr: *mut u8,
    next_fresh: u32,
    free: Vec<u32>,
}

// SAFETY: D3D12 heaps/resources are free-threaded; `cpu_ptr` is immutable for
// the lifetime of `HeapChunk`.
unsafe impl Send for HeapChunk {}

struct PoolEntry {
    slot: TileSlot,
    /// Last `watch::writes` generation at which this whole 64 KiB physical page
    /// was synced from title RAM into `slot.ptr` (deduplicates aliased uploads).
    synced_watch: u64,
    /// Number of live `TiledBacking` tile slots currently referencing this
    /// physical page. When it drops to 0, the tile is retired back to
    /// `chunk.free` once in-flight GPU work completes (preventing pool growth!).
    refs: u32,
}

#[derive(Default)]
struct TiledPool {
    chunks: Vec<HeapChunk>,
    pages: HashMap<PhysPage, PoolEntry>,
    /// Index of pages grouped by `section` handle for O(K) cleanup on
    /// `title_memory_section_gone`.
    by_section: HashMap<usize, Vec<u32>>,
    allocated_bytes: u64,
}

static POOL: LazyLock<Mutex<TiledPool>> = LazyLock::new(Mutex::default);

/// Returns a recycled tile back to its chunk free-list ONLY after all in-flight
/// GPU work has completed (via `host::retire`).
struct RetiredTiles(Vec<(u32, u32)>);

impl Drop for RetiredTiles {
    fn drop(&mut self) {
        if self.0.is_empty() {
            return;
        }
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        for &(chunk, tile) in &self.0 {
            if let Some(c) = pool.chunks.get_mut(chunk as usize) {
                c.free.push(tile);
            }
        }
    }
}

impl TiledPool {
    fn alloc_chunk(&mut self, host: &crate::host::Host) -> Option<u32> {
        if self.allocated_bytes.saturating_add(CHUNK_BYTES) > budget_bytes() {
            return None;
        }
        if let Some([_, _, nonlocal_budget, nonlocal_used]) = host.video_memory() {
            let cap = nonlocal_budget / 100 * 84;
            if nonlocal_used.saturating_add(CHUNK_BYTES) > cap {
                return None;
            }
        }
        let desc = D3D12_HEAP_DESC {
            SizeInBytes: CHUNK_BYTES,
            Properties: D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_CUSTOM,
                CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_WRITE_BACK,
                MemoryPoolPreference: D3D12_MEMORY_POOL_L0,
                CreationNodeMask: 0,
                VisibleNodeMask: 0,
            },
            Alignment: K64 as u64,
            Flags: D3D12_HEAP_FLAG_ALLOW_ONLY_BUFFERS,
        };
        let mut heap: Option<ID3D12Heap> = None;
        // SAFETY: plain heap creation on live device.
        unsafe { host.device.CreateHeap(&desc, &mut heap).ok()? };
        let heap = heap?;
        let buf_desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Alignment: 0,
            Width: CHUNK_BYTES,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_UNKNOWN,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            Flags: D3D12_RESOURCE_FLAG_NONE,
        };
        let mut cpu_buf: Option<ID3D12Resource> = None;
        // SAFETY: placed buffer covering the freshly created L0 WB heap.
        unsafe {
            host.device
                .CreatePlacedResource(
                    &heap,
                    0,
                    &buf_desc,
                    D3D12_RESOURCE_STATE_COMMON,
                    None,
                    &mut cpu_buf,
                )
                .ok()?;
        }
        let cpu_buf = cpu_buf?;
        let mut ptr: *mut c_void = std::ptr::null_mut();
        // SAFETY: persistent CPU Write-Back mapping of the placed buffer.
        unsafe { cpu_buf.Map(0, None, Some(&mut ptr)).ok()? };
        if ptr.is_null() {
            return None;
        }
        let id = self.chunks.len() as u32;
        crate::host::name(&cpu_buf, || format!("tiled pool L0 chunk #{id} (64 MiB)"));
        self.chunks.push(HeapChunk {
            heap,
            _cpu_buf: cpu_buf,
            cpu_ptr: ptr.cast(),
            next_fresh: 0,
            free: Vec::new(),
        });
        self.allocated_bytes += CHUNK_BYTES;
        Some(id)
    }

    fn acquire(&mut self, host: &crate::host::Host, page: PhysPage) -> Option<TileSlot> {
        if let Some(entry) = self.pages.get_mut(&page) {
            entry.refs = entry.refs.saturating_add(1);
            return Some(entry.slot);
        }
        let mut chosen: Option<(u32, u32)> = None;
        for (idx, chunk) in self.chunks.iter_mut().enumerate() {
            if let Some(t) = chunk.free.pop() {
                chosen = Some((idx as u32, t));
                break;
            }
            if (chunk.next_fresh as usize) < CHUNK_TILES {
                let t = chunk.next_fresh;
                chunk.next_fresh += 1;
                chosen = Some((idx as u32, t));
                break;
            }
        }
        let (chunk_id, tile) = match chosen {
            Some(c) => c,
            None => {
                let id = self.alloc_chunk(host)?;
                let chunk = &mut self.chunks[id as usize];
                let t = chunk.next_fresh;
                chunk.next_fresh += 1;
                (id, t)
            }
        };
        let ptr = unsafe { self.chunks[chunk_id as usize].cpu_ptr.add(tile as usize * K64) };
        // Zero newly assigned tile so initial partial copies never expose stale bytes.
        unsafe { std::ptr::write_bytes(ptr, 0, K64) };
        let slot = TileSlot {
            chunk: chunk_id,
            tile,
            ptr,
            page,
        };
        self.pages.insert(
            page,
            PoolEntry {
                slot,
                synced_watch: 0,
                refs: 1,
            },
        );
        self.by_section.entry(page.0).or_default().push(page.1);
        Some(slot)
    }

    fn release_pages(&mut self, pages: &[PhysPage]) -> Vec<(u32, u32)> {
        let mut freed = Vec::new();
        for &page in pages {
            let remove = if let Some(entry) = self.pages.get_mut(&page) {
                entry.refs = entry.refs.saturating_sub(1);
                entry.refs == 0
            } else {
                false
            };
            if remove && let Some(entry) = self.pages.remove(&page) {
                freed.push((entry.slot.chunk, entry.slot.tile));
            }
        }
        freed
    }
}

/// Releases all physical pages belonging to a closed `xmem` section back to the
/// pool's free lists once in-flight GPU work finishes.
pub(crate) fn section_gone(section: usize) {
    if !enabled() {
        return;
    }
    let retired = {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        let Some(pages) = pool.by_section.remove(&section) else {
            return;
        };
        let mut slots = Vec::with_capacity(pages.len());
        for p in pages {
            if let Some(entry) = pool.pages.remove(&(section, p)) {
                slots.push((entry.slot.chunk, entry.slot.tile));
            }
        }
        slots
    };
    if !retired.is_empty() {
        crate::host::retire(RetiredTiles(retired));
    }
}

/// Checks if a physical page in the pool was already freshly synced at `min_watch`.
pub(crate) fn is_page_synced(page: PhysPage, min_watch: u64) -> bool {
    if min_watch == 0 {
        return false;
    }
    let pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    pool.pages
        .get(&page)
        .is_some_and(|e| e.synced_watch >= min_watch)
}

/// Marks a physical page as freshly synced at generation `now` (after a whole
/// 64 KiB page copy from title RAM into `slot.ptr`).
pub(crate) fn mark_page_synced(page: PhysPage, now: u64) {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = pool.pages.get_mut(&page) {
        entry.synced_watch = entry.synced_watch.max(now);
    }
}

pub(crate) fn note_dedup_skip() {
    STATS[8].fetch_add(1, Ordering::Relaxed);
}

/// Per-`Shadow` tiled backing state for a `CreateReservedResource` mapped into
/// the shared `TiledPool`.
pub(crate) struct TiledBacking {
    /// Byte offset from `base` (64 KiB aligned) to `Shadow::start`.
    bias: u64,
    /// 64 KiB-aligned start virtual address in title memory.
    base: usize,
    n_tiles: usize,
    window: bool,
    len: u64,
    /// Current mapping of each 64 KiB tile (`None` = NULL hole tile).
    slots: Mutex<Vec<Option<TileSlot>>>,
    /// Last `inplace::view_epoch()` applied to this resource.
    map_gen: AtomicU64,
    /// Cached counts for telemetry cleanup on drop: `(mapped_tiles, hole_tiles)`.
    counts: Mutex<(u64, u64)>,
}

impl TiledBacking {
    pub(crate) fn bias(&self) -> u64 {
        self.bias
    }

    pub(crate) fn needs_sync(&self) -> bool {
        let last = self.map_gen.load(Ordering::Acquire);
        let end = self.base.saturating_add(self.n_tiles * K64);
        matches!(
            crate::inplace::range_changed_since(last, self.base, end),
            crate::inplace::RangeSync::Rescan
        )
    }

    /// Returns a pointer inside the tile holding byte offset `at` (from
    /// `Shadow::start`), or `None` if that tile is an unmapped `NULL` hole.
    /// Safe for any stretch staying within a single 64 KiB aligned piece
    /// (`PIECE == 64 KiB == TILE_SIZE`).
    pub(crate) fn ptr_at(&self, start: usize, at: usize) -> Option<*mut u8> {
        let va = start + at;
        let idx = (va - self.base) / K64;
        let in_tile = va & (K64 - 1);
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let slot = (*slots.get(idx)?)?;
        // SAFETY: `in_tile < K64`, and `slot.ptr` points to a 64 KiB tile.
        Some(unsafe { slot.ptr.add(in_tile) })
    }

    /// Snapshots `[from, to)` (offsets from `Shadow::start`) directly into
    /// `dst_base` (offset `0` of the snapshot buffer), filling `NULL` holes with
    /// zeros.
    pub(crate) fn snapshot_into(&self, start: usize, from: usize, to: usize, dst_base: *mut u8) {
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let mut at = from;
        while at < to {
            let va = start + at;
            let idx = (va - self.base) / K64;
            let in_tile = va & (K64 - 1);
            let n = (K64 - in_tile).min(to - at);
            match slots.get(idx).copied().flatten() {
                Some(slot) => unsafe {
                    std::ptr::copy_nonoverlapping(slot.ptr.add(in_tile), dst_base.add(at), n);
                },
                None => unsafe {
                    std::ptr::write_bytes(dst_base.add(at), 0, n);
                },
            }
            at += n;
        }
    }

    /// Queues tile-level copy jobs `(src, dst, len)` for `take_snapshots()` and
    /// immediately zeroes any `NULL` hole slices in `dst_base`.
    pub(crate) fn queue_snapshot_jobs(
        &self,
        start: usize,
        from: usize,
        to: usize,
        dst_base: usize,
        snaps: &mut Vec<(usize, usize, usize)>,
    ) {
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let mut at = from;
        while at < to {
            let va = start + at;
            let idx = (va - self.base) / K64;
            let in_tile = va & (K64 - 1);
            let n = (K64 - in_tile).min(to - at);
            match slots.get(idx).copied().flatten() {
                Some(slot) => {
                    snaps.push((slot.ptr as usize + in_tile, dst_base + at, n));
                }
                None => unsafe {
                    std::ptr::write_bytes((dst_base + at) as *mut u8, 0, n);
                },
            }
            at += n;
        }
    }

    /// Collects per-tile compare jobs `(tile_ptr_or_0, before_ptr, at, n)` for
    /// parallel execution in `shadow::find_all`. Locks `self.slots` once per
    /// range so all 64 KiB tile comparisons run across the `wide` thread pool.
    pub(crate) fn collect_compare_tiles(
        &self,
        start: usize,
        from: usize,
        to: usize,
        before_base: usize,
        out: &mut Vec<(usize, usize, usize, usize)>,
    ) {
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let mut at = from;
        while at < to {
            let va = start + at;
            let idx = (va - self.base) / K64;
            let in_tile = va & (K64 - 1);
            let n = (K64 - in_tile).min(to - at);
            let tile_ptr = match slots.get(idx).copied().flatten() {
                Some(slot) => unsafe { slot.ptr.add(in_tile) as usize },
                None => 0,
            };
            out.push((tile_ptr, before_base + at, at, n));
            at += n;
        }
    }

    /// Compares `[from, to)` against `before` (which starts at offset `from`)
    /// piece by piece (64 KiB aligned), returning differing `(at, n)` stretches.
    pub(crate) fn differing(
        &self,
        start: usize,
        from: usize,
        to: usize,
        before: &[u8],
    ) -> Vec<(usize, usize)> {
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let mut found = Vec::new();
        let mut at = from;
        while at < to {
            let va = start + at;
            let idx = (va - self.base) / K64;
            let in_tile = va & (K64 - 1);
            let n = (K64 - in_tile).min(to - at);
            let old = &before[at - from..at - from + n];
            let differs = match slots.get(idx).copied().flatten() {
                Some(slot) => {
                    let now = unsafe { std::slice::from_raw_parts(slot.ptr.add(in_tile), n) };
                    now != old
                }
                None => old.iter().any(|&b| b != 0),
            };
            if differs {
                found.push((at, n));
            }
            at += n;
        }
        found
    }

    /// Copies title memory `[from, to)` into the corresponding tiles of this
    /// reserved resource. Skips `NULL` holes (0 bytes copied!) and skips whole
    /// 64 KiB physical pages that were already freshly synced at `watch_now` via
    /// an aliased window/buffer!
    pub(crate) fn copy_from_title(
        &self,
        from: usize,
        to: usize,
        watch_now: Option<u64>,
    ) -> usize {
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let mut copied_bytes = 0usize;
        let mut at = from;
        while at < to {
            let idx = (at - self.base) / K64;
            let in_tile = at & (K64 - 1);
            let n = (K64 - in_tile).min(to - at);
            if let Some(slot) = slots.get(idx).copied().flatten() {
                let whole_page = in_tile == 0 && n == K64;
                if whole_page
                    && let Some(now) = watch_now
                    && is_page_synced(slot.page, now)
                {
                    note_dedup_skip();
                    at += n;
                    continue;
                }
                let dst = unsafe { slot.ptr.add(in_tile) };
                crate::host::copy_mapped(dst, at as *const u8, n, false);
                copied_bytes += n;
                if whole_page && let Some(now) = watch_now {
                    mark_page_synced(slot.page, now);
                }
            }
            at += n;
        }
        copied_bytes
    }

    /// Fast-path check against `inplace::view_epoch()`: if `xmem` mapped or
    /// unmapped any pages in this resource's VA range since the last check,
    /// updates ONLY the changed tiles via `UpdateTileMappings` and returns their
    /// 64 KiB page indices (relative to `base`) so `Shadow::copied[]` can be
    /// invalidated for newly mapped pages.
    pub(crate) fn sync_mappings(&self, buffer: &ID3D12Resource) -> Vec<usize> {
        let last = self.map_gen.load(Ordering::Acquire);
        let end = self.base.saturating_add(self.n_tiles * K64);
        match crate::inplace::range_changed_since(last, self.base, end) {
            crate::inplace::RangeSync::UpToDate => return Vec::new(),
            crate::inplace::RangeSync::AdvanceTo(ep) => {
                self.map_gen.store(ep, Ordering::Release);
                return Vec::new();
            }
            crate::inplace::RangeSync::Rescan => {}
        }
        let Some(host) = crate::host::host() else {
            return Vec::new();
        };
        let (epoch, wanted) = crate::inplace::pages_in_range(self.base, self.n_tiles);
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let mut changed_indices = Vec::new();
        let mut null_tiles: Vec<u32> = Vec::new();
        let mut by_chunk: Vec<(u32, Vec<(u32, u32)>)> = Vec::new();

        let mut to_release: Vec<PhysPage> = Vec::new();
        let retired = {
            let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
            for (i, want) in wanted.into_iter().enumerate() {
                let cur_page = slots[i].map(|s| s.page);
                if cur_page == want {
                    continue;
                }
                changed_indices.push(i);
                if let Some(old_page) = cur_page {
                    to_release.push(old_page);
                }
                match want {
                    None => {
                        slots[i] = None;
                        null_tiles.push(i as u32);
                    }
                    Some(page) => {
                        if let Some(slot) = pool.acquire(host, page) {
                            slots[i] = Some(slot);
                            if let Some((_, list)) = by_chunk.iter_mut().find(|(c, _)| *c == slot.chunk) {
                                list.push((i as u32, slot.tile));
                            } else {
                                by_chunk.push((slot.chunk, vec![(i as u32, slot.tile)]));
                            }
                        } else {
                            slots[i] = None;
                            null_tiles.push(i as u32);
                        }
                    }
                }
            }
            if !null_tiles.is_empty() || !by_chunk.is_empty() {
                apply_tile_updates(host, buffer, &pool, &by_chunk, &null_tiles);
            }
            if !to_release.is_empty() {
                pool.release_pages(&to_release)
            } else {
                Vec::new()
            }
        };
        if !retired.is_empty() {
            crate::host::retire(RetiredTiles(retired));
        }
        if !changed_indices.is_empty() {
            let new_mapped = slots.iter().filter(|s| s.is_some()).count() as u64;
            let new_holes = self.n_tiles as u64 - new_mapped;
            let mut c = self.counts.lock().unwrap_or_else(|e| e.into_inner());
            STATS[6].fetch_sub(c.0, Ordering::Relaxed);
            STATS[7].fetch_sub(c.1, Ordering::Relaxed);
            STATS[6].fetch_add(new_mapped, Ordering::Relaxed);
            STATS[7].fetch_add(new_holes, Ordering::Relaxed);
            *c = (new_mapped, new_holes);
        }
        self.map_gen.store(epoch, Ordering::Release);
        changed_indices
    }
}

impl Drop for TiledBacking {
    fn drop(&mut self) {
        let idx = if self.window { 0 } else { 2 };
        STATS[idx].fetch_sub(1, Ordering::Relaxed);
        STATS[idx + 1].fetch_sub(self.len, Ordering::Relaxed);
        let c = *self.counts.get_mut().unwrap_or_else(|e| e.into_inner());
        STATS[6].fetch_sub(c.0, Ordering::Relaxed);
        STATS[7].fetch_sub(c.1, Ordering::Relaxed);
        let to_release: Vec<PhysPage> = self
            .slots
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter_map(|s| s.map(|slot| slot.page))
            .collect();
        if !to_release.is_empty() {
            let freed = POOL
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .release_pages(&to_release);
            if !freed.is_empty() {
                crate::host::retire(RetiredTiles(freed));
            }
        }
    }
}

/// Applies tile mapping updates to `buffer` in batches (grouped by chunk heap +
/// one optional NULL-unmap call), merging consecutive `(res_tile, heap_tile)`
/// runs so `UpdateTileMappings` sends minimal ranges to the WDDM kernel.
fn apply_tile_updates(
    host: &crate::host::Host,
    buffer: &ID3D12Resource,
    pool: &TiledPool,
    by_chunk: &[(u32, Vec<(u32, u32)>)],
    null_tiles: &[u32],
) {
    let queue = host.queue();
    let t0 = std::time::Instant::now();
    let mut calls = 0u64;

    if !null_tiles.is_empty() {
        // Coalesce consecutive resource tiles into runs for NULL unmapping.
        let mut coords: Vec<D3D12_TILED_RESOURCE_COORDINATE> = Vec::new();
        let mut sizes: Vec<D3D12_TILE_REGION_SIZE> = Vec::new();
        let mut counts: Vec<u32> = Vec::new();
        let mut i = 0;
        while i < null_tiles.len() {
            let start = null_tiles[i];
            let mut run = 1u32;
            while i + (run as usize) < null_tiles.len()
                && null_tiles[i + (run as usize)] == start + run
            {
                run += 1;
            }
            coords.push(D3D12_TILED_RESOURCE_COORDINATE {
                X: start,
                Y: 0,
                Z: 0,
                Subresource: 0,
            });
            sizes.push(D3D12_TILE_REGION_SIZE {
                NumTiles: run,
                UseBox: false.into(),
                Width: 0,
                Height: 0,
                Depth: 0,
            });
            counts.push(run);
            i += run as usize;
        }
        let flags = vec![D3D12_TILE_RANGE_FLAG_NULL; counts.len()];
        // SAFETY: valid reserved buffer and NULL tile range arrays.
        unsafe {
            queue.UpdateTileMappings(
                buffer,
                coords.len() as u32,
                Some(coords.as_ptr()),
                Some(sizes.as_ptr()),
                None,
                flags.len() as u32,
                Some(flags.as_ptr()),
                None,
                Some(counts.as_ptr()),
                D3D12_TILE_MAPPING_FLAG_NONE,
            );
        }
        calls += 1;
    }

    for (chunk_id, pairs) in by_chunk {
        if pairs.is_empty() {
            continue;
        }
        let heap = &pool.chunks[*chunk_id as usize].heap;
        // Coalesce runs where BOTH resource tile and heap tile are consecutive!
        let mut coords: Vec<D3D12_TILED_RESOURCE_COORDINATE> = Vec::new();
        let mut sizes: Vec<D3D12_TILE_REGION_SIZE> = Vec::new();
        let mut offsets: Vec<u32> = Vec::new();
        let mut counts: Vec<u32> = Vec::new();
        let mut i = 0;
        while i < pairs.len() {
            let (r0, h0) = pairs[i];
            let mut run = 1u32;
            while i + (run as usize) < pairs.len()
                && pairs[i + (run as usize)] == (r0 + run, h0 + run)
            {
                run += 1;
            }
            coords.push(D3D12_TILED_RESOURCE_COORDINATE {
                X: r0,
                Y: 0,
                Z: 0,
                Subresource: 0,
            });
            sizes.push(D3D12_TILE_REGION_SIZE {
                NumTiles: run,
                UseBox: false.into(),
                Width: 0,
                Height: 0,
                Depth: 0,
            });
            offsets.push(h0);
            counts.push(run);
            i += run as usize;
        }
        let flags = vec![D3D12_TILE_RANGE_FLAG_NONE; counts.len()];
        // Chunk large updates to <= 128 ranges per call so WDDM never hits
        // the quadratic 1024-range cliff measured on RX 6600.
        const MAX_RANGES: usize = 128;
        let mut off = 0;
        while off < coords.len() {
            let end = (off + MAX_RANGES).min(coords.len());
            let k = (end - off) as u32;
            // SAFETY: valid reserved buffer, live chunk heap, and matching slices.
            unsafe {
                queue.UpdateTileMappings(
                    buffer,
                    k,
                    Some(coords[off..end].as_ptr()),
                    Some(sizes[off..end].as_ptr()),
                    heap,
                    k,
                    Some(flags[off..end].as_ptr()),
                    Some(offsets[off..end].as_ptr()),
                    Some(counts[off..end].as_ptr()),
                    D3D12_TILE_MAPPING_FLAG_NONE,
                );
            }
            calls += 1;
            off = end;
        }
    }

    if calls > 0 {
        STATS[4].fetch_add(calls, Ordering::Relaxed);
        STATS[5].fetch_add(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
    }
}

/// Checks whether `[base, base + n_tiles * K64)` contains any committed memory
/// outside `xmem`'s known `views` (e.g. a partial `MEM_PRIVATE` region). Only
/// pure `xmem` physical-page ranges (views + uncommitted placeholder holes) are
/// backed by the Tiled Pool so no valid `MEM_PRIVATE` page can ever become a
/// `NULL` tile.
fn has_unknown_committed_pages(base: usize, wanted: &[Option<PhysPage>]) -> bool {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, VirtualQuery,
    };
    let mut i = 0;
    while i < wanted.len() {
        if wanted[i].is_some() {
            i += 1;
            continue;
        }
        let addr = base + i * K64;
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let n = unsafe { VirtualQuery(addr as *const c_void, &mut info, size_of_val(&info)) };
        if n == 0 {
            return true;
        }
        if info.State == MEM_COMMIT {
            return true;
        }
        let reg_end = (info.BaseAddress as usize).saturating_add(info.RegionSize);
        let skip = reg_end.saturating_sub(addr) / K64;
        i += skip.max(1);
    }
    false
}

/// Attempts to back `[start, start + len)` with a `CreateReservedResource`
/// mapped into the shared `TiledPool` (for `xmem` physical-page memory that
/// `inplace::place_for` turned down due to scatter/holes/SEC_RESERVE).
pub(crate) fn create(
    start: u64,
    len: u64,
    window: bool,
) -> Option<(ID3D12Resource, u64, TiledBacking)> {
    if !enabled() || len == 0 || len >= (1 << 30) {
        return None;
    }
    let host = crate::host::host()?;
    if host.tiled_tier() < 3 {
        return None;
    }
    let base = (start as usize) & !(K64 - 1);
    let end_aligned = (start.checked_add(len)?).next_multiple_of(K64 as u64) as usize;
    let n_tiles = (end_aligned - base) / K64;
    if n_tiles == 0 {
        return None;
    }
    let (epoch, wanted) = crate::inplace::pages_in_range(base, n_tiles);
    let mapped_wanted = wanted.iter().filter(|p| p.is_some()).count();
    if mapped_wanted == 0 || has_unknown_committed_pages(base, &wanted) {
        STATS[9].fetch_add(1, Ordering::Relaxed);
        return None;
    }

    let width = (n_tiles * K64) as u64;
    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        Alignment: 0,
        Width: width,
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: DXGI_FORMAT_UNKNOWN,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        Flags: D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
    };
    let mut buffer: Option<ID3D12Resource> = None;
    // SAFETY: plain reserved buffer creation on Tier 3+ device.
    if unsafe {
        host.device
            .CreateReservedResource(&desc, D3D12_RESOURCE_STATE_COMMON, None, &mut buffer)
    }
    .is_err()
    {
        STATS[9].fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let buffer = buffer?;

    let mut slots: Vec<Option<TileSlot>> = vec![None; n_tiles];
    let mut by_chunk: Vec<(u32, Vec<(u32, u32)>)> = Vec::new();
    let mut acquired: Vec<PhysPage> = Vec::with_capacity(mapped_wanted);
    let rollback_freed = {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        let mut failed = false;
        for (i, want) in wanted.into_iter().enumerate() {
            if let Some(page) = want {
                let Some(slot) = pool.acquire(host, page) else {
                    failed = true;
                    break;
                };
                acquired.push(page);
                slots[i] = Some(slot);
                if let Some((_, list)) = by_chunk.iter_mut().find(|(c, _)| *c == slot.chunk) {
                    list.push((i as u32, slot.tile));
                } else {
                    by_chunk.push((slot.chunk, vec![(i as u32, slot.tile)]));
                }
            }
        }
        if failed {
            let freed = pool.release_pages(&acquired);
            Some(freed)
        } else {
            // Newly created ReservedResource starts with all tiles NULL, so only
            // non-null tiles need UpdateTileMappings!
            apply_tile_updates(host, &buffer, &pool, &by_chunk, &[]);
            None
        }
    };
    if let Some(freed) = rollback_freed {
        if !freed.is_empty() {
            crate::host::retire(RetiredTiles(freed));
        }
        STATS[9].fetch_add(1, Ordering::Relaxed);
        return None;
    }

    let bias = start - base as u64;
    // SAFETY: live reserved buffer.
    let gpu = unsafe { buffer.GetGPUVirtualAddress() } + bias;
    crate::host::name(&buffer, || {
        format!("tiled physical {start:#x}+{len:#x} ({mapped_wanted}/{n_tiles} tiles)")
    });

    let mapped_count = mapped_wanted as u64;
    let hole_count = (n_tiles - mapped_wanted) as u64;
    let stat_idx = if window { 0 } else { 2 };
    STATS[stat_idx].fetch_add(1, Ordering::Relaxed);
    STATS[stat_idx + 1].fetch_add(len, Ordering::Relaxed);
    STATS[6].fetch_add(mapped_count, Ordering::Relaxed);
    STATS[7].fetch_add(hole_count, Ordering::Relaxed);

    let backing = TiledBacking {
        bias,
        base,
        n_tiles,
        window,
        len,
        slots: Mutex::new(slots),
        map_gen: AtomicU64::new(epoch),
        counts: Mutex::new((mapped_count, hole_count)),
    };
    Some((buffer, gpu, backing))
}

/// Telemetry line printed alongside `inplace::say()` every 10 seconds.
pub(crate) fn say() {
    if !enabled() {
        return;
    }
    let (chunks, alloc_mb, unique_pages) = {
        let pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        (pool.chunks.len(), pool.allocated_bytes >> 20, pool.pages.len())
    };
    let win = STATS[0].load(Ordering::Relaxed);
    let win_mb = STATS[1].load(Ordering::Relaxed) >> 20;
    let buf = STATS[2].load(Ordering::Relaxed);
    let buf_mb = STATS[3].load(Ordering::Relaxed) >> 20;
    let utm = STATS[4].load(Ordering::Relaxed);
    let utm_us = STATS[5].load(Ordering::Relaxed);
    let mapped_tiles = STATS[6].load(Ordering::Relaxed);
    let hole_tiles = STATS[7].load(Ordering::Relaxed);
    let dedup = STATS[8].load(Ordering::Relaxed);
    let refused = STATS[9].load(Ordering::Relaxed);
    let total_tiles = mapped_tiles + hole_tiles;
    let hole_pct = if total_tiles > 0 {
        hole_tiles as f64 * 100.0 / total_tiles as f64
    } else {
        0.0
    };
    let avg_us = if utm > 0 { utm_us as f64 / utm as f64 } else { 0.0 };
    tracing::info!(
        "tiled physical-page pool (VICTORY_TILED_POOL=1): {chunks} L0 WB chunks ({alloc_mb} MB), \
         {unique_pages} unique pages ({} MB) backing {win} win ({win_mb} MB virtual) + {buf} buf ({buf_mb} MB virtual); \
         {hole_tiles} hole tiles = 0 B ({hole_pct:.1}% sparse saved!), {dedup} deduped uploads; \
         {utm} UpdateTileMappings ({avg_us:.0} us avg); {refused} skipped",
        (unique_pages * K64) >> 20,
    );
}
