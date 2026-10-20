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
    pub(super) callback_id: u64,
    pub(super) queued: bool,
}

/// A finalized target routed to a callback owned by the embedding runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalizationRecord {
    pub token: u64,
    pub callback_id: u64,
}
