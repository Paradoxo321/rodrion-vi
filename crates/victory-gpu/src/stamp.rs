//! "Already checked in this submission" (`VICTORY_REFRESH_STAMP=1`; off by
//! default).
//!
//! A world submission asks for some 20,000 refreshes of host copies, most
//! of them of ranges an earlier refresh of the same submission already
//! brought up to date: every one merged into the batch, flushed, and walked
//! page by page against the write-watch journal for nothing. With the
//! switch each copy remembers, by 64 KiB page, what it checked under the
//! current sequence number ([`now`]), and a request wholly inside checked
//! pages returns at once (`Batch::add`, `Shadow::update`).
//!
//! The sequence advances ([`advance`]) wherever title memory may change
//! under a copy while a submission is prepared, so that what was checked
//! before must be checked again: each submission's start (`Host::submit`),
//! our own writes into title memory (`watch::wrote`, `watch::wrote_own`,
//! `watch::writing` before and `host::write_mapped` after the bytes are
//! there, which every copy-back goes through), the runtime's
//! (`watch::title_memory_written`), the end of the submission's one
//! write-watch asking (`watch::ask_once_end`, after a step that writes title
//! memory), and a change of what title memory shows
//! (`watch::title_memory_remapped`).
//!
//! What changes: as with the submission's one asking (`watch::once`, on by
//! default), what the title's other threads write while a submission is
//! prepared is seen by the next submission. A range shaders may be writing
//! (a copy's `pending`) is never marked checked.
//!
//! `VICTORY_REFRESH_ONCE` (`host::refresh_once`) does the same for the one
//! range of a copy last brought up to date, and a write of ours takes only
//! its own bytes out; here every range counts, and any write ends them all.
//!
//! From clankenjoyer/victory-fps 5e30085, without its fault tracking. Model:
//! Ryujinx `MultiRegionHandle.QueryModified` (`_sequenceNumberBitmap`) and
//! `Buffer.SynchronizeMemory` (MIT).

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::watch::PAGE;

/// Whether the stamp is on (see the module).
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var_os("VICTORY_REFRESH_STAMP").is_some_and(|v| v == "1");
        if on {
            tracing::info!("refreshes of ranges a copy already checked in this submission return at once (VICTORY_REFRESH_STAMP=1)");
        }
        on
    })
}

/// The sequence number; 0 is never current.
static SEQ: AtomicU64 = AtomicU64::new(1);

/// Title memory may have changed under every copy: what was checked must be
/// checked again.
pub(crate) fn advance() {
    SEQ.fetch_add(1, Ordering::AcqRel);
}

/// The current sequence number.
pub(crate) fn now() -> u64 {
    SEQ.load(Ordering::Acquire)
}

/// Per copy: which of its pages were checked under which sequence.
#[derive(Default)]
pub(crate) struct Stamp {
    /// The sequence `bits` belong to (0: none), read without the lock.
    seq: AtomicU64,
    bits: Mutex<Bits>,
}

#[derive(Default)]
struct Bits {
    seq: u64,
    /// One bit per page of the copy, from the copy's first page.
    words: Vec<u64>,
    /// Pages of the copy not checked under `seq`.
    unchecked: usize,
}

/// The copy's pages: `(first page number, count)` for buffer `[start, end)`.
fn pages(start: usize, end: usize) -> (usize, usize) {
    (start / PAGE, end.div_ceil(PAGE) - start / PAGE)
}

impl Stamp {
    /// Whether `[from, to)` (inside the buffer `[start, end)`) was checked
    /// whole under sequence `seq`, which is current.
    pub(crate) fn current(&self, seq: u64, from: usize, to: usize, start: usize, _end: usize) -> bool {
        if from >= to || self.seq.load(Ordering::Acquire) != seq {
            return false;
        }
        let b = self.bits.lock().unwrap_or_else(|e| e.into_inner());
        if b.seq != seq {
            return false;
        }
        if b.unchecked == 0 {
            return true;
        }
        let base = start / PAGE;
        all_set(&b.words, from / PAGE - base, (to - 1) / PAGE - base)
    }

    /// `[from, to)` (inside `[start, end)`) was checked under `seq`, read
    /// before the check began: its whole pages are noted, unless the
    /// sequence advanced meanwhile.
    pub(crate) fn mark(&self, seq: u64, from: usize, to: usize, start: usize, end: usize) {
        self.note(seq, now(), from, to, start, end);
    }

