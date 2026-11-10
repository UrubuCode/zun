use super::Value;

pub trait CallHook {
    fn try_call(&self, method_rid: u32, args: &[Value]) -> Option<Value>;

    /// A thrown exception left pending by the most recent [`Self::try_call`]
    /// whose compiled body unwound (e.g. a JIT method that propagated a throw
    /// out of a bridged call). The interpreter consults this right after a
    /// successful `try_call` and, if `Some`, routes it as a thrown value instead
    /// of using the returned result. The default has no pending channel.
    fn take_pending_exception(&self) -> Option<Value> {
        None
    }

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

    /// Typed variant used by native hosts. The default preserves compatibility
    /// with hooks that only need the module/export name and runtime values.
    fn try_call_external_typed(
        &self,
        module: &str,
        name: &str,
        _signature: &[u8],
        args: &[Value],
    ) -> Option<Result<Value, super::RunError>> {
        self.try_call_external(module, name, args)
    }

    /// Append managed references retained by a tiering backend, such as OSR
    /// frame snapshots, to the interpreter's precise root set.
    fn append_roots(&self, _roots: &mut Vec<zun_gc::Obj>) {}

    /// Rewrite managed references retained by deferred tiering state after a
    /// compacting collection. Implementations that retain no snapshots can
    /// keep the default no-op.
    fn rewrite_managed_roots(&self, _relocation: &zun_gc::RelocationMap) {}

    fn on_backedge(&self, _method_rid: u32, _target_pc: usize) {}

    /// Try to run the remaining iterations of the loop headed at `header_pc`
    /// natively (loop on-stack replacement). On success the loop has run,
    /// `locals` has been OVERWRITTEN in place with the post-loop values, and
    /// `Some(resume_pc)` (an IL byte offset) is returned; the interpreter
    /// continues from there with an empty operand stack. Only valid to call
    /// when the operand stack is empty. The default returns `None`, which
    /// keeps the interpreter looping.
    fn try_loop_osr(
        &self,
        _method_rid: u32,
        _header_pc: usize,
        _locals: &mut [Value],
    ) -> Option<usize> {
        None
    }

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
