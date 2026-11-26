//! Which title memory the title wrote since a host copy took it (decision
//! 0007, "Copies of title memory"). The runtime reserves title GPU memory
//! with write watching (`victory-hle` xmem), so the kernel keeps a dirty bit
//! per 4 KiB page; asking for a range's written pages also clears them. Host
//! copies of the same memory can overlap (nested buffers, textures in
//! buffers), so what one asks is kept here for the others: a journal of
//! each page's last write, numbered by the asking.
//!
//! Physical-page views (xmem's sections) cannot be write-watched. There a
//! copy leaves each 64 KiB block it took read-only, and our vectored handler
//! takes the first write to it: it notes the block as written and gives the
//! write access back. The runtime tells us when it maps or unmaps views
//! ([`title_memory_remapped`]), which resets their protection. Blocks found
//! wholly uncommitted are skipped while they remain so; one query can
//! confirm a whole region, including ordinary reservations between views.
//!
//! Writes into physical pages through another view are seen too: the
//! runtime tells us the views of the pages it wrote itself ([`writing`],
//! xmem's `write_physical` and DirectStorage's page destinations), and a
//! write by the title through any view of a guarded page faults like one
//! at the guarded address (the guard makes every view of the page
//! read-only, [`open`] gives them all back; not a page of more than
//! [`MANY_VIEWS`] views). A
//! system call writing into a guarded block fails (none is known to).

use std::collections::BTreeMap;
use std::sync::Mutex;

/// The unit tracked and copied: 64 KiB, the console's page (the kernel's
/// 4 KiB pages fold into it). Finer units made the checks of the world's
/// big buffers cost more than the copies they saved.
pub(crate) const PAGE: usize = 64 << 10;
/// The kernel's page, the unit of its answers.
const KERNEL_PAGE: usize = 4096;

/// A title-memory range whose changed pages another subsystem needs to read.
pub(crate) struct Tracked {
    start: usize,
    end: usize,
    seen: Mutex<BTreeMap<usize, u64>>,
}

/// Tracks a runtime GPU-memory range, including physical-page views when
/// `views` is true. Other process memory is not tracked.
pub(crate) fn track(start: usize, len: usize, views: bool) -> Option<Tracked> {
    let end = start.checked_add(len)?;
    if len == 0 || writes(start, end, views).is_none() {
        return None;
    }
    Some(Tracked {
        start,
        end,
        seen: Mutex::default(),
    })
}

struct Journal {
    /// Numbers the askings; 0 is "never".
    asked: u64,
    /// Page number → the asking that last found it written.
    written: BTreeMap<usize, u64>,
    /// Room for the kernel's answer.
    found: Vec<usize>,
    /// Page ranges asked ahead for a submission's refreshes ([`ask_ahead`])
    /// and the asking that did: `writes` answers for a range inside one
    /// from the journal, without another kernel call. Kept sorted and joined.
    ahead: Vec<(usize, usize)>,
    /// As `ahead`, of view memory the guards answered for with the submission's
    /// one asking ([`ask_views`]).
    ahead_views: Vec<(usize, usize)>,
    ahead_asked: u64,
    /// The ahead asking stands for a whole submission ([`ask_once`]): no
    /// other ahead asking is made, and `ahead` stays, until
    /// [`ask_once_end`].
    kept: bool,
}

/// Shared by those the journal answers without an asking (`writes`, `asked`):
/// `wide_refresh` workers check the journal concurrently across all threads!
static JOURNAL: std::sync::RwLock<Journal> = std::sync::RwLock::new(Journal {
    asked: 0,
    written: BTreeMap::new(),
    found: Vec::new(),
    ahead: Vec::new(),
    ahead_views: Vec::new(),
    ahead_asked: 0,
    kept: false,
});

fn add_range(ranges: &mut Vec<(usize, usize)>, first: usize, last: usize) {
    if first >= last {
        return;
    }
    let from = ranges.partition_point(|r| r.1 < first);
    let mut to = from;
    let (mut a, mut b) = (first, last);
    while to < ranges.len() && ranges[to].0 <= b {
        (a, b) = (a.min(ranges[to].0), b.max(ranges[to].1));
        to += 1;
    }
    ranges.splice(from..to, [(a, b)]);
}

fn covered(ranges: &[(usize, usize)], first: usize, last: usize) -> bool {
    let at = ranges.partition_point(|r| r.1 <= first);
    ranges.get(at).is_some_and(|r| r.0 <= first && last <= r.1)
}

impl Journal {
    fn stands(&self, first: usize, last: usize, guarded: bool) -> bool {
        covered(&self.ahead, first, last) || (guarded && covered(&self.ahead_views, first, last))
    }
}

/// `VICTORY_WATCH_READ_FAST=1` (default off): answer clean guarded view memory
/// ranges in [`writes`] under shared read locks (`JOURNAL.read` + `GUARDS.read`)
/// without taking exclusive write locks on either.
pub(crate) fn watch_read_fast() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_WATCH_READ_FAST").is_some_and(|v| v == "1"))
}

/// `VICTORY_WATCH_UNLOCKED_ASK=1` (default off): run `GetWriteWatch` and `guard()`
/// outside the global `JOURNAL.write()` lock, serialized only per 16 MiB address stripe
/// (`STRIPES: [RwLock<()>; 64]`) so concurrent threads on disjoint ranges run in parallel
/// without losing writes or deadlocking.
pub(crate) fn watch_unlocked_ask() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_WATCH_UNLOCKED_ASK").is_some_and(|v| v == "1"))
}

const WATCH_STRIPES: usize = 64;
const PAGES_PER_STRIPE: usize = 256; // 256 * 64 KiB = 16 MiB per stripe

static STRIPES: [std::sync::RwLock<()>; WATCH_STRIPES] =
    [const { std::sync::RwLock::new(()) }; WATCH_STRIPES];

/// Sorted, deduplicated stripe indices covering 64 KiB pages `[first, last)`.
fn stripe_indices(first: usize, last: usize) -> Vec<usize> {
    if first >= last {
        return Vec::new();
    }
    let s0 = first / PAGES_PER_STRIPE;
    let s1 = (last - 1) / PAGES_PER_STRIPE;
    if s1 - s0 + 1 >= WATCH_STRIPES {
        return (0..WATCH_STRIPES).collect();
    }
    let mut out: Vec<usize> = (s0..=s1).map(|s| s & (WATCH_STRIPES - 1)).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// `[0]` clean read-fast hits, `[1]` slow-path calls in `writes`,
/// `[2]` microseconds spent in slow path, `[3]` slow `GetWriteWatch` calls, `[4]` slow `guard()` calls.
pub(crate) static WATCH_FAST_STATS: [std::sync::atomic::AtomicU64; 5] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 5];

/// `VICTORY_GUARD_ABSENT_FAST=1` (default off): allow [`try_clean_view_read`] to
/// treat known-unmapped (`g.absent`) placeholder holes as clean under shared read locks
/// (`title_memory_remapped` immediately clears `g.absent` whenever `xmem` maps a view).
pub(crate) fn guard_absent_fast() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_GUARD_ABSENT_FAST").is_some_and(|v| v == "1"))
}

/// Shared-read fast check for clean (and optionally known-absent) guarded view memory
/// `[first, last)` (64 KiB pages).
fn try_clean_view_read_ex(
    j: &Journal,
    g: &Guards,
    first: usize,
    last: usize,
    trust_absent: bool,
) -> Option<(u64, Vec<(usize, u64)>)> {
    if j.asked == 0 || first >= last {
        return None;
    }
    let covered = if trust_absent {
        g.clean.all_present_with(&g.absent, first, last)
    } else {
        g.clean.all_present(first, last)
    };
    if !covered {
        return None;
    }
    Some((
        j.asked,
        j.written
            .range(first..last)
            .map(|(&p, &w)| (p, w))
            .collect(),
    ))
}

/// Shared-read fast check for clean guarded view memory `[first, last)` (64 KiB pages).
/// Caller holds `JOURNAL.read()` and `GUARDS.read()` (exact same lock order as write path).
fn try_clean_view_read(
    j: &Journal,
    g: &Guards,
    first: usize,
    last: usize,
) -> Option<(u64, Vec<(usize, u64)>)> {
    if j.asked == 0 || first >= last || !g.clean.all_present(first, last) {
        return None;
    }
    Some((
        j.asked,
        j.written
            .range(first..last)
            .map(|(&p, &w)| (p, w))
            .collect(),
    ))
}

pub(crate) fn fast_stats_line(frames: f64) {
    use std::sync::atomic::Ordering::Relaxed;
    let fast = WATCH_FAST_STATS[0].swap(0, Relaxed) as f64 / frames;
    let slow = WATCH_FAST_STATS[1].swap(0, Relaxed) as f64 / frames;
    let slow_ms = WATCH_FAST_STATS[2].swap(0, Relaxed) as f64 / 1000.0 / frames;
    let gww = WATCH_FAST_STATS[3].swap(0, Relaxed) as f64 / frames;
    let grd = WATCH_FAST_STATS[4].swap(0, Relaxed) as f64 / frames;
    if fast + slow > 0.0 {
        tracing::info!(
            "watch::writes per frame (FAST={}, UNLOCKED={}): {fast:.0} read-fast, {slow:.0} slow ({slow_ms:.2} ms: {gww:.0} GetWriteWatch, {grd:.0} guard)",
            u8::from(watch_read_fast()),
            u8::from(watch_unlocked_ask()),
        );
    }
}

/// Ranges of one submission closer than this are asked about together:
/// walking the pages between them costs less than another kernel call (a
/// call that resets written pages costs 30-70 microseconds whatever the
/// range).
const AHEAD_GAP: usize = (8 << 20) / PAGE;
/// The most one asking covers (its answer needs 8 bytes per 4 KiB page).
const AHEAD_MAX: usize = (1 << 30) / PAGE;

/// Asks the kernel about many ranges `(start, end)` at once, before the
/// [`writes`] of each: ranges close together are one asking, and `writes`
/// then answers from the journal until [`ask_ahead_done`]. The world's
/// submissions refresh thousands of host copies (6,664 askings a frame,
/// 456 ms, in run 14). Ranges that are not write-watched, or lie alone, are
/// left to `writes`.
pub(crate) fn ask_ahead(ranges: &[(usize, usize)]) {
    let mut pieces: Vec<(usize, usize)> = ranges
        .iter()
        .filter(|r| r.0 < r.1)
        .map(|&(start, end)| (start / PAGE, end.div_ceil(PAGE)))
        .collect();
    if pieces.len() < 2 {
        return;
    }
    pieces.sort_unstable();
    let mut j = JOURNAL.write().unwrap_or_else(|e| e.into_inner());
    let j = &mut *j;
    // The submission's one asking stands ([`ask_once`]): what it does not
    // hold is left to `writes`.
    if j.kept {
        return;
    }
    j.asked += 1;
    j.ahead_asked = j.asked;
    j.ahead.clear();
    j.ahead_views.clear();
    let mut i = 0;
    while i < pieces.len() {
        let first = pieces[i].0;
        let mut last = pieces[i].1;
        let mut k = i + 1;
        while k < pieces.len()
            && pieces[k].0 <= last + AHEAD_GAP
            && last.max(pieces[k].1) - first <= AHEAD_MAX
        {
            last = last.max(pieces[k].1);
            k += 1;
        }
        ask_run(j, &pieces[i..k]);
        i = k;
    }
}

/// Since the last frame line: the kernel askings that took a millisecond
/// or more, their microseconds, and the longest asking's. An asking walks
/// page tables, some ten microseconds for most ranges; in run 75 the 2,644
/// askings of a frame took 219 ms in the lines where the pass over the
/// heap's views ran long, 25-50 ms in the others. These numbers say
/// whether a few askings wait for something or all of them are slow.
#[cfg(feature = "diagnostics")]
static SLOW: [std::sync::atomic::AtomicU64; 3] = [const { std::sync::atomic::AtomicU64::new(0) }; 3];

#[cfg(feature = "diagnostics")]
fn slow(us: u64) {
    use std::sync::atomic::Ordering::Relaxed;
    if us >= 1000 {
        SLOW[0].fetch_add(1, Relaxed);
        SLOW[1].fetch_add(us, Relaxed);
    }
    SLOW[2].fetch_max(us, Relaxed);
}

/// [`SLOW`], taken.
#[cfg(feature = "diagnostics")]
pub(crate) fn slow_asks() -> [u64; 3] {
    [0, 1, 2].map(|i| SLOW[i].swap(0, std::sync::atomic::Ordering::Relaxed))
}

/// The submission's refreshes are done: `writes` asks the kernel again.
pub(crate) fn ask_ahead_done() {
    let mut j = JOURNAL.write().unwrap_or_else(|e| e.into_inner());
    if !j.kept {
        j.ahead.clear();
        j.ahead_views.clear();
    }
}