    /// [`Stamp::mark`] with the sequence now `latest`.
    fn note(&self, seq: u64, latest: u64, from: usize, to: usize, start: usize, end: usize) {
        if from >= to || latest != seq {
            return;
        }
        let (base, count) = pages(start, end);
        // Pages the range holds whole (as far as they are in the buffer):
        // a page only partly in it is copied in part.
        let mut p0 = from / PAGE;
        if (p0 * PAGE).max(start) < from {
            p0 += 1;
        }
        let mut p1 = to.div_ceil(PAGE);
        if p1 > p0 && ((p1 * PAGE).min(end)) > to {
            p1 -= 1;
        }
        if p0 >= p1 {
            return;
        }
        let mut b = self.bits.lock().unwrap_or_else(|e| e.into_inner());
        if b.seq > seq {
            return;
        }
        if b.seq != seq {
            b.seq = seq;
            b.words.clear();
            b.words.resize(count.div_ceil(64), 0);
            b.unchecked = count;
        }
        let new = set_range(&mut b.words, p0 - base, p1 - 1 - base);
        b.unchecked -= new.min(b.unchecked);
        self.seq.store(seq, Ordering::Release);
    }
}

/// Whether bits `[first, last]` are all set.
fn all_set(words: &[u64], first: usize, last: usize) -> bool {
    let mut at = first;
    while at <= last {
        let (w, b) = (at / 64, at % 64);
        let n = (last - at + 1).min(64 - b);
        let mask = if n == 64 { u64::MAX } else { ((1u64 << n) - 1) << b };
        match words.get(w) {
            Some(&v) if v & mask == mask => {}
            _ => return false,
        }
        at += n;
    }
    true
}

/// Sets bits `[first, last]`; how many were not set.
fn set_range(words: &mut [u64], first: usize, last: usize) -> usize {
    let mut at = first;
    let mut new = 0;
    while at <= last {
        let (w, b) = (at / 64, at % 64);
        let n = (last - at + 1).min(64 - b);
        let mask = if n == 64 { u64::MAX } else { ((1u64 << n) - 1) << b };
        let Some(v) = words.get_mut(w) else {
            break;
        };
        new += (mask & !*v).count_ones() as usize;
        *v |= mask;
        at += n;
    }
    new
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: usize = 0x1000_0000 + 100;
    const END: usize = START + 10 * PAGE;

    #[test]
    fn a_checked_range_is_current_until_the_sequence_changes() {
        let s = Stamp::default();
        assert!(!s.current(5, START, START + PAGE, START, END));
        s.note(5, 5, START, END, START, END);
        assert!(s.current(5, START, END, START, END));
        assert!(s.current(5, START + 3 * PAGE, START + 4 * PAGE, START, END));
        assert!(!s.current(6, START, START + 1, START, END));
        // A new sequence starts from nothing checked.
        s.note(6, 6, START, START + 2 * PAGE, START, END);
        assert!(!s.current(6, START + 5 * PAGE, START + 6 * PAGE, START, END));
        assert!(!s.current(5, START, END, START, END));
    }

    #[test]
    fn only_pages_held_whole_are_marked() {
        let s = Stamp::default();
        let page = START.div_ceil(PAGE) * PAGE; // the copy's second page
        // Half of a page, then the next page whole.
        s.note(7, 7, page + PAGE / 2, page + 2 * PAGE, START, END);
        assert!(!s.current(7, page, page + PAGE, START, END));
        assert!(s.current(7, page + PAGE, page + 2 * PAGE, START, END));
        assert!(!s.current(7, page + PAGE, page + 2 * PAGE + 1, START, END));
        // The copy's first page is in the buffer from START on: whole if
        // the range starts there.
        s.note(7, 7, START, page, START, END);
        assert!(s.current(7, START, page, START, END));
        // And its last page up to END.
        s.note(7, 7, END / PAGE * PAGE, END, START, END);
        assert!(s.current(7, END - 1, END, START, END));
        s.note(7, 7, END - 10, END, START, END);
        assert!(!s.current(7, END - PAGE, END, START, END));
    }

    #[test]
    fn a_mark_made_while_the_sequence_advanced_is_ignored() {
        let s = Stamp::default();
        s.note(3, 4, START, END, START, END);
        assert!(!s.current(3, START, END, START, END));
        assert!(!s.current(4, START, END, START, END));
        // Nor does an older mark undo a newer one.
        s.note(4, 4, START, END, START, END);
        s.note(3, 3, START, END, START, END);
        assert!(s.current(4, START, END, START, END));
    }

    #[test]
    fn the_sequence_advances() {
        let a = now();
        advance();
        assert!(now() > a);
    }

    #[test]
    fn bit_ranges_cross_words() {
        let mut w = vec![0u64; 3];
        assert_eq!(set_range(&mut w, 60, 130), 71);
        assert!(all_set(&w, 60, 130));
        assert!(!all_set(&w, 59, 130));
        assert!(!all_set(&w, 60, 131));
        assert_eq!(set_range(&mut w, 0, 191), 192 - 71);
        assert!(all_set(&w, 0, 191));
        assert!(!all_set(&w, 0, 192));
    }
}
