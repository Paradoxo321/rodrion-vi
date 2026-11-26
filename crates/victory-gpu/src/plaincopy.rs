//! Copies of title memory made in place, by this process, with a way back
//! when a page is not there (`VICTORY_COPY_PLAIN=1`; off by default unless
//! enabled).
//!
//! Extends Hyphen `0b0074e`'s VEH-guarded `rep movsb` read path with an
//! analogous guarded write path (`write`) for `write_mapped`, so both
//! title->host reads (~150 MB/frame) and host->title copy-backs (~60 MB/frame)
//! run at CPU ERMS memory speed instead of `NtReadVirtualMemory` /
//! `NtWriteVirtualMemory` syscalls, falling back cleanly on any missing,
//! read-only, or guard page.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".globl victory_copy_guarded",
    ".globl victory_copy_guarded_at",
    ".globl victory_copy_guarded_fault",
    "victory_copy_guarded:",
    // rcx: to, rdx: from, r8: bytes. rsi and rdi are callee-saved: kept in
    // r9 and r10, which are caller-saved, so nothing is pushed on the stack.
    "    mov r9, rsi",
    "    mov r10, rdi",
    "    mov rdi, rcx",
    "    mov rsi, rdx",
    "    mov rcx, r8",
    "victory_copy_guarded_at:",
    "    rep movsb",
    "    mov rsi, r9",
    "    mov rdi, r10",
    "    mov eax, 1",
    "    ret",
    "victory_copy_guarded_fault:",
    "    mov rsi, r9",
    "    mov rdi, r10",
    "    xor eax, eax",
    "    ret",

    ".globl victory_write_guarded",
    ".globl victory_write_guarded_at",
    ".globl victory_write_guarded_fault",
    "victory_write_guarded:",
    "    mov r9, rsi",
    "    mov r10, rdi",
    "    mov rdi, rcx",
    "    mov rsi, rdx",
    "    mov rcx, r8",
    "victory_write_guarded_at:",
    "    rep movsb",
    "    mov rsi, r9",
    "    mov rdi, r10",
    "    mov eax, 1",
    "    ret",
    "victory_write_guarded_fault:",
    "    mov rsi, r9",
    "    mov rdi, r10",
    "    xor eax, eax",
    "    ret",

    ".globl victory_safe_put",
    ".globl victory_safe_put_at",
    ".globl victory_safe_put_after",
    ".globl victory_safe_put_failed",
    // rcx destination, rdx what was there, r8 what is to be there, r9 blocks
    // of eight dwords (32 bytes): stores only the dwords in which `now` differs
    // from `was` using AVX2 `vpmaskmovd` (cool_fox safecopy.rs).
    "victory_safe_put:",
    "    test r9, r9",
    "    jz 4f",
    "    vpcmpeqd ymm3, ymm3, ymm3",
    "2:",
    "    vmovdqu ymm1, ymmword ptr [r8]",
    "    vpcmpeqd ymm2, ymm1, ymmword ptr [rdx]",
    "    vmovmskps eax, ymm2",
    "    cmp eax, 255",
    "    je 3f",
    "    vpxor ymm2, ymm2, ymm3",
    "victory_safe_put_at:",
    "    vpmaskmovd ymmword ptr [rcx], ymm2, ymm1",
    "victory_safe_put_after:",
    "    nop",
    "3:",
    "    add rcx, 32",
    "    add rdx, 32",
    "    add r8, 32",
    "    dec r9",
    "    jnz 2b",
    "    vzeroupper",
    "4:",
    "    mov eax, 1",
    "    ret",
    "victory_safe_put_failed:",
    "    vzeroupper",
    "    xor eax, eax",
    "    ret",
);

#[cfg(target_arch = "x86_64")]
unsafe extern "C" {
    fn victory_copy_guarded(to: *mut u8, from: *const u8, len: usize) -> u32;
    static victory_copy_guarded_at: u8;
    static victory_copy_guarded_fault: u8;

    fn victory_write_guarded(to: *mut u8, from: *const u8, len: usize) -> u32;
    static victory_write_guarded_at: u8;
    static victory_write_guarded_fault: u8;

    fn victory_safe_put(dst: *mut u8, was: *const u8, now: *const u8, blocks: usize) -> u32;
    static victory_safe_put_at: u8;
    static victory_safe_put_after: u8;
    static victory_safe_put_failed: u8;
}

