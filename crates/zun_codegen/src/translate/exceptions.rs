//! The `Throw` opcode and the post-call pending-exception protocol. Extracted
//! verbatim from `translate/mod.rs`: the same-frame throw lowering, the lazily
//! created exceptional-exit block (`exc_exit_block`/`finish_exc_exit`), the
//! after-call routing check (`after_call_pending_check`), its `rebuild_stack`
//! helper, and the `route_pending` import. `finish_exc_exit` is still called
//! exactly once by the dispatcher after the main loop.

use super::*;

impl Translator<'_> {
    pub(crate) fn op_throw(
        &mut self,
        i: usize,
        stack: &mut Vec<Slot>,
        block_sigs: &mut HashMap<Block, Vec<JitKind>>,
        offset_of_index: &[usize],
        index_of_offset: &BTreeMap<usize, usize>,
    ) -> Result<(), String> {
        // Same-frame throw: branch to the innermost enclosing
        // catch-all handler (smallest region, as the interpreter
        // picks), carrying only the thrown value — the interpreter
        // clears the operand stack and pushes the exception before
        // entering the handler, so the catch block starts with just
        // it. `exception_regions_jit_safe` guarantees an enclosing
        // handler exists and the thrown value is a representable kind.
        let (exc, exc_kind) = stack.pop().ok_or("throw operand")?;
        let throw_off = offset_of_index[i];
        let handler = self
            .handlers
            .iter()
            .filter(|h| h.try_start as usize <= throw_off && throw_off < h.try_end as usize)
            .min_by_key(|h| h.try_end - h.try_start)
            .ok_or("throw has no enclosing handler")?;
        let target_idx = *index_of_offset
            .get(&(handler.handler_start as usize))
            .ok_or("handler start not on an instruction boundary")?;
        let target = *self
            .blocks
            .get(&target_idx)
            .ok_or("handler block missing")?;
        if matches!(exc_kind, JitKind::Object | JitKind::Array | JitKind::String) {
            self.builder.declare_value_needs_stack_map(exc);
        }
        let args = self.thread_edge(block_sigs, target, &[(exc, exc_kind)]);
        self.builder.ins().jump(target, &args);
        Ok(())
    }

    pub(crate) fn route_pending_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64)); // catch type rid
        sig.returns.push(AbiParam::new(types::I64)); // caught object handle, or 0
        let id = self
            .module
            .declare_function("zun_jit_route_pending", Linkage::Import, &sig)
            .expect("declare zun_jit_route_pending");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    /// The method's single exceptional-exit block (created lazily). Its body is
    /// emitted by [`Self::finish_exc_exit`] after the main translation, so this
    /// only needs to hand back the block to branch to.
    pub(crate) fn exc_exit_block(&mut self) -> Block {
        if let Some(block) = self.exc_exit {
            return block;
        }
        if self.osr_exit.is_some() {
            // The OSR loop exit has no way to propagate a pending throw yet.
            self.osr_error =
                Some("OSR loop exit does not support bridged calls that may throw".into());
        }
        let block = self.builder.create_block();
        self.exc_exit = Some(block);
        block
    }

    /// Emit the exceptional-exit block's body: return an ABI-shaped dummy value
    /// (never used — the pending exception the interpreter reads overrides it),
    /// leaving the pending slot set so callers up the chain also unwind.
    pub(crate) fn finish_exc_exit(&mut self) {
        let Some(block) = self.exc_exit else {
            return;
        };
        self.builder.switch_to_block(block);
        if !self.method_returns_value {
            self.builder.ins().return_(&[]);
        } else if self.method_return_kind == JitKind::Other {
            let zero = self.builder.ins().iconst(types::I64, 0);
            self.builder.ins().return_(&[zero, zero]);
        } else {
            let zero = typed_zero(&mut self.builder, self.method_return_kind);
            self.builder.ins().return_(&[zero]);
        }
    }

    /// After a method call (when the bridge is enabled), check the pending-
    /// exception slot and route a propagated throw. If the call sits inside a
    /// protected region, route via `zun_jit_route_pending(catch_type)`: a caught
    /// throw branches to the same-frame catch block with the thrown object (and
    /// the slot is cleared by the helper), while an unmatched throw falls through
    /// to the exceptional exit (leaving it pending to propagate). Outside any
    /// region, a pending throw always goes to the exceptional exit. The non-throw
    /// path continues in a fresh block with the operand stack threaded through.
    pub(crate) fn after_call_pending_check(
        &mut self,
        block_sigs: &mut HashMap<Block, Vec<JitKind>>,
        stack: &mut Vec<Slot>,
        call_offset: usize,
        index_of_offset: &BTreeMap<usize, usize>,
    ) -> Result<(), String> {
        if !self.bridge_enabled {
            return Ok(());
        }
        let addr = self
            .builder
            .ins()
            .iconst(types::I64, self.pending_slot_addr as i64);
        let zero = self.builder.ins().iconst(types::I64, 0);

        // The innermost protected region enclosing this call, if any (smallest,
        // matching the interpreter's handler selection).
        let handler = self
            .handlers
            .iter()
            .filter(|h| (h.try_start as usize) <= call_offset && call_offset < h.try_end as usize)
            .min_by_key(|h| h.try_end - h.try_start)
            .copied();

        if let Some(handler) = handler {
            // Routed path: ask the runtime whether a pending throw is caught here.
            let catch_rid = handler.catch_type.map_or(0, |token| token.rid());
            let fref = self.route_pending_fref();
            let crid = self.builder.ins().iconst(types::I64, catch_rid as i64);
            let call = self.builder.ins().call(fref, &[crid]);
            let caught = self.builder.inst_results(call)[0];
            self.builder.declare_value_needs_stack_map(caught);
            let caught_nz = self.builder.ins().icmp(IntCC::NotEqual, caught, zero);
            let catch_prep = self.builder.create_block();
            let after_route = self.builder.create_block();
            self.builder
                .ins()
                .brif(caught_nz, catch_prep, &[], after_route, &[]);

            // Caught: branch to the catch block with the thrown object (the catch
            // body begins with just the exception, as for a same-frame `throw`).
            self.builder.switch_to_block(catch_prep);
            let &target_idx = index_of_offset
                .get(&(handler.handler_start as usize))
                .ok_or("handler start not on an instruction boundary")?;
            let catch_block = *self
                .blocks
                .get(&target_idx)
                .ok_or("handler block missing")?;
            let catch_args =
                self.thread_edge(block_sigs, catch_block, &[(caught, JitKind::Object)]);
            self.builder.ins().jump(catch_block, &catch_args);

            // Not caught here (or no pending): a still-set slot propagates out.
            self.builder.switch_to_block(after_route);
            let pending = self
                .builder
                .ins()
                .load(types::I64, MemFlags::trusted(), addr, 0);
            let still = self.builder.ins().icmp(IntCC::NotEqual, pending, zero);
            let cont = self.builder.create_block();
            let args = self.thread_edge(block_sigs, cont, stack);
            let exc = self.exc_exit_block();
            self.builder.ins().brif(still, exc, &[], cont, &args);
            self.builder.switch_to_block(cont);
            *stack = Self::rebuild_stack(&self.builder, block_sigs, cont);
            return Ok(());
        }

        // No enclosing region: a pending throw propagates straight out.
        let pending = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), addr, 0);
        let is_pending = self.builder.ins().icmp(IntCC::NotEqual, pending, zero);
        let cont = self.builder.create_block();
        let args = self.thread_edge(block_sigs, cont, stack);
        let exc = self.exc_exit_block();
        self.builder.ins().brif(is_pending, exc, &[], cont, &args);
        self.builder.switch_to_block(cont);
        *stack = Self::rebuild_stack(&self.builder, block_sigs, cont);
        Ok(())
    }

    /// Rebuild the operand stack from a continuation block's parameters.
    pub(crate) fn rebuild_stack(
        builder: &FunctionBuilder,
        block_sigs: &HashMap<Block, Vec<JitKind>>,
        block: Block,
    ) -> Vec<Slot> {
        match block_sigs.get(&block) {
            Some(kinds) => builder
                .block_params(block)
                .iter()
                .zip(kinds)
                .map(|(&value, &kind)| (value, kind))
                .collect(),
            None => Vec::new(),
        }
    }
}
