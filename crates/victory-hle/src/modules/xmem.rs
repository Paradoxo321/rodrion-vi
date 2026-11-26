//! `xmem.dll`: console memory, served from host memory. The GPU side works
//! from CPU addresses (unified memory), so memory types and bandwidth
//! classes change nothing here; what constrains addresses (alignment,
//! 2 MiB pages, address-bit limits) is honoured.
#![allow(non_snake_case)]
// Title code calls these through its import table, passing pointers the real
// API would trust.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::alloc::{Layout, alloc, dealloc};
use std::collections::{BTreeMap, HashMap};
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, Once};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_COMMITMENT_LIMIT, EXCEPTION_ACCESS_VIOLATION, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_CONTINUE_SEARCH,
    EXCEPTION_POINTERS,
};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, MEM_ADDRESS_REQUIREMENTS, MEM_COMMIT, MEM_EXTENDED_PARAMETER,
    MEM_PRESERVE_PLACEHOLDER, MEM_RELEASE, MEM_REPLACE_PLACEHOLDER, MEM_RESERVE,
    MEM_RESERVE_PLACEHOLDER, MEMORY_BASIC_INFORMATION, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile3,
    MemExtendedParameterAddressRequirements, PAGE_EXECUTE_READWRITE, PAGE_NOACCESS, PAGE_READONLY,
    PAGE_READWRITE, SEC_COMMIT, SEC_RESERVE, UnmapViewOfFile2, VirtualAlloc, VirtualAlloc2,
    VirtualFree, VirtualProtect, VirtualQuery, VirtualQueryEx,
};
use windows_sys::Win32::System::ProcessStatus::{
    K32GetPerformanceInfo, K32GetProcessMemoryInfo, PERFORMANCE_INFORMATION,
    PROCESS_MEMORY_COUNTERS,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, SwitchToThread,
};
use windows_sys::core::BOOL;

use crate::{HleModule, hle_export};

const MIB: usize = 1 << 20;

/// Whether we run under Wine (its ntdll exports `wine_get_version`).
pub(crate) fn on_wine() -> bool {
    static WINE: LazyLock<bool> = LazyLock::new(|| {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
        // SAFETY: plain lookups in ntdll, always loaded.
        unsafe {
            let ntdll = GetModuleHandleA(c"ntdll.dll".as_ptr().cast());
            !ntdll.is_null() && GetProcAddress(ntdll, c"wine_get_version".as_ptr().cast()).is_some()
        }
    });
    *WINE
}

/// Standard memory the title has handed to the system.
static TRANSFERRED: AtomicUsize = AtomicUsize::new(0);
/// Console physical pages currently allocated via `XMemAllocatePhysicalPages`.
static PHYSICAL_BYTES: AtomicUsize = AtomicUsize::new(0);

// Console additions to the host's flags.
/// 2 MiB pages; the host reads this bit as large pages, which need a
/// privilege.
const PAGES_2M: u32 = 0x2000_0000;
/// With `PAGES_2M`: 64 KiB pages; the host reads it as AWE memory.
const PAGES_64K_BIT: u32 = 0x40_0000;
const PAGES_64K: u32 = PAGES_2M | PAGES_64K_BIT;
/// The host's protection constants and `PAGE_GUARD`. Above them the console
/// adds GPU access bits (0x800-0x40000). Write-combining and uncached are
/// dropped too: our GPU side reads this memory through the CPU.
const HOST_PROTECTION: u32 = 0x1ff;
/// XMem flags of memory both processors reach: all of ours.
const GRAPHICS: u64 = 3;
/// XMem flag bit of 32-bit graphics memory.
const ADDRESS_32BIT: u64 = 4;
/// The host's write watching (not in windows-sys' memory module).
const MEM_WRITE_WATCH: u32 = 0x20_0000;

/// The addresses XMem `flags` allow, first and last: the low 4 GiB for
/// 32-bit memory; else from 4 GiB (kept for 32-bit requests, which the
/// title's big pools would otherwise crowd out) up to 46 down to 40 bits
/// from the field at bit 12, or 40 bits without one: the console keeps
/// title memory there, and its ray-tracing structures must be (
/// the title's BVH builder refuses one above; the title asks for its GPU heap
/// with no field). Decision 0001.
fn address_range(flags: u64) -> (usize, usize) {
    let bits = match (flags & ADDRESS_32BIT, flags >> 12 & 7) {
        (0, 0) => 40,
        (0, field) => 47 - field,
        _ => return (0, u32::MAX as usize),
    };
    (1 << 32, (1 << bits) - 1)
}

pub extern "system" fn XMemVirtualAlloc(
    base: *mut c_void,
    size: usize,
    kind: u32,
    flags: u64,
    protect: u32,
) -> *mut c_void {
    let (lowest, highest) = address_range(flags);
    let mut requirements = MEM_ADDRESS_REQUIREMENTS {
        LowestStartingAddress: lowest as *mut c_void,
        HighestEndingAddress: highest as *mut c_void,
        Alignment: if kind & (PAGES_2M | PAGES_64K_BIT) == PAGES_2M {
            2 * MIB
        } else {
            0
        },
    };
    let mut protect = match protect & HOST_PROTECTION {
        0 => PAGE_NOACCESS,
        p => p,
    };
    // A reservation of 64 KiB pages takes physical pages later
    // (`XMemMapPhysicalPages`); on the host only a placeholder can.
    // shortcut: committing part of such a reservation later would fail
    // (the title's 23 calls never do); replace that part of the
    // placeholder then.
    let placeholder = kind & PAGES_64K == PAGES_64K && kind & MEM_COMMIT == 0;
    if placeholder {
        if kind & MEM_REPLACE_PLACEHOLDER != 0 {
            // Part of a placeholder the title split: it stays one.
            return base;
        }
        protect = PAGE_NOACCESS;
    }
    let mut kind = kind & !(PAGES_2M | PAGES_64K_BIT);
    if placeholder {
        kind |= MEM_RESERVE | MEM_RESERVE_PLACEHOLDER;
    }
    let mut parameter = MEM_EXTENDED_PARAMETER::default();
    parameter.Anonymous1._bitfield = MemExtendedParameterAddressRequirements as u64;
    parameter.Anonymous2.Pointer = (&raw mut requirements).cast();
    // Address requirements apply to reservations at an address of our choice.
    let constrained = base.is_null();
    if constrained {
        kind |= MEM_RESERVE;
        // The GPU side copies only the pages the title wrote since its last
        // copy (`victory_gpu`, decision 0007); placeholders cannot be
        // watched (their physical-page views are copied whole).
        if kind & MEM_RESERVE_PLACEHOLDER == 0 {
            kind |= MEM_WRITE_WATCH;
        }
    }
    // SAFETY: the title's own arguments, cleaned of console bits; the
    // parameter points at `requirements`, which outlives the call.
    let at = unsafe {
        VirtualAlloc2(
            GetCurrentProcess(),
            base,
            size,
            kind,
            protect,
            if constrained {
                &mut parameter
            } else {
                std::ptr::null_mut()
            },
            constrained.into(),
        )
    };
    if base.is_null() {
        tracing::debug!(
            size,
            kind = format_args!("{kind:#x}"),
            flags = format_args!("{flags:#x}"),
            at = format_args!("{at:p}"),
            "XMemVirtualAlloc"
        );
    }
    at
}

/// `MEMORY_BASIC_INFORMATION` (the console's first 0x30 bytes are the
/// same), then the XMem flags and tag.
#[repr(C)]
pub struct BasicInformation {
    host: MEMORY_BASIC_INFORMATION,
    flags: u64,
    tag_code: u32,
    tag: [u8; 4],
}

// shortcut: every committed page reports graphics memory and no tag, since
// all of it is GPU-reachable here (the title's BVH builder, the
// only caller, wants graphics memory); remember the flags of
// `XMemVirtualAlloc` regions if a title tells CPU-only memory apart.
pub extern "system" fn XMemVirtualQuery(
    process: HANDLE,
    address: *const c_void,
    out: *mut BasicInformation,
) -> BOOL {
    let size = size_of::<MEMORY_BASIC_INFORMATION>();
    // SAFETY: the title passes room for the whole structure; the host
    // fails on a null one.
    unsafe {
        if VirtualQueryEx(process, address, out.cast(), size) == 0 {
            return 0;
        }
        let info = &mut *out;
        info.flags = if info.host.State == MEM_COMMIT {
            GRAPHICS
        } else {
            0
        };
        info.tag_code = 0;
        info.tag = [0; 4];
    }
    1
}

// `XMemAlloc` attributes: alignment as log2 (5 bits at 24, 4 bytes and up
// from 2), then the memory type, then bit 33: a 32-bit address.
const ALIGNMENT_SHIFT: u64 = 24;
const ALLOC_32BIT: u64 = 1 << 33;
/// Our block header: the block's size and alignment, just below the
/// pointer handed out.
const HEADER: usize = 2 * size_of::<usize>();

fn layout(size: usize, attributes: u64) -> Option<Layout> {
    // An alignment of at least 16 leaves room for the header.
    let align = 1usize << (attributes >> ALIGNMENT_SHIFT & 0x1f).max(4);
    Layout::from_size_align(size.checked_add(align)?, align).ok()
}

type AllocRoutine = extern "system" fn(usize, u64) -> *mut c_void;
type FreeRoutine = extern "system" fn(*mut c_void, u64);

/// The allocation and free routines the title's exe exports as
/// `__XMemAllocationHooks__` (the title does): `XMemAlloc` and
/// `XMemFree` call them, for the title's own calls and its middleware's;
/// the `…Default` exports do not.
fn hooks() -> [usize; 2] {
    static AT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let at = match AT.get() {
        Some(&at) => at,
        // Looked up once the title is mapped, which is before it runs.
        None => match crate::runtime::main_image() {
            Some(base) => *AT.get_or_init(|| {
                use windows_sys::Win32::System::LibraryLoader::GetProcAddress;
                // SAFETY: a mapped image and a NUL-terminated name.
                let at = unsafe {
                    GetProcAddress(base as _, c"__XMemAllocationHooks__".as_ptr().cast())
                }
                .map_or(0, |f| f as usize);
                tracing::info!(found = at != 0, "xmem allocation hooks");
                at
            }),
            None => 0,
        },
    };
    if at == 0 {
        return [0; 2];
    }
    // SAFETY: the export is two routine pointers.
    unsafe { (at as *const [usize; 2]).read() }
}

/// The title's general allocator, for middleware too: its hook if it has
/// one, else [`XMemAllocDefault`].
pub extern "system" fn XMemAlloc(size: usize, attributes: u64) -> *mut c_void {
    match hooks()[0] {
        0 => XMemAllocDefault(size, attributes),
        hook => {
            // SAFETY: the title's own routine of this type.
            let alloc: AllocRoutine = unsafe { std::mem::transmute(hook) };
            alloc(size, attributes)
        }
    }
}

/// See [`XMemAlloc`].
pub extern "system" fn XMemFree(address: *mut c_void, attributes: u64) {
    match hooks()[1] {
        0 => XMemFreeDefault(address, attributes),
        hook => {
            // SAFETY: the title's own routine of this type.
            let free: FreeRoutine = unsafe { std::mem::transmute(hook) };
            free(address, attributes)
        }
    }
}

/// The system allocator behind [`XMemAlloc`].
pub extern "system" fn XMemAllocDefault(size: usize, attributes: u64) -> *mut c_void {
    if attributes & ALLOC_32BIT != 0 {
        return XMemVirtualAlloc(
            std::ptr::null_mut(),
            size,
            MEM_RESERVE | MEM_COMMIT,
            GRAPHICS | ADDRESS_32BIT,
            PAGE_READWRITE,
        );
    }
    let Some(layout) = layout(size, attributes) else {
        return std::ptr::null_mut();
    };
    // shortcut: the host heap's addresses lie above 2^40, unlike the
    // console's; nothing seen builds ray-tracing structures in this memory
    // (the title uses its GPU heap). Sub-allocate from a
    // reservation below 2^40 if one does.
    // SAFETY: the layout has a non-zero size.
    let block = unsafe { alloc(layout) };
    if block.is_null() {
        return block.cast();
    }
    // SAFETY: the block holds the alignment (at least the header) and then
    // `size` bytes.
    unsafe {
        let user = block.add(layout.align());
        user.sub(HEADER)
            .cast::<[usize; 2]>()
            .write([layout.size(), layout.align()]);
        user.cast()
    }
}