/// Whether a submission asks the kernel once, before its first refresh,
/// about every range its lists refresh (`VICTORY_ASK_ONCE=1`; off by
/// default).
///
/// A submission's refreshes are done in groups, between the title's own
/// buffer copies (each copy flushes the refreshes recorded before it), and
/// every group asked the kernel again: 2,644 askings a frame, 70 a
/// submission. With the switch the journal answers all of them from one
/// asking made when the submission starts.
///
/// What changes: memory the title's other threads write while the
/// submission is being prepared is seen by the next submission's asking,
/// not by this one's later groups. What this submission's lists read was
/// written before they were submitted, so before the asking. Our own
/// writes into title memory while preparing are told to the journal
/// ([`wrote`]) or end the asking ([`ask_once_end`]).
pub(crate) fn once() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    // On by default since run 81 (with card buffers: 375-450 ms a frame
    // against 573-647, the picture unchanged over four minutes);
    // `VICTORY_ASK_ONCE=0` asks per range, as before.
    *ON.get_or_init(|| std::env::var_os("VICTORY_ASK_ONCE").is_none_or(|v| v != "0"))
}

/// `VICTORY_ASK_QUIET=<frames>` (0 or unset: off): 1 MiB blocks in which no
/// write was found for the last `<frames>` frames are queried with
/// `GetWriteWatch` only once per frame instead of on every submission.
fn quiet_frames() -> u64 {
    static FRAMES: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *FRAMES.get_or_init(|| {
        std::env::var("VICTORY_ASK_QUIET")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0)
    })
}

fn quiet_gap() -> usize {
    static GAP: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *GAP.get_or_init(|| {
        std::env::var("VICTORY_ASK_QUIET_GAP_MB")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(32)
    })
}

const QUIET_BLOCK: usize = (1 << 20) / PAGE;

#[derive(Clone, Copy)]
struct QuietBlock {
    asked: u64,
    written: u64,
    partial: bool,
}

const QUIET_NEVER: QuietBlock = QuietBlock {
    asked: u64::MAX,
    written: u64::MAX,
    partial: false,
};

struct QuietState {
    blocks: std::collections::HashMap<usize, QuietBlock>,
    told: [u64; 5],
    frame: u64,
}

static QUIET_BLOCKS: Mutex<Option<QuietState>> = Mutex::new(None);

fn ask_blocks(
    j: &mut Journal,
    state: &mut QuietState,
    first: usize,
    last: usize,
    frame: u64,
    keep: u64,
) -> bool {
    let Some(count) = ask_kernel_found(j, first * QUIET_BLOCK, last * QUIET_BLOCK) else {
        return false;
    };
    for block in first..last {
        let q = state.blocks.entry(block).or_insert(QUIET_NEVER);
        let lately = q.written != u64::MAX && frame.wrapping_sub(q.written) <= keep;
        state.told[if q.asked == frame || lately { 1 } else { 2 }] += 1;
        q.asked = frame;
    }
    for k in 0..count {
        let block = j.found[k] / PAGE / QUIET_BLOCK;
        if let Some(q) = state.blocks.get_mut(&block) {
            q.written = frame;
        }
    }
    true
}

fn ask_once_quiet(ranges: &[(usize, usize)], keep: u64, views: bool) {
    let frame = crate::frame::current();
    let mut held = QUIET_BLOCKS.lock().unwrap_or_else(|e| e.into_inner());
    let state = held.get_or_insert_with(|| QuietState {
        blocks: Default::default(),
        told: [0; 5],
        frame,
    });
    let mut j = JOURNAL.write().unwrap_or_else(|e| e.into_inner());
    let j = &mut *j;
    j.asked += 1;
    j.ahead_asked = j.asked;
    j.ahead.clear();
    j.ahead_views.clear();
    if state.frame != frame {
        state.told[0] += frame.wrapping_sub(state.frame).min(1 << 20);
        state.frame = frame;
        if state.told[0] >= 120 {
            let n = state.told[0] as f64;
            tracing::info!(
                "title memory asked about per frame (VICTORY_ASK_QUIET={keep}): {:.0} hot 1 MiB blocks, {:.0} quiet blocks (1st of frame), {:.0} ranges from journal, {:.0} standalone; {} blocks tracked",
                state.told[1] as f64 / n,
                state.told[2] as f64 / n,
                state.told[3] as f64 / n,
                state.told[4] as f64 / n,
                state.blocks.len()
            );
            state.told = [0; 5];
        }
    }
    let mut wanted: Vec<usize> = Vec::new();
    for &(start, end) in ranges {
        if start >= end {
            continue;
        }
        let (first, last) = (start / PAGE, end.div_ceil(PAGE));
        for block in first / QUIET_BLOCK..=(last - 1) / QUIET_BLOCK {
            let q = *state.blocks.entry(block).or_insert(QUIET_NEVER);
            if q.partial {
                continue;
            }
            let lately = q.written != u64::MAX && frame.wrapping_sub(q.written) <= keep;
            if q.asked != frame || lately {
                wanted.push(block);
            }
        }
    }
    wanted.sort_unstable();
    wanted.dedup();
    let gap = quiet_gap();
    let mut i = 0;
    while i < wanted.len() {
        let mut k = i + 1;
        while k < wanted.len() && wanted[k] <= wanted[k - 1] + 1 + gap && wanted[k] - wanted[i] < 4096 {
            k += 1;
        }
        let run = &wanted[i..k];
        let whole = ask_blocks(j, state, run[0], run[run.len() - 1] + 1, frame, keep);
        if !whole && run.len() > 1 {
            for &block in run {
                if !ask_blocks(j, state, block, block + 1, frame, keep)
                    && let Some(q) = state.blocks.get_mut(&block)
                {
                    q.partial = true;
                }
            }
        } else if !whole && let Some(q) = state.blocks.get_mut(&run[0]) {
            q.partial = true;
        }
        i = k;
    }
    for &(start, end) in ranges {
        if start >= end {
            continue;
        }
        let (first, last) = (start / PAGE, end.div_ceil(PAGE));
        if known_view(start) {
            if views
                && !covered(&j.ahead_views, first, last)
                && let Some(written) = guard(first * PAGE, last * PAGE)
            {
                let now = j.ahead_asked;
                for page in written {
                    whole_4k(page, now);
                    j.written.insert(page, now);
                }
                add_range(&mut j.ahead_views, first, last);
            }
            continue;
        }
        let covered_q = (first / QUIET_BLOCK..=(last - 1) / QUIET_BLOCK)
            .all(|block| state.blocks.get(&block).is_some_and(|q| !q.partial && q.asked == frame));
        if covered_q {
            state.told[3] += 1;
            add_range(&mut j.ahead, first, last);
        } else if !covered(&j.ahead, first, last) && ask_kernel(j, first, last) {
            state.told[4] += 1;
            add_range(&mut j.ahead, first, last);
        }
    }
    j.kept = !(j.ahead.is_empty() && j.ahead_views.is_empty());
}

/// (Vertex / ShakiraRED) Whether the submission's one asking also asks the guards
/// for view memory once (`VICTORY_ASK_ONCE_VIEWS=1`), letting `wide_refresh`
/// threads read view dirtiness concurrently without locking `GUARDS`.
pub(crate) fn ask_views() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_ASK_ONCE_VIEWS").is_some_and(|v| v == "1"))
}

/// With [`once`]: asks about `ranges` (everything a submission's lists
/// refresh) as [`ask_ahead`] does, and lets that asking stand until
/// [`ask_once_end`].
pub(crate) fn ask_once(ranges: &[(usize, usize)]) {
    if !once() {
        return;
    }
    ask_once_end();
    let views = ask_views();
    let keep = quiet_frames();
    if keep > 0 {
        ask_once_quiet(ranges, keep, views);
        return;
    }
    ask_ahead(ranges);
    let mut j = JOURNAL.write().unwrap_or_else(|e| e.into_inner());
    let j = &mut *j;
    if j.ahead.is_empty() {
        // `ask_ahead` asked nothing (fewer than two ranges).
        j.asked += 1;
        j.ahead_asked = j.asked;
    }
    for &(start, end) in ranges {
        if start >= end {
            continue;
        }
        let (first, last) = (start / PAGE, end.div_ceil(PAGE));
        if known_view(start) {
            if views
                && !covered(&j.ahead_views, first, last)
                && let Some(written) = guard(first * PAGE, last * PAGE)
            {
                let now = j.ahead_asked;
                for page in written {
                    whole_4k(page, now);
                    j.written.insert(page, now);
                }
                add_range(&mut j.ahead_views, first, last);
            }
            continue;
        }
        if !covered(&j.ahead, first, last) && ask_kernel(j, first, last) {
            add_range(&mut j.ahead, first, last);
        }
    }
    j.kept = !(j.ahead.is_empty() && j.ahead_views.is_empty());
}

/// The submission's one asking stands no longer: its refreshes are done,
/// or title memory was written in a way the journal was not told.
pub(crate) fn ask_once_end() {
    crate::stamp::advance();
    let mut j = JOURNAL.write().unwrap_or_else(|e| e.into_inner());
    if j.kept {
        j.kept = false;
        j.ahead.clear();
        j.ahead_views.clear();
    }
}

/// We wrote title memory `[at, at + len)` while a submission's one asking
/// stands, updating the journal without advancing the global refresh stamp
/// (caller tracks written ranges per batch via `VICTORY_BATCH_RANGE_FLUSH=1`).
pub(crate) fn wrote_keep_stamp(at: usize, len: usize) {
    let mut j = JOURNAL.write().unwrap_or_else(|e| e.into_inner());
    if !j.kept || len == 0 {
        return;
    }
    j.asked += 1;
    let now = j.asked;
    j.ahead_asked = now;
    for page in at / PAGE..at.saturating_add(len).div_ceil(PAGE) {
        whole_4k(page, now);
        j.written.insert(page, now);
    }
}

/// We wrote title memory `[at, at + len)` while a submission's one asking
/// stands (the title's buffer copies, done at submission): its pages count
/// as written after that asking, so the refreshes that follow copy them.
pub(crate) fn wrote(at: usize, len: usize) {
    crate::stamp::advance();
    let mut j = JOURNAL.write().unwrap_or_else(|e| e.into_inner());
    if !j.kept || len == 0 {
        return;
    }
    j.asked += 1;
    let now = j.asked;
    // The asking that stands moves with it: `asked` and `writes` give it as
    // "now", and a page noted written after "now" stays stale for every
    // copy that takes it (copied at `now`, written at `now + 1`): copied
    // again at every refresh, and a reason to settle flights
    // (fxceless/victory-fork#410).
    j.ahead_asked = now;
    for page in at / PAGE..at.saturating_add(len).div_ceil(PAGE) {
        whole_4k(page, now);
        j.written.insert(page, now);
    }
}

/// (Neptune) Whether what the copy-back writes into guarded view title memory
/// goes through a private unguarded section view (`VICTORY_QUIET_BACK=1`):
/// zero page faults, zero `VirtualProtect` shootdowns, and the copy it came
/// from is told it already holds those bytes so it does not re-read/compare
/// them on the next submission.
pub(crate) fn quiet_back() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_QUIET_BACK").is_some_and(|v| v == "1"))
}

/// (Neptune) Whether view memory `[at, at + len)` is all guarded and clean:
/// nothing was written there since a copy last took it ([`quiet_back`]).
pub(crate) fn guarded_clean(at: usize, len: usize) -> bool {
    let (first, last) = (at / BLOCK, at.saturating_add(len).div_ceil(BLOCK));
    first < last && guards_shared().clean.all_present(first, last)
}

/// (Neptune) After our own write of view memory `[at, at + len)` through a
/// private section view (`crate::inplace::host_alias`), which no guard saw:
/// the journal records its pages as written now at every title view that shows
/// them, and returns `(page, prev_asking, new_asking)` so the source shadow
/// can advance its own `copied[]` mark without re-reading its own write.
pub(crate) fn ours_through(at: usize, len: usize) -> Vec<(usize, u64, u64)> {
    crate::stamp::advance();
    let mut j = JOURNAL.write().unwrap_or_else(|e| e.into_inner());
    j.asked += 1;
    let now = j.asked;
    let mut marks = Vec::new();
    for page in at / PAGE..at.saturating_add(len).div_ceil(PAGE) {
        whole_4k(page, now);
        let before = j.written.insert(page, now).unwrap_or(0);
        marks.push((page, before, now));
        for alias in crate::inplace::pages_of(page * PAGE) {
            if alias / PAGE != page {
                whole_4k(alias / PAGE, now);
                j.written.insert(alias / PAGE, now);
            }
        }
    }
    marks
}

/// (Neptune) Whether written-ness is also tracked per 4 KiB sub-page (`VICTORY_WATCH_4K=1`):
/// allows `Shadow::copy_stale` to copy only the modified 4 KiB sub-pages instead of all 64 KiB.
pub(crate) fn watch_4k() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_WATCH_4K").is_some_and(|v| v == "1"))
}

const PART_4K: usize = 1 << 16;

