pub(super) const GC_INTERVAL: u32 = 256;
const MAX_GC_INTERVAL: u32 = 16 * 1024;

pub(super) fn next_gc_interval(current: u32, reclaimed: usize) -> u32 {
    if reclaimed == 0 {
        current.saturating_mul(2).min(MAX_GC_INTERVAL)
    } else {
        GC_INTERVAL
    }
}
