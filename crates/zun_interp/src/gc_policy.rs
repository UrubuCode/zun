pub(super) const GC_INTERVAL: u32 = 256;
const MAX_GC_INTERVAL: u32 = 16 * 1024;

pub(super) fn next_gc_interval(current: u32, reclaimed: usize, young_before: usize) -> u32 {
    // Treat a collection that recovers less than a quarter of the nursery as
    // high-survival. This follows the generational budget rule: a nursery
    // whose survivors dominate should be given more allocation budget before
    // the next minor collection.
    let high_survival = young_before > 0 && reclaimed.saturating_mul(4) < young_before;
    if reclaimed == 0 || high_survival {
        current.saturating_mul(2).min(MAX_GC_INTERVAL)
    } else {
        GC_INTERVAL
    }
}

#[cfg(test)]
mod tests {
    use super::next_gc_interval;

    #[test]
    fn increases_budget_when_the_nursery_has_high_survival() {
        assert_eq!(next_gc_interval(256, 1, 16), 512);
        assert_eq!(next_gc_interval(16 * 1024, 1, 16), 16 * 1024);
    }

    #[test]
    fn resets_budget_when_collection_is_productive() {
        assert_eq!(next_gc_interval(2048, 8, 16), 256);
    }
}
