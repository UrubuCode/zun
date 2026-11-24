//! The `Call`/`CallVirt` and `CallIndirect` opcodes, plus the interpreter-bridge
//! call infrastructure (`emit_bridge_call`, `emit_bridged_dispatch`,
//! `emit_bridged_indirect_dispatch`, `bridge_fref`, `call_indirect_fref`).
//! Extracted verbatim from `translate/mod.rs`. `op_call` handles the generic
//! direct/virtual user-call body and delegates the string-method cases to
//! `op_call_string` (in `strings.rs`); the post-call pending-exception check
//! lives in `exceptions.rs`.

use super::*;

impl Translator<'_> {
    pub(crate) fn op_call(
        &mut self,
        instr: &zun_il::Instruction,
        i: usize,
        stack: &mut Vec<Slot>,
        block_sigs: &mut HashMap<Block, Vec<JitKind>>,
        offset_of_index: &[usize],
        index_of_offset: &BTreeMap<usize, usize>,
    ) -> Result<(), String> {
        let rid = match instr.operand {
            Operand::Token(t) => t.rid(),
            _ => return Err("call operand".into()),
        };
        let method_name = self
            .zm
            .tables
            .method_defs
            .get(rid as usize - 1)
            .map(|method| self.zm.strings.get(method.name));
        if instr.opcode == Opcode::CallVirt
            && stack.iter().any(|(_, kind)| *kind == JitKind::String)
            && self.op_call_string(method_name, stack)?
        {
            return Ok(());
        }
        let callee = read_body(
            &self.zm.code,
            self.zm.tables.method_defs[rid as usize - 1].body_offset,
        )
        .map_err(|e| e.to_string())?;
        let nargs = callee.param_count as usize;
        if stack.len() < nargs {
            return Err("stack underflow at call".into());
        }
        let raw_args: Vec<Slot> = stack.split_off(stack.len() - nargs);
        let mut arg_values: Vec<Slot> = Vec::with_capacity(nargs);
        for (slot, &param_kind_byte) in raw_args
            .into_iter()
            .zip(&callee.local_kinds[..callee.param_count as usize])
        {
            if matches!(slot.1, JitKind::Object | JitKind::String) {
                self.builder.declare_value_needs_stack_map(slot.0);
            }
            arg_values.push(self.coerce_slot(
                slot,
                JitKind::from_byte(param_kind_byte),
                "call argument",
            )?);
        }
        // A direct call to an ineligible (non-compiled) callee is
        // routed to the interpreter through the bridge. Eligibility
        // only admits this for a non-throwing, marshalable callee.
        if instr.opcode == Opcode::Call && self.bridge_enabled && !self.func_ids.contains_key(&rid)
        {
            self.emit_bridge_call(rid, &arg_values, &callee, stack)?;
            self.after_call_pending_check(block_sigs, stack, offset_of_index[i], index_of_offset)?;
            return Ok(());
        }
        let direct_string_call = instr.opcode == Opcode::CallVirt
            && arg_values
                .first()
                .is_some_and(|(_, kind)| *kind == JitKind::String);
        let inst = if instr.opcode == Opcode::CallVirt && !direct_string_call {
            let receiver = arg_values
                .first()
                .map(|(value, _)| *value)
                .ok_or("virtual call receiver")?;
            let receiver_kind = arg_values.first().map(|(_, kind)| *kind);
            super::objects::require_reference_receiver(
                receiver_kind.ok_or("virtual call receiver")?,
                "callvirt",
            )?;
            // A null receiver has no header to dispatch on: raise instead.
            self.emit_null_check(
                receiver,
                NullDeref::Call { rid },
                stack,
                block_sigs,
                offset_of_index[i],
                index_of_offset,
            )?;
            let slot = self.types.virtual_slot(rid).ok_or("virtual method slot")?;
            let type_rid = self
                .builder
                .ins()
                .load(types::I32, MemFlags::trusted(), receiver, 0);
            let type_rid = self.builder.ins().uextend(types::I64, type_rid);
            let ptrs = self
                .builder
                .ins()
                .iconst(types::I64, self.virtual_table_ptrs as i64);
            let ptr_offset = self.builder.ins().imul_imm(type_rid, 8);
            let table_ptr_addr = self.builder.ins().iadd(ptrs, ptr_offset);
            let table = self
                .builder
                .ins()
                .load(types::I64, MemFlags::trusted(), table_ptr_addr, 0);
            let address = self.builder.ins().iadd_imm(table, (slot * 8) as i64);
            let target = self
                .builder
                .ins()
                .load(types::I64, MemFlags::trusted(), address, 0);
            let mut sig = Signature::new(self.builder.func.signature.call_conv);
            let mut call_args = Vec::new();
            for (slot, &param_kind_byte) in arg_values
                .iter()
                .zip(&callee.local_kinds[..callee.param_count as usize])
            {
                let param_kind = JitKind::from_byte(param_kind_byte);
                if param_kind == JitKind::Other {
                    sig.params.push(AbiParam::new(types::I64));
                    sig.params.push(AbiParam::new(types::I64));
                    let any = self.any_of(*slot)?;
                    let payload = self.join_payload(any);
                    call_args.push(any.tag);
                    call_args.push(payload);
                } else {
                    sig.params.push(AbiParam::new(clif_ty(param_kind)));
                    call_args.push(slot.0);
                }
            }
            if callee.returns_value {
                let return_kind = JitKind::from_byte(callee.return_kind);
                if return_kind == JitKind::Other {
                    sig.returns.push(AbiParam::new(types::I64));
                    sig.returns.push(AbiParam::new(types::I64));
                } else {
                    sig.returns.push(AbiParam::new(clif_ty(return_kind)));
                }
            }
            let sig_ref = self.builder.import_signature(sig);
            self.builder
                .ins()
                .call_indirect(sig_ref, target, &call_args)
        } else {
            let fid = *self
                .func_ids
                .get(&rid)
                .ok_or("call to non-compiled method")?;
            let fref = self.module.declare_func_in_func(fid, self.builder.func);
            let mut call_args = Vec::new();
            for (slot, &param_kind_byte) in arg_values
                .iter()
                .zip(&callee.local_kinds[..callee.param_count as usize])
            {
                if JitKind::from_byte(param_kind_byte) == JitKind::Other {
                    let any = self.any_of(*slot)?;
                    let payload = self.join_payload(any);
                    call_args.push(any.tag);
                    call_args.push(payload);
                } else {
                    call_args.push(slot.0);
                }
            }
            self.builder.ins().call(fref, &call_args)
        };
        let results = self.builder.inst_results(inst).to_vec();
        if callee.returns_value {
            let kind = JitKind::from_byte(callee.return_kind);
            if kind == JitKind::Other {
                // The callee returns (tag, payload); the triple is built right away.
                let any = self.any_from_words(results[0], results[1]);
                self.push_any(stack, any);
            } else {
                let r = results[0];
                if matches!(kind, JitKind::Object | JitKind::String) {
                    self.builder.declare_value_needs_stack_map(r);
                }
                stack.push((r, kind));
            }
        } else {
            // A void callee pushes no native value, but the IL mirrors the
            // interpreter (which pushes `Void`) and follows the call with a
            // `Pop`. Push a throwaway placeholder so that `Pop` balances; it is
            // never read (a void result has no uses in well-formed IL).
            let placeholder = self.builder.ins().iconst(types::I32, 0);
            stack.push((placeholder, JitKind::Bool));
        }
        // A direct/virtual callee may itself have propagated a throw
        // (through a bridged call); check and unwind/route if so.
        self.after_call_pending_check(block_sigs, stack, offset_of_index[i], index_of_offset)?;
        Ok(())
    }

    pub(crate) fn op_call_indirect(
        &mut self,
        instr: &zun_il::Instruction,
        i: usize,
        stack: &mut Vec<Slot>,
        block_sigs: &mut HashMap<Block, Vec<JitKind>>,
        offset_of_index: &[usize],
        index_of_offset: &BTreeMap<usize, usize>,
    ) -> Result<(), String> {
        // The signature method gives the call's arity and return kind.
        let token = match instr.operand {
            Operand::Token(token) => token,
            _ => return Err("callindirect operand".into()),
        };
        let signature = read_body(
            &self.zm.code,
            self.zm.tables.method_defs[token.rid() as usize - 1].body_offset,
        )
        .map_err(|e| e.to_string())?;
        let argc = signature.param_count as usize;
        if stack.len() < argc + 1 {
            return Err("stack underflow at callindirect".into());
        }
        let raw_args: Vec<Slot> = stack.split_off(stack.len() - argc);
        let mut arg_values: Vec<Slot> = Vec::with_capacity(argc);
        for (slot, &param_kind_byte) in raw_args
            .into_iter()
            .zip(&signature.local_kinds[..argc])
        {
            arg_values.push(self.coerce_slot(
                slot,
                JitKind::from_byte(param_kind_byte),
                "indirect call argument",
            )?);
        }
        let (fn_value, fn_kind) = stack.pop().ok_or("callindirect function value")?;
        if fn_kind != JitKind::Function {
            return Err("callindirect expects a function value".into());
        }
        let return_kind = signature
            .returns_value
            .then(|| JitKind::from_byte(signature.return_kind));
        // The function value is carried as its method rid (the
        // no-closure case: environment word 0). Dispatch through the
        // handle-reconstructing indirect bridge, which reaches a
        // captured environment once the producer slice can place one
        // in the carrier; today the environment is always 0.
        let environment = self.builder.ins().iconst(types::I64, 0);
        self.emit_bridged_indirect_dispatch(
            fn_value,
            environment,
            &arg_values,
            return_kind,
            stack,
        )?;
        self.after_call_pending_check(block_sigs, stack, offset_of_index[i], index_of_offset)?;
        Ok(())
    }

    pub(crate) fn bridge_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64)); // method rid
        sig.params.push(AbiParam::new(types::I64)); // packed args pointer
        sig.params.push(AbiParam::new(types::I64)); // argument count
        sig.params.push(AbiParam::new(types::I64)); // caller stack pointer
        sig.params.push(AbiParam::new(types::I64)); // caller frame pointer
        sig.returns.push(AbiParam::new(types::I64)); // packed result
        let id = self
            .module
            .declare_function("zun_jit_call_interp", Linkage::Import, &sig)
            .expect("declare zun_jit_call_interp");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn call_indirect_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64)); // handle identity (rid | reserved<<32)
        sig.params.push(AbiParam::new(types::I64)); // closure environment address, or 0
        sig.params.push(AbiParam::new(types::I64)); // packed args pointer
        sig.params.push(AbiParam::new(types::I64)); // argument count
        sig.params.push(AbiParam::new(types::I64)); // caller stack pointer
        sig.params.push(AbiParam::new(types::I64)); // caller frame pointer
        sig.returns.push(AbiParam::new(types::I64)); // packed result
        let id = self
            .module
            .declare_function("zun_jit_call_indirect", Linkage::Import, &sig)
            .expect("declare zun_jit_call_indirect");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    /// Lower a direct call to an ineligible, non-throwing callee as a bridged
    /// call: pack the arguments into a stack buffer in the shim ABI, call the
    /// host `zun_jit_call_interp`, and unpack the result. Object arguments and
    /// the result are kept in the call's stack map (the bridge runs the
    /// interpreter, a GC safepoint).
    pub(crate) fn emit_bridge_call(
        &mut self,
        rid: u32,
        arg_values: &[Slot],
        callee: &MethodBody<'_>,
        stack: &mut Vec<Slot>,
    ) -> Result<(), String> {
        let target = self.builder.ins().iconst(types::I64, rid as i64);
        let return_kind = callee
            .returns_value
            .then(|| JitKind::from_byte(callee.return_kind));
        self.emit_bridged_dispatch(target, arg_values, return_kind, stack)
    }

    /// Shared lowering for a bridged call to a method `target` (a Cranelift value
    /// holding the method rid — a constant for a direct bridged call, or the
    /// popped function value for `CallIndirect`): pack the args in the shim ABI,
    /// call `zun_jit_call_interp`, and unpack the result by `return_kind`.
    pub(crate) fn emit_bridged_dispatch(
        &mut self,
        target: Value,
        arg_values: &[Slot],
        return_kind: Option<JitKind>,
        stack: &mut Vec<Slot>,
    ) -> Result<(), String> {
        let (buf, arg_words) = self.pack_bridge_args(arg_values, return_kind)?;
        let fref = self.bridge_fref();
        let argc_c = self.builder.ins().iconst(types::I64, arg_words as i64);
        let caller_sp = self.builder.ins().get_stack_pointer(types::I64);
        let caller_fp = self.builder.ins().get_frame_pointer(types::I64);
        let call = self
            .builder
            .ins()
            .call(fref, &[target, buf, argc_c, caller_sp, caller_fp]);
        let result = self.builder.inst_results(call)[0];
        self.unpack_bridge_result(buf, arg_words, result, return_kind, stack);
        Ok(())
    }

    /// Write the arguments of a bridged call into one outgoing buffer, right before
    /// the call: one word per scalar, two (tag, payload) per `any`, plus two result
    /// words when the callee returns an `any`. Returns the buffer address and the
    /// number of argument words. The buffer is in no stack map: the host consumes
    /// it before anything can collect, and the caller keeps using its SSA values.
    fn pack_bridge_args(
        &mut self,
        arg_values: &[Slot],
        return_kind: Option<JitKind>,
    ) -> Result<(Value, usize), String> {
        let arg_words: usize = arg_values
            .iter()
            .map(|&(_, kind)| crate::any_abi::abi_word_count(kind))
            .sum();
        let result_words = if return_kind == Some(JitKind::Other) { 2 } else { 0 };
        let words = arg_words + result_words;
        let buf = if words == 0 {
            self.builder.ins().iconst(types::I64, 0)
        } else {
            let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                (words * 8) as u32,
                3,
            ));
            self.builder.ins().stack_addr(types::I64, slot, 0)
        };
        let mut word = 0usize;
        for &slot in arg_values {
            let (value, kind) = slot;
            if kind == JitKind::Other {
                let any = self.any_of(slot)?;
                let payload = self.join_payload(any);
                self.builder
                    .ins()
                    .store(MemFlags::trusted(), any.tag, buf, (word * 8) as i32);
                self.builder
                    .ins()
                    .store(MemFlags::trusted(), payload, buf, (word * 8 + 8) as i32);
                word += 2;
                continue;
            }
            if matches!(kind, JitKind::Object | JitKind::Array | JitKind::String) {
                self.builder.declare_value_needs_stack_map(value);
            }
            let packed = crate::shim::pack_result(&mut self.builder, value, kind);
            self.builder
                .ins()
                .store(MemFlags::trusted(), packed, buf, (word * 8) as i32);
            word += 1;
        }
        Ok((buf, arg_words))
    }

    /// Push the result of a bridged call: a packed word, or (for an `any`) the
    /// result carrier the host wrote after the arguments, read right away.
    fn unpack_bridge_result(
        &mut self,
        buf: Value,
        arg_words: usize,
        result: Value,
        return_kind: Option<JitKind>,
        stack: &mut Vec<Slot>,
    ) {
        let Some(kind) = return_kind else {
            // A void callee still leaves the placeholder the IL `pop`s (as a direct
            // call does), or that `pop` would underflow.
            let placeholder = self.builder.ins().iconst(types::I32, 0);
            stack.push((placeholder, JitKind::Bool));
            return;
        };
        if kind == JitKind::Other {
            let tag = self.builder.ins().load(
                types::I64,
                MemFlags::trusted(),
                buf,
                (arg_words * 8) as i32,
            );
            let payload = self.builder.ins().load(
                types::I64,
                MemFlags::trusted(),
                buf,
                (arg_words * 8 + 8) as i32,
            );
            let any = self.any_from_words(tag, payload);
            self.push_any(stack, any);
            return;
        }
        let v = crate::shim::unpack_arg(&mut self.builder, result, kind);
        if matches!(kind, JitKind::Object | JitKind::Array) {
            self.builder.declare_value_needs_stack_map(v);
        }
        stack.push((v, kind));
    }

    /// Lowering for a `CallIndirect` on a concrete function value: pack the
    /// explicit arguments in the shim ABI and call `zun_jit_call_indirect` with
    /// the function value's two carrier words (`identity` = packed rid/bound
    /// discriminant, `environment` = closure-environment address or 0). The host
    /// reconstructs the `FunctionHandle` and dispatches on the current heap,
    /// carrying any captured environment the one-word rid path could not.
    ///
    /// The environment word is a managed reference live across this call (a GC
    /// safepoint), so it joins the call's stack map like any object argument.
    pub(crate) fn emit_bridged_indirect_dispatch(
        &mut self,
        identity: Value,
        environment: Value,
        arg_values: &[Slot],
        return_kind: Option<JitKind>,
        stack: &mut Vec<Slot>,
    ) -> Result<(), String> {
        let (buf, arg_words) = self.pack_bridge_args(arg_values, return_kind)?;
        // The closure environment is a managed root across the bridged call.
        self.builder.declare_value_needs_stack_map(environment);
        let fref = self.call_indirect_fref();
        let argc_c = self.builder.ins().iconst(types::I64, arg_words as i64);
        let caller_sp = self.builder.ins().get_stack_pointer(types::I64);
        let caller_fp = self.builder.ins().get_frame_pointer(types::I64);
        let call = self
            .builder
            .ins()
            .call(fref, &[identity, environment, buf, argc_c, caller_sp, caller_fp]);
        let result = self.builder.inst_results(call)[0];
        self.unpack_bridge_result(buf, arg_words, result, return_kind, stack);
        Ok(())
    }
}