struct Part4k([std::sync::atomic::AtomicU64; PART_4K]);

static WHOLE_4K: [std::sync::atomic::AtomicPtr<Part4k>; 1 << 16] =
    [const { std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()) }; 1 << 16];
static SUBS_4K: [std::sync::atomic::AtomicPtr<Part4k>; 1 << 16] =
    [const { std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()) }; 1 << 16];

fn part_4k(table: &[std::sync::atomic::AtomicPtr<Part4k>; 1 << 16], index: usize, make: bool) -> Option<&'static Part4k> {
    use std::sync::atomic::Ordering::{AcqRel, Acquire};
    let slot = table.get(index / PART_4K)?;
    let mut at = slot.load(Acquire);
    if at.is_null() {
        if !make {
            return None;
        }
        let layout = std::alloc::Layout::new::<Part4k>();
        // SAFETY: zero-initialized array of `AtomicU64` is valid.
        let fresh = unsafe { std::alloc::alloc_zeroed(layout) }.cast::<Part4k>();
        if fresh.is_null() {
            return None;
        }
        match slot.compare_exchange(std::ptr::null_mut(), fresh, AcqRel, Acquire) {
            Ok(_) => at = fresh,
            Err(other) => {
                // SAFETY: ours alone, allocated with `layout`.
                unsafe { std::alloc::dealloc(fresh.cast(), layout) };
                at = other;
            }
        }
    }
    // SAFETY: a part once installed is never freed.
    Some(unsafe { &*at })
}

#[inline]
fn whole_4k(page: usize, now: u64) {
    if watch_4k()
        && let Some(p) = part_4k(&WHOLE_4K, page, true)
    {
        p.0[page % PART_4K].store(now, std::sync::atomic::Ordering::Release);
    }
}

#[inline]
fn found_4k(address: usize, now: u64) {
    if !watch_4k() {
        return;
    }
    let sub = address / KERNEL_PAGE;
    match part_4k(&SUBS_4K, sub, true) {
        Some(p) => p.0[sub % PART_4K].store(now, std::sync::atomic::Ordering::Release),
        None => whole_4k(address / PAGE, now),
    }
}

/// Returns the 16-bit mask of 4 KiB sub-pages of 64 KiB `page` written after `at`,
/// or `None` if the whole 64 KiB page must be copied.
pub(crate) fn stale_mask(page: usize, at: u64) -> Option<u16> {
    use std::sync::atomic::Ordering::Acquire;
    if at == 0 || !watch_4k() {
        return None;
    }
    if part_4k(&WHOLE_4K, page, false).is_some_and(|p| p.0[page % PART_4K].load(Acquire) > at) {
        return None;
    }
    let sub = page * (PAGE / KERNEL_PAGE);
    let part = part_4k(&SUBS_4K, sub, false)?;
    let mut mask = 0u16;
    for k in 0..(PAGE / KERNEL_PAGE) {
        if part.0[(sub + k) % PART_4K].load(Acquire) > at {
            mask |= 1u16 << k;
        }
    }
    (mask != 0).then_some(mask)
}

/// One kernel asking for `pieces` (page ranges in order); by halves if it
/// fails, as a run across two reservations or into memory that is not
/// write-watched does. A piece alone is left to [`writes`].
fn ask_run(j: &mut Journal, pieces: &[(usize, usize)]) {
    if pieces.len() < 2 {
        return;
    }
    let first = pieces[0].0;
    let last = pieces.iter().map(|p| p.1).max().unwrap_or(first);
    if ask_kernel(j, first, last) {
        add_range(&mut j.ahead, first, last);
        return;
    }
    let (a, b) = pieces.split_at(pieces.len() / 2);
    ask_run(j, a);
    ask_run(j, b);
}

/// Asks the kernel for the written pages of pages `[first, last)` and notes
/// them in the journal under the ahead asking; false (and nothing changed)
/// if the range is not write-watched memory of one reservation.
fn ask_kernel(j: &mut Journal, first: usize, last: usize) -> bool {
    ask_kernel_found(j, first, last).is_some()
}

fn ask_kernel_found(j: &mut Journal, first: usize, last: usize) -> Option<usize> {
    use windows_sys::Win32::System::Memory::GetWriteWatch;
    const WRITE_WATCH_FLAG_RESET: u32 = 1;
    let mut count = (last - first) * PAGE / KERNEL_PAGE;
    j.found.resize(count.max(j.found.len()), 0);
    let mut granularity = 0;
    #[cfg(feature = "diagnostics")]
    let started = std::time::Instant::now();
    // SAFETY: the answer buffer holds `count` entries.
    let failed = unsafe {
        GetWriteWatch(
            WRITE_WATCH_FLAG_RESET,
            (first * PAGE) as *const _,
            (last - first) * PAGE,
            j.found.as_mut_ptr().cast(),
            &mut count,
            &mut granularity,
        )
    } != 0;
    if failed {
        return None;
    }
    #[cfg(feature = "diagnostics")]
    {
        use crate::copystat::{C, add};
        add(C::Watches, 1);
        add(C::WatchUs, started.elapsed().as_micros() as u64);
        slow(started.elapsed().as_micros() as u64);
    }
    let now = j.ahead_asked;
    for k in 0..count {
        let addr = j.found[k];
        found_4k(addr, now);
        let page = addr / PAGE;
        j.written.insert(page, now);
    }
    Some(count)
}

/// What [`writes`] would say of `[start, end)` from the journal alone, if
/// the range lies in one asked ahead ([`ask_ahead`], [`ask_once`]): the
/// pages written since tracking began and the asking that last found each
/// written, with the number of the asking that stands (as [`writes`] gives
/// it). `None` if the kernel would have to be asked.
pub(crate) fn asked(start: usize, end: usize) -> Option<(u64, Vec<(usize, u64)>)> {
    let (first, last) = (start / PAGE, end.div_ceil(PAGE));
    let j = JOURNAL.read().unwrap_or_else(|e| e.into_inner());
    j.stands(first, last, true)
        .then(|| (j.ahead_asked, j.written.range(first..last).map(|(&p, &w)| (p, w)).collect()))
}