/// Whether copies of title memory into host memory are made in place
/// (`VICTORY_COPY_PLAIN=1`; on by default unless `=0`).
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = cfg!(target_arch = "x86_64")
            && std::env::var_os("VICTORY_COPY_PLAIN").is_none_or(|v| v != "0");
        if on {
            install();
            tracing::info!(
                "copies of title memory made in place with VEH fallback (VICTORY_COPY_PLAIN=1)"
            );
        }
        on
    })
}

/// Whether host->title writes in `write_mapped` also use VEH-guarded in-place
/// copy (`VICTORY_WRITE_PLAIN=0` disables; follows `VICTORY_COPY_PLAIN` by default).
pub(crate) fn write_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        on() && std::env::var_os("VICTORY_WRITE_PLAIN").is_none_or(|v| v != "0")
    })
}

/// Read stats: [copies, bytes, faults, skipped_recent, skipped_small].
static COUNT: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
/// Write stats: [copies, bytes, faults, skipped_recent].
static WCOUNT: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

const LEAST: usize = 16;

static RECENT: [std::sync::atomic::AtomicUsize; 1024] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; 1024];
static EMPTIED: AtomicU64 = AtomicU64::new(0);

static WRECENT: [std::sync::atomic::AtomicUsize; 1024] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; 1024];
static WEMPTIED: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

fn blocks(from: usize, len: usize) -> (usize, usize) {
    ((from >> 16) | 1 << 63, ((from + len.max(1) - 1) >> 16) | 1 << 63)
}

fn faulted_lately_in(
    table: &[std::sync::atomic::AtomicUsize; 1024],
    emptied_at: &AtomicU64,
    from: usize,
    len: usize,
) -> bool {
    let now = now_ms();
    let emptied = emptied_at.load(Relaxed);
    if now.saturating_sub(emptied) >= 2_000
        && emptied_at.compare_exchange(emptied, now, Relaxed, Relaxed).is_ok()
    {
        for place in table {
            place.store(0, Relaxed);
        }
        return false;
    }
    let (first, last) = blocks(from, len);
    table[first & 1023].load(Relaxed) == first || table[last & 1023].load(Relaxed) == last
}

fn note_fault_in(table: &[std::sync::atomic::AtomicUsize; 1024], from: usize, len: usize) {
    let (first, last) = blocks(from, len);
    table[first & 1023].store(first, Relaxed);
    table[last & 1023].store(last, Relaxed);
}

#[cfg(target_arch = "x86_64")]
fn install() {
    use windows_sys::Win32::Foundation::{
        EXCEPTION_ACCESS_VIOLATION, EXCEPTION_GUARD_PAGE, EXCEPTION_IN_PAGE_ERROR,
    };
    use windows_sys::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_CONTINUE_SEARCH,
        EXCEPTION_POINTERS,
    };
    unsafe extern "system" fn on_fault(info: *mut EXCEPTION_POINTERS) -> i32 {
        let (r, context) = unsafe { (&*(*info).ExceptionRecord, &mut *(*info).ContextRecord) };
        let rip = context.Rip;
        const READ: usize = 0;
        const WRITE: usize = 1;
        if rip == (&raw const victory_copy_guarded_at) as u64 {
            let read = r.NumberParameters >= 1 && r.ExceptionInformation[0] == READ;
            let gone = (r.ExceptionCode == EXCEPTION_ACCESS_VIOLATION
                || r.ExceptionCode == EXCEPTION_IN_PAGE_ERROR)
                && read;
            if gone || r.ExceptionCode == EXCEPTION_GUARD_PAGE {
                context.Rip = (&raw const victory_copy_guarded_fault) as u64;
                return EXCEPTION_CONTINUE_EXECUTION;
            }
        } else if rip == (&raw const victory_write_guarded_at) as u64 {
            let write = r.NumberParameters >= 1 && r.ExceptionInformation[0] == WRITE;
            let gone = (r.ExceptionCode == EXCEPTION_ACCESS_VIOLATION
                || r.ExceptionCode == EXCEPTION_IN_PAGE_ERROR)
                && write;
            if gone || r.ExceptionCode == EXCEPTION_GUARD_PAGE {
                context.Rip = (&raw const victory_write_guarded_fault) as u64;
                return EXCEPTION_CONTINUE_EXECUTION;
            }
        } else if rip >= (&raw const victory_safe_put_at) as u64
            && rip < (&raw const victory_safe_put_after) as u64
        {
            let write = r.NumberParameters >= 1 && r.ExceptionInformation[0] == WRITE;
            let gone = (r.ExceptionCode == EXCEPTION_ACCESS_VIOLATION
                || r.ExceptionCode == EXCEPTION_IN_PAGE_ERROR)
                && write;
            if gone || r.ExceptionCode == EXCEPTION_GUARD_PAGE {
                context.Rip = (&raw const victory_safe_put_failed) as u64;
                return EXCEPTION_CONTINUE_EXECUTION;
            }
        }
        EXCEPTION_CONTINUE_SEARCH
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        AddVectoredExceptionHandler(1, Some(on_fault));
    });
}