/// Frees what [`XMemAllocDefault`] returned, given the same attributes.
pub extern "system" fn XMemFreeDefault(address: *mut c_void, attributes: u64) {
    if address.is_null() {
        return;
    }
    // SAFETY: the address came from `XMemAlloc` with these attributes: a
    // whole reservation, or a header-prefixed block.
    unsafe {
        if attributes & ALLOC_32BIT != 0 {
            let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
            if VirtualQueryEx(
                GetCurrentProcess(),
                address,
                &mut mbi,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            ) != 0
                && mbi.RegionSize > 0
            {
                victory_gpu::title_memory_freeing(address as usize, mbi.RegionSize);
            }
            VirtualFree(address, 0, MEM_RELEASE);
            return;
        }
        let user = address.cast::<u8>();
        let [size, align] = user.sub(HEADER).cast::<[usize; 2]>().read();
        dealloc(
            user.sub(align),
            Layout::from_size_align_unchecked(size, align),
        );
    }
}

/// A retail console has no tools memory.
pub extern "system" fn XMemIsToolsMemAvailable() -> BOOL {
    0
}

// Physical pages: on the console, 64 KiB frames a title maps into its own
// reservations, at more than one address if it likes. Here each allocation
// is a pagefile-backed section, its pages are 64 KiB slices of it, and a
// mapping is a view of consecutive slices in place of a host placeholder.

const PAGE: usize = 64 << 10;

/// (KLBF Lab 63 / WillyboPeep) Standing view of a whole physical-page section
/// plus per-page committed bitmap (`VICTORY_SECTION_VIEWS=1`, on by default).
/// Avoids MapViewOfFile + VirtualAlloc + UnmapViewOfFile (global TLB shootdown)
/// on every DirectStorage streaming write.
struct Whole {
    size: usize,
    base: std::sync::atomic::AtomicUsize,
    committed: Box<[std::sync::atomic::AtomicU64]>,
}

const WHOLE_REFUSED: usize = 1;

struct Section(HANDLE, bool, Whole);

// SAFETY: a section handle may be used from any thread.
unsafe impl Send for Section {}
// SAFETY: as above.
unsafe impl Sync for Section {}

impl Drop for Section {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Memory::UnmapViewOfFile;
        let base = *self.2.base.get_mut();
        if base != 0 && base != WHOLE_REFUSED {
            // SAFETY: our own standing view of the whole section; no slices remain.
            unsafe { UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: base as *mut c_void }) };
        }
        victory_gpu::title_memory_section_gone(self.0 as usize);
        // SAFETY: our handle; views keep the section alive on their own.
        unsafe { CloseHandle(self.0) };
    }
}

fn section_views_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_SECTION_VIEWS").is_none_or(|v| v != "0"))
}

impl Section {
    fn new(handle: HANDLE, section_committed: bool, size: usize) -> Self {
        let words = size.div_ceil(PAGE).div_ceil(64);
        let init = if section_committed || on_wine() { u64::MAX } else { 0 };
        Section(
            handle,
            section_committed,
            Whole {
                size,
                base: std::sync::atomic::AtomicUsize::new(0),
                committed: (0..words).map(|_| std::sync::atomic::AtomicU64::new(init)).collect(),
            },
        )
    }

    fn is_committed(&self, index: usize, n: usize) -> bool {
        if self.1 || on_wine() {
            return true;
        }
        (index..index + n).all(|p| {
            self.2
                .committed
                .get(p / 64)
                .is_some_and(|w| w.load(Ordering::Acquire) & (1 << (p % 64)) != 0)
        })
    }

    fn mark_committed(&self, index: usize, n: usize) {
        for p in index..index + n {
            if let Some(word) = self.2.committed.get(p / 64) {
                word.fetch_or(1 << (p % 64), Ordering::Release);
            }
        }
    }

    fn whole(&self) -> Option<usize> {
        use windows_sys::Win32::System::Memory::{FILE_MAP_WRITE, MapViewOfFile, UnmapViewOfFile};
        match self.2.base.load(Ordering::Acquire) {
            0 => {}
            WHOLE_REFUSED => return None,
            base => return Some(base),
        }
        // SAFETY: standing view of a section we own until Section::drop.
        let at = unsafe { MapViewOfFile(self.0, FILE_MAP_WRITE, 0, 0, 0) };
        let made = if at.Value.is_null() {
            WHOLE_REFUSED
        } else {
            at.Value as usize
        };
        match self.2.base.compare_exchange(0, made, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => (made != WHOLE_REFUSED).then_some(made),
            Err(theirs) => {
                if made != WHOLE_REFUSED {
                    // SAFETY: redundant view nobody used.
                    unsafe { UnmapViewOfFile(at) };
                }
                (theirs != WHOLE_REFUSED).then_some(theirs)
            }
        }
    }

    fn standing(&self, index: usize, n: usize) -> Option<*mut u8> {
        if !section_views_on() {
            return None;
        }
        let (offset, len) = (index.checked_mul(PAGE)?, n.checked_mul(PAGE)?);
        if offset.checked_add(len)? > self.2.size {
            return None;
        }
        let base = self.whole()?;
        if !self.is_committed(index, n) {
            if !commit(base + offset, len, self.1) {
                return None;
            }
            self.mark_committed(index, n);
        }
        Some((base + offset) as *mut u8)
    }
}

/// Sub-section size in 64 KiB pages when `VICTORY_POOL_COMMIT=1` splits the
/// physical page pool into committed sections (`OpenExistingHeapFromFileMapping`)
/// so D3D12 can lazily open only the active 64 MiB chunks as unified zero-copy
/// heaps instead of pinning all 6.1 GiB at once.
fn pool_commit_chunk_pages() -> Option<usize> {
    static CHUNK: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *CHUNK.get_or_init(|| {
        if on_wine() {
            return None;
        }
        if !std::env::var_os("VICTORY_POOL_COMMIT").is_some_and(|v| v == "1") {
            return None;
        }
        let mb = std::env::var("VICTORY_POOL_COMMIT_MB")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(64)
            .clamp(4, 512);
        tracing::info!(
            chunk_mb = mb,
            "physical page pool uses committed sub-sections for zero-copy UMA (VICTORY_POOL_COMMIT=1)"
        );
        Some((mb << 20) / PAGE)
    })
}

/// Where a page lives, or what a view starts with: a section and a page
/// index in it.
#[derive(Clone)]
struct Slice {
    section: Arc<Section>,
    index: usize,
}

impl Slice {
    /// The key in `Physical::shown` of the page `k` pages after this one.
    fn key(&self, k: usize) -> (usize, usize) {
        (Arc::as_ptr(&self.section) as usize, self.index + k)
    }
}

/// Whether `XMemMapPhysicalPages` backs unaliased physical page runs with
/// virtual-contiguous `VirtualAlloc2(MEM_RESERVE | MEM_COMMIT | MEM_REPLACE_PLACEHOLDER | MEM_WRITE_WATCH)`
/// private memory (`VICTORY_UMA_POOL=1`), turning scattered 64 KiB physical
/// pages into single contiguous `MEM_PRIVATE` blocks that `OpenExistingHeapFromAddress1`
/// and hardware `GetWriteWatch` accept at 0% RAM waste!
/// Aliased pages (`map_alias` / multi-VA) automatically demote coherently to
/// `MapViewOfFile3` section views.
fn uma_pool() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var_os("VICTORY_UMA_POOL").is_some_and(|v| v == "1");
        if on {
            tracing::info!(
                "hybrid virtual-contiguous UMA physical page pool active (VICTORY_UMA_POOL=1)"
            );
        }
        on
    })
}

const MEM_COALESCE_PLACEHOLDERS: u32 = 0x1;

// ---------------------------------------------------------------------------
// Lock-free VEH barrier for transient remap / demote windows
// ---------------------------------------------------------------------------
//
// When `demote_private` converts a `Mapping::Private` block into `Mapping::Section`
// views, or when `unmap` splits a multi-page mapping to unmap only a sub-range
// (or `XMemMapPhysicalPages` / `map_alias` replaces pages in place), Windows
// requires `VirtualFree(MEM_PRESERVE_PLACEHOLDER)` / `UnmapViewOfFile2` before
// `MapViewOfFile3` / `VirtualAlloc2`. For ~50-200 µs, pages that the title
// expects to stay mapped are briefly `PAGE_READONLY` (while saving private
// bytes) or unmapped (`nothing mapped`).
//
// A 16-slot lock-free ring buffer records every active (`done_ms == u64::MAX`)
// and recently completed (`now_ms - done_ms <= 250`) preserved range so that
// even if several remaps happen back-to-back during Windows'
// `KiUserExceptionDispatcher` delivery lag, a concurrent title thread faulting
// on a preserved range waits for the remap to finish and resumes cleanly!

const REMAP_SLOTS: usize = 16;

struct RemapSlot {
    start: AtomicUsize,
    end: AtomicUsize,
    owner_tid: AtomicU32,
    /// `0` = unused, `u64::MAX` = active in flight, otherwise completion timestamp (ms).
    done_ms: AtomicU64,
}

impl RemapSlot {
    const fn new() -> Self {
        Self {
            start: AtomicUsize::new(0),
            end: AtomicUsize::new(0),
            owner_tid: AtomicU32::new(0),
            done_ms: AtomicU64::new(0),
        }
    }
}

static REMAP_RING: [RemapSlot; REMAP_SLOTS] = [const { RemapSlot::new() }; REMAP_SLOTS];
static REMAP_NEXT: AtomicUsize = AtomicUsize::new(0);
static REMAP_DEPTH: AtomicU32 = AtomicU32::new(0);
static REMAP_EPOCH: LazyLock<std::time::Instant> = LazyLock::new(std::time::Instant::now);
static REMAP_VEH_ONCE: Once = Once::new();
static REMAP_VEH_CAUGHT: AtomicU64 = AtomicU64::new(0);

fn install_remap_veh() {
    REMAP_VEH_ONCE.call_once(|| {
        let _ = *REMAP_EPOCH;
        // SAFETY: `remap_veh` is lock-free, allocation-free, and valid for the
        // lifetime of the process. Registered with `First = 1` so it runs before
        // `victory-loader`'s crash logger (`First = 0`) and `watch.rs`.
        unsafe {
            AddVectoredExceptionHandler(1, Some(remap_veh));
        }
    });
}

fn remap_now_ms() -> u64 {
    // Never return 0 (unused) or u64::MAX (in-flight sentinel).
    (REMAP_EPOCH.elapsed().as_millis() as u64).saturating_add(1)
}

