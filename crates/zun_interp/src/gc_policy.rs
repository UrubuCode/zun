use std::sync::OnceLock;

const GC_INTERVAL: u32 = 256;
const MAX_GC_INTERVAL: u32 = 16 * 1024;

/// Parses `ZUN_GC_STRESS`. Anything that is not a positive integer means "not
/// set": a typo must not turn into a budget of 0, which would collect on every
/// poll without the user having asked for it.
fn parse_stress(value: Option<&str>) -> Option<u32> {
    value?.trim().parse::<u32>().ok().filter(|&n| n > 0)
}

/// Read once per process: `poll_gc` runs on every allocation and loop backedge.
fn stress() -> Option<u32> {
    static STRESS: OnceLock<Option<u32>> = OnceLock::new();
    *STRESS.get_or_init(|| parse_stress(std::env::var("ZUN_GC_STRESS").ok().as_deref()))
}

pub(super) fn initial_gc_interval() -> u32 {
    initial_interval_with(stress())
}

pub(super) fn next_gc_interval(current: u32, reclaimed: usize, young_before: usize) -> u32 {
    next_interval_with(stress(), current, reclaimed, young_before)
}

fn initial_interval_with(stress: Option<u32>) -> u32 {
    stress.unwrap_or(GC_INTERVAL)
}

fn next_interval_with(
    stress: Option<u32>,
    current: u32,
    reclaimed: usize,
    young_before: usize,
) -> u32 {
    // A pinned budget must survive every collection: without this, one
    // unproductive collection doubles it and the mode stops stressing.
    if let Some(pinned) = stress {
        return pinned;
    }
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
    use super::{initial_interval_with, next_gc_interval, next_interval_with, parse_stress};

    #[test]
    fn increases_budget_when_the_nursery_has_high_survival() {
        assert_eq!(next_gc_interval(256, 1, 16), 512);
        assert_eq!(next_gc_interval(16 * 1024, 1, 16), 16 * 1024);
    }

    #[test]
    fn resets_budget_when_collection_is_productive() {
        assert_eq!(next_gc_interval(2048, 8, 16), 256);
    }

    #[test]
    fn stress_budget_does_not_grow_or_reset() {
        // Unproductive, high-survival and productive collections alike.
        assert_eq!(next_interval_with(Some(4), 4, 0, 16), 4);
        assert_eq!(next_interval_with(Some(4), 4, 1, 16), 4);
        assert_eq!(next_interval_with(Some(4), 4, 8, 16), 4);
        assert_eq!(next_interval_with(Some(1), 1, 0, 0), 1);
        let mut budget = 1;
        for _ in 0..32 {
            budget = next_interval_with(Some(1), budget, 0, 16);
        }
        assert_eq!(budget, 1);
    }

    #[test]
    fn unset_stress_keeps_the_adaptive_policy() {
        assert_eq!(next_interval_with(None, 256, 1, 16), 512);
        assert_eq!(next_interval_with(None, 16 * 1024, 1, 16), 16 * 1024);
        assert_eq!(next_interval_with(None, 2048, 8, 16), 256);
    }

    #[test]
    fn initial_budget_follows_the_stress_setting() {
        assert_eq!(initial_interval_with(None), 256);
        assert_eq!(initial_interval_with(Some(1)), 1);
        assert_eq!(initial_interval_with(Some(16)), 16);
    }

    #[test]
    fn parses_only_positive_integers() {
        assert_eq!(parse_stress(Some("4")), Some(4));
        assert_eq!(parse_stress(Some(" 8 ")), Some(8));
        assert_eq!(parse_stress(Some("0")), None);
        assert_eq!(parse_stress(Some("abc")), None);
        assert_eq!(parse_stress(Some("-1")), None);
        assert_eq!(parse_stress(Some("")), None);
        assert_eq!(parse_stress(None), None);
    }
}