/// The pages of `[start, end)` (whole pages) as last written, for pages
/// written since tracking began: `(page number, asking)` in page order, and
/// this asking's number, which is higher than any in the list but those the
/// kernel just reported. `None` if the memory is not write-watched (nor,
/// with `guarded`, view memory, which is then guarded).
pub(crate) fn writes(start: usize, end: usize, guarded: bool) -> Option<(u64, Vec<(usize, u64)>)> {
    use std::sync::atomic::Ordering::Relaxed;
    use windows_sys::Win32::System::Memory::GetWriteWatch;
    const WRITE_WATCH_FLAG_RESET: u32 = 1;
    let (first, last) = (start / PAGE, end.div_ceil(PAGE));
    // Fast parallel path 1: range covered by `ask_once` / `ask_ahead`.
    {
        let j = JOURNAL.read().unwrap_or_else(|e| e.into_inner());
        if j.stands(first, last, guarded) {
            return Some((
                j.ahead_asked,
                j.written
                    .range(first..last)
                    .map(|(&p, &w)| (p, w))
                    .collect(),
            ));
        }
        if guarded && watch_read_fast() && !watch_unlocked_ask() {
            let g = guards_shared();
            if let Some(ans) = try_clean_view_read_ex(&j, &g, first, last, guard_absent_fast()) {
                WATCH_FAST_STATS[0].fetch_add(1, Relaxed);
                return Some(ans);
            }
        }
    }
    // Belobug Port 6 (65f9707): view memory the caller asks of the kernel alone
    // (`guarded == false`, e.g. textures) would always fail `GetWriteWatch`: skip
    // both `JOURNAL.write()` and the doomed kernel syscall (~300 calls/frame).
    if !guarded && known_view(first * PAGE) {
        return None;
    }
    if watch_unlocked_ask() {
        let stripes = stripe_indices(first, last);
        if guarded && watch_read_fast() {
            let _sr: Vec<_> = stripes
                .iter()
                .map(|&s| STRIPES[s].read().unwrap_or_else(|e| e.into_inner()))
                .collect();
            let clean = guards_shared().clean.all_present(first, last);
            if clean {
                let j = JOURNAL.read().unwrap_or_else(|e| e.into_inner());
                if j.asked > 0 && first < last {
                    WATCH_FAST_STATS[0].fetch_add(1, Relaxed);
                    return Some((
                        j.asked,
                        j.written
                            .range(first..last)
                            .map(|(&p, &w)| (p, w))
                            .collect(),
                    ));
                }
            }
        }
        let slow_began = std::time::Instant::now();
        WATCH_FAST_STATS[1].fetch_add(1, Relaxed);
        let _sw: Vec<_> = stripes
            .iter()
            .map(|&s| STRIPES[s].write().unwrap_or_else(|e| e.into_inner()))
            .collect();
        {
            let j = JOURNAL.read().unwrap_or_else(|e| e.into_inner());
            if j.stands(first, last, guarded) {
                WATCH_FAST_STATS[2].fetch_add(slow_began.elapsed().as_micros() as u64, Relaxed);
                return Some((
                    j.ahead_asked,
                    j.written
                        .range(first..last)
                        .map(|(&p, &w)| (p, w))
                        .collect(),
                ));
            }
        }
        thread_local! {
            static FOUND_TLS: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        let known_view = guarded && known_view(first * PAGE);
        let mut count = (last - first) * PAGE / KERNEL_PAGE;
        let mut granularity = 0;
        let mut gww_pages: Vec<usize> = Vec::new();
        let failed = if known_view {
            true
        } else {
            WATCH_FAST_STATS[3].fetch_add(1, Relaxed);
            FOUND_TLS.with(|buf| {
                let mut found = buf.borrow_mut();
                if found.len() < count {
                    found.resize(count, 0);
                }
                let f = unsafe {
                    GetWriteWatch(
                        WRITE_WATCH_FLAG_RESET,
                        (first * PAGE) as *const _,
                        (last - first) * PAGE,
                        found.as_mut_ptr().cast(),
                        &mut count,
                        &mut granularity,
                    )
                } != 0;
                if !f && count > 0 {
                    gww_pages.extend_from_slice(&found[..count]);
                }
                f
            })
        };
        let guarded_pages = if failed {
            if !guarded {
                WATCH_FAST_STATS[2].fetch_add(slow_began.elapsed().as_micros() as u64, Relaxed);
                return None;
            }
            WATCH_FAST_STATS[4].fetch_add(1, Relaxed);
            match guard(first * PAGE, last * PAGE) {
                Some(w) => w,
                None => {
                    WATCH_FAST_STATS[2].fetch_add(slow_began.elapsed().as_micros() as u64, Relaxed);
                    return None;
                }
            }
        } else {
            Vec::new()
        };
        let mut j = JOURNAL.write().unwrap_or_else(|e| e.into_inner());
        j.asked += 1;
        let now = j.asked;
        for addr in gww_pages {
            found_4k(addr, now);
            j.written.insert(addr / PAGE, now);
        }
        for page in guarded_pages {
            whole_4k(page, now);
            j.written.insert(page, now);
        }
        let out = j
            .written
            .range(first..last)
            .map(|(&p, &w)| (p, w))
            .collect();
        drop(j);
        WATCH_FAST_STATS[2].fetch_add(slow_began.elapsed().as_micros() as u64, Relaxed);
        return Some((now, out));
    }
    let slow_began = std::time::Instant::now();
    WATCH_FAST_STATS[1].fetch_add(1, Relaxed);
    let mut j = JOURNAL.write().unwrap_or_else(|e| e.into_inner());
    let j = &mut *j;
    // Asked ahead while waiting on the write lock: the journal has it.
    if j.stands(first, last, guarded) {
        WATCH_FAST_STATS[2].fetch_add(slow_began.elapsed().as_micros() as u64, Relaxed);
        return Some((
            j.ahead_asked,
            j.written
                .range(first..last)
                .map(|(&p, &w)| (p, w))
                .collect(),
        ));
    }
    let mut count = (last - first) * PAGE / KERNEL_PAGE;
    j.found.resize(count.max(j.found.len()), 0);
    let mut granularity = 0;
    #[cfg(feature = "diagnostics")]
    let started = std::time::Instant::now();
    // View memory the guards already hold cannot be write-watched: the
    // kernel asking would only fail. Copies of it are flushed many times a
    // frame, each time paying that failed call before the guards answer.
    let known_view = guarded && known_view(first * PAGE);
    if !known_view {
        WATCH_FAST_STATS[3].fetch_add(1, Relaxed);
    }
    // SAFETY: the answer buffer holds `count` entries.
    let failed = known_view
        || unsafe {
            GetWriteWatch(
                WRITE_WATCH_FLAG_RESET,
                (first * PAGE) as *const _,
                (last - first) * PAGE,
                j.found.as_mut_ptr().cast(),
                &mut count,
                &mut granularity,
            )
        } != 0;
    #[cfg(feature = "diagnostics")]
    crate::copystat::add(
        if known_view {
            crate::copystat::C::WatchKnownView
        } else {
            crate::copystat::C::WatchFailed
        },
        failed as u64,
    );
    let guarded = if failed {
        count = 0;
        if !guarded {
            return None;
        }
        WATCH_FAST_STATS[4].fetch_add(1, Relaxed);
        #[cfg(feature = "diagnostics")]
        let guarding = std::time::Instant::now();
        let written = guard(first * PAGE, last * PAGE);
        #[cfg(feature = "diagnostics")]
        crate::copystat::add(crate::copystat::C::GuardUs, guarding.elapsed().as_micros() as u64);
        written?
    } else {
        Vec::new()
    };
    #[cfg(feature = "diagnostics")]
    {
        use crate::copystat::{C, add};
        add(C::Watches, 1);
        add(C::WatchUs, started.elapsed().as_micros() as u64);
        slow(started.elapsed().as_micros() as u64);
    }
    j.asked += 1;
    let now = j.asked;
    for k in 0..count {
        let addr = j.found[k];
        found_4k(addr, now);
        j.written.insert(addr / PAGE, now);
    }
    for page in guarded {
        whole_4k(page, now);
        j.written.insert(page, now);
    }
    WATCH_FAST_STATS[2].fetch_add(
        slow_began.elapsed().as_micros() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );
    Some((
        now,
        j.written
            .range(first..last)
            .map(|(&p, &w)| (p, w))
            .collect(),
    ))
}

impl Tracked {
    /// Takes changed pages of `[from, to)`, cut to the tracked range. A
    /// partial page stays changed so a later take can read its other bytes.
    pub(crate) fn take(&self, from: usize, to: usize) -> Vec<(usize, usize)> {
        let (from, to) = (from.max(self.start), to.min(self.end));
        if from >= to {
            return Vec::new();
        }
        let Some((_, pages)) = writes(from, to, true) else {
            return Vec::new();
        };
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for (page, write) in pages {
            if seen.get(&page).is_some_and(|&last| last >= write) {
                continue;
            }
            let (start, end) = ((page * PAGE).max(from), ((page + 1) * PAGE).min(to));
            if start >= end {
                continue;
            }
            // Keep partial pages dirty: the caller has not taken the rest.
            if (page * PAGE).max(self.start) >= from
                && ((page + 1) * PAGE).min(self.end) <= to
            {
                seen.insert(page, write);
            }
            match ranges.last_mut() {
                Some((_, last)) if *last == start => *last = end,
                _ => ranges.push((start, end)),
            }
        }
        ranges
    }
}

/// Guarded blocks of view memory.
const BLOCK: usize = PAGE;

/// Whether the block holding `start` is view memory the guards already
/// hold (read-only by our hand, or noted absent): asking the kernel's write
/// watch about it can only fail. `VICTORY_VIEW_ASK=1` (or
/// `VICTORY_GUARD_ASK=1`) asks the kernel anyway, as before.
fn known_view(start: usize) -> bool {
    static ASK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let ask = *ASK.get_or_init(|| {
        ["VICTORY_VIEW_ASK", "VICTORY_GUARD_ASK"]
            .iter()
            .any(|n| std::env::var_os(n).is_some_and(|v| v == "1"))
    });
    if ask {
        return false;
    }
    let g = guards_shared();
    let first = start / BLOCK;
    g.protected.contains(&first) || g.absent.contains(&first)
}

/// Compact 256-block (16 MiB) bitset map for guarded view blocks.
/// Replaces per-block `BTreeSet<usize>` allocations (~150,000 blocks/frame)
/// with word-level bitmask checks and zero-allocation range scans.
struct BlockSet {
    regions: std::collections::BTreeMap<usize, [u64; 4]>,
}

impl BlockSet {
    const fn new() -> Self {
        Self { regions: std::collections::BTreeMap::new() }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    #[inline]
    fn contains(&self, block: &usize) -> bool {
        let block = *block;
        let Some(words) = self.regions.get(&(block >> 8)) else {
            return false;
        };
        let bit = block & 255;
        (words[bit >> 6] & (1u64 << (bit & 63))) != 0
    }

    #[inline]
    fn insert(&mut self, block: usize) {
        let words = self.regions.entry(block >> 8).or_insert([0; 4]);
        let bit = block & 255;
        words[bit >> 6] |= 1u64 << (bit & 63);
    }

    #[inline]
    fn remove(&mut self, block: &usize) -> bool {
        let block = *block;
        let key = block >> 8;
        let Some(words) = self.regions.get_mut(&key) else {
            return false;
        };
        let bit = block & 255;
        let mask = 1u64 << (bit & 63);
        let had = (words[bit >> 6] & mask) != 0;
        words[bit >> 6] &= !mask;
        if *words == [0; 4] {
            self.regions.remove(&key);
        }
        had
    }

    fn extend_range(&mut self, first: usize, last: usize) {
        if first >= last {
            return;
        }
        let mut at = first;
        while at < last {
            let reg = at >> 8;
            let end = ((reg + 1) << 8).min(last);
            let words = self.regions.entry(reg).or_insert([0; 4]);
            let mut b = at & 255;
            let b_end = (end - (reg << 8)).min(256);
            while b < b_end {
                let w = b >> 6;
                let lo = b & 63;
                let hi = ((w + 1) << 6).min(b_end) - (w << 6);
                let mask = if hi - lo == 64 { u64::MAX } else { ((1u64 << (hi - lo)) - 1) << lo };
                words[w] |= mask;
                b = (w + 1) << 6;
            }
            at = end;
        }
    }

    fn all_present(&self, first: usize, last: usize) -> bool {
        if first >= last {
            return true;
        }
        let mut at = first;
        while at < last {
            let reg = at >> 8;
            let Some(words) = self.regions.get(&reg) else {
                return false;
            };
            let end = ((reg + 1) << 8).min(last);
            let mut b = at & 255;
            let b_end = (end - (reg << 8)).min(256);
            while b < b_end {
                let w = b >> 6;
                let lo = b & 63;
                let hi = ((w + 1) << 6).min(b_end) - (w << 6);
                let mask = if hi - lo == 64 { u64::MAX } else { ((1u64 << (hi - lo)) - 1) << lo };
                if (words[w] & mask) != mask {
                    return false;
                }
                b = (w + 1) << 6;
            }
            at = end;
        }
        true
    }

    /// True if every block in `[first, last)` is present in `self` OR in `other`.
    fn all_present_with(&self, other: &BlockSet, first: usize, last: usize) -> bool {
        if first >= last {
            return true;
        }
        let mut at = first;
        while at < last {
            let reg = at >> 8;
            let w_a = self.regions.get(&reg);
            let w_b = other.regions.get(&reg);
            if w_a.is_none() && w_b.is_none() {
                return false;
            }
            let end = ((reg + 1) << 8).min(last);
            let mut b = at & 255;
            let b_end = (end - (reg << 8)).min(256);
            while b < b_end {
                let w = b >> 6;
                let lo = b & 63;
                let hi = ((w + 1) << 6).min(b_end) - (w << 6);
                let mask = if hi - lo == 64 { u64::MAX } else { ((1u64 << (hi - lo)) - 1) << lo };
                let combined = w_a.map_or(0, |a| a[w]) | w_b.map_or(0, |b| b[w]);
                if (combined & mask) != mask {
                    return false;
                }
                b = (w + 1) << 6;
            }
            at = end;
        }
        true
    }

    fn count_range(&self, first: usize, last: usize) -> usize {
        if first >= last || self.regions.is_empty() {
            return 0;
        }
        let mut total = 0usize;
        let first_reg = first >> 8;
        let last_reg = (last - 1) >> 8;
        for (&reg, words) in self.regions.range(first_reg..=last_reg) {
            let base = reg << 8;
            let lo_b = first.saturating_sub(base);
            let hi_b = (last - base).min(256);
            let mut b = lo_b;
            while b < hi_b {
                let w = b >> 6;
                let lo = b & 63;
                let hi = ((w + 1) << 6).min(hi_b) - (w << 6);
                let mask = if hi - lo == 64 { u64::MAX } else { ((1u64 << (hi - lo)) - 1) << lo };
                total += (words[w] & mask).count_ones() as usize;
                b = (w + 1) << 6;
            }
        }
        total
    }

    fn collect_range(&self, first: usize, last: usize) -> Vec<usize> {
        let mut out = Vec::new();
        if first >= last || self.regions.is_empty() {
            return out;
        }
        let first_reg = first >> 8;
        let last_reg = (last - 1) >> 8;
        for (&reg, words) in self.regions.range(first_reg..=last_reg) {
            let base = reg << 8;
            let lo_b = first.saturating_sub(base);
            let hi_b = (last - base).min(256);
            let mut b = lo_b;
            while b < hi_b {
                let w = b >> 6;
                let lo = b & 63;
                let hi = ((w + 1) << 6).min(hi_b) - (w << 6);
                let mask = if hi - lo == 64 { u64::MAX } else { ((1u64 << (hi - lo)) - 1) << lo };
                let mut bits = words[w] & mask;
                while bits != 0 {
                    let tz = bits.trailing_zeros() as usize;
                    out.push(base + (w << 6) + tz);
                    bits &= bits - 1;
                }
                b = (w + 1) << 6;
            }
        }
        out
    }

    fn remove_range(&mut self, first: usize, last: usize) {
        if first >= last || self.regions.is_empty() {
            return;
        }
        let first_reg = first >> 8;
        let last_reg = (last - 1) >> 8;
        let mut empty = Vec::new();
        for (&reg, words) in self.regions.range_mut(first_reg..=last_reg) {
            let base = reg << 8;
            let lo_b = first.saturating_sub(base);
            let hi_b = (last - base).min(256);
            let mut b = lo_b;
            while b < hi_b {
                let w = b >> 6;
                let lo = b & 63;
                let hi = ((w + 1) << 6).min(hi_b) - (w << 6);
                let mask = if hi - lo == 64 { u64::MAX } else { ((1u64 << (hi - lo)) - 1) << lo };
                words[w] &= !mask;
                b = (w + 1) << 6;
            }
            if *words == [0; 4] {
                empty.push(reg);
            }
        }
        for reg in empty {
            self.regions.remove(&reg);
        }
    }
}

struct Guards {
    /// Blocks we made read-only.
    protected: BlockSet,
    /// Of those, the ones not written since a copy took them.
    clean: BlockSet,
    /// Blocks already copied as zeros. While they remain wholly uncommitted,
    /// the copy can skip hundreds of MB of failing kernel reads per world frame.
    absent: BlockSet,
    /// Blocks whose physical page was guarded together with its other views
    /// (those views' blocks are in it too): a write to one opens them all.
    /// A write range can be huge (`fread`), so only these are looked up.
    aliased: BlockSet,
}

static GUARDS: std::sync::RwLock<Guards> = std::sync::RwLock::new(Guards {
    protected: BlockSet::new(),
    clean: BlockSet::new(),
    absent: BlockSet::new(),
    aliased: BlockSet::new(),
});

fn guards() -> std::sync::RwLockWriteGuard<'static, Guards> {
    GUARDS.write().unwrap_or_else(|e| e.into_inner())
}

fn guards_shared() -> std::sync::RwLockReadGuard<'static, Guards> {
    GUARDS.read().unwrap_or_else(|e| e.into_inner())
}

/// For view memory: the pages of `[start, end)`'s blocks written since
/// they were last guarded (all of a block's pages), guarding them again
/// before the caller copies them. `None` if the memory is not a view.
fn guard(start: usize, end: usize) -> Option<Vec<usize>> {
    // A range whose first block the guards hold already (read-only by our
    // hand, or noted absent) is a view: no question to Windows. The
    // question walks the page tables to the end of the pages that share the
    // first one's state, and a guarded view is one long run of read-only
    // pages: 0.6 ms for the title's 174 MB heap, 16 times a frame, and most
    // of the 68 ms a frame of "guards" (run 58).
    // `VICTORY_GUARD_ASK=1` asks every time, as before.
    static ASK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let known = !*ASK.get_or_init(|| std::env::var_os("VICTORY_GUARD_ASK").is_some_and(|v| v == "1")) && {
        if watch_read_fast() {
            let g = guards_shared();
            let (first, last) = (start / BLOCK, end.div_ceil(BLOCK));
            if g.clean.all_present(first, last) {
                return Some(Vec::new());
            }
            g.protected.contains(&first) || g.absent.contains(&first)
        } else {
            let g = guards();
            let first = start / BLOCK;
            g.protected.contains(&first) || g.absent.contains(&first)
        }
    };
    if !known && !is_view(start, end) {
        return None;
    }
    install();
    let mut g = guards();
    let mut written = Vec::new();
    let mut run: Option<usize> = None;
    let (first, last) = (start / BLOCK, end.div_ceil(BLOCK));
    // Zero-allocation fast path: nearly all queried view ranges are wholly clean.
    if g.clean.all_present(first, last) {
        return Some(written);
    }
    let mut uncommitted_until = first;
    for block in first..=last {
        let is_clean = block < last && g.clean.contains(&block);
        let mut open = block < last && !is_clean;
        if open && g.absent.contains(&block) {
            // Ordinary commits need no runtime mapping notification. Reuse
            // this asking's query for the following blocks in the same region.
            if block >= uncommitted_until {
                uncommitted_until = uncommitted_end(block);
            }
            open = block >= uncommitted_until;
            if open {
                g.absent.remove(&block);
            }
        }
        if open {
            written.extend(block * BLOCK / PAGE..(block + 1) * BLOCK / PAGE);
            if g.protected.contains(&block) {
                // Written through the runtime, still read-only.
                g.clean.insert(block);
            } else {
                run.get_or_insert(block);
                continue;
            }
        }
        if let Some(from) = run.take() {
            protect(&mut g, from, block);
        }
    }
    Some(written)
}

