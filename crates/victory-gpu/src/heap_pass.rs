//! Bookkeeping of the pass over the view heap's buffer views for shaders
//! that pick them by a computed index (`draw::State::heap_buffers`): which
//! slots it looks at again, and the span of each host copy their views
//! read. Kept apart from D3D12 so that it can be tested alone.
//!
//! The pass used to look again at every buffer view of the heap (81,000 in
//! the world) every 16 passes, and to rebuild the spans of all host copies
//! from all views every pass. Now a slot is looked at again only when
//! - its buffer view was written or went (the heap's own set of changes);
//! - the view table entry its record names was freed (the title released
//!   the resource: `descriptor::forget`), which leaves the record as it was;
//! - its record in title memory is not the one seen at the last look (a
//!   record copied by the title itself, past the API); a compare of 12
//!   bytes per slot, no lock and no lookup;
//! - it found no host copy at its last look: every 16 passes, as before;
//! - every [`FULL_EVERY`] passes, all of them, as a safety net.
//!
//! The span of a copy is worked out again only when one of its slots went.
//! `VICTORY_HEAP_PASS_FULL=1` brings back the old pass (all slots every 16
//! passes, all spans every pass) to compare.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

/// Fast non-cryptographic hasher for integer slot/copy keys (~200k ops/frame
/// in `Picked::plan`/`forget`/`keep`), replacing SipHash.
#[derive(Default)]
struct SlotHash(u64);

impl Hasher for SlotHash {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_usize(usize::from(b));
        }
    }
    #[inline]
    fn write_usize(&mut self, n: usize) {
        let h = (self.0.rotate_left(5) ^ n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.0 = h ^ (h >> 32);
    }
    #[inline]
    fn write_u32(&mut self, n: u32) {
        self.write_usize(n as usize);
    }
}

type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<SlotHash>>;
type FastSet<K> = HashSet<K, BuildHasherDefault<SlotHash>>;

/// Passes between two looks at every slot, besides those marked.
pub(crate) const FULL_EVERY: u64 = 256;
/// Passes between two looks at slots that found no host copy (and between
/// two looks at every slot with the old pass).
pub(crate) const ORPHANS_EVERY: u64 = 16;

/// Whether `VICTORY_HEAP_PASS_FULL=1` asks for the old pass.
pub(crate) fn full_pass() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_HEAP_PASS_FULL").is_some_and(|v| v == "1"))
}

/// `VICTORY_HEAP_RECORD_FAST=1` (default off): in `Picked::plan_ex`, skip slots
/// in `changed` whose 12-byte descriptor record `[tag, i, gen]` is identical to
/// the kept slot's record and whose view-table entry was not forgotten, plus
/// batch `apply_batch` updates without redundant `forget`+`keep` churn.
pub(crate) fn record_fast() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_HEAP_RECORD_FAST").is_some_and(|v| v == "1"))
}

/// A slot's record in title memory: the tag and two words
/// (`descriptor::record_at`).
pub(crate) type Record = [u32; 3];

struct Slot {
    copy: usize,
    from: u64,
    to: u64,
    record: Record,
    /// The view is an unordered access one: shaders may store through it
    /// (`crate::heap_marks` asks about every slot kept, every pass).
    unordered: bool,
}

struct Copy<C> {
    copy: C,
    slots: FastSet<u32>,
    from: u64,
    to: u64,
    /// A slot went: the span is worked out again from those left.
    stale: bool,
}

/// Passes between two scans of all kept slots' raw records in title memory
/// for past-the-API copies (`VICTORY_HEAP_SCAN_EVERY`, default 16).
pub(crate) fn scan_every() -> u64 {
    static N: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("VICTORY_HEAP_SCAN_EVERY")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&n| n >= 1)
            .unwrap_or(16)
    })
}

/// What a pass looks at (see the module).
pub(crate) struct Picked<C> {
    passes: u64,
    slots: FastMap<u32, Slot>,
    copies: FastMap<usize, Copy<C>>,
    /// Slots that found no host copy, with the record seen.
    orphans: FastMap<u32, Record>,
    /// View table entry -> slots whose record named it at their last look
    /// (some may name another by now: checked when used).
    by_entry: FastMap<u32, Vec<u32>>,
    /// Table entries freed since the last pass; `None`: not known.
    forgotten: Option<Vec<u32>>,
    /// The old pass (`VICTORY_HEAP_PASS_FULL=1`).
    full: bool,
    /// Interval for scanning all kept slots for past-the-API record changes.
    scan_every: u64,
}

