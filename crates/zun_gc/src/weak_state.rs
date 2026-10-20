#[derive(Debug, Clone, Copy)]
pub(super) struct WeakSlot {
    pub(super) target: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Ephemeron {
    pub(super) entries: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct FinalizerEntry {
    pub(super) target: usize,
    pub(super) token: u64,
    pub(super) queued: bool,
}
