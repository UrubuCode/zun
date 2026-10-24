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
    pub(super) callback: Option<zun_core::FunctionHandle>,
    /// Address identity of the unregister token object, or `0` when the
    /// registration was made without one. Spec: registrations without an
    /// unregister token can never be removed. Held weakly — the token address
    /// is rewritten on compaction but does not keep the token object alive.
    pub(super) unregister_token: usize,
    pub(super) queued: bool,
}

/// A finalized target routed to a callback owned by the embedding runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalizationRecord {
    pub token: u64,
    pub callback_id: u64,
    /// Optional callable callback, including its closure environment.
    pub callback: Option<zun_core::FunctionHandle>,
}
