//! Renderer port #1: the title's 32-bit graphics heap as the card's own
//! memory (`VICTORY_PORT_CONSTANTS=1`, off by default).
//!
//! Ported from Belobug/VIctory-Belobug-AMD & gleaf-6/VIctory-7900XTX- (`e4a531f`):
//! the title allocates its 185 MB 32-bit graphics heap in one reservation and maps
//! its 5 pieces out of order; `xmem::arrange` gives fresh section slices in address
//! order so the whole 185 MB forms one stretch of one section, and this module opens
//! it through a separate `MapViewOfFile` view as a single `ID3D12Heap` via
//! `OpenExistingHeapFromAddress1`. Title buffers (including the 174 MB ring buffer)
//! and loose windows inside it are placed in-place so the GPU reads and writes title
//! memory directly with zero CPU copies, zero copy-backs, and zero fills.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Graphics::Direct3D12::*;
use windows_sys::Win32::System::Memory as mem;

use crate::inplace::{Placed, cover, stretch};

const K64: u64 = 64 << 10;

/// The ported memory lies below 4 GiB (the title's 32-bit graphics heap).
const LOW: u64 = 1 << 32;

/// Maximum bytes of one in-place region (pages stay pinned while heap lives).
const MOST: u64 = 512 << 20;

/// `VICTORY_PORT_CONSTANTS=1` (default off): open contiguous 32-bit graphics
/// section stretches in-place through a dedicated host view.
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("VICTORY_PORT_CONSTANTS").is_ok_and(|v| v.trim() == "1"))
}

struct Region {
    start: u64,
    end: u64,
    heap: ID3D12Heap,
}

// SAFETY: D3D12 heaps are free-threaded.
unsafe impl Send for Region {}

static REGIONS: Mutex<Vec<Region>> = Mutex::new(Vec::new());

/// `[min_start, max_end]` across all open regions for fast lock-free reject.
static BOUNDS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

/// Telemetry counters:
/// `[0]` regions opened, `[1]` region bytes, `[2]` buffers/windows placed,
/// `[3]` refused (not one stretch), `[4]` refused (not committed),
/// `[5]` refused (host/map error), `[6]` refused (buffer placement error),
/// `[7]` regions forgotten, `[8]` unused,
/// `[9]` CPU fills reading ported memory, `[10]` fill bytes,
/// `[11]` CPU copies touching ported memory, `[12]` copy bytes.
static STATS: [AtomicU64; 13] = [const { AtomicU64::new(0) }; 13];

/// Whether `[at, at + len)` is eligible (below 4 GiB and switch enabled).
pub(crate) fn wanted(at: u64, len: u64) -> bool {
    on() && in_low_32bit(at, len)
}

#[inline]
fn in_low_32bit(at: u64, len: u64) -> bool {
    len > 0 && at.checked_add(len).is_some_and(|end| end <= LOW)
}

/// Place title memory `[start, start + len)` in-place inside its 32-bit
/// graphics section region, if eligible and contiguous.
pub(crate) fn place_for(start: u64, len: u64, window: bool) -> Option<Placed> {
    if !wanted(start, len) {
        return None;
    }
    let host = crate::host::host()?;
    match place_in(&host.device, start, len) {
        Ok(placed) => {
            crate::inplace::counted(window, len, true);
            let n = STATS[2].fetch_add(1, Ordering::Relaxed) + 1;
            if n <= 32 || n.is_power_of_two() {
                tracing::info!(
                    start = format_args!("{start:#x}"),
                    len,
                    window,
                    "renderer port 1: title memory in place, read and written by the card where the title has it"
                );
            }
            Some(placed)
        }
        Err((at, why)) => {
            let n = STATS[at].fetch_add(1, Ordering::Relaxed) + 1;
            if n <= 8 || n.is_power_of_two() {
                tracing::warn!(
                    start = format_args!("{start:#x}"),
                    len,
                    window,
                    why,
                    "renderer port 1: title memory not in place, falling back to host copy"
                );
            }
            None
        }
    }
}

pub(crate) fn place_in(device: &ID3D12Device, start: u64, len: u64) -> Result<Placed, (usize, String)> {
    let at = start & !(K64 - 1);
    let end = (start + len.max(1)).next_multiple_of(K64);
    let mut regions = REGIONS.lock().unwrap_or_else(|e| e.into_inner());
    let i = match regions.iter().position(|r| r.start <= at && end <= r.end) {
        Some(i) => i,
        None => {
            let region = open_region(device, at, end)?;
            regions.push(region);
            let n = regions.len() - 1;
            let r = &regions[n];
            STATS[0].fetch_add(1, Ordering::Relaxed);
            STATS[1].fetch_add(r.end - r.start, Ordering::Relaxed);
            bounds(&regions);
            n
        }
    };
    let region = &regions[i];
    let buffer = crate::inplace::place(device, &region.heap, at - region.start, end - at)
        .map_err(|e| (6, format!("no buffer placed in the region: {e}")))?;
    // SAFETY: a live buffer.
    let gpu = unsafe { buffer.GetGPUVirtualAddress() } + (start - at);
    // `section: true` because this heap lives on our own separate MapViewOfFile
    // view and does NOT pin the title's placeholder VA.
    Ok(Placed { buffer, heap: region.heap.clone(), gpu, bias: start - at, section: true })
}

