use super::Value;

pub trait CallHook {
    fn try_call(&self, method_rid: u32, args: &[Value]) -> Option<Value>;

    /// Try a host-provided external call identified by module and export.
    /// Returning `None` preserves the ordinary module-loader path.
    fn try_call_external(
        &self,
        _module: &str,
        _name: &str,
        _args: &[Value],
    ) -> Option<Result<Value, super::RunError>> {
        None
    }

    /// Append managed references retained by a tiering backend, such as OSR
    /// frame snapshots, to the interpreter's precise root set.
    fn append_roots(&self, _roots: &mut Vec<zun_gc::Obj>) {}

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