unsafe extern "system" fn remap_veh(info: *mut EXCEPTION_POINTERS) -> i32 {
    if info.is_null() {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    // SAFETY: Windows passes valid `EXCEPTION_POINTERS` and `ExceptionRecord`.
    let rec = unsafe { &*(*info).ExceptionRecord };
    if rec.ExceptionCode != EXCEPTION_ACCESS_VIOLATION || rec.NumberParameters < 2 {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let addr = rec.ExceptionInformation[1];
    let tid = unsafe { GetCurrentThreadId() };

    // Pass 1: check if `addr` is inside any currently active remap slot.
    for slot in &REMAP_RING {
        let done = slot.done_ms.load(Ordering::Acquire);
        if done != u64::MAX {
            continue;
        }
        if slot.owner_tid.load(Ordering::Relaxed) == tid {
            continue;
        }
        let s = slot.start.load(Ordering::Relaxed);
        let e = slot.end.load(Ordering::Relaxed);
        if s < e && addr >= s && addr < e {
            let start_ms = remap_now_ms();
            let mut spins = 0u32;
            while slot.done_ms.load(Ordering::Acquire) == u64::MAX {
                if spins < 200 {
                    std::hint::spin_loop();
                    spins += 1;
                } else {
                    unsafe { SwitchToThread() };
                    if remap_now_ms().wrapping_sub(start_ms) > 5_000 {
                        return EXCEPTION_CONTINUE_SEARCH;
                    }
                }
            }
            REMAP_VEH_CAUGHT.fetch_add(1, Ordering::Relaxed);
            return EXCEPTION_CONTINUE_EXECUTION;
        }
    }

    // Pass 2: the remap may have finished while Windows was delivering the
    // exception to user mode (`KiUserExceptionDispatcher` lag), even if other
    // remaps ran afterwards. Check all 16 ring slots for a recent completion.
    let now_ms = remap_now_ms();
    let mut matched_recent = false;
    for slot in &REMAP_RING {
        let done = slot.done_ms.load(Ordering::Acquire);
        if done == 0 || done == u64::MAX || now_ms.wrapping_sub(done) > 250 {
            continue;
        }
        let s = slot.start.load(Ordering::Relaxed);
        let e = slot.end.load(Ordering::Relaxed);
        if s < e && addr >= s && addr < e {
            matched_recent = true;
            break;
        }
    }
    if matched_recent {
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { VirtualQuery(addr as *const c_void, &mut mbi, size_of_val(&mbi)) } != 0
            && mbi.State == MEM_COMMIT
        {
            let is_write = rec.ExceptionInformation[0] == 1;
            let prot = mbi.Protect & 0xff;
            let ok = if is_write {
                prot == PAGE_READWRITE || prot == PAGE_EXECUTE_READWRITE
            } else {
                prot == PAGE_READWRITE
                    || prot == PAGE_EXECUTE_READWRITE
                    || prot == PAGE_READONLY
            };
            if ok {
                REMAP_VEH_CAUGHT.fetch_add(1, Ordering::Relaxed);
                return EXCEPTION_CONTINUE_EXECUTION;
            }
        }
    }
    EXCEPTION_CONTINUE_SEARCH
}

struct RemapTransit {
    slots: [Option<usize>; 2],
    active: bool,
}

impl RemapTransit {
    /// Marks up to two preserved ranges `(k1, k2)` as undergoing a brief
    /// unmap-then-remap transition. Re-entrant when nested inside a wider
    /// enclosing `RemapTransit` (the outer guard already covers the superset).
    fn enter(k1: (usize, usize), k2: (usize, usize)) -> Self {
        let has1 = k1.0 < k1.1;
        let has2 = k2.0 < k2.1;
        if !has1 && !has2 {
            return Self {
                slots: [None, None],
                active: false,
            };
        }
        if REMAP_DEPTH.fetch_add(1, Ordering::Relaxed) > 0 {
            return Self {
                slots: [None, None],
                active: true,
            };
        }
        install_remap_veh();
        let tid = unsafe { GetCurrentThreadId() };
        let mut slots = [None, None];
        let push = |range: (usize, usize)| -> usize {
            let idx = REMAP_NEXT.fetch_add(1, Ordering::Relaxed) & (REMAP_SLOTS - 1);
            let slot = &REMAP_RING[idx];
            slot.owner_tid.store(tid, Ordering::Relaxed);
            slot.start.store(range.0, Ordering::Relaxed);
            slot.end.store(range.1, Ordering::Relaxed);
            slot.done_ms.store(u64::MAX, Ordering::Release);
            idx
        };
        if has1 {
            slots[0] = Some(push(k1));
        }
        if has2 {
            slots[1] = Some(push(k2));
        }
        Self {
            slots,
            active: true,
        }
    }
}

impl Drop for RemapTransit {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if REMAP_DEPTH.fetch_sub(1, Ordering::Relaxed) == 1 {
            let now = remap_now_ms();
            for idx in self.slots.into_iter().flatten() {
                REMAP_RING[idx].done_ms.store(now, Ordering::Release);
            }
        }
    }
}

enum Mapping {
    /// Classic `MapViewOfFile3` view of consecutive slices of one section.
    Section(usize, Slice),
    /// Virtual-contiguous `MEM_PRIVATE | MEM_COMMIT | MEM_WRITE_WATCH` block
    /// replacing a placeholder, holding `slices.len() * PAGE` bytes. Every
    /// slice here is mapped at this single virtual address only.
    Private(Vec<Slice>),
}

impl Mapping {
    fn len(&self) -> usize {
        match self {
            Mapping::Section(len, _) => *len,
            Mapping::Private(slices) => slices.len() * PAGE,
        }
    }

    fn slice_at(&self, page_offset: usize) -> Slice {
        match self {
            Mapping::Section(_, s) => Slice {
                index: s.index + page_offset,
                section: s.section.clone(),
            },
            Mapping::Private(slices) => slices[page_offset].clone(),
        }
    }

    fn is_private(&self) -> bool {
        matches!(self, Mapping::Private(_))
    }
}

#[derive(Default)]
struct Physical {
    next_page: usize,
    pages: HashMap<usize, Slice>,
    /// Mapped ranges by virtual base address.
    views: BTreeMap<usize, Mapping>,
    /// Where each page shows: by its section and its index there, the
    /// address of that page in every view that shows it now ([`PageRuns::written`];
    /// as upstream's c75fdba keeps it).
    shown: HashMap<(usize, usize), Vec<usize>>,
    /// Physical pages currently residing inside a `Mapping::Private` block:
    /// `slice.key(0) -> virtual address`.
    private_at: HashMap<(usize, usize), usize>,
    /// Physical pages whose latest non-zero bytes live in their backing
    /// `Section` (from DirectStorage writes while unmapped/section-backed, or
    /// saved when a `Mapping::Private` block was unmapped/demoted).
    in_section: std::collections::HashSet<(usize, usize)>,
}

static PHYSICAL: LazyLock<Mutex<Physical>> = LazyLock::new(|| {
    Mutex::new(Physical {
        // Page numbers are opaque to titles; 0 stays unused.
        next_page: 0x10_0000,
        ..Default::default()
    })
});

fn physical() -> MutexGuard<'static, Physical> {
    PHYSICAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// `count` is in and out (pages wanted, pages given); all or nothing here.
pub extern "system" fn XMemAllocatePhysicalPages(
    flags: u64,
    count: *mut usize,
    pages: *mut usize,
) -> BOOL {
    if count.is_null() || pages.is_null() {
        return 0;
    }
    // SAFETY: checked non-null.
    let n = unsafe { *count };
    let Some(size) = n.checked_mul(PAGE).filter(|&s| s > 0) else {
        return 0;
    };
    let chunk_pages = pool_commit_chunk_pages().unwrap_or(n);
    let mut chunks: Vec<(Arc<Section>, usize)> = Vec::with_capacity(n.div_ceil(chunk_pages));
    let mut left = n;
    while left > 0 {
        let take = left.min(chunk_pages);
        let chunk_bytes = take * PAGE;
        let want_commit = on_wine() || pool_commit_chunk_pages().is_some();
        // SAFETY: plain section creation.
        let (handle, committed) = unsafe {
            let mut h = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE | if want_commit { SEC_COMMIT } else { SEC_RESERVE },
                (chunk_bytes as u64 >> 32) as u32,
                chunk_bytes as u32,
                std::ptr::null(),
            );
            let mut committed = want_commit;
            if h.is_null() && want_commit && !on_wine() {
                // Fall back to SEC_RESERVE cleanly if system commit is tight.
                h = CreateFileMappingW(
                    INVALID_HANDLE_VALUE,
                    std::ptr::null(),
                    PAGE_READWRITE | SEC_RESERVE,
                    (chunk_bytes as u64 >> 32) as u32,
                    chunk_bytes as u32,
                    std::ptr::null(),
                );
                committed = false;
            }
            (h, committed)
        };
        if handle.is_null() {
            let error = std::io::Error::last_os_error();
            tracing::warn!(n, take, %error, "XMemAllocatePhysicalPages failed");
            // SAFETY: checked non-null.
            unsafe { count.write(0) };
            return 0;
        }
        victory_gpu::title_memory_section(handle as usize, chunk_bytes as u64, committed);
        chunks.push((Arc::new(Section::new(handle, committed, chunk_bytes)), take));
        left -= take;
    }
    tracing::debug!(
        n,
        sections = chunks.len(),
        flags = format_args!("{flags:#x}"),
        "XMemAllocatePhysicalPages"
    );
    let mut physical = physical();
    let first = physical.next_page;
    physical.next_page += n * FRAMES;
    let mut global_idx = 0usize;
    for (section, take) in chunks {
        for index in 0..take {
            let slice = Slice {
                section: section.clone(),
                index,
            };
            let number = first + global_idx * FRAMES;
            physical.pages.insert(number, slice);
            // SAFETY: the title passes room for `n` page numbers.
            unsafe { pages.add(global_idx).write(number) };
            global_idx += 1;
        }
    }
    PHYSICAL_BYTES.fetch_add(size, Ordering::Relaxed);
    1
}

/// Page numbers count 4 KiB frames, so a 64 KiB page's is a multiple of
/// 16: the title keeps them as 21-bit `number >> 4` and rebuilds
/// them with `<< 4` (128 GiB of pages at most).
const FRAMES: usize = PAGE >> 12;

/// Pages still mapped stay readable through their views until unmapped.
pub extern "system" fn XMemFreePhysicalPages(n: usize, pages: *const usize) -> BOOL {
    if pages.is_null() {
        return 0;
    }
    let mut physical = physical();
    let mut freed = 0usize;
    for i in 0..n {
        // SAFETY: the title passes `n` page numbers.
        if let Some(slice) = physical.pages.remove(&unsafe { *pages.add(i) }) {
            physical.in_section.remove(&slice.key(0));
            freed += PAGE;
        }
    }
    if freed > 0 {
        let _ = PHYSICAL_BYTES.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |cur| {
            Some(cur.saturating_sub(freed))
        });
    }
    1
}

/// A view of our own on consecutive slices of one section, committed and
/// writable, unmapped when dropped.
struct View {
    at: MEMORY_MAPPED_VIEW_ADDRESS,
    len: usize,
}

impl View {
    /// Maps `n` slices from `slice` on.
    fn map(slice: &Slice, n: usize) -> Option<View> {
        use windows_sys::Win32::System::Memory::{FILE_MAP_WRITE, MapViewOfFile};
        let (offset, len) = ((slice.index * PAGE) as u64, n * PAGE);
        // SAFETY: slices of a section we own; the view is ours until dropped.
        let at = unsafe {
            MapViewOfFile(
                slice.section.0,
                FILE_MAP_WRITE,
                (offset >> 32) as u32,
                offset as u32,
                len,
            )
        };
        if at.Value.is_null() {
            return None;
        }
        let view = View { at, len };
        if !commit(at.Value as usize, len, slice.section.1) {
            return None;
        }
        slice.section.mark_committed(slice.index, n);
        Some(view)
    }

    fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: the view holds `len` committed read-write bytes, ours
        // alone while `self` lives (other views of the slices only alias
        // them, as the console's do).
        unsafe { std::slice::from_raw_parts_mut(self.at.Value.cast(), self.len) }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Memory::UnmapViewOfFile;
        // SAFETY: our view.
        unsafe { UnmapViewOfFile(self.at) };
    }
}

/// The pages behind bytes `[offset, offset + len)` of `pages` (64 KiB
/// each), as runs of consecutive slices of one section: each run's first
/// slice and its length in pages. One look at the page table, which the
/// title's own mapping calls share with every DirectStorage worker. `None`
/// if a page is unknown or `pages` ends early.
fn runs(pages: &[usize], offset: usize, len: usize) -> Option<Vec<(Slice, usize)>> {
    let end = offset.checked_add(len)?;
    let pages = pages.get(offset / PAGE..end.div_ceil(PAGE))?;
    let physical = physical();
    let mut runs: Vec<(Slice, usize)> = Vec::new();
    for page in pages {
        let slice = physical.pages.get(page)?;
        match runs.last_mut() {
            Some((first, n))
                if Arc::ptr_eq(&first.section, &slice.section)
                    && slice.index == first.index + *n =>
            {
                *n += 1;
            }
            _ => runs.push((slice.clone(), 1)),
        }
    }
    Some(runs)
}

/// Copies `data` into physical pages, from `offset` bytes into the first,
/// whether or not they are mapped anywhere (DirectStorage destinations).
/// One view per run of consecutive pages, not one per page: each view
/// costs a map, a commit and an unmap (which flushes every core's TLB).
/// False if a page is unknown or the host fails.
#[cfg(test)]
pub(crate) fn write_physical(pages: &[usize], offset: usize, data: &[u8]) -> bool {
    resolve_pages(pages, offset, data.len()).is_some_and(|runs| runs.write(data))
}

/// A destination's physical pages resolved to their sections: the page
/// numbers are read once, and the sections are held, so the title may
/// change its page list or free the pages before the write without the
/// write going astray (DirectStorage resolves a request's list when the
/// title submits it).
pub(crate) struct PageRuns {
    runs: Vec<(Slice, usize)>,
    within: usize,
    len: usize,
}

/// Bytes `[offset, offset + len)` of `pages`, resolved; `None` if a page
/// is unknown.
pub(crate) fn resolve_pages(pages: &[usize], offset: usize, len: usize) -> Option<PageRuns> {
    Some(PageRuns {
        runs: runs(pages, offset, len)?,
        within: offset % PAGE,
        len,
    })
}

impl PageRuns {
    /// The bytes in place when they are one run of one section (and none of its
    /// pages currently reside in a `Mapping::Private` UMA block).
    pub(crate) fn window(&self) -> Option<PageWindow> {
        let [(slice, n)] = self.runs.as_slice() else {
            return None;
        };
        {
            let mut physical = physical();
            if !physical.private_at.is_empty()
                && (0..*n).any(|k| physical.private_at.contains_key(&slice.key(k)))
            {
                return None;
            }
            if uma_pool() {
                for k in 0..*n {
                    physical.in_section.insert(slice.key(k));
                }
            }
        }
        if let Some(at) = slice.section.standing(slice.index, *n) {
            return Some(PageWindow {
                _view: None,
                _section: Some(slice.section.clone()),
                at,
                within: self.within,
                len: self.len,
            });
        }
        let mut view = View::map(slice, *n)?;
        let at = view.bytes().as_mut_ptr();
        Some(PageWindow {
            _view: Some(view),
            _section: None,
            at,
            within: self.within,
            len: self.len,
        })
    }