/// Whether `[start, end)` is view memory. The title's views lie in
/// placeholders, mapped and unmapped block by block as it streams, so a
/// range can begin in a block that is not mapped right now: the first
/// pages of the range that are there decide. A range with nothing there
/// counts as a view (its blocks are noted absent, and looked at again when
/// the runtime maps them).
///
/// Looking at the first address only took such a range for "not a view":
/// its copy was marked not watched for good and copied whole at every
/// refresh, around its unmapped pages (run 54: 300 such ranges, 570 MB
/// copied a frame for them, 255 copies a frame reading around holes).
/// `VICTORY_GUARD_HOLES=0` looks at the first address only, as before.
fn is_view(start: usize, end: usize) -> bool {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_MAPPED, MEMORY_BASIC_INFORMATION, VirtualQuery,
    };
    static HOLES: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let holes = *HOLES.get_or_init(|| std::env::var_os("VICTORY_GUARD_HOLES").is_none_or(|v| v != "0"));
    let mut at = start;
    loop {
        // SAFETY: plain data filled in by the query.
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a valid out pointer of the right size.
        let n = unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) };
        if n == 0 {
            return false;
        }
        if info.Type == MEM_MAPPED {
            return true;
        }
        // Committed memory that is no view (the title's heaps, ours), or
        // the answer as it was before.
        if info.State == MEM_COMMIT || !holes {
            return false;
        }
        // A hole (a placeholder, or free): on to what follows it.
        let next = (info.BaseAddress as usize).saturating_add(info.RegionSize);
        if next <= at || next >= end {
            return true;
        }
        at = next;
    }
}

/// Makes blocks `[from, to)` read-only and notes them guarded and clean;
/// block by block if they span views, leaving out any that fail, and
/// noting those not committed (unmapped) absent. The same physical pages
/// at every other view of them are made read-only too: a write through
/// one of those is this memory written, and only a fault can say so.
/// `VICTORY_VIEW_GUARD=0` leaves the other views as they are, as before.
fn protect(g: &mut Guards, from: usize, to: usize) {
    protect_here(g, from, to);
    if !view_guard() {
        return;
    }
    for (base, bytes) in crate::inplace::views_between(from.saturating_mul(BLOCK), to.saturating_mul(BLOCK)) {
        let first = (base / BLOCK).max(from);
        let last = (base.saturating_add(bytes).div_ceil(BLOCK)).min(to);
        for block in first..last {
            let shown = crate::inplace::pages_of(block * BLOCK);
            // One view (nothing else to guard), or the page the title shows
            // wherever it has nothing (thousands of views): each view of
            // that one is guarded by itself.
            if shown.len() < 2 || shown.len() > MANY_VIEWS {
                continue;
            }
            #[cfg(feature = "diagnostics")]
            ALIASES[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            for alias in shown {
                let alias = alias / BLOCK;
                g.aliased.insert(alias);
                if alias != block {
                    protect_alias(g, alias);
                }
            }
        }
    }
}

/// Views of one page past which its views are not guarded together.
pub(crate) const MANY_VIEWS: usize = 64;

/// Whether all the views of a guarded page are guarded together
/// (`VICTORY_VIEW_GUARD`).
fn view_guard() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_VIEW_GUARD").is_none_or(|v| v != "0"))
}

/// Diagnostics: blocks guarded together with their other views, blocks
/// opened because another view of theirs was written, and blocks opened
/// because a view of theirs was mapped; since the start.
#[cfg(feature = "diagnostics")]
pub(crate) static ALIASES: [std::sync::atomic::AtomicU64; 3] = [const { std::sync::atomic::AtomicU64::new(0) }; 3];

/// Makes `block` read-only: another view of pages a copy has just taken
/// through its own view, so that a write through this one faults too. It is
/// not noted clean. The copies of title memory at this view have not taken
/// the pages: noted clean, the block would not be reported written at their
/// next check and they would keep the bytes from before the write (`guard`
/// reports a block that is read-only and not clean as written, then notes it
/// clean). A block that is clean stays so: had the pages been written since
/// its copies took them, `open` would have opened it.
fn protect_alias(g: &mut Guards, block: usize) {
    use windows_sys::Win32::System::Memory::{PAGE_READONLY, VirtualProtect};
    if g.protected.contains(&block) {
        return;
    }
    let mut old = 0;
    // SAFETY: title view memory; only its protection changes.
    if unsafe { VirtualProtect((block * BLOCK) as *const _, BLOCK, PAGE_READONLY, &mut old) } != 0 {
        g.protected.insert(block);
    }
}

/// [`protect`] for one range, without the other views of its pages.
fn protect_here(g: &mut Guards, from: usize, to: usize) {
    use windows_sys::Win32::System::Memory::{PAGE_READONLY, VirtualProtect};
    let set = |from: usize, to: usize| {
        let mut old = 0;
        // SAFETY: title view memory; only its protection changes.
        unsafe {
            VirtualProtect(
                (from * BLOCK) as *const _,
                (to - from) * BLOCK,
                PAGE_READONLY,
                &mut old,
            ) != 0
        }
    };
    let note = |g: &mut Guards, from: usize, to: usize| {
        g.protected.extend_range(from, to);
        g.clean.extend_range(from, to);
    };
    if set(from, to) {
        note(g, from, to);
        return;
    }
    let mut block = from;
    while block < to {
        if set(block, block + 1) {
            note(g, block, block + 1);
            block += 1;
        } else {
            let until = uncommitted_end(block).min(to);
            if until > block {
                // Skip the entire uncommitted hole in ONE query instead of 48+ syscalls!
                g.absent.extend_range(block, until);
                block = until;
            } else {
                block += 1;
            }
        }
    }
}

/// The end (exclusive) of the whole uncommitted blocks starting at `block`.
fn uncommitted_end(block: usize) -> usize {
    use windows_sys::Win32::System::Memory::{
        MEM_FREE, MEM_RESERVE, MEMORY_BASIC_INFORMATION, VirtualQuery,
    };
    // SAFETY: plain data filled in by the query.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a valid out pointer of the right size.
    let n = unsafe { VirtualQuery((block * BLOCK) as *const _, &mut info, size_of_val(&info)) };
    if n != 0 && matches!(info.State, MEM_RESERVE | MEM_FREE) {
        (info.BaseAddress as usize).saturating_add(info.RegionSize) / BLOCK
    } else {
        block
    }
}