impl<C: Clone> Picked<C> {
    pub(crate) fn new(full: bool) -> Self {
        Self::with_scan_every(full, if cfg!(test) { 1 } else { scan_every() })
    }

    pub(crate) fn with_scan_every(full: bool, scan_every: u64) -> Self {
        Picked {
            passes: 0,
            slots: FastMap::default(),
            copies: FastMap::default(),
            orphans: FastMap::default(),
            by_entry: FastMap::default(),
            forgotten: None,
            full,
            scan_every: scan_every.max(1),
        }
    }

    /// View table entries were freed (`None`: some were, which is not
    /// known): their slots are looked at next pass. Every heap's pass is
    /// told, though one of them asks.
    pub(crate) fn note_forgotten(&mut self, entries: Option<&[u32]>) {
        match (entries, &mut self.forgotten) {
            (Some(new), Some(kept)) => kept.extend_from_slice(new),
            _ => self.forgotten = None,
        }
    }

    /// The slots this pass looks at again, in order. `changed`: the slots
    /// whose buffer view was written or went since the last pass (and the
    /// entries [`Picked::note_forgotten`] was told of: if it was told it
    /// does not know, all slots are looked at); `all`: every slot that holds a
    /// buffer view; `record`: a slot's record now; `entry`: the table
    /// entry a record names, if it names one.
    pub(crate) fn plan(
        &mut self,
        changed: Vec<u32>,
        all: impl FnOnce() -> Vec<u32>,
        record: impl Fn(u32) -> Option<Record>,
        entry: impl Fn(Record) -> Option<u32>,
    ) -> Vec<u32> {
        self.plan_ex(changed, all, record, entry, false).0
    }

    /// Extended planner returning `(due_slots, skipped_unchanged_count)`.
    /// When `skip_unchanged` is true (`VICTORY_HEAP_RECORD_FAST=1`), slots in
    /// `changed` whose 12-byte descriptor record `[tag, i, gen]` matches the
    /// kept slot's record (and whose view-table entry was not forgotten) are
    /// skipped without re-resolving or churning `Picked`.
    pub(crate) fn plan_ex(
        &mut self,
        changed: Vec<u32>,
        all: impl FnOnce() -> Vec<u32>,
        record: impl Fn(u32) -> Option<Record>,
        entry: impl Fn(Record) -> Option<u32>,
        skip_unchanged: bool,
    ) -> (Vec<u32>, u64) {
        let pass = self.passes;
        self.passes += 1;
        let forgotten = self.forgotten.replace(Vec::new());
        if self.full {
            if pass.is_multiple_of(ORPHANS_EVERY) {
                self.clear();
                return (all(), 0);
            }
            return (changed, 0);
        }
        let Some(forgotten) = forgotten else {
            self.clear();
            return (all(), 0);
        };
        if pass.is_multiple_of(FULL_EVERY) {
            self.clear();
            return (all(), 0);
        }
        let forgotten_set: FastSet<u32> = if skip_unchanged && !forgotten.is_empty() {
            forgotten.iter().copied().collect()
        } else {
            FastSet::default()
        };
        let mut skipped = 0u64;
        let mut due: FastSet<u32> = FastSet::default();
        if skip_unchanged {
            for slot in changed {
                if let Some(kept) = self.slots.get(&slot)
                    && kept.record[0] != 0
                    && record(slot) == Some(kept.record)
                    && !entry(kept.record).is_some_and(|e| forgotten_set.contains(&e))
                {
                    skipped += 1;
                    continue;
                }
                due.insert(slot);
            }
        } else {
            due.extend(changed);
        }
        for e in forgotten {
            if let Some(slots) = self.by_entry.remove(&e) {
                // Only those whose record still names the entry.
                due.extend(
                    slots
                        .into_iter()
                        .filter(|s| self.record_of(*s).is_some_and(|r| entry(r) == Some(e))),
                );
            }
        }
        if pass.is_multiple_of(self.scan_every) {
            for (&slot, kept) in &self.slots {
                if record(slot) != Some(kept.record) {
                    due.insert(slot);
                }
            }
        }
        let orphans_too = pass.is_multiple_of(ORPHANS_EVERY);
        for (&slot, &seen) in &self.orphans {
            if orphans_too || (pass.is_multiple_of(self.scan_every) && record(slot) != Some(seen)) {
                due.insert(slot);
            }
        }
        let mut due: Vec<u32> = due.into_iter().collect();
        due.sort_unstable();
        (due, skipped)
    }