    /// The first `len` bytes have been written through a view of our own:
    /// the GPU side is told at every title address that shows them
    /// (`victory_gpu::title_memory_written`), for the pages as resolved (a page the title has since unmapped or freed shows
    /// nowhere and is not told).
    pub(crate) fn written(&self, len: usize) {
        let mut pieces = Vec::new();
        {
            let physical = physical();
            let (mut within, mut left) = (self.within, len.min(self.len));
            for (slice, n) in &self.runs {
                for k in 0..*n {
                    if left == 0 {
                        break;
                    }
                    let bytes = (PAGE - within).min(left);
                    if let Some(addresses) = physical.shown.get(&slice.key(k)) {
                        pieces.extend(addresses.iter().map(|&at| (at + within, bytes)));
                    }
                    (within, left) = (0, left - bytes);
                }
            }
        }
        for (at, bytes) in merged(pieces) {
            victory_gpu::title_memory_written(at, bytes, true);
        }
    }

    /// Copies `data` (at most `len` bytes) in. False if the host fails.
    pub(crate) fn write(&self, data: &[u8]) -> bool {
        let (mut within, mut done) = (self.within, 0);
        for (slice, n) in &self.runs {
            if done == data.len() {
                break;
            }
            let has_private = {
                let p = physical();
                !p.private_at.is_empty() && (0..*n).any(|k| p.private_at.contains_key(&slice.key(k)))
            };
            if !has_private {
                let k = (n * PAGE - within).min(data.len() - done);
                if let Some(at) = slice.section.standing(slice.index, *n) {
                    // SAFETY: standing view of `n` committed pages in `slice.section`.
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            data.as_ptr().add(done),
                            at.add(within),
                            k,
                        );
                    }
                } else {
                    let Some(mut view) = View::map(slice, *n) else {
                        return false;
                    };
                    view.bytes()[within..within + k].copy_from_slice(&data[done..done + k]);
                }
                if uma_pool() {
                    let mut p = physical();
                    for idx in 0..*n {
                        p.in_section.insert(slice.key(idx));
                    }
                }
                (within, done) = (0, done + k);
            } else {
                // Page by page while holding `physical` so a private page cannot
                // be unmapped mid-write; writes directly into the private VA!
                let mut p = physical();
                for idx in 0..*n {
                    if done == data.len() {
                        break;
                    }
                    let take = (PAGE - within).min(data.len() - done);
                    let key = slice.key(idx);
                    if let Some(&va) = p.private_at.get(&key) {
                        // SAFETY: `va` is a committed `PAGE`-byte private mapping
                        // held live by `physical` lock.
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                data.as_ptr().add(done),
                                (va + within) as *mut u8,
                                take,
                            );
                        }
                    } else {
                        let page_slice = Slice {
                            section: slice.section.clone(),
                            index: slice.index + idx,
                        };
                        let Some(mut view) = View::map(&page_slice, 1) else {
                            return false;
                        };
                        view.bytes()[within..within + take].copy_from_slice(&data[done..done + take]);
                        p.in_section.insert(key);
                    }
                    (within, done) = (0, done + take);
                }
            }
        }
        true
    }
}

/// Bytes `[offset, offset + len)` of physical pages, writable in place
/// until dropped: DirectStorage decodes into it instead of a buffer it
/// would then copy.
pub(crate) struct PageWindow {
    _view: Option<View>,
    _section: Option<Arc<Section>>,
    at: *mut u8,
    within: usize,
    len: usize,
}

impl PageWindow {
    pub(crate) fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: `at` points to committed pages kept alive by `_view` or `_section`.
        unsafe { std::slice::from_raw_parts_mut(self.at.add(self.within), self.len) }
    }
}

/// A window on bytes `[offset, offset + len)` of `pages` when they are
/// consecutive pages of one allocation (how titles usually hand them out);
/// `None` otherwise (pages of several allocations, out of order, or
/// unknown, or a host failure), which `write_physical` then handles or
/// reports.
#[cfg(test)]
pub(crate) fn page_window(pages: &[usize], offset: usize, len: usize) -> Option<PageWindow> {
    resolve_pages(pages, offset, len)?.window()
}

/// The title addresses that show bytes `[offset, offset + len)` of `pages`
/// now, byte for byte, as runs (address, bytes) in address order: a write
/// to the pages through a view of our own ([`page_window`],
/// [`write_physical`]) lands there too, and nothing that watches those
/// addresses sees it (the GPU side's guards fault only on a write at the
/// guarded address itself). Empty if no view shows them. Byte for byte,
/// not by whole pages: the title packs several small textures into one
/// page and fills each with a request of its own.
#[cfg(test)]
pub(crate) fn shown_at(pages: &[usize], offset: usize, len: usize) -> Vec<(usize, usize)> {
    let Some(end) = offset.checked_add(len) else {
        return Vec::new();
    };
    let first = offset / PAGE;
    let Some(touched) = pages.get(first..end.div_ceil(PAGE)) else {
        return Vec::new();
    };
    let mut pieces: Vec<(usize, usize)> = Vec::new();
    {
        let physical = physical();
        for (i, page) in touched.iter().enumerate() {
            let Some(addresses) = physical.pages.get(page).and_then(|slice| physical.shown.get(&slice.key(0))) else {
                continue;
            };
            // The bytes of this page that the write covers.
            let lo = if i == 0 { offset % PAGE } else { 0 };
            let hi = (end - (first + i) * PAGE).min(PAGE);
            pieces.extend(addresses.iter().map(|&at| (at + lo, hi - lo)));
        }
    }
    merged(pieces)
}

/// `pieces` (address, bytes) sorted and joined where they touch.
fn merged(mut pieces: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    pieces.sort_unstable();
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (at, bytes) in pieces {
        match runs.last_mut() {
            Some((start, n)) if *start + *n == at => *n += bytes,
            _ => runs.push((at, bytes)),
        }
    }
    runs
}

/// Fast check whether a 64 KiB committed page at `va` is all zeros.
fn page_is_zero(va: usize) -> bool {
    // SAFETY: caller guarantees `[va, va + PAGE)` is a committed readable page.
    let words = unsafe { std::slice::from_raw_parts(va as *const u64, PAGE / 8) };
    words.iter().all(|&w| w == 0)
}

/// Saves a `Mapping::Private` page's contents back to its backing `Section`
/// only when non-zero (zero pages stay absent from `in_section`).
fn save_private_page(
    slice: &Slice,
    va: usize,
    in_section: &mut std::collections::HashSet<(usize, usize)>,
) {
    if page_is_zero(va) {
        in_section.remove(&slice.key(0));
        return;
    }
    if let Some(mut view) = View::map(slice, 1) {
        // SAFETY: `va` is a committed readable 64 KiB page.
        let src = unsafe { std::slice::from_raw_parts(va as *const u8, PAGE) };
        view.bytes().copy_from_slice(src);
        in_section.insert(slice.key(0));
    }
}

/// Maps `slices` at `[base, base + slices.len() * PAGE)` using classic
/// `MapViewOfFile3` section views (one view per consecutive section run,
/// falling back to page-by-page if a run crosses an `AllocationBase` boundary).
fn map_section_runs(physical: &mut Physical, base: usize, slices: &[Slice]) -> bool {
    let mut i = 0;
    while i < slices.len() {
        let first = &slices[i];
        let run = 1 + slices[i + 1..]
            .iter()
            .zip(1..)
            .take_while(|&(s, k)| Arc::ptr_eq(&s.section, &first.section) && s.index == first.index + k)
            .count();
        if !map(physical, base + i * PAGE, run * PAGE, first.clone()) {
            if run == 1 {
                return false;
            }
            for k in 0..run {
                if !map(physical, base + (i + k) * PAGE, PAGE, slices[i + k].clone()) {
                    return false;
                }
            }
        }
        i += run;
    }
    true
}

/// Guarantees `[base, base + len)` is ONE exact Windows placeholder ready for
/// `MEM_REPLACE_PLACEHOLDER` (`VirtualAlloc2` or `MapViewOfFile3`), even when
/// `[base, base + len)` currently spans multiple split placeholders whose first
/// or last placeholder extends outside `[base, base + len)`.
/// Returns `false` if `[base, base + len)` spans multiple distinct `AllocationBase`
/// reservations (which Windows cannot coalesce into one placeholder).
unsafe fn prepare_placeholder(base: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let end = base + len;
    // Fast path: single enclosing placeholder split directly to `[base, end)`.
    if unsafe { VirtualFree(base as *mut c_void, len, MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER) } != 0 {
        return true;
    }
    let process = unsafe { GetCurrentProcess() };
    let mut cur = base;
    let mut pieces = 0usize;
    let mut alloc_base: *mut c_void = std::ptr::null_mut();
    while cur < end {
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe {
            VirtualQueryEx(
                process,
                cur as *const c_void,
                &mut info,
                size_of_val(&info),
            )
        } == 0
        {
            return false;
        }
        if alloc_base.is_null() {
            alloc_base = info.AllocationBase;
        } else if info.AllocationBase != alloc_base {
            // Spans two separate VirtualAlloc2 reservations: cannot coalesce!
            return false;
        }
        let r_base = info.BaseAddress as usize;
        let r_end = r_base.saturating_add(info.RegionSize);
        if r_end <= cur {
            return false;
        }
        let cut_start = r_base.max(base);
        let cut_end = r_end.min(end);
        if cut_start > r_base || cut_end < r_end {
            unsafe {
                VirtualFree(
                    cut_start as *mut c_void,
                    cut_end - cut_start,
                    MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER,
                );
            }
        }
        pieces += 1;
        cur = cut_end;
    }
    if pieces > 1 {
        unsafe {
            VirtualFree(
                base as *mut c_void,
                len,
                MEM_RELEASE | MEM_COALESCE_PLACEHOLDERS,
            );
        }
    }
    true
}

/// Maps `slices` at `[base, base + slices.len() * PAGE)` as contiguous
/// `MEM_PRIVATE | MEM_COMMIT | MEM_WRITE_WATCH` block(s) replacing placeholders.
/// Automatically splits at `AllocationBase` boundaries if the range spans two
/// adjacent `XMemVirtualAlloc` reservations!
fn map_private(physical: &mut Physical, base: usize, slices: Vec<Slice>) -> bool {
    let total = slices.len();
    if total == 0 {
        return true;
    }
    if map_private_single(physical, base, slices.clone()) {
        return true;
    }
    // Slow path: split by `AllocationBase` (or per-page fallback) when the
    // requested range crosses two adjacent placeholder reservations!
    let process = unsafe { GetCurrentProcess() };
    let mut i = 0;
    while i < total {
        let start_va = base + i * PAGE;
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe {
            VirtualQueryEx(
                process,
                start_va as *const c_void,
                &mut info,
                size_of_val(&info),
            )
        } == 0
        {
            return false;
        }
        let ab = info.AllocationBase;
        let mut run = 1usize;
        while i + run < total {
            let mut next: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            let next_va = base + (i + run) * PAGE;
            if unsafe {
                VirtualQueryEx(
                    process,
                    next_va as *const c_void,
                    &mut next,
                    size_of_val(&next),
                )
            } == 0
                || next.AllocationBase != ab
            {
                break;
            }
            run += 1;
        }
        let sub = slices[i..i + run].to_vec();
        if !map_private_single(physical, start_va, sub.clone())
            && !map_section_runs(physical, start_va, &sub)
        {
            return false;
        }
        i += run;
    }
    true
}

fn map_private_single(physical: &mut Physical, base: usize, slices: Vec<Slice>) -> bool {
    let len = slices.len() * PAGE;
    if len == 0 {
        return true;
    }
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: splits/coalesces any fragmented placeholders within ONE AllocationBase
    // to `[base, base + len)`, then replaces with contiguous `MEM_PRIVATE | MEM_WRITE_WATCH`.
    let ptr = unsafe {
        if !prepare_placeholder(base, len) {
            return false;
        }
        VirtualAlloc2(
            process,
            base as *const c_void,
            len,
            MEM_RESERVE | MEM_COMMIT | MEM_REPLACE_PLACEHOLDER | MEM_WRITE_WATCH,
            PAGE_READWRITE,
            std::ptr::null_mut(),
            0,
        )
    };
    if ptr as usize != base {
        return false;
    }
    for (k, slice) in slices.iter().enumerate() {
        let key = slice.key(0);
        if physical.in_section.remove(&key)
            && let Some(view) = View::map(slice, 1)
        {
            // SAFETY: `[base + k*PAGE, ...)` is newly committed private RW memory.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    view.at.Value.cast::<u8>(),
                    (base + k * PAGE) as *mut u8,
                    PAGE,
                );
            }
        }
    }
    // Notify GPU side that `[base, base + len)` has new contents (private memory,
    // NOT a section view — so `inplace::cover` returns `Cover::None` and
    // `OpenExistingHeapFromAddress1` accepts it!).
    victory_gpu::title_memory_remapped(base, len);
    for (k, slice) in slices.iter().enumerate() {
        let key = slice.key(0);
        let addr = base + k * PAGE;
        physical.shown.entry(key).or_default().push(addr);
        physical.private_at.insert(key, addr);
    }
    physical.views.insert(base, Mapping::Private(slices));
    true
}