/// Gives write access back to the guarded blocks of `[start, end)` and
/// notes them written; true if any was guarded. The same physical pages at
/// every other view of them are opened too: the write that faulted (or is
/// ours, `crate::host::copy_mapped`) goes through one view, and every
/// guard over the pages must copy them again.
fn open(g: &mut Guards, start: usize, end: usize) -> bool {
    let any = open_here(g, start, end);
    if start >= end || g.aliased.is_empty() {
        return any;
    }
    let aliased = g.aliased.collect_range(start / BLOCK, end.div_ceil(BLOCK));
    for block in aliased {
        g.aliased.remove(&block);
        for alias in crate::inplace::pages_of(block * BLOCK) {
            if alias / BLOCK != block && open_here(g, alias, alias.saturating_add(BLOCK)) {
                #[cfg(feature = "diagnostics")]
                ALIASES[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            g.aliased.remove(&(alias / BLOCK));
        }
    }
    any
}

/// [`open`] for one range, without the other views of its pages.
/// Coalesces contiguous protected blocks into batched `VirtualProtect` calls.
fn open_here(g: &mut Guards, start: usize, end: usize) -> bool {
    use windows_sys::Win32::System::Memory::{PAGE_READWRITE, VirtualProtect};
    let first = start / BLOCK;
    let last = end.div_ceil(BLOCK);
    let blocks = g.protected.collect_range(first, last);
    if blocks.is_empty() {
        return false;
    }
    g.protected.remove_range(first, last);
    g.clean.remove_range(first, last);
    let mut i = 0;
    while i < blocks.len() {
        let run_start = blocks[i];
        let mut run_end = run_start + 1;
        i += 1;
        while i < blocks.len() && blocks[i] == run_end {
            run_end += 1;
            i += 1;
        }
        let mut old = 0;
        // SAFETY: contiguous blocks we made read-only.
        if unsafe {
            VirtualProtect(
                (run_start * BLOCK) as *const _,
                (run_end - run_start) * BLOCK,
                PAGE_READWRITE,
                &mut old,
            )
        } == 0
        {
            // Fallback per block if the run spans separate views/VADs.
            for block in run_start..run_end {
                let mut old = 0;
                // SAFETY: a block we made read-only.
                unsafe { VirtualProtect((block * BLOCK) as *const _, BLOCK, PAGE_READWRITE, &mut old) };
            }
        }
    }
    true
}

/// Before our own write into title memory `[at, at + len)` through the
/// kernel (which does not fault into our handler): opens its guarded blocks.
/// The runtime calls it (as `kernel_will_write`) before the title's kernel
/// reads into its own memory.
pub fn writing(at: usize, len: usize) {
    crate::stamp::advance();
    // The length is the title's (`fread`'s size times count, saturated).
    open(&mut guards(), at, at.saturating_add(len));
    // A placed texture there changes with no descriptor written.
    crate::descriptor::host::remapped(at, len);
}

/// What `VICTORY_PAGE_WRITES` asks to be done with a write the guards do
/// not see once its bytes are there ([`title_memory_written`]), as bits:
/// 1, the guarded blocks under it are opened and count as written (host
/// copies of buffers and textures read through tables take them again);
/// 2, the placed textures picked by index that lie under it are due again;
/// 4, both also for reads to plain addresses when they have finished.
/// Unset or `guards`: bit 1 alone. `0`: nothing, counted only (the default
/// until run 199). `1`: bits 1 and 2. `slots`: bit 2 alone. `all`:
/// everything.
/// (Bit 1 by default: in every run 10,000 to 50,000 guarded blocks of host
/// copies lay under such writes and kept the bytes from before; runs 197
/// and 198 opened them, 9,766 and 10,796 blocks, with nothing worse for
/// it. Bit 2 makes texture slots due and is the one that flooded the
/// queue in an earlier form: still asked for by name.)
pub(crate) fn page_writes() -> u8 {
    static MODE: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("VICTORY_PAGE_WRITES").as_deref().map(str::trim) {
        Ok("0") => 0,
        Ok("1") => 3,
        Ok("slots") => 2,
        Ok("all") => 7,
        _ => 1,
    })
}

/// Since the start: writes into physical pages through the runtime's own
/// views that landed at addresses the title has mapped (one for each run
/// of addresses); their bytes; guarded blocks that lay under them (host
/// copies that would otherwise keep the bytes from before); reads to
/// plain addresses that finished.
pub(crate) static WRITTEN: [std::sync::atomic::AtomicU64; 4] = [const { std::sync::atomic::AtomicU64::new(0) }; 4];

/// The bytes of a write the guards do not see are there: title memory
/// `[at, at + len)`. `pages`: DirectStorage wrote them into physical pages
/// through a view of the runtime's own, and this is an address where the
/// title has those pages mapped (no title address faulted, nothing was
/// mapped or unmapped; the shortcut this file's header names). Otherwise a
/// read to a plain address has finished, of which [`writing`] was told
/// before it began.
///
/// Upstream's c75fdba does the same for its trackers (its `Writing` marks
/// the pages once more when dropped; page destinations are reported at
/// each address showing them). Here it is counted always and acted on as
/// [`page_writes`] says. Called without the runtime's page-table lock;
/// takes the guards' lock, then, alone, the lock of the ranges kept for
/// the texture pass.
pub fn title_memory_written(at: usize, len: usize, pages: bool) {
    use std::sync::atomic::Ordering::Relaxed;
    if len == 0 {
        return;
    }
    crate::stamp::advance();
    let mode = page_writes();
    let end = at.saturating_add(len);
    if pages {
        WRITTEN[0].fetch_add(1, Relaxed);
        WRITTEN[1].fetch_add(len as u64, Relaxed);
    } else {
        WRITTEN[3].fetch_add(1, Relaxed);
        if mode & 4 == 0 {
            return;
        }
    }
    {
        let mut g = guards();
        let guarded = g.protected.count_range(at / BLOCK, end.div_ceil(BLOCK)) as u64;
        if pages {
            WRITTEN[2].fetch_add(guarded, Relaxed);
        }
        if mode & 1 != 0 && guarded != 0 {
            open(&mut g, at, end);
        }
    }
    // A placed texture there changed with no descriptor written.
    crate::descriptor::host::page_written(at, len);
}

/// The runtime mapped a view of the physical bytes `[offset, offset + len)`
/// of `section` (`crate::inplace::title_memory_view`): the view is
/// writable, so the same bytes guarded at another view must be checked
/// again — and are, the pages noted written, which also has the next check
/// guard the new view (`protect`).
pub(crate) fn views_mapped(section: usize, offset: u64, len: usize) {
    if !view_guard() {
        return;
    }
    let ranges = crate::inplace::views_of_section(section, offset, len);
    if ranges.is_empty() {
        return;
    }
    let mut g = guards();
    for &(at, n) in &ranges {
        if open_here(&mut g, at, at.saturating_add(n)) {
            #[cfg(feature = "diagnostics")]
            ALIASES[2].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Held shared by a read of title memory in place, and alone by the runtime
/// while it changes what title memory shows.
///
/// The runtime does not change a mapping in one step. To unmap one page of
/// a view it unmaps the whole view and maps what stays again
/// (`xmem.rs` `unmap`), and a view it maps is a reservation until the next
/// call commits it (`map`, then `commit`). For that moment pages the title
/// never gave up are "reserved, never committed", and a thread reading
/// them in place takes an access violation: run 71 in `Layout::detile`,
/// run 74 in the hash before it, both under `Staging::refresh_now`, which
/// had asked whether the texture was all mapped and been told yes. Asking
/// cannot help; not changing the mapping while such a read is under way
/// does.
///
/// Only with `VICTORY_MAPPING_RANGES=0` ([`by_range`]); otherwise reads and
/// changes hold the bytes they touch ([`SPANS`]).
static MAPPING: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// Whether a read in place and a change of mapping keep apart only when
/// their bytes meet ([`SPANS`]), not whatever memory each touches
/// ([`MAPPING`]); `VICTORY_MAPPING_RANGES=0` holds all of title memory for
/// each, as before. On by default.
///
/// With one lock over all of title memory each texture refresh waited for
/// every change of mapping anywhere, and each change for the refresh under
/// way. While the world streams in the title changes mappings 650-1,100
/// times a frame, many a page at a time (`XMemMapPhysicalPagesScatter`
/// maps each page on its own). With the one lock the refreshes waited 203
/// ms of an 801 ms frame for title memory to hold still; holding only the
/// bytes touched, 0 ms of a 486 ms frame (medians, a pair of runs).
pub(crate) fn by_range() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_MAPPING_RANGES").is_none_or(|v| v != "0"))
}

/// Reads of title memory in place under way, and changes of its mapping,
/// by the bytes they touch ([`by_range`]).
struct Spans {
    /// To tell them apart.
    next: u64,
    /// Reads under way: number, start and end.
    reads: Vec<(u64, usize, usize)>,
    /// Changes: number, start, end, and whether under way (else waiting for
    /// the reads over its bytes to end; no read begins over them meanwhile,
    /// so that one read after another cannot keep a change waiting).
    changes: Vec<(u64, usize, usize, bool)>,
    /// Threads waiting on [`SPANS_LEFT`].
    waiting: usize,
}

static SPANS: Mutex<Spans> = Mutex::new(Spans {
    next: 0,
    reads: Vec::new(),
    changes: Vec::new(),
    waiting: 0,
});

/// Told when a read or a change ends while a thread waits.
static SPANS_LEFT: std::sync::Condvar = std::sync::Condvar::new();

fn spans() -> std::sync::MutexGuard<'static, Spans> {
    SPANS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Waits until a read or a change ends.
fn spans_wait(mut s: std::sync::MutexGuard<'static, Spans>) -> std::sync::MutexGuard<'static, Spans> {
    s.waiting += 1;
    let mut s = SPANS_LEFT.wait(s).unwrap_or_else(|e| e.into_inner());
    s.waiting -= 1;
    s
}

/// Whether `[a, b)` and `[c, d)` share a byte.
fn meet(a: usize, b: usize, c: usize, d: usize) -> bool {
    a < d && c < b
}

impl Spans {
    fn read(&mut self, at: usize, end: usize) -> Span {
        self.next += 1;
        self.reads.push((self.next, at, end));
        Span(self.next, false)
    }
}

/// A read's or a change's place in [`SPANS`] (its number, and whether a
/// change), given up when dropped.
struct Span(u64, bool);

impl Drop for Span {
    fn drop(&mut self) {
        let mut s = spans();
        if self.1 {
            s.changes.retain(|c| c.0 != self.0);
        } else if let Some(i) = s.reads.iter().position(|r| r.0 == self.0) {
            s.reads.swap_remove(i);
        }
        if s.waiting > 0 {
            SPANS_LEFT.notify_all();
        }
    }
}

/// What a read or a change of mapping holds: its bytes ([`by_range`]) or
/// the lock over all of title memory.
enum Held<G> {
    All(#[allow(dead_code)] G),
    Bytes(#[allow(dead_code)] Span),
}

/// Since the last frame-rate line: changes of title memory's mapping, the
/// microseconds they waited for reads in place, and those they were under
/// way.
pub(crate) static REMAPS: [std::sync::atomic::AtomicU64; 3] = [const { std::sync::atomic::AtomicU64::new(0) }; 3];

/// What the runtime holds while it changes what title memory shows
/// ([`title_memory_remapping`]).
pub struct Remapping {
    _held: Held<std::sync::RwLockWriteGuard<'static, ()>>,
    since: std::time::Instant,
}

impl Drop for Remapping {
    fn drop(&mut self) {
        REMAPS[2].fetch_add(self.since.elapsed().as_micros() as u64, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The runtime is about to map, unmap or move physical pages of title
/// memory `[at, at + len)`: until what it returns is dropped, no read of
/// those bytes in place is under way and none begins
/// ([`title_memory_reading`]). Taken around the whole change, the unmapping
/// and the mapping again, over all the bytes whose mapping it changes.
pub fn title_memory_remapping(at: usize, len: usize) -> Remapping {
    use std::sync::atomic::Ordering::Relaxed;
    let started = std::time::Instant::now();
    let held = if by_range() {
        let end = at.saturating_add(len);
        let mut s = spans();
        s.next += 1;
        let id = s.next;
        s.changes.push((id, at, end, false));
        while s.reads.iter().any(|r| meet(r.1, r.2, at, end))
            || s.changes.iter().any(|c| c.3 && meet(c.1, c.2, at, end))
        {
            s = spans_wait(s);
        }
        if let Some(c) = s.changes.iter_mut().find(|c| c.0 == id) {
            c.3 = true;
        }
        Held::Bytes(Span(id, true))
    } else {
        Held::All(MAPPING.write().unwrap_or_else(|e| e.into_inner()))
    };
    REMAPS[0].fetch_add(1, Relaxed);
    REMAPS[1].fetch_add(started.elapsed().as_micros() as u64, Relaxed);
    Remapping {
        _held: held,
        since: std::time::Instant::now(),
    }
}

/// What a read of title memory in place holds ([`title_memory_reading`]).
pub struct Reading(#[allow(dead_code)] Held<std::sync::RwLockReadGuard<'static, ()>>);

/// For reading title memory `[at, at + len)` in place: until what it
/// returns is dropped, the runtime changes no mapping of those bytes, so
/// memory found mapped stays mapped. Not to be taken twice by one thread,
/// nor held across anything that maps title memory.
pub fn title_memory_reading(at: usize, len: usize) -> Reading {
    if !by_range() {
        return Reading(Held::All(MAPPING.read().unwrap_or_else(|e| e.into_inner())));
    }
    let end = at.saturating_add(len);
    let mut s = spans();
    while s.changes.iter().any(|c| meet(c.1, c.2, at, end)) {
        s = spans_wait(s);
    }
    Reading(Held::Bytes(s.read(at, end)))
}

/// [`title_memory_reading`] if it can be had at once: `None` while the
/// runtime changes a mapping of those bytes (or waits to), for a caller
/// that has another way to read and would rather not wait.
pub fn title_memory_reading_now(at: usize, len: usize) -> Option<Reading> {
    if !by_range() {
        return match MAPPING.try_read() {
            Ok(guard) => Some(Reading(Held::All(guard))),
            Err(std::sync::TryLockError::Poisoned(e)) => Some(Reading(Held::All(e.into_inner()))),
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
    }
    let end = at.saturating_add(len);
    let mut s = spans();
    if s.changes.iter().any(|c| meet(c.1, c.2, at, end)) {
        return None;
    }
    Some(Reading(Held::Bytes(s.read(at, end))))
}

/// The runtime mapped or unmapped title memory `[at, at + len)`: what it
/// shows is new and read-write, so it is no longer guarded (nor known
/// unmapped).
pub fn title_memory_remapped(at: usize, len: usize) {
    crate::stamp::advance();
    // A placed texture there arrives, goes or changes with no descriptor
    // written.
    crate::descriptor::host::remapped(at, len);
    // The views the runtime told us of there are no longer what it shows.
    crate::inplace::views_gone(at, len);
    // And nothing stays in place over it.
    crate::inplace::title_memory_freeing(at, len);
    let mut g = guards();
    let first = at / BLOCK;
    let last = at.saturating_add(len).div_ceil(BLOCK);
    g.absent.remove_range(first, last);
    g.protected.remove_range(first, last);
    g.clean.remove_range(first, last);
    g.aliased.remove_range(first, last);
}

/// Registers the handler that takes writes to guarded blocks, once, ahead
/// of the crash reporter's.
fn install() {
    use windows_sys::Win32::Foundation::{EXCEPTION_ACCESS_VIOLATION, NTSTATUS};
    use windows_sys::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_CONTINUE_SEARCH,
        EXCEPTION_POINTERS,
    };
    unsafe extern "system" fn on_write(info: *mut EXCEPTION_POINTERS) -> i32 {
        // SAFETY: the system passes the exception being dispatched.
        let r = unsafe { &*(*info).ExceptionRecord };
        const READ: usize = 0;
        const WRITE: usize = 1;
        if r.ExceptionCode != EXCEPTION_ACCESS_VIOLATION as NTSTATUS || r.NumberParameters < 2 {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        let kind = r.ExceptionInformation[0];
        let at = r.ExceptionInformation[1];
        if kind != WRITE {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        // Held across both checks: another thread may have opened the block
        // since this one faulted, and a copy may have guarded it again since
        // (then it is ours again, not read-only for some other reason).
        let mut g = guards();
        if open(&mut g, at, at + 1) {
            #[cfg(feature = "diagnostics")]
            crate::copystat::add(crate::copystat::C::GuardFaults, 1);
            return EXCEPTION_CONTINUE_EXECUTION;
        }
        if writable(at) {
            EXCEPTION_CONTINUE_EXECUTION
        } else {
            EXCEPTION_CONTINUE_SEARCH
        }
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: the handler only touches our guard state and protections.
    ONCE.call_once(|| unsafe {
        AddVectoredExceptionHandler(1, Some(on_write));
    });
}

/// Whether `at` is committed read-write memory now.
fn writable(at: usize) -> bool {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_READWRITE, VirtualQuery,
    };
    // SAFETY: plain data filled in by the query.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a valid out pointer of the right size.
    let n = unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) };
    n != 0 && info.State == MEM_COMMIT && info.Protect == PAGE_READWRITE
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
    };
    const MEM_WRITE_WATCH: u32 = 0x20_0000;

    #[test]
    fn written_pages_are_reported_to_every_asker() {
        // SAFETY: a fresh write-watched allocation, freed at the end.
        let p = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                4 * PAGE,
                MEM_RESERVE | MEM_COMMIT | MEM_WRITE_WATCH,
                PAGE_READWRITE,
            )
        } as usize;
        assert_ne!(p, 0);
        let (start, end) = (p, p + 4 * PAGE);
        let (_, before) = writes(start, end, false).expect("watched");
        assert!(before.is_empty());
        // SAFETY: committed pages of ours.
        unsafe { ((p + PAGE + 8) as *mut u8).write(1) };
        let (now, after) = writes(start, end, false).expect("watched");
        assert_eq!(after, [(p / PAGE + 1, now)]);
        // Another asker after the kernel's bit was cleared still sees it.
        let (later, again) = writes(p + PAGE, p + 2 * PAGE, false).expect("watched");
        assert!(later > now);
        assert_eq!(again, [(p / PAGE + 1, now)]);
        let mut plain = vec![0u8; PAGE];
        assert!(
            writes(
                plain.as_mut_ptr() as usize,
                plain.as_ptr() as usize + PAGE,
                true
            )
            .is_none()
        );
        // SAFETY: our allocation.
        unsafe { VirtualFree(p as *mut _, 0, MEM_RELEASE) };
    }

    /// A kernel read into a guarded view block fails; opened first, it
    /// lands and the block counts as written.
    #[test]
    fn kernel_reads_into_guarded_blocks_after_opening() {
        use std::io::{Read, Seek, Write};
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, FILE_MAP_WRITE, MapViewOfFile, UnmapViewOfFile,
        };
        // SAFETY: a pagefile section of two blocks, mapped read-write.
        let view = unsafe {
            let section = CreateFileMappingW(
                -1isize as _,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                (2 * BLOCK) as u32,
                std::ptr::null(),
            );
            assert!(!section.is_null());
            MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, 2 * BLOCK).Value as usize
        };
        assert_ne!(view, 0);
        assert!(guard(view, view + 2 * BLOCK).is_some()); // now read-only
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"from the disk").unwrap();
        // `ReadFile` inside: the kernel writes the view.
        let read = |file: &mut std::fs::File| {
            file.rewind().unwrap();
            // SAFETY: 13 bytes of our view, nothing else refers to them.
            let into = unsafe { std::slice::from_raw_parts_mut((view + 100) as *mut u8, 13) };
            file.read(into).is_ok_and(|n| n == 13)
        };
        assert!(
            !read(&mut file),
            "the kernel does not fault into our handler"
        );
        writing(view + 100, 13);
        assert!(read(&mut file));
        // SAFETY: the bytes just read.
        let landed = unsafe { std::slice::from_raw_parts((view + 100) as *const u8, 13) };
        assert_eq!(landed, b"from the disk");
        assert_eq!(
            guard(view, view + 2 * BLOCK),
            Some((view / PAGE..view / PAGE + 1).collect())
        );
        // SAFETY: our view.
        unsafe {
            UnmapViewOfFile(
                windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: view as *mut _,
                },
            )
        };
        title_memory_remapped(view, 2 * BLOCK);
    }

    #[test]
    fn writes_in_partly_committed_view_blocks_are_reported() {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
            SEC_RESERVE, UnmapViewOfFile,
        };

        const SIZE: usize = 3 * BLOCK;
        struct View(MEMORY_MAPPED_VIEW_ADDRESS);
        impl Drop for View {
            fn drop(&mut self) {
                title_memory_remapped(self.0.Value as usize, SIZE);
                // SAFETY: our view is still mapped.
                unsafe { UnmapViewOfFile(self.0) };
            }
        }
        // SAFETY: a new reserved pagefile section. The view holds its own
        // reference after we close the handle, including on assertion failure.
        let view = unsafe {
            let section = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE | SEC_RESERVE,
                0,
                SIZE as u32,
                std::ptr::null(),
            );
            assert!(!section.is_null());
            let view = MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, SIZE);
            CloseHandle(section);
            assert!(!view.Value.is_null());
            View(view)
        };
        let start = view.0.Value as usize;
        let commit = |at: usize, len| {
            // SAFETY: these pages belong to our reserved view.
            let committed = unsafe { VirtualAlloc(at as _, len, MEM_COMMIT, PAGE_READWRITE) };
            assert_eq!(committed as usize, at);
        };
        let early = start + BLOCK + 3 * KERNEL_PAGE;
        let late = start + 2 * BLOCK + 3 * KERNEL_PAGE;
        commit(start, BLOCK);
        commit(early, KERNEL_PAGE);
        let refresh = || guard(start, start + SIZE).unwrap();
        assert_eq!(refresh(), [start / PAGE, early / PAGE, late / PAGE]);
        // The last block is still entirely reserved.
        assert!(!refresh().contains(&(late / PAGE)));

        commit(late, KERNEL_PAGE); // Ordinary commit, without a runtime mapping notification.
        for at in [early, late] {
            assert!(writable(at));
            // SAFETY: each address is inside its committed read-write page.
            unsafe { (at as *mut u8).write_volatile(1) };
        }
        assert_eq!(refresh(), [early / PAGE, late / PAGE]);
    }

    /// A range of view memory whose first block is not mapped is a view
    /// all the same: its blocks are reported once, then the ones written
    /// or mapped since.
    #[test]
    fn a_range_that_begins_in_an_unmapped_block_is_guarded() {
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, MEM_PRESERVE_PLACEHOLDER, MEM_REPLACE_PLACEHOLDER,
            MEM_RESERVE_PLACEHOLDER, MapViewOfFile3, PAGE_NOACCESS, UnmapViewOfFile2,
            VirtualAlloc2,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        // SAFETY: a fresh placeholder of three blocks and a pagefile
        // section of three; the middle block is mapped first.
        let (p, section) = unsafe {
            let p = VirtualAlloc2(
                GetCurrentProcess(),
                std::ptr::null(),
                3 * BLOCK,
                MEM_RESERVE | MEM_RESERVE_PLACEHOLDER,
                PAGE_NOACCESS,
                std::ptr::null_mut(),
                0,
            ) as usize;
            assert_ne!(p, 0);
            let section = CreateFileMappingW(
                -1isize as _,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                (3 * BLOCK) as u32,
                std::ptr::null(),
            );
            assert!(!section.is_null());
            (p, section)
        };
        let map = |block: usize| {
            // SAFETY: splits the placeholder at the block and maps the
            // section's block there.
            let view = unsafe {
                VirtualFree(
                    (p + block * BLOCK) as *mut _,
                    BLOCK,
                    MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER,
                );
                MapViewOfFile3(
                    section,
                    GetCurrentProcess(),
                    (p + block * BLOCK) as *const _,
                    (block * BLOCK) as u64,
                    BLOCK,
                    MEM_REPLACE_PLACEHOLDER,
                    PAGE_READWRITE,
                    std::ptr::null_mut(),
                    0,
                )
            };
            assert_eq!(view.Value as usize, p + block * BLOCK);
            view
        };
        let pages = |block: usize| (p + block * BLOCK) / PAGE..(p + (block + 1) * BLOCK) / PAGE;
        let all: Vec<usize> = (0..3).flat_map(pages).collect();
        // Nothing mapped at all: a view with every block absent.
        assert!(is_view(p, p + 3 * BLOCK));
        let middle = map(1);
        title_memory_remapped(p + BLOCK, BLOCK);
        // The range begins in the placeholder, its middle block is a view.
        assert!(is_view(p, p + 3 * BLOCK));
        assert_eq!(guard(p, p + 3 * BLOCK), Some(all));
        assert_eq!(guard(p, p + 3 * BLOCK), Some(Vec::new()), "nothing written since");
        // SAFETY: inside the mapped middle block.
        unsafe { ((p + BLOCK + 8) as *mut u8).write_volatile(1) };
        assert_eq!(guard(p, p + 3 * BLOCK), Some(pages(1).collect()));
        let first = map(0);
        title_memory_remapped(p, BLOCK);
        assert_eq!(guard(p, p + 3 * BLOCK), Some(pages(0).collect()));
        assert_eq!(guard(p, p + 3 * BLOCK), Some(Vec::new()));
        // Memory of ours that is no view stays none.
        let mut local = [0u8; 16];
        let at = local.as_mut_ptr() as usize;
        assert!(!is_view(at, at + local.len()));
        // SAFETY: our views and placeholder.
        unsafe {
            UnmapViewOfFile2(GetCurrentProcess(), first, MEM_PRESERVE_PLACEHOLDER);
            UnmapViewOfFile2(GetCurrentProcess(), middle, MEM_PRESERVE_PLACEHOLDER);
            for block in 0..3 {
                VirtualFree((p + block * BLOCK) as *mut _, 0, MEM_RELEASE);
            }
            windows_sys::Win32::Foundation::CloseHandle(section);
        }
        title_memory_remapped(p, 3 * BLOCK);
    }

    /// Blocks of a placeholder between views are reported once, then not
    /// again (they cannot be guarded) until the runtime maps them.
    #[test]
    fn unmapped_blocks_are_reported_once_until_mapped() {
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, MEM_PRESERVE_PLACEHOLDER, MEM_REPLACE_PLACEHOLDER,
            MEM_RESERVE_PLACEHOLDER, MapViewOfFile3, PAGE_NOACCESS, UnmapViewOfFile2,
            VirtualAlloc2,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        // SAFETY: a fresh placeholder of three blocks and a pagefile
        // section of two; block 0 shows the section's first block, as the
        // runtime maps physical pages.
        let (p, section) = unsafe {
            let p = VirtualAlloc2(
                GetCurrentProcess(),
                std::ptr::null(),
                3 * BLOCK,
                MEM_RESERVE | MEM_RESERVE_PLACEHOLDER,
                PAGE_NOACCESS,
                std::ptr::null_mut(),
                0,
            ) as usize;
            assert_ne!(p, 0);
            let section = CreateFileMappingW(
                -1isize as _,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                (2 * BLOCK) as u32,
                std::ptr::null(),
            );
            assert!(!section.is_null());
            (p, section)
        };
        let map = |block: usize| {
            // SAFETY: splits the placeholder at the block and maps the
            // section's block there.
            let view = unsafe {
                VirtualFree(
                    (p + block * BLOCK) as *mut _,
                    BLOCK,
                    MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER,
                );
                MapViewOfFile3(
                    section,
                    GetCurrentProcess(),
                    (p + block * BLOCK) as *const _,
                    (block * BLOCK) as u64,
                    BLOCK,
                    MEM_REPLACE_PLACEHOLDER,
                    PAGE_READWRITE,
                    std::ptr::null_mut(),
                    0,
                )
            };
            assert_eq!(view.Value as usize, p + block * BLOCK);
            view
        };
        let first = map(0);
        let pages = |block: usize| (p + block * BLOCK) / PAGE..(p + (block + 1) * BLOCK) / PAGE;
        let all: Vec<usize> = (0..3).flat_map(pages).collect();
        assert_eq!(guard(p, p + 3 * BLOCK), Some(all));
        assert_eq!(
            guard(p, p + 3 * BLOCK),
            Some(Vec::new()),
            "nothing written since"
        );
        let second = map(1);
        title_memory_remapped(p + BLOCK, BLOCK);
        assert_eq!(guard(p, p + 3 * BLOCK), Some(pages(1).collect()));
        assert_eq!(guard(p, p + 3 * BLOCK), Some(Vec::new()));
        // SAFETY: our views and placeholder.
        unsafe {
            UnmapViewOfFile2(GetCurrentProcess(), first, MEM_PRESERVE_PLACEHOLDER);
            UnmapViewOfFile2(GetCurrentProcess(), second, MEM_PRESERVE_PLACEHOLDER);
            for block in 0..3 {
                VirtualFree((p + block * BLOCK) as *mut _, 0, MEM_RELEASE);
            }
            windows_sys::Win32::Foundation::CloseHandle(section);
        }
        title_memory_remapped(p, 3 * BLOCK);
    }

    /// A pagefile section mapped twice: two views of the same physical
    /// pages, the second left for the test; the section handle, the two
    /// addresses.
    fn two_views(bytes: usize) -> (usize, usize, usize) {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, FILE_MAP_WRITE, MapViewOfFile,
        };
        // SAFETY: a fresh pagefile section, mapped twice; the views keep
        // the section alive and the handle is closed by the caller.
        unsafe {
            let section = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                bytes as u32,
                std::ptr::null(),
            );
            assert!(!section.is_null());
            let one = MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, bytes).Value as usize;
            let two = MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, bytes).Value as usize;
            assert_ne!(one, 0);
            assert_ne!(two, 0);
            crate::inplace::title_memory_view(one, bytes, section as usize, 0);
            crate::inplace::title_memory_view(two, bytes, section as usize, 0);
            (section as usize, one, two)
        }
    }

    /// The section of [`two_views`] is closed and its views unmapped; the
    /// guard is told they are gone first.
    fn drop_views(section: usize, views: &[usize], bytes: usize) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Memory::{MEMORY_MAPPED_VIEW_ADDRESS, UnmapViewOfFile};
        for &view in views {
            title_memory_remapped(view, bytes);
            // SAFETY: a view of the section mapped by `two_views`.
            unsafe {
                UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: view as *mut _,
                })
            };
        }
        // SAFETY: the section's last reference.
        unsafe { CloseHandle(section as *mut std::ffi::c_void) };
    }

    /// The title wrote the pages of a guarded texture through another view
    /// of them: the fault at that view is the guard's to see, and the
    /// guarded view's next check reports the pages written.
    #[test]
    fn a_write_through_another_view_is_reported_at_the_guarded_one() {
        const SIZE: usize = 2 * BLOCK;
        let (section, guarded, other) = two_views(SIZE);
        let pages = |block: usize| (guarded + block * BLOCK) / PAGE;
        // The first check takes it in (and every view of its pages).
        assert_eq!(guard(guarded, guarded + SIZE), Some(vec![pages(0), pages(1)]));
        assert_eq!(guard(guarded, guarded + SIZE), Some(Vec::new()), "nothing written yet");
        // Writing through the other view faults into the handler, which
        // opens that view and the guarded one: the next check sees it.
        // SAFETY: the second view, read-write once the handler has opened it.
        unsafe { ((other + BLOCK + 8) as *mut u8).write(1) };
        assert_eq!(guard(guarded, guarded + SIZE), Some(vec![pages(1)]));
        assert_eq!(guard(guarded, guarded + SIZE), Some(Vec::new()));
        drop_views(section, &[guarded, other], SIZE);
    }

    /// A view of guarded pages is mapped anew: it is writable, so the
    /// guarded view checks its pages again (and guards the new view).
    #[test]
    fn a_view_mapped_beside_a_guarded_one_makes_it_check_again() {
        const SIZE: usize = 2 * BLOCK;
        let (section, guarded, other) = two_views(SIZE);
        let pages = |block: usize| (guarded + block * BLOCK) / PAGE;
        assert_eq!(guard(guarded, guarded + SIZE), Some(vec![pages(0), pages(1)]));
        assert_eq!(guard(guarded, guarded + SIZE), Some(Vec::new()), "nothing written yet");
        title_memory_remapped(other, SIZE);
        crate::inplace::title_memory_view(other, SIZE, section, 0);
        assert_eq!(guard(guarded, guarded + SIZE), Some(vec![pages(0), pages(1)]));
        assert_eq!(guard(guarded, guarded + SIZE), Some(Vec::new()));
        drop_views(section, &[guarded, other], SIZE);
    }

    /// A write of the runtime's own, told at one view of the pages
    /// (`writing`): the other views' next checks report the pages written.
    #[test]
    fn a_write_told_at_one_view_is_reported_at_the_others() {
        const SIZE: usize = 2 * BLOCK;
        let (section, guarded, other) = two_views(SIZE);
        let pages = |block: usize| (guarded + block * BLOCK) / PAGE;
        assert_eq!(guard(guarded, guarded + SIZE), Some(vec![pages(0), pages(1)]));
        assert_eq!(guard(guarded, guarded + SIZE), Some(Vec::new()), "nothing written yet");
        writing(other + BLOCK, BLOCK);
        assert_eq!(guard(guarded, guarded + SIZE), Some(vec![pages(1)]));
        assert_eq!(guard(guarded, guarded + SIZE), Some(Vec::new()));
        // Half a page and the next: both blocks, not just the first's.
        writing(other + BLOCK / 2, BLOCK);
        assert_eq!(guard(guarded, guarded + SIZE), Some(vec![pages(0), pages(1)]));
        drop_views(section, &[guarded, other], SIZE);
    }

    /// Both views of the pages are checked, each for a copy of its own
    /// (the title shows one physical page at several addresses, and each
    /// address has its host copy): a write through either is reported at
    /// both, whichever is checked first. Guarding the pages for the first
    /// view's check must not note the second view clean, or its copy would
    /// keep the bytes from before the write.
    #[test]
    fn a_write_is_reported_at_every_checked_view() {
        const SIZE: usize = 2 * BLOCK;
        let (section, one, two) = two_views(SIZE);
        let pages = |view: usize, block: usize| (view + block * BLOCK) / PAGE;
        assert_eq!(guard(one, one + SIZE), Some(vec![pages(one, 0), pages(one, 1)]));
        assert_eq!(guard(two, two + SIZE), Some(vec![pages(two, 0), pages(two, 1)]), "the second view's first check");
        assert_eq!(guard(one, one + SIZE), Some(Vec::new()), "nothing written yet");
        assert_eq!(guard(two, two + SIZE), Some(Vec::new()), "nothing written yet");
        // SAFETY: the second view, read-write once the handler has opened it.
        unsafe { ((two + BLOCK + 8) as *mut u8).write(1) };
        assert_eq!(guard(one, one + SIZE), Some(vec![pages(one, 1)]));
        assert_eq!(guard(two, two + SIZE), Some(vec![pages(two, 1)]), "written through this view, checked second");
        // SAFETY: the first view, as above.
        unsafe { ((one + 8) as *mut u8).write(1) };
        assert_eq!(guard(two, two + SIZE), Some(vec![pages(two, 0)]));
        assert_eq!(guard(one, one + SIZE), Some(vec![pages(one, 0)]), "written through this view, checked second");
        assert_eq!(guard(one, one + SIZE), Some(Vec::new()));
        assert_eq!(guard(two, two + SIZE), Some(Vec::new()));
        drop_views(section, &[one, two], SIZE);
    }

    /// A page shown in more than `MANY_VIEWS` views is guarded at each by
    /// itself: the others stay writable, and nothing looks through them all.
    #[test]
    fn a_page_of_many_views_is_guarded_one_view_at_a_time() {
        let (section, one, two) = two_views(BLOCK);
        let filler = one + (1 << 45);
        for n in 0..MANY_VIEWS {
            crate::inplace::title_memory_view(filler + n * BLOCK, BLOCK, section, 0);
        }
        assert_eq!(guard(one, one + BLOCK), Some(vec![one / PAGE]));
        assert!(writable(two), "another view of it is left alone");
        // SAFETY: the guarded view, opened by the handler.
        unsafe { ((one + 8) as *mut u8).write(1) };
        assert_eq!(guard(one, one + BLOCK), Some(vec![one / PAGE]));
        title_memory_remapped(filler, MANY_VIEWS * BLOCK);
        drop_views(section, &[one, two], BLOCK);
    }

    /// A write or mapping that runs past the end of the address space (a
    /// title's `fread` with a huge count) covers up to the end; the sum used
    /// to overflow. Up there no test has guarded blocks.
    #[test]
    fn ranges_past_the_address_space_end_do_not_overflow() {
        let top = usize::MAX - 16;
        writing(top, PAGE);
        title_memory_remapped(top, usize::MAX);
    }
}

