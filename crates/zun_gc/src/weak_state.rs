#[derive(Debug, Clone, Copy)]
pub(super) struct WeakSlot {
    pub(super) target: Option<usize>,
    /// Set when the referent is a function value, which has no heap object of its
    /// own: the handle with its environment cleared. A function that carries a
    /// managed environment is weakly held through `target` (the environment) and
    /// rebuilt on dereference; one without an environment owns no managed storage
    /// and is simply always live.
    pub(super) callee: Option<zun_core::FunctionHandle>,
    /// Whether that function was created with a managed environment, so a cleared
    /// `target` means the closure was collected rather than never weak at all.
    pub(super) env_bearing: bool,
    /// Set when the referent is an unregistered symbol. A symbol owns no managed
    /// storage and is never reclaimed, so the reference is always live.
    pub(super) symbol: Option<zun_core::SymbolId>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Ephemeron {
    pub(super) entries: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct FinalizerEntry {
    pub(super) target: usize,
    /// The held value delivered to the cleanup callback. Arbitrary (boxed into
    /// `Any`): an object reference here is GC-traced while the registration is
    /// pending and rewritten on compaction.
    pub(super) held: zun_core::Any,
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
    /// The held value to deliver to the cleanup callback (arbitrary `Any`).
    pub held: zun_core::Any,
    pub callback_id: u64,
    /// Optional callable callback, including its closure environment.
    pub callback: Option<zun_core::FunctionHandle>,
}
