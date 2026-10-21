use super::Value;

pub trait CallHook {
    fn try_call(&self, method_rid: u32, args: &[Value]) -> Option<Value>;

    fn on_backedge(&self, _method_rid: u32, _target_pc: usize) {}

    /// Observe a hot loop together with the live Tier 0 evaluation state.
    /// Implementations may retain the snapshot for a later OSR transfer;
    /// execution remains in the interpreter until that transfer is explicit.
    fn on_backedge_state(
        &self,
        method_rid: u32,
        target_pc: usize,
        _pc: usize,
        _stack: &[Value],
        _locals: &[Value],
    ) {
        self.on_backedge(method_rid, target_pc);
    }
}