#[cfg(test)]
mod mapping_tests {
    use super::{PAGE, by_range, title_memory_reading, title_memory_remapping};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    /// Addresses only these tests hold (nothing is read there).
    const HERE: usize = 0x7e00_0000_0000;

    /// A change of mapping waits for a read in place of its bytes that is
    /// under way, and such a read waits for the change.
    #[test]
    fn reads_in_place_and_remappings_wait_for_each_other() {
        // A reader holds; the change does not begin until it lets go.
        let changed = Arc::new(AtomicBool::new(false));
        let reading = title_memory_reading(HERE + PAGE / 2, PAGE);
        let (began, wait) = mpsc::channel();
        let change = {
            let changed = changed.clone();
            std::thread::spawn(move || {
                began.send(()).unwrap();
                let _changing = title_memory_remapping(HERE + PAGE, PAGE);
                changed.store(true, Ordering::SeqCst);
            })
        };
        wait.recv().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!changed.load(Ordering::SeqCst), "the mapping changed under a read in place");
        drop(reading);
        change.join().unwrap();
        assert!(changed.load(Ordering::SeqCst));
        // A change holds; the read does not begin until it is done.
        let read = Arc::new(AtomicBool::new(false));
        let changing = title_memory_remapping(HERE, 2 * PAGE);
        let (began, wait) = mpsc::channel();
        let reader = {
            let read = read.clone();
            std::thread::spawn(move || {
                began.send(()).unwrap();
                let _reading = title_memory_reading(HERE + PAGE + 16, 16);
                read.store(true, Ordering::SeqCst);
            })
        };
        wait.recv().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!read.load(Ordering::SeqCst), "a read in place began during a change of mapping");
        drop(changing);
        reader.join().unwrap();
        assert!(read.load(Ordering::SeqCst));
    }

    /// Reads and changes whose bytes do not meet do not wait for each other,
    /// and a change waiting for a read keeps new reads of its bytes out (but
    /// not of others).
    #[test]
    fn reads_and_remappings_of_other_bytes_go_on() {
        if !by_range() {
            return; // VICTORY_MAPPING_RANGES=0: one lock over all of it
        }
        let at = HERE + 16 * PAGE;
        let reading = title_memory_reading(at, PAGE);
        // Another page changes while the read is under way, and another
        // read goes on while that change is.
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            let changing = title_memory_remapping(at + PAGE, PAGE);
            let (read, was_read) = mpsc::channel();
            std::thread::spawn(move || {
                let _reading = title_memory_reading(at + 2 * PAGE, PAGE);
                read.send(()).unwrap();
            });
            let other = was_read.recv_timeout(Duration::from_secs(10)).is_ok();
            drop(changing);
            done.send(other).unwrap();
        });
        assert_eq!(finished.recv_timeout(Duration::from_secs(10)), Ok(true), "kept apart though their bytes do not meet");
        // A change of the read's page waits, and keeps a new read of its
        // bytes out meanwhile; a read elsewhere is not kept out.
        let changed = Arc::new(AtomicBool::new(false));
        let change = {
            let changed = changed.clone();
            std::thread::spawn(move || {
                let _changing = title_memory_remapping(at, PAGE);
                changed.store(true, Ordering::SeqCst);
            })
        };
        while !super::spans().changes.iter().any(|c| c.1 == at) {
            std::thread::yield_now();
        }
        assert!(super::title_memory_reading_now(at + 8, 8).is_none(), "a read began over a change that waits");
        assert!(super::title_memory_reading_now(at + 3 * PAGE, 8).is_some());
        std::thread::sleep(Duration::from_millis(50));
        assert!(!changed.load(Ordering::SeqCst), "the mapping changed under a read in place");
        drop(reading);
        change.join().unwrap();
        assert!(changed.load(Ordering::SeqCst));
    }

    #[test]
    fn watch_read_fast_clean_view_skips_write_lock_and_preserves_writes() {
        let mut j = super::Journal {
            asked: 5,
            written: std::collections::BTreeMap::new(),
            found: Vec::new(),
            ahead: Vec::new(),
            ahead_views: Vec::new(),
            ahead_asked: 0,
            kept: false,
        };
        j.written.insert(10, 5);
        j.written.insert(15, 4);
        let mut g = super::Guards {
            protected: super::BlockSet::new(),
            clean: super::BlockSet::new(),
            absent: super::BlockSet::new(),
            aliased: super::BlockSet::new(),
        };
        g.protected.extend_range(10, 12);
        g.clean.extend_range(10, 12);

        // Clean range must be answered via shared read path without losing page 10's write mark.
        let got = super::try_clean_view_read(&j, &g, 10, 12);
        assert_eq!(got, Some((5, vec![(10, 5)])));

        // Range extending onto an open/dirty block (12 not in clean) must NOT take fast path.
        assert_eq!(super::try_clean_view_read(&j, &g, 10, 13), None);

        // Uninitialized asking (asked == 0) must NOT take fast path.
        j.asked = 0;
        assert_eq!(super::try_clean_view_read(&j, &g, 10, 12), None);
    }

    #[test]
    fn watch_unlocked_ask_stripes_cover_range_and_are_strictly_sorted() {
        // Single stripe
        assert_eq!(super::stripe_indices(0, 10), vec![0]);
        // Crossing stripe boundary
        assert_eq!(super::stripe_indices(255, 257), vec![0, 1]);
        // Wrapping around 64 stripes: must remain strictly sorted ascending (deadlock-free lock order)
        let s = super::stripe_indices(63 * 256 + 250, 64 * 256 + 10);
        assert_eq!(s, vec![0, 63]);
        // Empty range
        assert_eq!(super::stripe_indices(10, 10), Vec::<usize>::new());
        // Full wrap (>= 1 GiB) covers all 64 stripes in ascending order
        let full = super::stripe_indices(5, 5 + 64 * 256);
        assert_eq!(full.len(), 64);
        assert!(full.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn guard_absent_fast_accepts_clean_plus_absent_and_rejects_remapped_or_open() {
        let mut j = super::Journal {
            asked: 9,
            written: std::collections::BTreeMap::new(),
            found: Vec::new(),
            ahead: Vec::new(),
            ahead_views: Vec::new(),
            ahead_asked: 0,
            kept: false,
        };
        j.written.insert(10, 9);
        let mut g = super::Guards {
            protected: super::BlockSet::new(),
            clean: super::BlockSet::new(),
            absent: super::BlockSet::new(),
            aliased: super::BlockSet::new(),
        };
        g.protected.extend_range(10, 12);
        g.clean.extend_range(10, 12);
        g.absent.extend_range(12, 14);

        // Without trust_absent, mixed clean+absent falls back.
        assert_eq!(super::try_clean_view_read_ex(&j, &g, 10, 14, false), None);
        // With trust_absent, mixed clean+absent takes fast read path preserving page 10 write.
        assert_eq!(
            super::try_clean_view_read_ex(&j, &g, 10, 14, true),
            Some((9, vec![(10, 9)])),
            "clean+absent sparse range must hit fast path when trust_absent=true"
        );
        // Simulating title_memory_remapped on block 12 (removes from absent): MUST fall back!
        g.absent.remove(&12);
        assert_eq!(
            super::try_clean_view_read_ex(&j, &g, 10, 14, true),
            None,
            "remapped hole removed from absent must fall back to guard()"
        );
    }
}
