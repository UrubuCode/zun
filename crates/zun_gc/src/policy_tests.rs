//! The adaptive budgets: the nursery budget follows the survival rate and the
//! tenured size, and the compaction trigger follows the tenured size.

use super::*;

const MIB: usize = 1024 * 1024;

#[test]
fn the_nursery_budget_starts_at_the_floor() {
    let heap = Heap::new();
    assert_eq!(heap.nursery_budget(), NURSERY_BUDGET_FLOOR);
}

#[test]
fn a_high_survival_rate_grows_the_budget_to_a_multiple_of_the_survivors() {
    let mut heap = Heap::new();
    heap.tenured_live_bytes = 64 * MIB;
    // Every byte of a 32 KiB epoch survived.
    heap.note_young_collection(32 * 1024, 32 * 1024);
    assert_eq!(heap.nursery_budget(), 32 * 1024 * NURSERY_SURVIVAL_MULTIPLE);
    // The next epoch fills that budget and again everything survives: it grows
    // again, until the ceiling.
    let budget = heap.nursery_budget();
    heap.note_young_collection(budget, budget);
    assert_eq!(heap.nursery_budget(), budget * NURSERY_SURVIVAL_MULTIPLE);
    let budget = heap.nursery_budget();
    heap.note_young_collection(budget, budget);
    assert_eq!(heap.nursery_budget(), NURSERY_BUDGET_CEILING);
}

#[test]
fn the_budget_decays_toward_the_floor_when_nothing_survives() {
    let mut heap = Heap::new();
    heap.tenured_live_bytes = 64 * MIB;
    heap.nursery_budget = NURSERY_BUDGET_CEILING;
    let mut previous = heap.nursery_budget();
    for _ in 0..32 {
        heap.note_young_collection(previous, 0);
        let next = heap.nursery_budget();
        assert!(next <= previous && next >= NURSERY_BUDGET_FLOOR);
        previous = next;
    }
    assert_eq!(previous, NURSERY_BUDGET_FLOOR);
}

#[test]
fn a_small_heap_keeps_a_small_nursery_whatever_survives() {
    let mut heap = Heap::new();
    heap.note_young_collection(NURSERY_BUDGET_FLOOR, NURSERY_BUDGET_FLOOR);
    assert!(heap.nursery_budget() <= NURSERY_TENURED_MIN_CEILING);
}

#[test]
fn the_compaction_trigger_scales_with_the_tenured_size() {
    // A heap that left nothing alive compacts as soon as 64 KiB was promoted
    // and the minimum interval has passed.
    let mut heap = Heap::new();
    heap.promoted_since_compaction = COMPACT_MIN_PROMOTED_BYTES;
    let fired: Vec<bool> = (0..COMPACT_MIN_INTERVAL).map(|_| heap.native_compaction_due()).collect();
    assert_eq!(fired, vec![false, false, false, true]);

    // A heap that left 8 MiB alive does not compact for 1 MiB of promotion, nor
    // however many collections pass...
    let mut heap = Heap::new();
    heap.tenured_live_bytes = 8 * MIB;
    heap.promoted_since_compaction = MIB;
    assert!((0..COMPACT_BACKSTOP_INTERVAL - 1).all(|_| !heap.native_compaction_due()));
    // ...but does once the tenured storage has doubled.
    heap.promoted_since_compaction = 8 * MIB;
    assert!(heap.native_compaction_due());
}

#[test]
fn the_backstop_waits_until_allocation_has_paid_for_the_copy() {
    let mut heap = Heap::new();
    heap.tenured_live_bytes = 8 * MIB;
    // Sixty-four young collections, but only 1 MiB allocated since the last
    // compaction: copying 8 MiB now would not be paid for.
    heap.allocated_since_compaction = MIB;
    assert!((0..COMPACT_BACKSTOP_INTERVAL + 8).all(|_| !heap.native_compaction_due()));
    // After allocating as much as is tenured, the backstop fires.
    heap.allocated_since_compaction = 8 * MIB;
    assert!(heap.native_compaction_due());
}

#[test]
fn a_compaction_records_what_it_left_alive_and_resets_the_counters() {
    let mut heap = Heap::new();
    let keep = heap.alloc(1, 2);
    heap.allocated_since_compaction = 5 * MIB;
    heap.promoted_since_compaction = 5 * MIB;
    let mut roots = vec![keep];
    unsafe { heap.collect_and_compact(&mut roots, &|_| std::rc::Rc::from(Vec::new())) };
    assert_eq!(
        heap.tenured_live_bytes,
        (zun_core::OBJECT_HEADER_SLOTS + 2) * std::mem::size_of::<u64>()
    );
    assert_eq!(heap.promoted_since_compaction, 0);
    assert_eq!(heap.allocated_since_compaction, 0);
}

#[test]
fn a_young_collection_over_a_large_tenured_heap_moves_only_the_survivors() {
    let layout = |_rid: u32| -> std::rc::Rc<[usize]> { std::rc::Rc::from(Vec::new()) };
    let mut heap = Heap::new();
    heap.set_verify_barriers(true);
    // Tenure two thousand objects.
    let mut roots: Vec<Obj> = (0..2000).map(|_| heap.alloc(1, 1)).collect();
    unsafe { heap.collect_young_evacuating(&mut roots, &layout, &mut |_| {}) };
    assert_eq!(heap.generation_counts(), (0, 2000));
    // A fresh epoch: one survivor, many dead.
    let keep = heap.alloc(2, 0);
    for _ in 0..500 {
        heap.alloc(3, 0);
    }
    let mut relocated = 0usize;
    let mut epoch_roots = roots.clone();
    epoch_roots.push(keep);
    unsafe {
        heap.collect_young_evacuating(&mut epoch_roots, &layout, &mut |relocation| {
            for root in &roots {
                // No tenured object moved.
                assert_eq!(relocation.relocate(*root), *root);
            }
            relocated = relocation.len();
        })
    };
    assert_eq!(relocated, 1);
    assert_eq!(heap.generation_counts(), (0, 2001));
}