#[cfg(not(target_arch = "x86_64"))]
fn install() {}

#[cfg(target_arch = "x86_64")]
fn guarded(to: *mut u8, from: *const u8, len: usize) -> bool {
    install();
    unsafe { victory_copy_guarded(to, from, len) != 0 }
}

#[cfg(not(target_arch = "x86_64"))]
fn guarded(_: *mut u8, _: *const u8, _: usize) -> bool {
    false
}

#[cfg(target_arch = "x86_64")]
fn guarded_write(to: *mut u8, from: *const u8, len: usize) -> bool {
    install();
    unsafe { victory_write_guarded(to, from, len) != 0 }
}

#[cfg(not(target_arch = "x86_64"))]
fn guarded_write(_: *mut u8, _: *const u8, _: usize) -> bool {
    false
}

/// Copies title memory at `from` into host memory at `to`.
pub(crate) fn copy(to: *mut u8, from: *const u8, len: usize) -> bool {
    if !on() {
        return false;
    }
    if len < LEAST {
        COUNT[4].fetch_add(1, Relaxed);
        return false;
    }
    if faulted_lately_in(&RECENT, &EMPTIED, from as usize, len) {
        COUNT[3].fetch_add(1, Relaxed);
        return false;
    }
    let whole = guarded(to, from, len);
    if whole {
        COUNT[0].fetch_add(1, Relaxed);
        COUNT[1].fetch_add(len as u64, Relaxed);
    } else {
        COUNT[2].fetch_add(1, Relaxed);
        note_fault_in(&RECENT, from as usize, len);
    }
    whole
}

/// Copies host memory at `from` into title memory at `to`.
pub(crate) fn write(to: *mut u8, from: *const u8, len: usize) -> bool {
    if !write_on() || len == 0 {
        return false;
    }
    if faulted_lately_in(&WRECENT, &WEMPTIED, to as usize, len) {
        WCOUNT[3].fetch_add(1, Relaxed);
        return false;
    }
    let whole = guarded_write(to, from, len);
    if whole {
        WCOUNT[0].fetch_add(1, Relaxed);
        WCOUNT[1].fetch_add(len as u64, Relaxed);
    } else {
        WCOUNT[2].fetch_add(1, Relaxed);
        note_fault_in(&WRECENT, to as usize, len);
    }
    whole
}

/// Whether AVX2 masked dword diff-writes (`put_changed`) are available and on.
pub(crate) fn put_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        #[cfg(target_arch = "x86_64")]
        let avx2 = std::is_x86_feature_detected!("avx2");
        #[cfg(not(target_arch = "x86_64"))]
        let avx2 = false;
        avx2 && write_on()
    })
}