    fn record_of(&self, slot: u32) -> Option<Record> {
        self.slots
            .get(&slot)
            .map(|s| s.record)
            .or_else(|| self.orphans.get(&slot).copied())
    }

    fn clear(&mut self) {
        self.slots.clear();
        self.copies.clear();
        self.orphans.clear();
        self.by_entry.clear();
    }

    /// What slot `slot` held is let go (it is looked at now).
    pub(crate) fn forget(&mut self, slot: u32) {
        self.orphans.remove(&slot);
        let Some(old) = self.slots.remove(&slot) else {
            return;
        };
        if let Some(copy) = self.copies.get_mut(&old.copy) {
            copy.slots.remove(&slot);
            if copy.slots.is_empty() {
                self.copies.remove(&old.copy);
            } else if old.from <= copy.from || old.to >= copy.to {
                copy.stale = true;
            }
        }
    }

    /// Slot `slot` (with `record`, naming table entry `entry`) reads title
    /// memory `[from, to)` through host copy `copy` (`key`: its identity);
    /// `unordered`: through an unordered access view.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn keep(
        &mut self,
        slot: u32,
        key: usize,
        copy: &C,
        [from, to]: [u64; 2],
        record: Record,
        entry: Option<u32>,
        unordered: bool,
    ) {
        self.forget(slot);
        self.note_entry(slot, entry);
        self.slots.insert(slot, Slot { copy: key, from, to, record, unordered });
        let c = self.copies.entry(key).or_insert_with(|| Copy {
            copy: copy.clone(),
            slots: FastSet::default(),
            from,
            to,
            stale: false,
        });
        c.slots.insert(slot);
        c.from = c.from.min(from);
        c.to = c.to.max(to);
    }

    /// Updates slot `slot` without redundant `forget`+`keep` churn when its
    /// `(copy, from, to)` is unchanged.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn keep_fast(
        &mut self,
        slot: u32,
        key: usize,
        copy: &C,
        [from, to]: [u64; 2],
        record: Record,
        entry: Option<u32>,
        unordered: bool,
    ) {
        if let Some(existing) = self.slots.get_mut(&slot)
            && existing.copy == key
            && existing.from == from
            && existing.to == to
        {
            let old_record = existing.record;
            existing.record = record;
            existing.unordered = unordered;
            if old_record != record {
                self.note_entry(slot, entry);
            }
            return;
        }
        self.keep(slot, key, copy, [from, to], record, entry, unordered);
    }

    /// Slot `slot` holds a buffer view whose memory has no host copy yet.
    pub(crate) fn orphan(&mut self, slot: u32, record: Record, entry: Option<u32>) {
        self.forget(slot);
        self.note_entry(slot, entry);
        self.orphans.insert(slot, record);
    }

    fn note_entry(&mut self, slot: u32, entry: Option<u32>) {
        // Only kept with the incremental pass (the old one does not use it).
        if let Some(e) = entry
            && !self.full
        {
            // (Maybe twice: the pass's set of due slots takes it once.)
            self.by_entry.entry(e).or_default().push(slot);
        }
    }

    /// The views kept.
    pub(crate) fn views(&self) -> usize {
        self.slots.len()
    }

    /// Every slot kept (whether or not this pass looked at it), and whether
    /// its view is an unordered access one.
    pub(crate) fn unordered(&self) -> impl Iterator<Item = (u32, bool)> + '_ {
        self.slots.iter().map(|(&slot, s)| (slot, s.unordered))
    }

    /// What the view of `slot` reads, if the slot is kept: its host copy,
    /// the range `(from, to)`, and whether the view is an unordered access
    /// one.
    pub(crate) fn view(&self, slot: u32) -> Option<(&C, u64, u64, bool)> {
        let s = self.slots.get(&slot)?;
        Some((&self.copies.get(&s.copy)?.copy, s.from, s.to, s.unordered))
    }

    /// Per host copy, the span its views read: `(copy, from, to)`.
    pub(crate) fn spans(&mut self) -> Vec<(C, u64, u64)> {
        if self.full {
            // As the old pass did: from every view, every pass.
            let mut spans: HashMap<usize, (u64, u64)> = HashMap::new();
            for s in self.slots.values() {
                let span = spans.entry(s.copy).or_insert((s.from, s.to));
                span.0 = span.0.min(s.from);
                span.1 = span.1.max(s.to);
            }
            return spans
                .into_iter()
                .filter_map(|(key, (from, to))| Some((self.copies.get(&key)?.copy.clone(), from, to)))
                .collect();
        }
        let slots = &self.slots;
        self.copies
            .values_mut()
            .map(|c| {
                if c.stale {
                    c.stale = false;
                    let (mut from, mut to) = (u64::MAX, 0);
                    for s in c.slots.iter().filter_map(|s| slots.get(s)) {
                        from = from.min(s.from);
                        to = to.max(s.to);
                    }
                    (c.from, c.to) = (from, to);
                }
                (c.copy.clone(), c.from, c.to)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A heap of slots with records, as title memory holds them.
    struct Heap {
        records: HashMap<u32, Record>,
    }

    impl Heap {
        fn record(&self, slot: u32) -> Option<Record> {
            self.records.get(&slot).copied()
        }
    }

    fn entry(r: Record) -> Option<u32> {
        Some(r[1])
    }

    /// One pass: plans, then "looks" at each due slot: a slot reads 16
    /// bytes from `slot * 16` of copy `slot / 4`.
    fn pass(p: &mut Picked<u32>, heap: &Heap, changed: Vec<u32>, forgotten: Vec<u32>) -> Vec<u32> {
        p.note_forgotten(Some(&forgotten));
        let due = p.plan(
            changed,
            || {
                let mut all: Vec<u32> = heap.records.keys().copied().collect();
                all.sort_unstable();
                all
            },
            |s| heap.record(s),
            entry,
        );
        for &slot in &due {
            let Some(record) = heap.record(slot) else {
                p.forget(slot);
                continue;
            };
            let copy = slot / 4;
            let from = u64::from(slot) * 16;
            p.keep(slot, copy as usize, &copy, [from, from + 16], record, entry(record), slot % 2 == 1);
        }
        due
    }

    fn heap(n: u32) -> Heap {
        Heap {
            records: (0..n).map(|s| (s, [1, 100 + s, 1])).collect(),
        }
    }

    #[test]
    fn only_slots_changed_since_the_last_pass_are_looked_at_again() {
        let mut heap = heap(64);
        let mut p = Picked::new(false);
        // The first pass looks at all of them.
        assert_eq!(pass(&mut p, &heap, vec![], vec![]).len(), 64);
        assert_eq!(p.views(), 64);
        // Nothing changed: nothing is looked at.
        assert!(pass(&mut p, &heap, vec![], vec![]).is_empty());
        // A view written through the API.
        heap.records.insert(5, [1, 500, 1]);
        assert_eq!(pass(&mut p, &heap, vec![5], vec![]), vec![5]);
        // A record copied past the API: found by its words alone.
        heap.records.insert(9, [1, 900, 1]);
        assert_eq!(pass(&mut p, &heap, vec![], vec![]), vec![9]);
        // The resource of entry 117 (slot 17) released: the record is as
        // it was, the slot is looked at all the same; slot 5 no longer
        // names its old entry 105 and is not.
        assert_eq!(pass(&mut p, &heap, vec![], vec![117, 105]), vec![17]);
        // And the next pass leaves them alone again.
        assert!(pass(&mut p, &heap, vec![], vec![]).is_empty());
    }

    #[test]
    fn a_slot_that_went_shrinks_its_copy_span() {
        let mut heap = heap(8);
        let mut p = Picked::new(false);
        pass(&mut p, &heap, vec![], vec![]);
        let span = |p: &mut Picked<u32>, copy: u32| p.spans().into_iter().find(|s| s.0 == copy).map(|s| (s.1, s.2));
        assert_eq!(span(&mut p, 1), Some((64, 128)));
        heap.records.remove(&7);
        pass(&mut p, &heap, vec![7], vec![]);
        assert_eq!(span(&mut p, 1), Some((64, 112)));
        for s in 4..7 {
            heap.records.remove(&s);
        }
        pass(&mut p, &heap, vec![4, 5, 6], vec![]);
        assert_eq!(span(&mut p, 1), None);
        assert_eq!(span(&mut p, 0), Some((0, 64)));
    }

    #[test]
    fn orphans_are_tried_again_every_16_passes_and_all_slots_now_and_then() {
        let heap = heap(4);
        let mut p: Picked<u32> = Picked::new(false);
        let _ = p.plan(vec![], || vec![0, 1, 2, 3], |s| heap.record(s), entry);
        for s in 0..3 {
            p.keep(s, 0, &0, [0, 16], heap.record(s).unwrap(), None, false);
        }
        p.orphan(3, heap.record(3).unwrap(), None);
        for n in 1..FULL_EVERY {
            let due = p.plan(vec![], || vec![0, 1, 2, 3], |s| heap.record(s), entry);
            let want: Vec<u32> = if n.is_multiple_of(ORPHANS_EVERY) { vec![3] } else { vec![] };
            assert_eq!(due, want, "pass {n}");
        }
        let due = p.plan(vec![], || vec![0, 1, 2, 3], |s| heap.record(s), entry);
        assert_eq!(due, vec![0, 1, 2, 3]);
    }

    #[test]
    fn every_kept_slot_says_whether_its_view_is_unordered_looked_at_or_not() {
        let mut heap = heap(8);
        let mut p = Picked::new(false);
        pass(&mut p, &heap, vec![], vec![]);
        // Nothing is looked at in this pass; all slots are still told of.
        assert!(pass(&mut p, &heap, vec![], vec![]).is_empty());
        let mut all: Vec<(u32, bool)> = p.unordered().collect();
        all.sort_unstable();
        assert_eq!(all, (0..8).map(|s| (s, s % 2 == 1)).collect::<Vec<_>>());
        // The copy and range of a slot, with the flag.
        assert_eq!(p.view(5), Some((&1, 80, 96, true)));
        assert_eq!(p.view(4), Some((&1, 64, 80, false)));
        // A slot that went is no longer told of.
        heap.records.remove(&5);
        pass(&mut p, &heap, vec![5], vec![]);
        assert_eq!(p.view(5), None);
        assert_eq!(p.unordered().count(), 7);
    }

    #[test]
    fn the_old_pass_looks_at_all_every_16_passes() {
        let heap = heap(4);
        let mut p: Picked<u32> = Picked::new(true);
        for n in 0..40u64 {
            let due = p.plan(vec![2], || vec![0, 1, 2, 3], |s| heap.record(s), entry);
            let want: Vec<u32> = if n.is_multiple_of(ORPHANS_EVERY) { vec![0, 1, 2, 3] } else { vec![2] };
            assert_eq!(due, want, "pass {n}");
        }
    }

    #[test]
    fn record_fast_skips_unchanged_slots_in_changed_but_rechecks_modified_or_forgotten() {
        let mut heap = heap(16);
        let mut p = Picked::new(false);
        pass(&mut p, &heap, vec![], vec![]);
        // Slot 2 unchanged, slot 5 modified, slot 7's table entry (107) forgotten.
        heap.records.insert(5, [1, 555, 2]);
        p.note_forgotten(Some(&[107]));
        let (due, skipped) = p.plan_ex(
            vec![2, 5, 7],
            || (0..16).collect(),
            |s| heap.record(s),
            entry,
            true,
        );
        assert_eq!(
            due,
            vec![5, 7],
            "unchanged slot 2 must be skipped by record_fast while modified slot 5 and forgotten slot 7 stay due"
        );
        assert_eq!(skipped, 1, "exactly 1 unchanged slot must be counted as skipped");
    }

    #[test]
    fn record_fast_keep_in_place_avoids_stale_when_bounds_unchanged_and_updates_when_changed() {
        let heap = heap(8);
        let mut p = Picked::new(false);
        pass(&mut p, &heap, vec![], vec![]);
        // Update slot 0 with identical copy/bounds [0, 16] via keep_fast -> no stale span.
        p.keep_fast(0, 0, &0, [0, 16], [1, 100, 2], Some(100), true);
        assert_eq!(p.view(0), Some((&0, 0, 16, true)));
        // Update slot 0 to new bounds [0, 32] -> copy 0 span grows from 64 to 64 (still covers [0, 32]).
        p.keep_fast(0, 0, &0, [0, 128], [1, 100, 3], Some(100), false);
        assert_eq!(p.view(0), Some((&0, 0, 128, false)));
        let span0 = p.spans().into_iter().find(|s| s.0 == 0).unwrap();
        assert_eq!((span0.1, span0.2), (0, 128));
    }
}