fn open_region(device: &ID3D12Device, at: u64, end: u64) -> Result<Region, (usize, String)> {
    let Some((first, bytes, section, offset)) = stretch(at as usize, (end - at) as usize) else {
        return Err((3, format!("not one stretch of a section: {:?}", cover(at as usize, (end - at) as usize))));
    };
    let (mut first, mut bytes, mut offset) = (first as u64, bytes as u64, offset);
    if first < LOW && first + bytes > LOW {
        bytes = LOW.saturating_sub(first);
    }
    if bytes > MOST {
        offset += at - first;
        (first, bytes) = (at, end - at);
    }
    if bytes > MOST {
        return Err((3, format!("{bytes:#x} bytes, over the most of a region")));
    }
    if first > at || first + bytes < end || offset % K64 != 0 {
        return Err((3, format!("the stretch {first:#x}+{bytes:#x} does not hold it")));
    }
    // SAFETY: section handle kept open by xmem while title view exists; range is valid.
    let view = unsafe {
        mem::MapViewOfFile(
            section as _,
            mem::FILE_MAP_WRITE,
            (offset >> 32) as u32,
            offset as u32,
            bytes as usize,
        )
    }
    .Value as usize;
    if view == 0 {
        return Err((5, format!("no view of the stretch: {}", std::io::Error::last_os_error())));
    }
    let unmap = || {
        // SAFETY: our own view, nothing opened on it.
        unsafe { mem::UnmapViewOfFile(mem::MEMORY_MAPPED_VIEW_ADDRESS { Value: view as *mut _ }) };
    };
    if !committed(view, bytes as usize) {
        unmap();
        return Err((4, "pages of the stretch are not committed".to_owned()));
    }
    match crate::inplace::open(device, crate::inplace::From::Part(view, bytes as usize)) {
        Ok(heap) => {
            tracing::info!(
                start = format_args!("{first:#x}"),
                bytes,
                section = format_args!("{section:#x}"),
                offset = format_args!("{offset:#x}"),
                view = format_args!("{view:#x}"),
                "renderer port 1: a stretch of the title's 32-bit graphics memory opened as a host heap"
            );
            Ok(Region { start: first, end: first + bytes, heap })
        }
        Err(e) => {
            unmap();
            Err((5, format!("refused by the host: {e}")))
        }
    }
}

fn committed(at: usize, len: usize) -> bool {
    let mut here = at;
    while here < at + len {
        // SAFETY: plain data filled in by VirtualQuery.
        let mut info: mem::MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: valid out pointer of right size.
        if unsafe { mem::VirtualQuery(here as *const _, &mut info, size_of_val(&info)) } == 0
            || info.State != mem::MEM_COMMIT
        {
            return false;
        }
        here = (info.BaseAddress as usize).saturating_add(info.RegionSize);
    }
    true
}

fn bounds(regions: &[Region]) {
    let lo = regions.iter().map(|r| r.start).min().unwrap_or(0);
    let hi = regions.iter().map(|r| r.end).max().unwrap_or(0);
    BOUNDS[0].store(lo, Ordering::Relaxed);
    BOUNDS[1].store(hi, Ordering::Relaxed);
}

/// The title maps other pages at `[at, at + len)`: forget overlapping regions.
pub(crate) fn forget(at: usize, len: usize) {
    let (at, end) = (at as u64, (at as u64).saturating_add(len as u64));
    if BOUNDS[1].load(Ordering::Relaxed) <= at || BOUNDS[0].load(Ordering::Relaxed) >= end {
        return;
    }
    let mut regions = REGIONS.lock().unwrap_or_else(|e| e.into_inner());
    let before = regions.len();
    regions.retain(|r| r.end <= at || r.start >= end);
    STATS[7].fetch_add((before - regions.len()) as u64, Ordering::Relaxed);
    bounds(&regions);
}

#[inline]
fn touches(at: usize, len: usize) -> bool {
    let hi = BOUNDS[1].load(Ordering::Relaxed);
    hi != 0 && (at as u64) < hi && (at as u64).saturating_add(len as u64) > BOUNDS[0].load(Ordering::Relaxed)
}

pub(crate) fn filled(src: usize, len: usize) {
    if touches(src, len) {
        STATS[9].fetch_add(1, Ordering::Relaxed);
        STATS[10].fetch_add(len as u64, Ordering::Relaxed);
    }
}

pub(crate) fn copied(dst: usize, src: usize, len: usize) {
    if touches(dst, len) || touches(src, len) {
        STATS[11].fetch_add(1, Ordering::Relaxed);
        STATS[12].fetch_add(len as u64, Ordering::Relaxed);
    }
}

pub(crate) fn log(frames: f64) {
    if !on() {
        return;
    }
    let s: Vec<u64> = STATS.iter().map(|s| s.load(Ordering::Relaxed)).collect();
    let per: Vec<f64> = STATS[9..].iter().map(|s| s.swap(0, Ordering::Relaxed) as f64 / frames.max(1.0)).collect();
    tracing::info!(
        "renderer port 1 (VICTORY_PORT_CONSTANTS=1): {} regions in place ({} MB), {} buffers/windows placed; \
         per frame CPU: {:.0} fills ({:.1} KB), {:.1} copies ({:.1} KB); \
         refused: {} not-stretch, {} uncommitted, {} host, {} buffer; {} forgotten",
        s[0],
        s[1] >> 20,
        s[2],
        per[0],
        per[1] / 1024.0,
        per[2],
        per[3] / 1024.0,
        s[3],
        s[4],
        s[5],
        s[6],
        s[7],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_memory_below_4_gib_is_eligible_for_port_constants() {
        if !on() {
            assert!(!wanted(0x3431_0000, 0x1000));
        }
        assert!(in_low_32bit(0x34e1_0000, 0xae0_0000));
        assert!(!in_low_32bit(0x34e1_0000, 0));
        assert!(!in_low_32bit(0xffff_0000, 0x2_0000));
        assert!(!in_low_32bit(0xe_1c80_0000, 0x1000));
        assert!(!in_low_32bit(u64::MAX, 2));
    }
}