/// Demotes a `Mapping::Private` block at `at` back into `Mapping::Section`
/// views so one or more of its pages can be aliased at a second virtual address.
/// Caller MUST have called `title_memory_freeing_sync(at, len)` BEFORE taking
/// `title_memory_remapping`.
fn demote_private(physical: &mut Physical, at: usize) -> bool {
    let Some(Mapping::Private(slices)) = physical.views.remove(&at) else {
        return false;
    };
    let len = slices.len() * PAGE;
    // Protect concurrent title threads across `PAGE_READONLY` -> `VirtualFree` -> `map_section_runs`.
    let _transit = RemapTransit::enter((at, at + len), (0, 0));
    // Freeze concurrent writes before copying private bytes into the backing section
    // so no title write is lost between `save_private_page` and `VirtualFree`!
    let mut old_prot = 0u32;
    unsafe {
        VirtualProtect(at as *const c_void, len, PAGE_READONLY, &mut old_prot);
    }
    for (k, slice) in slices.iter().enumerate() {
        let key = slice.key(0);
        let addr = at + k * PAGE;
        save_private_page(slice, addr, &mut physical.in_section);
        physical.private_at.remove(&key);
        if let Some(addresses) = physical.shown.get_mut(&key) {
            addresses.retain(|&a| a != addr);
            if addresses.is_empty() {
                physical.shown.remove(&key);
            }
        }
    }
    victory_gpu::title_memory_remapped(at, len);
    // SAFETY: `[at, at + len)` is our private placeholder-replaced allocation.
    let freed = unsafe {
        VirtualFree(at as *mut c_void, len, MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER) != 0
    };
    if !freed {
        tracing::warn!(
            address = format_args!("{at:#x}"),
            len,
            error = %std::io::Error::last_os_error(),
            "demote_private VirtualFree failed (in-place heap still held?)"
        );
        return false;
    }
    map_section_runs(physical, at, &slices)
}

/// Finds the `Mapping::Private` block base address covering `va`, if any.
fn private_block_at(physical: &Physical, va: usize) -> Option<(usize, usize)> {
    physical
        .views
        .range(..=va)
        .next_back()
        .and_then(|(&base, m)| (m.is_private() && base + m.len() > va).then_some((base, m.len())))
}

/// Maps `n` pages at `address`, in place of what is mapped there; null
/// `pages` unmaps. Returns `address`, or null if a page or the host fails.
pub extern "system" fn XMemMapPhysicalPages(
    address: *mut c_void,
    n: usize,
    pages: *const usize,
) -> *mut c_void {
    let base = address as usize;
    let mut physical = physical();
    let (from, to) = reach(&physical, base, n * PAGE);
    let slices: Option<Vec<Slice>> = if pages.is_null() {
        None
    } else {
        // SAFETY: the title passes `n` page numbers.
        let page_nums = unsafe { std::slice::from_raw_parts(pages, n) };
        let mut out = Vec::with_capacity(n);
        for &p in page_nums {
            let Some(slice) = physical.pages.get(&p).cloned() else {
                tracing::warn!(page = p, "XMemMapPhysicalPages: unknown page");
                return std::ptr::null_mut();
            };
            out.push(slice);
        }
        Some(out)
    };
    // Collect any `Mapping::Private` blocks that must be unmapped or demoted,
    // and synchronously drain any overlapping in-place GPU heaps BEFORE taking
    // `title_memory_remapping` (strict deadlock-free lock order!).
    let mut demote_bases: Vec<(usize, usize)> = Vec::new();
    let mut aliased = false;
    if let Some(ref slices) = slices {
        let mut seen_local = std::collections::HashSet::with_capacity(slices.len());
        for slice in slices {
            let key = slice.key(0);
            if !seen_local.insert(key) {
                aliased = true;
            }
            if let Some(addrs) = physical.shown.get(&key) {
                // Any mapping outside `[base, base + n*PAGE)` means aliasing!
                if addrs.iter().any(|&a| a < base || a >= base + n * PAGE) {
                    aliased = true;
                    if let Some(&priv_va) = physical.private_at.get(&key)
                        && (priv_va < base || priv_va >= base + n * PAGE)
                        && let Some(block) = private_block_at(&physical, priv_va)
                        && !demote_bases.contains(&block)
                    {
                        demote_bases.push(block);
                    }
                }
            }
        }
    }
    let has_private_overlap = has_private_in(&physical, from, to);
    if has_private_overlap {
        victory_gpu::title_memory_freeing_sync(from, to - from);
    }
    for &(dbase, dlen) in &demote_bases {
        victory_gpu::title_memory_freeing_sync(dbase, dlen);
    }
    // Demote any aliased `Mapping::Private` blocks BEFORE `unmap` so a block
    // overlapping `[base, base + n*PAGE)` is demoted cleanly before splitting.
    for (dbase, dlen) in demote_bases {
        let _dc = victory_gpu::title_memory_remapping(dbase, dlen);
        demote_private(&mut physical, dbase);
    }
    let (from, to) = reach(&physical, base, n * PAGE);
    // Until the pages are mapped again: `unmap` takes whole views away and
    // maps what stays of them again.
    let _changing = victory_gpu::title_memory_remapping(from, to - from);
    let _transit = if slices.is_some() {
        RemapTransit::enter((from, to), (0, 0))
    } else {
        RemapTransit::enter((0, 0), (0, 0))
    };
    let was_mapped = has_any_view_in(&physical, base, base.saturating_add(n * PAGE));
    unmap(&mut physical, base, n * PAGE);
    let Some(slices) = slices else {
        return address;
    };
    // If overwriting a live mapping whose pages carry live data in their backing
    // section, map via `MapViewOfFile3` so concurrent readers never observe the
    // transient zero-filled window between `VirtualAlloc2` and `copy_nonoverlapping`.
    let has_live_data = was_mapped
        && slices
            .iter()
            .any(|s| physical.in_section.contains(&s.key(0)));
    if uma_pool()
        && !aliased
        && !has_live_data
        && map_private(&mut physical, base, slices.clone())
    {
        return address;
    }
    if !map_section_runs(&mut physical, base, &slices) {
        return std::ptr::null_mut();
    }
    address
}

fn has_any_view_in(physical: &Physical, from: usize, to: usize) -> bool {
    if from >= to {
        return false;
    }
    if let Some((&at, m)) = physical.views.range(..=from).next_back()
        && at + m.len() > from
    {
        return true;
    }
    physical
        .views
        .range(from.saturating_add(1)..to)
        .next()
        .is_some()
}

fn has_private_in(physical: &Physical, from: usize, to: usize) -> bool {
    if from >= to || physical.private_at.is_empty() {
        return false;
    }
    if let Some((&at, m)) = physical.views.range(..=from).next_back()
        && at + m.len() > from
        && m.is_private()
    {
        return true;
    }
    physical
        .views
        .range(from.saturating_add(1)..to)
        .any(|(_, m)| m.is_private())
}

/// One page at each address; null `pages`, or a page number 0, unmaps
/// (the title passes 0 for addresses to clear). Coalesces consecutive VA runs
/// into single `XMemMapPhysicalPages` calls so scattered pages at contiguous
/// VAs form ONE `MEM_PRIVATE` unified-memory block!
pub extern "system" fn XMemMapPhysicalPagesScatter(
    addresses: *const *mut c_void,
    n: usize,
    pages: *const usize,
) -> BOOL {
    if addresses.is_null() {
        return 0;
    }
    let mut i = 0;
    while i < n {
        // SAFETY: the title passes `n` addresses and, unless null, `n` pages.
        let addr0 = unsafe { *addresses.add(i) } as usize;
        let page0 = if pages.is_null() { 0 } else { unsafe { *pages.add(i) } };
        let is_unmap = page0 == 0;
        let mut run = 1usize;
        while i + run < n {
            let next_addr = unsafe { *addresses.add(i + run) } as usize;
            let next_page = if pages.is_null() { 0 } else { unsafe { *pages.add(i + run) } };
            if next_addr == addr0.wrapping_add(run * PAGE) && (next_page == 0) == is_unmap {
                run += 1;
            } else {
                break;
            }
        }
        let page_ptr = if is_unmap {
            std::ptr::null()
        } else {
            unsafe { pages.add(i) }
        };
        if XMemMapPhysicalPages(addr0 as *mut c_void, run, page_ptr).is_null() {
            return 0;
        }
        i += run;
    }
    1
}

/// Maps the physical pages shown at `[from, from + len)` at `to` as well,
/// in place of what `to` shows; `from` 0 unmaps. The GPU's page-mapping
/// copies (`victory_gpu`, queue `CopyPageMappingsBatchX`):
/// the title maps pages into a pool view, then has the GPU map them into its tiled
/// heap. Page by page; false if a source page is not mapped physical memory.
pub fn map_alias(to: u64, from: u64, len: u64) -> bool {
    let (to, from, len) = (to as usize, from as usize, len as usize);
    let mut physical = physical();
    let (start, end) = reach(&physical, to, len);
    // Collect any `Mapping::Private` blocks overlapping `from` that must be
    // demoted to section views so `from` and `to` share the same physical pages!
    let mut demote_from: Vec<(usize, usize)> = Vec::new();
    if from != 0 && len > 0 && !physical.private_at.is_empty() {
        let mut k = 0;
        while k < len {
            if let Some(block) = private_block_at(&physical, from + k) {
                if !demote_from.contains(&block) {
                    demote_from.push(block);
                }
                k = (block.0 + block.1).saturating_sub(from);
            } else {
                k += PAGE;
            }
        }
    }
    if has_private_in(&physical, start, end) {
        victory_gpu::title_memory_freeing_sync(start, end - start);
    }
    for &(dbase, dlen) in &demote_from {
        victory_gpu::title_memory_freeing_sync(dbase, dlen);
    }
    for (dbase, dlen) in demote_from {
        let _dc = victory_gpu::title_memory_remapping(dbase, dlen);
        demote_private(&mut physical, dbase);
    }
    let (start, end) = reach(&physical, to, len);
    // As in `XMemMapPhysicalPages`: no read in place meets the gap.
    let _changing = victory_gpu::title_memory_remapping(start, end - start);
    let _transit = if from != 0 && len > 0 {
        RemapTransit::enter((start, end), (0, 0))
    } else {
        RemapTransit::enter((0, 0), (0, 0))
    };
    unmap(&mut physical, to, len);
    for k in (0..len).step_by(PAGE) {
        if from == 0 {
            break;
        }
        let at = from + k;
        let Some(slice) = physical
            .views
            .range(..=at)
            .next_back()
            .filter(|(base, m)| at < *base + m.len())
            .map(|(base, m)| m.slice_at((at - base) / PAGE))
        else {
            return false;
        };
        if !map(&mut physical, to + k, PAGE, slice) {
            return false;
        }
    }
    true
}

/// Maps `slice` and the slices after it over `[base, base + len)`, which
/// must lie in one placeholder.
fn map(physical: &mut Physical, base: usize, len: usize, slice: Slice) -> bool {
    // SAFETY: splits the placeholder around the range (failing harmlessly
    // when the range is the whole placeholder), then maps the section
    // there; both act only on placeholders.
    let view = unsafe {
        if !prepare_placeholder(base, len) {
            return false;
        }
        MapViewOfFile3(
            slice.section.0,
            GetCurrentProcess(),
            base as *const c_void,
            (slice.index * PAGE) as u64,
            len,
            MEM_REPLACE_PLACEHOLDER,
            PAGE_READWRITE,
            std::ptr::null_mut(),
            0,
        )
    };
    if view.Value as usize != base {
        tracing::warn!(
            address = format_args!("{base:#x}"),
            len,
            error = %std::io::Error::last_os_error(),
            "mapping physical pages failed (not a placeholder?)"
        );
        return false;
    }
    if !(section_views_on() && slice.section.is_committed(slice.index, len / PAGE)) {
        if !commit(base, len, slice.section.1) {
            // SAFETY: the view just mapped, back to a placeholder.
            unsafe { UnmapViewOfFile2(GetCurrentProcess(), view, MEM_PRESERVE_PLACEHOLDER) };
            return false;
        }
        slice.section.mark_committed(slice.index, len / PAGE);
    }
    // New contents, read-write: the GPU side's copies no longer guard it.
    victory_gpu::title_memory_remapped(base, len);
    // What the range shows now, for the GPU side's in-place census.
    victory_gpu::title_memory_view(base, len, slice.section.0 as usize, (slice.index * PAGE) as u64);
    for k in 0..len.div_ceil(PAGE) {
        physical.shown.entry(slice.key(k)).or_default().push(base + k * PAGE);
    }
    physical.views.insert(base, Mapping::Section(len, slice));
    true
}

