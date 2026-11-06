//! Object and static-field opcodes (`NewObj`, `UnboxAny`, `CastClass`,
//! `BoxAny`, `LoadStatic`, `StoreStatic`) and their metadata/`FuncRef` helpers
//! (`alloc_fref`, `field_info`, `static_info`, `static_get_fref`,
//! `static_set_fref`, `type_field_range`). Extracted verbatim from
//! `translate/mod.rs`; `LoadField`/`StoreField` stay inline in the dispatcher
//! and call `field_info`/`store_ref_fref` across the module boundary.

use super::*;

impl Translator<'_> {
    pub(crate) fn op_new_obj(&mut self, instr: &zun_il::Instruction, stack: &mut Vec<Slot>) -> Result<(), String> {
        let type_rid = match instr.operand {
            Operand::Token(t) => t.rid(),
            _ => return Err("newobj operand".into()),
        };
        let (_field_start, count) = type_field_range(self.zm, type_rid)?;
        // Pop field initializers (last pushed = last field).
        if stack.len() < count {
            return Err("stack underflow at newobj".into());
        }
        let args: Vec<Slot> = stack.split_off(stack.len() - count);
        // Allocation may reach the GC. Keep reference-valued
        // initializers live while the new object is being built.
        for &(value, kind) in &args {
            if kind == JitKind::Object {
                self.builder.declare_value_needs_stack_map(value);
            }
        }
        // ptr = zun_rt_alloc(type_rid, count, caller_sp)
        let alloc = self.alloc_fref();
        let trid = self.builder.ins().iconst(types::I32, type_rid as i64);
        let nfields = self.builder.ins().iconst(types::I64, count as i64);
        let caller_sp = self.builder.ins().get_stack_pointer(types::I64);
        let inst = self.builder.ins().call(alloc, &[trid, nfields, caller_sp]);
        let ptr = self.builder.inst_results(inst)[0];
        self.builder.declare_value_needs_stack_map(ptr);
        // Store each field as a full i64 slot (same encoding the
        // interpreter uses), at the shared core layout offset.
        for (i, (val, kind)) in args.into_iter().enumerate() {
            if kind == JitKind::Object {
                let store = self.store_ref_fref();
                let field = self.builder.ins().iconst(types::I64, i as i64);
                self.builder.ins().call(store, &[ptr, field, val]);
            } else {
                let raw = crate::shim::pack_result(&mut self.builder, val, kind);
                let off = zun_core::field_offset(i) as i32;
                self.builder.ins().store(MemFlags::trusted(), raw, ptr, off);
            }
        }
        stack.push((ptr, JitKind::Object));
        Ok(())
    }

    pub(crate) fn op_unbox_any(&mut self, stack: &mut Vec<Slot>) -> Result<(), String> {
        let value = stack.pop().ok_or("stack underflow")?;
        if value.1 == JitKind::Other {
            let payload = self
                .builder
                .ins()
                .load(
                    types::I64,
                    MemFlags::trusted(),
                    value.0,
                    crate::any_abi::PAYLOAD_OFFSET,
                );
            let kind = self.method_return_kind;
            if !matches!(kind, JitKind::Num(_) | JitKind::Bool) {
                return Err("unbox.any carrier target must be scalar".into());
            }
            let result = match kind {
                JitKind::Bool => self.builder.ins().ireduce(types::I32, payload),
                JitKind::Num(NumKind::I8 | NumKind::U8) => self.builder.ins().ireduce(types::I8, payload),
                JitKind::Num(NumKind::I16 | NumKind::U16) => self.builder.ins().ireduce(types::I16, payload),
                JitKind::Num(NumKind::I32 | NumKind::U32) => self.builder.ins().ireduce(types::I32, payload),
                JitKind::Num(NumKind::F32) => {
                    let bits = self.builder.ins().ireduce(types::I32, payload);
                    self.builder.ins().bitcast(types::F32, MemFlags::new(), bits)
                }
                JitKind::Num(NumKind::F64) => {
                    self.builder.ins().bitcast(types::F64, MemFlags::new(), payload)
                }
                JitKind::Num(NumKind::I64 | NumKind::U64) => payload,
                _ => unreachable!(),
            };
            stack.push((result, kind));
        } else {
            stack.push(value);
        }
        Ok(())
    }

    pub(crate) fn op_cast_class(&mut self, stack: &mut Vec<Slot>) -> Result<(), String> {
        let value = stack.pop().ok_or("stack underflow")?;
        if !matches!(value.1, JitKind::Object | JitKind::Array) {
            return Err("castclass requires a managed object value".into());
        }
        self.builder.declare_value_needs_stack_map(value.0);
        stack.push(value);
        Ok(())
    }

    pub(crate) fn op_box_any(&mut self, stack: &mut Vec<Slot>) -> Result<(), String> {
        let value = stack.pop().ok_or("stack underflow")?;
        if !matches!(value.1, JitKind::Num(_) | JitKind::Bool) {
            return Err("box.any requires a scalar JIT value".into());
        }
        stack.push(value);
        Ok(())
    }

    pub(crate) fn op_load_static(&mut self, instr: &zun_il::Instruction, stack: &mut Vec<Slot>) -> Result<(), String> {
        let (kind, owner, field) = self.static_info(&instr.operand)?;
        let get = self.static_get_fref();
        let owner_c = self.builder.ins().iconst(types::I64, owner as i64);
        let field_c = self.builder.ins().iconst(types::I64, field as i64);
        let kind_c = self.builder.ins().iconst(types::I64, kind.to_byte() as i64);
        let call = self.builder.ins().call(get, &[owner_c, field_c, kind_c]);
        let raw = self.builder.inst_results(call)[0];
        let v = crate::shim::unpack_arg(&mut self.builder, raw, kind);
        if matches!(kind, JitKind::Object | JitKind::Array) {
            self.builder.declare_value_needs_stack_map(v);
        }
        stack.push((v, kind));
        Ok(())
    }

    pub(crate) fn op_store_static(&mut self, instr: &zun_il::Instruction, stack: &mut Vec<Slot>) -> Result<(), String> {
        let (kind, owner, field) = self.static_info(&instr.operand)?;
        let (val, _) = stack.pop().ok_or("stack underflow")?;
        if matches!(kind, JitKind::Object | JitKind::Array) {
            self.builder.declare_value_needs_stack_map(val);
        }
        let raw = crate::shim::pack_result(&mut self.builder, val, kind);
        let set = self.static_set_fref();
        let owner_c = self.builder.ins().iconst(types::I64, owner as i64);
        let field_c = self.builder.ins().iconst(types::I64, field as i64);
        let kind_c = self.builder.ins().iconst(types::I64, kind.to_byte() as i64);
        self.builder.ins().call(set, &[owner_c, field_c, raw, kind_c]);
        stack.push((val, kind)); // stsfld leaves the stored value
        Ok(())
    }

    pub(crate) fn alloc_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let conv = self.module.isa().default_call_conv();
        let mut sig = cranelift_codegen::ir::Signature::new(conv);
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_rt_alloc", Linkage::Import, &sig)
            .expect("declare zun_rt_alloc");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    /// Resolve a field token to its `(kind, slot)` using module metadata.
    pub(crate) fn field_info(&self, operand: &Operand) -> Result<(JitKind, usize), String> {
        let rid = match operand {
            Operand::Token(t) => t.rid(),
            _ => return Err("field operand".into()),
        };
        let field = self.types.get_field(rid).ok_or("bad field token")?;
        Ok((JitKind::from_byte(field.kind), field.slot))
    }

    /// Resolve a static-field token to `(kind, owner type RID, field RID)`. The
    /// `(owner, field)` pair is the shared slot key; it is computed exactly as the
    /// interpreter's `static_field_key`, so both tiers address the same slot.
    pub(crate) fn static_info(&self, operand: &Operand) -> Result<(JitKind, u32, u32), String> {
        let field_rid = match operand {
            Operand::Token(t) => t.rid(),
            _ => return Err("static field operand".into()),
        };
        let field = self
            .zm
            .tables
            .field_defs
            .get(field_rid as usize - 1)
            .ok_or("bad static field token")?;
        let owner = self
            .zm
            .tables
            .type_defs
            .iter()
            .enumerate()
            .find(|(_, ty)| ty.field_start <= field_rid && field_rid < ty.field_end)
            .map(|(index, _)| index as u32 + 1)
            .ok_or("static field has no owner type")?;
        Ok((JitKind::from_byte(field.kind), owner, field_rid))
    }

    pub(crate) fn static_get_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_static_get", Linkage::Import, &sig)
            .expect("declare zun_jit_static_get");
        self.module.declare_func_in_func(id, self.builder.func)
    }

    pub(crate) fn static_set_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        let id = self
            .module
            .declare_function("zun_jit_static_set", Linkage::Import, &sig)
            .expect("declare zun_jit_static_set");
        self.module.declare_func_in_func(id, self.builder.func)
    }
}

pub(crate) fn type_field_range(zm: &ZmModule, type_rid: u32) -> Result<(u32, usize), String> {
    let ty = zm
        .tables
        .type_defs
        .get(type_rid as usize - 1)
        .ok_or("bad type token")?;
    Ok((ty.field_start, (ty.field_end - ty.field_start) as usize))
}