/// Writes `now` to `dst` ONLY where it differs from `was` (whole dwords via
/// AVX2 `vpmaskmovd`, 32 bytes/cycle, without touching unchanged cache lines
/// or write-watch pages). Returns `false` if unavailable or a page faulted.
pub(crate) fn put_changed(dst: *mut u8, was: &[u8], now: &[u8]) -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        if !put_on() || was.len() != now.len() || now.is_empty() {
            return false;
        }
        let n = now.len();
        if faulted_lately_in(&WRECENT, &WEMPTIED, dst as usize, n) {
            WCOUNT[3].fetch_add(1, Relaxed);
            return false;
        }
        install();
        let head = (dst as usize).wrapping_neg() & 3;
        let head = head.min(n);
        let blocks = (n - head) / 32;
        let tail = head + blocks * 32;
        let mut all = true;
        if blocks > 0 {
            all &= unsafe {
                victory_safe_put(dst.add(head), was[head..].as_ptr(), now[head..].as_ptr(), blocks) != 0
            };
        }
        let single = |from: usize, to: usize| {
            let mut ok = true;
            let mut i = from;
            while i < to {
                let len = (4 - ((dst as usize) + i) % 4).min(to - i);
                if was[i..i + len] != now[i..i + len] {
                    ok &= unsafe { guarded_write(dst.add(i), now[i..].as_ptr(), len) };
                }
                i += len;
            }
            ok
        };
        if all {
            all &= single(0, head);
        }
        if all {
            all &= single(tail, n);
        }
        if all {
            WCOUNT[0].fetch_add(1, Relaxed);
            WCOUNT[1].fetch_add(n as u64, Relaxed);
        } else {
            WCOUNT[2].fetch_add(1, Relaxed);
            note_fault_in(&WRECENT, dst as usize, n);
        }
        all
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (dst, was, now);
        false
    }
}

pub(crate) fn line(frames: f64) {
    if !on() {
        return;
    }
    let [copies, bytes, gone, lately, small] =
        [0, 1, 2, 3, 4].map(|i| COUNT[i].swap(0, Relaxed) as f64 / frames);
    let [wcopies, wbytes, wgone, wlately] =
        [0, 1, 2, 3].map(|i| WCOUNT[i].swap(0, Relaxed) as f64 / frames);
    tracing::info!(
        "plain copy per frame (VICTORY_COPY_PLAIN=1): reads {copies:.0} ({:.1} MB, {gone:.1} faults, {lately:.0} recent, {small:.0} small); writes {wcopies:.0} ({:.1} MB, {wgone:.1} faults, {wlately:.0} recent)",
        bytes / 1e6,
        wbytes / 1e6
    );
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE,
        VirtualAlloc, VirtualFree,
    };

    const PAGE: usize = 4096;

    #[test]
    fn guarded_read_and_write_handle_faults() {
        let base = unsafe { VirtualAlloc(std::ptr::null(), 3 * PAGE, MEM_RESERVE, PAGE_NOACCESS) }
            as *mut u8;
        assert!(!base.is_null());
        assert!(!unsafe { VirtualAlloc(base.cast(), PAGE, MEM_COMMIT, PAGE_READWRITE) }.is_null());
        assert!(
            !unsafe { VirtualAlloc(base.wrapping_add(2 * PAGE).cast(), PAGE, MEM_COMMIT, PAGE_READONLY) }
                .is_null()
        );
        let src = vec![0x5Au8; PAGE];
        assert!(guarded_write(base, src.as_ptr(), PAGE));
        let mut dst = vec![0u8; PAGE];
        assert!(guarded(dst.as_mut_ptr(), base, PAGE));
        assert_eq!(dst, src);
        // Uncommitted middle page fails both read and write cleanly.
        assert!(!guarded(dst.as_mut_ptr(), base.wrapping_add(PAGE), PAGE));
        assert!(!guarded_write(base.wrapping_add(PAGE), src.as_ptr(), PAGE));
        // Read-only page succeeds read, fails write cleanly.
        assert!(guarded(dst.as_mut_ptr(), base.wrapping_add(2 * PAGE), PAGE));
        assert!(!guarded_write(base.wrapping_add(2 * PAGE), src.as_ptr(), PAGE));
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }
}