/// Commits the section pages a view shows (once per page, whichever view).
fn commit(base: usize, len: usize, section_committed: bool) -> bool {
    if section_committed || on_wine() {
        return true; // section was created with SEC_COMMIT
    }
    // SAFETY: `[base, base + len)` is a view of a reserved section.
    if !unsafe { VirtualAlloc(base as *const c_void, len, MEM_COMMIT, PAGE_READWRITE) }.is_null() {
        return true;
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_COMMITMENT_LIMIT as i32) {
        let mut info = PERFORMANCE_INFORMATION {
            cb: size_of::<PERFORMANCE_INFORMATION>() as u32,
            ..Default::default()
        };
        // SAFETY: the buffer and size describe a writable PERFORMANCE_INFORMATION.
        let queried = unsafe { K32GetPerformanceInfo(&mut info, info.cb) } != 0;
        victory_loader::log_crash(&commit_limit_message(len, &error, queried.then_some(&info)));
    } else {
        tracing::warn!(len, %error, "committing physical pages failed");
    }
    false
}

fn commit_limit_message(
    len: usize,
    error: &std::io::Error,
    info: Option<&PERFORMANCE_INFORMATION>,
) -> String {
    let mut message = format!(
        "VIctory: Windows commit limit reached while committing {len} bytes of physical pages: {error}.\n"
    );
    if let Some(info) = info {
        use std::fmt::Write;

        let committed = info.CommitTotal * info.PageSize / MIB;
        let limit = info.CommitLimit * info.PageSize / MIB;
        let _ = writeln!(
            message,
            "System commit: {committed} MiB of {limit} MiB in use."
        );
    }
    message.push_str(
        "Close other applications or increase the Windows paging file size, then retry.\n",
    );
    message
}

/// Start and end of the bytes whose mapping `unmap(physical, base, len)`
/// and mapping `[base, base + len)` again change: the range, and the whole
/// of the views reaching out of it (`unmap` takes them away and maps what
/// stays of them again).
fn reach(physical: &Physical, base: usize, len: usize) -> (usize, usize) {
    let end = base.saturating_add(len);
    if len == 0 {
        return (base, end); // `unmap` changes nothing
    }
    let mut reach = (base, end);
    // As `unmap` finds them: the view holding `base`, the last starting
    // inside (views do not overlap).
    if let Some((&at, m)) = physical.views.range(..=base).next_back()
        && at + m.len() > base
    {
        reach = (at, end.max(at + m.len()));
    }
    if let Some((&at, m)) = physical.views.range(base + 1..end).next_back() {
        reach.1 = reach.1.max(at + m.len());
    }
    reach
}

/// Returns `[base, base + len)` to placeholders; views reaching past it
/// are mapped again outside it.
fn unmap(physical: &mut Physical, base: usize, len: usize) {
    // No bytes: no view to return. The search below would still find the
    // view holding `base` and cut it in two there, and its range of views
    // starting inside would run backwards (which `BTreeMap` panics on).
    if len == 0 {
        return;
    }
    let end = base + len;
    // Views do not overlap: the one holding `base`, then those starting
    // inside (the GPU's page mappings make one view per page).
    let first = physical
        .views
        .range(..=base)
        .next_back()
        .filter(|(at, m)| *at + m.len() > base)
        .map(|(at, _)| *at);
    let hit: Vec<usize> = first
        .into_iter()
        .chain(physical.views.range(base + 1..end).map(|(at, _)| *at))
        .collect();
    for at in hit {
        let Some(mapping) = physical.views.remove(&at) else {
            continue;
        };
        match mapping {
            Mapping::Section(size, slice) => {
                let k1 = if at < base { (at, base) } else { (0, 0) };
                let k2 = if at + size > end { (end, at + size) } else { (0, 0) };
                let _transit = RemapTransit::enter(k1, k2);
                if uma_pool() && REMAP_DEPTH.load(Ordering::Relaxed) > 0 {
                    let mut old_prot = 0u32;
                    unsafe {
                        VirtualProtect(at as *const c_void, size, PAGE_READONLY, &mut old_prot);
                    }
                }
                for k in 0..size.div_ceil(PAGE) {
                    let key = slice.key(k);
                    if uma_pool() && !page_is_zero(at + k * PAGE) {
                        physical.in_section.insert(key);
                    }
                    if let Some(addresses) = physical.shown.get_mut(&key) {
                        addresses.retain(|&a| a != at + k * PAGE);
                        if addresses.is_empty() {
                            physical.shown.remove(&key);
                        }
                    }
                }
                victory_gpu::title_memory_remapped(at, size);
                // SAFETY: a view we mapped; it becomes a placeholder again.
                unsafe {
                    UnmapViewOfFile2(
                        GetCurrentProcess(),
                        MEMORY_MAPPED_VIEW_ADDRESS {
                            Value: at as *mut c_void,
                        },
                        MEM_PRESERVE_PLACEHOLDER,
                    )
                };
                if at < base {
                    map(physical, at, base - at, slice.clone());
                }
                if at + size > end {
                    let index = slice.index + (end - at) / PAGE;
                    map(physical, end, at + size - end, Slice { index, ..slice });
                }
            }
            Mapping::Private(slices) => {
                let size = slices.len() * PAGE;
                let k1 = if at < base { (at, base) } else { (0, 0) };
                let k2 = if at + size > end { (end, at + size) } else { (0, 0) };
                let _transit = RemapTransit::enter(k1, k2);
                if REMAP_DEPTH.load(Ordering::Relaxed) > 0 {
                    let mut old_prot = 0u32;
                    unsafe {
                        VirtualProtect(at as *const c_void, size, PAGE_READONLY, &mut old_prot);
                    }
                }
                for (k, slice) in slices.iter().enumerate() {
                    let key = slice.key(0);
                    let addr = at + k * PAGE;
                    save_private_page(slice, addr, &mut physical.in_section);
                    physical.private_at.remove(&key);
                    if let Some(addresses) = physical.shown.get_mut(&key) {
                        addresses.retain(|&a| a != addr);
                        if addresses.is_empty() {
                            physical.shown.remove(&key);
                        }
                    }
                }
                victory_gpu::title_memory_remapped(at, size);
                // SAFETY: private placeholder-replaced allocation; becomes a placeholder again.
                let freed = unsafe {
                    VirtualFree(
                        at as *mut c_void,
                        size,
                        MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER,
                    ) != 0
                };
                if !freed {
                    tracing::warn!(
                        address = format_args!("{at:#x}"),
                        size,
                        error = %std::io::Error::last_os_error(),
                        "unmap private block VirtualFree failed"
                    );
                }
                if at < base {
                    let prefix_n = (base - at) / PAGE;
                    map_section_runs(physical, at, &slices[..prefix_n]);
                }
                if at + size > end {
                    let suffix_start = (end - at) / PAGE;
                    map_section_runs(physical, end, &slices[suffix_start..]);
                }
            }
        }
    }
}

/// The oldest layout we know; newer titles pass other flag values (Minecraft
/// 0x20000001) and read the same leading fields.
#[repr(C)]
#[derive(Debug, Default)]
pub struct WorkingSet {
    pub title_limit: usize,
    pub title_used: usize,
    pub tools_limit: usize,
    pub tools_used: usize,
    pub stack_used: u32,
    pub physical_pages: u32,
    pub gpu_optimal_limit: usize,
    pub gpu_optimal_used: usize,
    pub tools_gpu_optimal_limit: usize,
    pub tools_gpu_optimal_used: usize,
    pub kernel_pool_used: u32,
    pub kernel_pool_limit: u32,
}

/// The title's budget and our process's commit as what it uses. No tools
/// memory: a retail console.
// shortcut: all fields are filled whatever the flags ask for, and use is
// not split by bandwidth class (GPU-optimal use reads 0); track
// allocations per class if a title budgets from that field.
pub extern "system" fn XMemGetWorkingSetStatistics(_which: u32, out: *mut WorkingSet) -> BOOL {
    if out.is_null() {
        return 0;
    }
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    // SAFETY: our own process and a sized structure.
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    let (gpu_optimal, standard) = victory_gpu::console::current().title_memory();
    let limit = (gpu_optimal + standard).saturating_sub(TRANSFERRED.load(Ordering::Relaxed));
    let phys = PHYSICAL_BYTES.load(Ordering::Relaxed);
    let title_used = if phys > 0 { phys.min(counters.PagefileUsage) } else { counters.PagefileUsage };
    let stats = WorkingSet {
        title_limit: limit,
        title_used,
        gpu_optimal_limit: gpu_optimal,
        ..Default::default()
    };
    // What the title is told, logged at the first asking and at every
    // 1,024th: "used" is all the process has committed, the layer's own
    // memory and the card's included (27 GB in run 60's gameplay against a
    // limit of 13.5 GB). A title that budgets its streaming from these two
    // figures believes it is far over its limit.
    // shortcut: the figure is not yet the title's own memory alone (its
    // heaps and the pages it mapped); count those if this line shows the
    // title asking.
    static ASKED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let asked = ASKED.fetch_add(1, Ordering::Relaxed);
    if asked.is_multiple_of(1024) {
        tracing::info!(
            asked = asked + 1,
            which = _which,
            limit_mb = stats.title_limit >> 20,
            used_mb = stats.title_used >> 20,
            "the title asks how much memory it uses"
        );
    }
    // SAFETY: checked non-null; the title passes room for the structure.
    unsafe { out.write(stats) };
    1
}

/// Hands title memory (standard, in 2 MiB pages) to the system. The other
/// transfers involve tools memory, which a retail console has none of.
pub extern "system" fn XMemTransferMemory(kind: u32, size: usize) -> BOOL {
    const TITLE_TO_SYSTEM: u32 = 0;
    tracing::info!(kind, size, "XMemTransferMemory");
    if size == 0 {
        return 1;
    }
    let Some(size) = size.checked_next_multiple_of(2 * MIB) else {
        return 0;
    };
    let standard = victory_gpu::console::current().title_memory().1;
    let moved = kind == TITLE_TO_SYSTEM
        && TRANSFERRED
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |t| {
                t.checked_add(size).filter(|&t| t <= standard)
            })
            .is_ok();
    moved.into()
}

pub(crate) fn module() -> HleModule {
    HleModule {
        name: "xmem",
        dlls: &["xmem.dll"],
        exports: vec![
            hle_export!(XMemAlloc),
            hle_export!(XMemFree),
            hle_export!(XMemAllocDefault),
            hle_export!(XMemFreeDefault),
            hle_export!(XMemIsToolsMemAvailable),
            hle_export!(XMemAllocatePhysicalPages),
            hle_export!(XMemFreePhysicalPages),
            hle_export!(XMemMapPhysicalPages),
            hle_export!(XMemMapPhysicalPagesScatter),
            hle_export!(XMemVirtualAlloc),
            hle_export!(XMemVirtualQuery),
            hle_export!(XMemGetWorkingSetStatistics),
            hle_export!(XMemTransferMemory),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(address: *const c_void) -> BasicInformation {
        // SAFETY: plain data; zero is a valid value.
        let mut info: BasicInformation = unsafe { std::mem::zeroed() };
        // SAFETY: a pseudo handle needs no closing.
        let process = unsafe { GetCurrentProcess() };
        assert_eq!(XMemVirtualQuery(process, address, &mut info), 1);
        info
    }

    #[test]
    fn alloc_honours_alignment_and_frees() {
        for (field, align) in [(0, 16), (4, 16), (0xc, 4096), (0x10, 64 * 1024)] {
            let attributes = field << ALIGNMENT_SHIFT | 1 << 29; // a graphics type
            let p = XMemAlloc(100, attributes);
            assert!(!p.is_null());
            assert_eq!(p as usize % align, 0, "field {field:#x}");
            // SAFETY: 100 bytes were allocated.
            unsafe { p.cast::<u8>().write_bytes(0xab, 100) };
            XMemFree(p, attributes);
        }
        let p = XMemAlloc(4096, ALLOC_32BIT);
        assert!(!p.is_null() && (p as usize) < 1 << 32);
        XMemFree(p, ALLOC_32BIT);
    }

    #[test]
    fn virtual_alloc_meets_console_constraints() {
        const GPU_READ_WRITE: u32 = 0x2000;
        let p = XMemVirtualAlloc(
            std::ptr::null_mut(),
            4 * MIB,
            PAGES_2M | MEM_RESERVE | MEM_COMMIT,
            GRAPHICS | 7 << 12, // 40-bit addresses
            PAGE_READWRITE | GPU_READ_WRITE,
        );
        assert!(!p.is_null());
        assert_eq!(p as usize % (2 * MIB), 0);
        assert!((p as usize) < 1 << 40);
        let info = query(p);
        assert_eq!(info.host.State, MEM_COMMIT);
        assert_eq!(info.host.Protect, PAGE_READWRITE);
        assert_eq!(info.flags & GRAPHICS, GRAPHICS);
        // Write-watched, for the GPU side's copies.
        let (mut found, mut count, mut granularity) = ([0usize; 4], 4, 0);
        // SAFETY: room for `count` addresses.
        let watched = unsafe {
            windows_sys::Win32::System::Memory::GetWriteWatch(
                0,
                p,
                4 * MIB,
                found.as_mut_ptr().cast(),
                &mut count,
                &mut granularity,
            )
        };
        assert_eq!(watched, 0);
        // SAFETY: our own reservation.
        assert_ne!(unsafe { VirtualFree(p, 0, MEM_RELEASE) }, 0);
        assert_eq!(query(p).flags, 0);
        // No address field: above the 32-bit range and below 2^40, where
        // the console keeps it.
        let p = XMemVirtualAlloc(
            std::ptr::null_mut(),
            16 << 30,
            MEM_RESERVE,
            GRAPHICS,
            PAGE_NOACCESS,
        );
        assert!(!p.is_null() && (p as usize) >= 1 << 32, "{p:p}");
        assert!((p as usize) + (16 << 30) <= 1 << 40, "{p:p}");
        // SAFETY: our own reservation.
        assert_ne!(unsafe { VirtualFree(p, 0, MEM_RELEASE) }, 0);
    }

    #[test]
    fn transfers_shrink_the_title_budget() {
        let mut before = WorkingSet::default();
        assert_eq!(XMemGetWorkingSetStatistics(0x2000_0001, &mut before), 1);
        assert!(before.title_used > 0);
        assert_eq!(XMemTransferMemory(0, 128 * MIB - 1), 1);
        assert_eq!(XMemTransferMemory(2, MIB), 0, "no tools memory");
        let standard = victory_gpu::console::current().title_memory().1;
        assert_eq!(XMemTransferMemory(0, standard), 0, "over budget");
        let mut after = WorkingSet::default();
        XMemGetWorkingSetStatistics(0x2000_0001, &mut after);
        assert_eq!(before.title_limit - after.title_limit, 128 * MIB);
    }

    #[test]
    fn statistics_have_the_console_layout() {
        assert_eq!(size_of::<WorkingSet>(), 0x50);
        assert_eq!(size_of::<BasicInformation>(), 0x40);
        assert_eq!(std::mem::offset_of!(WorkingSet, gpu_optimal_limit), 0x28);
    }

    #[test]
    fn commit_limit_report_includes_request_and_system_commit() {
        let error = std::io::Error::from_raw_os_error(ERROR_COMMITMENT_LIMIT as i32);
        let info = PERFORMANCE_INFORMATION {
            CommitTotal: 512,
            CommitLimit: 768,
            PageSize: 4096,
            ..Default::default()
        };
        let report = commit_limit_message(4 * MIB, &error, Some(&info));
        assert!(report.contains("4194304 bytes"));
        assert!(report.contains("2 MiB of 3 MiB in use"));
        let fallback = commit_limit_message(4 * MIB, &error, None);
        for report in [&report, &fallback] {
            assert!(report.contains("os error 1455"));
            assert!(report.contains("Windows paging file size"));
        }
        assert!(!fallback.contains("System commit:"));
    }

    #[test]
    fn physical_pages_map_alias_and_unmap() {
        let mut pages = [0usize; 3];
        let mut n = 3;
        assert_eq!(XMemAllocatePhysicalPages(0, &mut n, pages.as_mut_ptr()), 1);
        assert_eq!(n, 3);
        // Packed into 21 bits as `number >> 4`, as the title keeps them.
        for p in pages {
            assert_eq!((p >> 4 & 0x1f_ffff) << 4, p);
        }
        // Reservations of 64 KiB pages, as titles make them for mapping.
        let reserve = || {
            XMemVirtualAlloc(
                std::ptr::null_mut(),
                3 * PAGE,
                MEM_RESERVE | PAGES_64K,
                0,
                PAGE_READWRITE,
            )
        };
        let (a, b) = (reserve(), reserve());
        assert!(!a.is_null() && !b.is_null());
        let at = |p: *mut c_void, page: usize| p.wrapping_byte_add(page * PAGE);
        assert_eq!(XMemMapPhysicalPages(a, 3, pages.as_ptr()), a);

        // The same page at a second address shows the same bytes.
        assert_eq!(
            XMemMapPhysicalPages(at(b, 1), 1, pages[2..].as_ptr()),
            at(b, 1)
        );
        // SAFETY: both pages are mapped read-write.
        unsafe {
            at(a, 2).cast::<u8>().write(7);
            assert_eq!(at(b, 1).cast::<u8>().read(), 7);
            a.cast::<u8>().write(1);
        }
        // Writes into pages (DirectStorage's) show through their views.
        assert!(write_physical(&pages[2..], 1, &[9]));
        // SAFETY: mapped read-write.
        assert_eq!(unsafe { at(b, 1).cast::<u8>().add(1).read() }, 9);

        // Unmapping the middle page keeps its neighbours.
        assert_eq!(
            XMemMapPhysicalPages(at(a, 1), 1, std::ptr::null()),
            at(a, 1)
        );
        assert_eq!(query(at(a, 1)).host.State, MEM_RESERVE);
        assert_eq!(query(a).host.State, MEM_COMMIT);
        assert_eq!(query(at(a, 2)).host.State, MEM_COMMIT);
        // SAFETY: still mapped.
        assert_eq!(unsafe { at(a, 2).cast::<u8>().read() }, 7);

        // A scattered page lands in the hole.
        let addresses = [at(a, 1)];
        assert_eq!(
            XMemMapPhysicalPagesScatter(addresses.as_ptr(), 1, pages.as_ptr()),
            1
        );
        // SAFETY: page 0 is now mapped at both `a` and `a + PAGE`.
        assert_eq!(unsafe { at(a, 1).cast::<u8>().read() }, 1);
        // Page number 0 unmaps.
        assert_eq!(
            XMemMapPhysicalPagesScatter(addresses.as_ptr(), 1, [0usize].as_ptr()),
            1
        );
        assert_eq!(query(at(a, 1)).host.State, MEM_RESERVE);
        // The GPU's page-mapping copy: what `a + 2 pages` shows, at `b`.
        assert!(map_alias(b as u64, at(a, 2) as u64, PAGE as u64));
        // SAFETY: mapped read-write.
        assert_eq!(unsafe { b.cast::<u8>().read() }, 7);
        assert!(map_alias(b as u64, 0, PAGE as u64));
        assert_eq!(query(b).host.State, MEM_RESERVE);
        assert!(!map_alias(b as u64, at(a, 1) as u64, PAGE as u64));
        assert_eq!(XMemFreePhysicalPages(3, pages.as_ptr()), 1);
        assert_eq!(XMemIsToolsMemAvailable(), 0);
    }

    /// A thread that reads a mapped page in place, as the GPU side reads a
    /// texture, never meets the moment in which the runtime has taken the
    /// page's view away to map it again: mapping the same pages at the same
    /// address unmaps the view and maps a new one, and without the lock the
    /// reader faults on a page "reserved, never committed" (the crashes of
    /// runs 71 and 74).
    #[test]
    fn a_read_in_place_never_meets_the_gap_of_a_remapping() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let mut pages = [0usize; 3];
        let mut n = 3;
        assert_eq!(XMemAllocatePhysicalPages(0, &mut n, pages.as_mut_ptr()), 1);
        let a = XMemVirtualAlloc(
            std::ptr::null_mut(),
            3 * PAGE,
            MEM_RESERVE | PAGES_64K,
            0,
            PAGE_READWRITE,
        );
        assert!(!a.is_null());
        assert_eq!(XMemMapPhysicalPages(a, 3, pages.as_ptr()), a);
        // SAFETY: mapped read-write.
        unsafe { a.cast::<u8>().write(1) };
        let first = a as usize;
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let reader = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut reads = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let _reading = victory_gpu::title_memory_reading(first, 1);
                    // SAFETY: the page is mapped whenever no change of
                    // mapping is under way, and none is while `_reading`
                    // is held.
                    assert_eq!(unsafe { (first as *const u8).read_volatile() }, 1);
                    reads += 1;
                }
                reads
            })
        };
        for _ in 0..2000 {
            // The same pages at the same address: the view goes and comes
            // back, with the same bytes.
            assert_eq!(XMemMapPhysicalPages(a, 3, pages.as_ptr()), a);
        }
        stop.store(true, Ordering::Relaxed);
        assert!(reader.join().expect("the reader never faulted") > 0);
        assert_eq!(XMemMapPhysicalPages(a, 3, std::ptr::null()), a);
        assert_eq!(XMemFreePhysicalPages(3, pages.as_ptr()), 1);
    }

    #[test]
    fn mapping_no_pages_changes_nothing() {
        let pages = allocate(2);
        let a = XMemVirtualAlloc(
            std::ptr::null_mut(),
            2 * PAGE,
            MEM_RESERVE | PAGES_64K,
            0,
            PAGE_READWRITE,
        );
        assert!(!a.is_null());
        assert_eq!(XMemMapPhysicalPages(a, 2, pages.as_ptr()), a);
        // SAFETY: both pages are mapped read-write.
        unsafe { a.cast::<u8>().write(5) };
        let second = a.wrapping_byte_add(PAGE);

        // A count of 0 at the start of a view, inside it and past it: with
        // pages, without (an unmap), and as the GPU's page-mapping copy.
        for at in [a, second, a.wrapping_byte_add(2 * PAGE)] {
            assert_eq!(XMemMapPhysicalPages(at, 0, pages.as_ptr()), at);
            assert_eq!(XMemMapPhysicalPages(at, 0, std::ptr::null()), at);
            assert!(map_alias(at as u64, a as u64, 0));
            assert!(map_alias(at as u64, 0, 0));
        }

        // The view is still the one view it was.
        let view = physical().views.get(&(a as usize)).map(|v| v.len());
        assert_eq!(view, Some(2 * PAGE));
        assert_eq!(query(a).host.State, MEM_COMMIT);
        assert_eq!(query(second).host.State, MEM_COMMIT);
        // SAFETY: still mapped.
        assert_eq!(unsafe { a.cast::<u8>().read() }, 5);

        // A count of 1 still unmaps that page alone.
        assert_eq!(XMemMapPhysicalPages(second, 1, std::ptr::null()), second);
        assert_eq!(query(second).host.State, MEM_RESERVE);
        assert_eq!(query(a).host.State, MEM_COMMIT);
        assert_eq!(XMemMapPhysicalPages(a, 1, std::ptr::null()), a);
        assert_eq!(XMemFreePhysicalPages(2, pages.as_ptr()), 1);
    }

    /// `n` pages of one new allocation.
    fn allocate(n: usize) -> Vec<usize> {
        let (mut pages, mut given) = (vec![0; n], n);
        assert_eq!(
            XMemAllocatePhysicalPages(0, &mut given, pages.as_mut_ptr()),
            1
        );
        assert_eq!(given, n);
        pages
    }

    /// The bytes of one page, through a view of our own.
    fn page_bytes(page: usize) -> Vec<u8> {
        let slice = physical().pages.get(&page).cloned().expect("known page");
        View::map(&slice, 1).expect("mapped").bytes().to_vec()
    }

    #[test]
    fn runs_group_consecutive_pages_of_one_allocation() {
        let (a, b) = (allocate(3), allocate(1));
        let lengths = |pages: &[usize], offset, len| {
            runs(pages, offset, len).map(|r| r.iter().map(|&(_, n)| n).collect::<Vec<_>>())
        };
        assert_eq!(lengths(&a, 0, 3 * PAGE), Some(vec![3]));
        // Only the pages the bytes reach.
        assert_eq!(lengths(&a, PAGE + 5, 10), Some(vec![1]));
        assert_eq!(lengths(&a, PAGE - 1, 2), Some(vec![2]));
        assert_eq!(lengths(&a, 0, 0), Some(vec![]));
        // Out of order, or across allocations: one run each.
        assert_eq!(
            lengths(&[a[0], a[2], a[1]], 0, 3 * PAGE),
            Some(vec![1, 1, 1])
        );
        assert_eq!(lengths(&[a[1], a[2], b[0]], 0, 3 * PAGE), Some(vec![2, 1]));
        // A slice that follows on in another section starts a run.
        assert_eq!(lengths(&[b[0], a[1]], 0, 2 * PAGE), Some(vec![1, 1]));
        // Too few pages for the bytes, or an unknown one.
        assert_eq!(lengths(&a, 0, 3 * PAGE + 1), None);
        assert_eq!(lengths(&[a[0], 0x10], 0, 2 * PAGE), None);
        assert_eq!(lengths(&a, usize::MAX, 2), None);
        assert_eq!(XMemFreePhysicalPages(3, a.as_ptr()), 1);
        assert_eq!(XMemFreePhysicalPages(1, b.as_ptr()), 1);
    }

    #[test]
    fn writes_cross_runs_and_allocations() {
        let (a, b) = (allocate(2), allocate(1));
        let pages = [a[0], a[1], b[0]];
        let data: Vec<u8> = (0..2 * PAGE + 64).map(|i| (i * 7 % 251) as u8).collect();
        let offset = PAGE - 32;
        // Past the third page: the second run is too short.
        assert!(!write_physical(&pages, offset, &data));
        assert!(write_physical(&pages, offset, &data[..2 * PAGE]));
        let seen: Vec<u8> = pages.iter().flat_map(|&p| page_bytes(p)).collect();
        assert!(seen[..offset].iter().all(|&b| b == 0), "before the offset");
        assert_eq!(seen[offset..offset + 2 * PAGE], data[..2 * PAGE]);
        assert!(seen[offset + 2 * PAGE..].iter().all(|&b| b == 0), "after");
        assert!(
            !write_physical(&[a[0], 0x10], 0, &data[..PAGE + 1]),
            "unknown page"
        );
        assert_eq!(XMemFreePhysicalPages(2, a.as_ptr()), 1);
        assert_eq!(XMemFreePhysicalPages(1, b.as_ptr()), 1);
    }

    /// Pages say where the title has them mapped, byte for byte (after
    /// fork PR #571's test; its runs are whole pages).
    #[test]
    fn pages_say_where_they_are_mapped() {
        let pages = allocate(3);
        let reserve = |n: usize| {
            let at = XMemVirtualAlloc(std::ptr::null_mut(), n * PAGE, MEM_RESERVE | PAGES_64K, 0, PAGE_READWRITE);
            assert!(!at.is_null());
            at
        };
        // `b` keeps a page unmapped at either end, so that no run at `b`
        // ever touches one at `a`, wherever the host puts the two.
        let (a, b) = (reserve(3), reserve(4));
        assert!(shown_at(&pages, 0, 3 * PAGE).is_empty(), "mapped nowhere");
        // All three at `a`; the last two at `b`, from its second page on.
        assert_eq!(XMemMapPhysicalPages(a, 3, pages.as_ptr()), a);
        let second = b.wrapping_byte_add(PAGE);
        assert_eq!(XMemMapPhysicalPages(second, 2, pages[1..].as_ptr()), second);
        let (a, second) = (a as usize, second as usize);
        let sorted = |mut runs: Vec<(usize, usize)>| {
            runs.sort_unstable();
            runs
        };
        assert_eq!(shown_at(&pages, 0, PAGE), vec![(a, PAGE)]);
        // A request for part of a page names those bytes alone.
        assert_eq!(shown_at(&pages, 0x4000, 0x3000), vec![(a + 0x4000, 0x3000)]);
        // Across two pages, shown at two addresses each: one run at each.
        assert_eq!(
            shown_at(&pages, PAGE + 1, PAGE),
            sorted(vec![(a + PAGE + 1, PAGE), (second + 1, PAGE)])
        );
        // A page unmapped in the middle of a view: the rest still shows.
        let middle = (a + PAGE) as *mut c_void;
        assert_eq!(XMemMapPhysicalPages(middle, 1, std::ptr::null()), middle);
        assert_eq!(
            shown_at(&pages, 0, 3 * PAGE),
            sorted(vec![(a, PAGE), (a + 2 * PAGE, PAGE), (second, 2 * PAGE)])
        );
        // Too few pages for the bytes: nothing (the write fails too).
        assert!(shown_at(&pages, 0, 3 * PAGE + 1).is_empty());
        for (at, n) in [(a, 3), (second, 2)] {
            let at = at as *mut c_void;
            assert_eq!(XMemMapPhysicalPages(at, n, std::ptr::null()), at);
        }
        assert!(shown_at(&pages, 0, 3 * PAGE).is_empty());
        assert_eq!(XMemFreePhysicalPages(3, pages.as_ptr()), 1);
    }

    #[test]
    fn page_windows_cover_one_run_in_place() {
        let (a, b) = (allocate(3), allocate(1));
        let mut window = page_window(&a, PAGE + 3, PAGE).expect("one run");
        assert_eq!(window.bytes().len(), PAGE);
        window.bytes().fill(0x5a);
        drop(window);
        let (second, third) = (page_bytes(a[1]), page_bytes(a[2]));
        assert!(second[..3].iter().all(|&b| b == 0));
        assert!(second[3..].iter().all(|&b| b == 0x5a));
        assert!(third[..3].iter().all(|&b| b == 0x5a));
        assert!(third[3..].iter().all(|&b| b == 0));
        // Several runs, or an unknown page: no window.
        assert!(page_window(&[a[0], b[0]], 0, PAGE + 1).is_none());
        assert!(page_window(&[a[1], a[0]], 0, PAGE + 1).is_none());
        assert!(page_window(&[0x10], 0, 1).is_none());
        // One page of several is still one run.
        assert!(page_window(&[a[0], b[0]], 0, PAGE).is_some());
        assert_eq!(XMemFreePhysicalPages(3, a.as_ptr()), 1);
        assert_eq!(XMemFreePhysicalPages(1, b.as_ptr()), 1);
    }

    #[test]
    fn uma_pool_scattered_private_alias_and_remap() {
        use windows_sys::Win32::System::Memory::{MEM_PRIVATE, VirtualQuery};
        // Allocate two separate physical pools and interleave their pages (100% scattered!).
        let a = allocate(4);
        let b = allocate(4);
        let scattered = [a[3], b[1], a[0], b[2]];
        let va = XMemVirtualAlloc(std::ptr::null_mut(), 4 * PAGE, MEM_RESERVE | PAGES_64K, 0, PAGE_READWRITE);
        let va2 = XMemVirtualAlloc(std::ptr::null_mut(), 4 * PAGE, MEM_RESERVE | PAGES_64K, 0, PAGE_READWRITE);
        assert!(!va.is_null() && !va2.is_null());

        // Map scattered pages directly as ONE contiguous MEM_PRIVATE | MEM_WRITE_WATCH block.
        {
            let mut p = physical();
            let slices: Vec<Slice> = scattered.iter().map(|n| p.pages[n].clone()).collect();
            assert!(map_private(&mut p, va as usize, slices));
        }
        // Verify VirtualQuery reports ONE single contiguous MEM_PRIVATE MEM_COMMIT block of 4*PAGE!
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        assert_ne!(unsafe { VirtualQuery(va, &mut info, size_of_val(&info)) }, 0);
        assert_eq!(info.Type, MEM_PRIVATE);
        assert_eq!(info.State, MEM_COMMIT);
        assert!(info.RegionSize >= 4 * PAGE);

        // Write distinct markers into each scattered page via CPU.
        unsafe {
            for i in 0..4 {
                va.cast::<u8>().add(i * PAGE).write((0xA0 + i) as u8);
            }
        }
        // DirectStorage write directly into privately-mapped page 1 (`b[1]`)!
        let runs = resolve_pages(&scattered, PAGE + 16, 32).unwrap();
        assert!(runs.write(&[0x5C; 32]));
        unsafe {
            assert_eq!(
                std::slice::from_raw_parts(va.cast::<u8>().add(PAGE + 16), 32),
                &[0x5C; 32]
            );
        }

        // Alias page 1 (`va + PAGE`) at `va2`: must coherently demote `va` to section views!
        assert!(map_alias(va2 as u64, (va as usize + PAGE) as u64, PAGE as u64));
        unsafe {
            assert_eq!(va2.cast::<u8>().read(), 0xA1);
            assert_eq!(std::slice::from_raw_parts(va2.cast::<u8>().add(16), 32), &[0x5C; 32]);
            // Writes through `va2` must immediately reflect at `va + PAGE`!
            va2.cast::<u8>().add(64).write(0xEE);
            assert_eq!(va.cast::<u8>().add(PAGE + 64).read(), 0xEE);
        }

        // Unmap both, then remap `scattered` as private at `va2`: exact bytes must survive!
        assert_eq!(XMemMapPhysicalPages(va, 4, std::ptr::null()), va);
        assert_eq!(XMemMapPhysicalPages(va2, 4, std::ptr::null()), va2);
        {
            let mut p = physical();
            // Ensure section bytes are marked present for page 1 (written via section view).
            let k1 = p.pages[&scattered[1]].key(0);
            p.in_section.insert(k1);
            let slices: Vec<Slice> = scattered.iter().map(|n| p.pages[n].clone()).collect();
            assert!(map_private(&mut p, va2 as usize, slices));
        }
        unsafe {
            assert_eq!(va2.cast::<u8>().read(), 0xA0);
            assert_eq!(va2.cast::<u8>().add(PAGE).read(), 0xA1);
            assert_eq!(va2.cast::<u8>().add(PAGE + 64).read(), 0xEE);
            assert_eq!(va2.cast::<u8>().add(2 * PAGE).read(), 0xA2);
            assert_eq!(va2.cast::<u8>().add(3 * PAGE).read(), 0xA3);
        }
        assert_eq!(XMemMapPhysicalPages(va2, 4, std::ptr::null()), va2);
        // Regression test: mapping 15 pages across a split 64 KiB placeholder
        // followed by a larger unsplit placeholder must split + coalesce cleanly!
        let big_pages = allocate(16);
        let big_va = XMemVirtualAlloc(std::ptr::null_mut(), 32 * PAGE, MEM_RESERVE | PAGES_64K, 0, PAGE_READWRITE);
        assert!(!big_va.is_null());
        assert_eq!(XMemMapPhysicalPages(big_va, 1, big_pages.as_ptr()), big_va);
        assert_eq!(XMemMapPhysicalPages(big_va, 1, std::ptr::null()), big_va);
        // Now [big_va, big_va + PAGE) is a 64 KiB placeholder next to a 31*PAGE placeholder!
        assert_eq!(XMemMapPhysicalPages(big_va, 15, big_pages.as_ptr()), big_va);
        assert_eq!(XMemMapPhysicalPages(big_va, 15, std::ptr::null()), big_va);
        assert_eq!(XMemFreePhysicalPages(16, big_pages.as_ptr()), 1);
        assert_eq!(XMemFreePhysicalPages(4, a.as_ptr()), 1);
        assert_eq!(XMemFreePhysicalPages(4, b.as_ptr()), 1);
    }

    #[test]
    fn concurrent_readers_and_writers_survive_demote_and_partial_unmap() {
        use std::sync::atomic::AtomicBool;
        let pages = allocate(4);
        let va = XMemVirtualAlloc(
            std::ptr::null_mut(),
            4 * PAGE,
            MEM_RESERVE | PAGES_64K,
            0,
            PAGE_READWRITE,
        );
        let alias = XMemVirtualAlloc(
            std::ptr::null_mut(),
            PAGE,
            MEM_RESERVE | PAGES_64K,
            0,
            PAGE_READWRITE,
        );
        assert!(!va.is_null() && !alias.is_null());
        assert_eq!(XMemMapPhysicalPages(va, 4, pages.as_ptr()), va);
        // Write sentinel values on pages 0 and 3 (which stay mapped throughout).
        let p0 = va as usize;
        let p3 = (va as usize) + 3 * PAGE;
        unsafe {
            (p0 as *mut u64).write_volatile(0xDEAD_BEEF_CAFE_BABEu64);
            (p3 as *mut u64).write_volatile(0);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let reader = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut count = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let v = unsafe { (p0 as *const u64).read_volatile() };
                    assert_eq!(v, 0xDEAD_BEEF_CAFE_BABEu64);
                    count += 1;
                }
                count
            })
        };
        let writer = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut count = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    count += 1;
                    unsafe { (p3 as *mut u64).write_volatile(count) };
                    let v = unsafe { (p3 as *const u64).read_volatile() };
                    assert_eq!(v, count, "lost write across demote/split");
                }
                count
            })
        };
        for _ in 0..200 {
            // Alias page 1 (demotes private block if UMA_POOL=1), then unalias.
            assert!(map_alias(alias as u64, (p0 + PAGE) as u64, PAGE as u64));
            assert!(map_alias(alias as u64, 0, PAGE as u64));
            // Partially unmap page 2 (splits the mapping while p0 & p3 are hammered!), then remap all 4.
            let mid = (p0 + 2 * PAGE) as *mut c_void;
            assert_eq!(XMemMapPhysicalPages(mid, 1, std::ptr::null()), mid);
            assert_eq!(XMemMapPhysicalPages(va, 4, pages.as_ptr()), va);
        }
        stop.store(true, Ordering::Relaxed);
        assert!(reader.join().expect("reader survived without AV") > 0);
        assert!(writer.join().expect("writer survived without AV or lost write") > 0);
        assert_eq!(XMemMapPhysicalPages(va, 4, std::ptr::null()), va);
        assert_eq!(XMemFreePhysicalPages(4, pages.as_ptr()), 1);
    }
}
