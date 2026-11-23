//! Object and static-field opcodes (`NewObj`, `UnboxAny`, `CastClass`,
//! `BoxAny`, `LoadStatic`, `StoreStatic`) and their metadata/`FuncRef` helpers
//! (`alloc_fref`, `field_info`, `static_info`, `static_get_fref`,
//! `static_set_fref`, `type_field_range`). Extracted verbatim from
//! `translate/mod.rs`; `LoadField`/`StoreField` stay inline in the dispatcher
//! and call `field_info`/`store_ref_fref` across the module boundary.

use super::*;

impl Translator<'_> {
    pub(crate) fn op_new_obj(
        &mut self,
        instr: &zun_il::Instruction,
        stack: &mut Vec<Slot>,
    ) -> Result<(), String> {
        let type_rid = match instr.operand {
            Operand::Token(t) => t.rid(),
            _ => return Err("newobj operand".into()),
        };
        let (field_start, count) = type_field_range(self.zm, type_rid)?;
        // Pop field initializers (last pushed = last field).
        if stack.len() < count {
            return Err("stack underflow at newobj".into());
        }
        let args: Vec<Slot> = stack.split_off(stack.len() - count);
        // A `null` (an `Object`-kind zero) may only initialize a class-reference field.
        for (i, &(_, kind)) in args.iter().enumerate() {
            let field = self
                .zm
                .tables
                .field_defs
                .get(field_start as usize + i - 1)
                .ok_or("newobj field")?;
            require_reference_agreement(kind, JitKind::from_byte(field.kind), "newobj")?;
        }
        // Allocation may reach the GC. Keep reference-valued
        // initializers live while the new object is being built.
        for &(value, kind) in &args {
            if kind == JitKind::Object {
                self.builder.declare_value_needs_stack_map(value);
            }
        }
        // ptr = zun_rt_alloc(type_rid, count, caller_sp, caller_fp)
        let alloc = self.alloc_fref();
        let trid = self.builder.ins().iconst(types::I32, type_rid as i64);
        let nfields = self.builder.ins().iconst(types::I64, count as i64);
        let caller_sp = self.builder.ins().get_stack_pointer(types::I64);
        let caller_fp = self.builder.ins().get_frame_pointer(types::I64);
        let inst = self
            .builder
            .ins()
            .call(alloc, &[trid, nfields, caller_sp, caller_fp]);
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

    pub(crate) fn op_unbox_any(
        &mut self,
        stack: &mut Vec<Slot>,
        next: Option<&zun_il::Instruction>,
    ) -> Result<(), String> {
        let value = stack.pop().ok_or("stack underflow")?;
        // `unbox.any; conv f64` is the typed read of a `number` out of an `any`
        // carrier. The target is the conversion, not the method return kind
        // (an OSR loop region returns its resume PC), and the tag decides how
        // the payload reads: a float is its bits, an int converts, anything
        // else (`undefined` from a hole or out-of-bounds read) is `NaN`.
        if value.1 == JitKind::Other
            && next.is_some_and(|n| {
                n.opcode == Opcode::Conv && NumKind::from_imm(imm32(&n.operand)) == Some(NumKind::F64)
            })
        {
            let tag = self.builder.ins().load(
                types::I64,
                MemFlags::trusted(),
                value.0,
                crate::any_abi::TAG_OFFSET,
            );
            let payload = self.builder.ins().load(
                types::I64,
                MemFlags::trusted(),
                value.0,
                crate::any_abi::PAYLOAD_OFFSET,
            );
            let as_float = self
                .builder
                .ins()
                .bitcast(types::F64, MemFlags::new(), payload);
            let as_int = self.builder.ins().fcvt_from_sint(types::F64, payload);
            let nan = self.builder.ins().f64const(f64::NAN);
            let is_float = self.builder.ins().icmp_imm(
                IntCC::Equal,
                tag,
                zun_core::AnyTag::Float as i64,
            );
            let is_int = self.builder.ins().icmp_imm(
                IntCC::Equal,
                tag,
                zun_core::AnyTag::Int as i64,
            );
            let int_or_nan = self.builder.ins().select(is_int, as_int, nan);
            let result = self.builder.ins().select(is_float, as_float, int_or_nan);
            stack.push((result, JitKind::Num(NumKind::F64)));
            return Ok(());
        }
        if value.1 == JitKind::Other {
            let payload = self.builder.ins().load(
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
                JitKind::Num(NumKind::I8 | NumKind::U8) => {
                    self.builder.ins().ireduce(types::I8, payload)
                }
                JitKind::Num(NumKind::I16 | NumKind::U16) => {
                    self.builder.ins().ireduce(types::I16, payload)
                }
                JitKind::Num(NumKind::I32 | NumKind::U32) => {
                    self.builder.ins().ireduce(types::I32, payload)
                }
                JitKind::Num(NumKind::F32) => {
                    let bits = self.builder.ins().ireduce(types::I32, payload);
                    self.builder
                        .ins()
                        .bitcast(types::F32, MemFlags::new(), bits)
                }
                JitKind::Num(NumKind::F64) => {
                    self.builder
                        .ins()
                        .bitcast(types::F64, MemFlags::new(), payload)
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

    pub(crate) fn op_load_static(
        &mut self,
        instr: &zun_il::Instruction,
        stack: &mut Vec<Slot>,
    ) -> Result<(), String> {
        let (kind, owner, field) = self.static_info(&instr.operand)?;
        if self.static_inline_f64(kind, field) {
            let v = self.inline_load_static_f64(owner, field);
            stack.push((v, kind));
            return Ok(());
        }
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

    pub(crate) fn op_store_static(
        &mut self,
        instr: &zun_il::Instruction,
        stack: &mut Vec<Slot>,
    ) -> Result<(), String> {
        let (kind, owner, field) = self.static_info(&instr.operand)?;
        let (val, _) = stack.pop().ok_or("stack underflow")?;
        if self.static_inline_f64(kind, field) {
            self.inline_store_static_f64(owner, field, val);
            stack.push((val, kind));
            return Ok(());
        }
        if matches!(kind, JitKind::Object | JitKind::Array) {
            self.builder.declare_value_needs_stack_map(val);
        }
        let raw = crate::shim::pack_result(&mut self.builder, val, kind);
        let set = self.static_set_fref();
        let owner_c = self.builder.ins().iconst(types::I64, owner as i64);
        let field_c = self.builder.ins().iconst(types::I64, field as i64);
        let kind_c = self.builder.ins().iconst(types::I64, kind.to_byte() as i64);
        self.builder
            .ins()
            .call(set, &[owner_c, field_c, raw, kind_c]);
        stack.push((val, kind)); // stsfld leaves the stored value
        Ok(())
    }


    /// Whether a static access of `kind` on `field` is lowered inline: only an
    /// `f64` field, on a little-endian JIT (not AOT, which has no thread cell),
    /// whose RID the flat store indexes. Everything else takes the helper.
    fn static_inline_f64(&self, kind: JitKind, field: u32) -> bool {
        self.statics_cell_addr != 0
            && cfg!(target_endian = "little")
            && zun_gc::STATIC_SLOT_STATE_OFFSET == zun_gc::STATIC_SLOT_OWNER_OFFSET + 4
            && matches!(kind, JitKind::Num(NumKind::F64))
            && field <= zun_gc::STATIC_MAX_FLAT_FIELD
    }

    /// Emit the shared prefix of an inline static access: the thread cell, the
    /// chunk table, the chunk for `field` (any null branches to `slow`). Leaves
    /// the builder in a block holding the chunk pointer, and returns it with the
    /// slot byte offset inside the chunk.
    fn static_slot_prefix(&mut self, field: u32, slow: Block) -> (Value, i32) {
        let cell = self
            .builder
            .ins()
            .iconst(types::I64, self.statics_cell_addr as i64);
        let table = self.builder.ins().load(types::I64, MemFlags::trusted(), cell, 0);
        let have_chunk_table = self.builder.create_block();
        self.builder.ins().brif(table, have_chunk_table, &[], slow, &[]);
        self.builder.switch_to_block(have_chunk_table);
        self.builder.seal_block(have_chunk_table);
        let chunk_index = (field >> zun_gc::STATIC_CHUNK_BITS) as i32;
        let chunk = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), table, chunk_index * 8);
        let have_chunk = self.builder.create_block();
        self.builder.ins().brif(chunk, have_chunk, &[], slow, &[]);
        self.builder.switch_to_block(have_chunk);
        self.builder.seal_block(have_chunk);
        let in_chunk = (field & ((1 << zun_gc::STATIC_CHUNK_BITS) - 1)) as usize;
        (chunk, (in_chunk * zun_gc::STATIC_SLOT_SIZE) as i32)
    }

    /// `ldsfld` of an `f64` static: load tag and payload straight out of the
    /// flat store when the slot belongs to this owner and holds a float; a cold
    /// block calls the helper for every other case (unset, other owner, other
    /// tag).
    fn inline_load_static_f64(&mut self, owner: u32, field: u32) -> Value {
        let slow = self.builder.create_block();
        let merge = self.builder.create_block();
        self.builder.append_block_param(merge, types::F64);
        self.builder.set_cold_block(slow);

        let (chunk, slot) = self.static_slot_prefix(field, slow);
        let flags = MemFlags::trusted();
        // Owner (low half) and state (high half) are adjacent: one compare.
        let word = self.builder.ins().load(
            types::I64,
            flags,
            chunk,
            slot + zun_gc::STATIC_SLOT_OWNER_OFFSET as i32,
        );
        let expected = ((zun_gc::STATIC_SLOT_SET as i64) << 32) | owner as i64;
        let expected = self.builder.ins().iconst(types::I64, expected);
        let owned_and_set = self.builder.ins().icmp(IntCC::Equal, word, expected);
        let tag = self.builder.ins().uload8(
            types::I32,
            flags,
            chunk,
            slot + zun_gc::STATIC_SLOT_TAG_OFFSET as i32,
        );
        let float_tag = self
            .builder
            .ins()
            .icmp_imm(IntCC::Equal, tag, zun_core::AnyTag::Float as u8 as i64);
        let hit = self.builder.ins().band(owned_and_set, float_tag);
        let payload = self.builder.ins().load(
            types::F64,
            flags,
            chunk,
            slot + zun_gc::STATIC_SLOT_PAYLOAD_OFFSET as i32,
        );
        self.builder
            .ins()
            .brif(hit, merge, &[BlockArg::from(payload)], slow, &[]);

        self.builder.switch_to_block(slow);
        self.builder.seal_block(slow);
        let get = self.static_get_fref();
        let owner_c = self.builder.ins().iconst(types::I64, owner as i64);
        let field_c = self.builder.ins().iconst(types::I64, field as i64);
        let kind = JitKind::Num(NumKind::F64);
        let kind_c = self.builder.ins().iconst(types::I64, kind.to_byte() as i64);
        let call = self.builder.ins().call(get, &[owner_c, field_c, kind_c]);
        let raw = self.builder.inst_results(call)[0];
        let value = crate::shim::unpack_arg(&mut self.builder, raw, kind);
        self.builder.ins().jump(merge, &[BlockArg::from(value)]);

        self.builder.switch_to_block(merge);
        self.builder.seal_block(merge);
        self.builder.block_params(merge)[0]
    }

    /// `stsfld` of an `f64` static: when the slot is already claimed by this
    /// owner (whether or not it holds a value) write tag, payload and state in
    /// place. A float holds no reference, so no barrier or root bookkeeping is
    /// involved; the cold block takes the helper (first write, other owner).
    fn inline_store_static_f64(&mut self, owner: u32, field: u32, value: Value) {
        let slow = self.builder.create_block();
        let done = self.builder.create_block();
        self.builder.set_cold_block(slow);

        let (chunk, slot) = self.static_slot_prefix(field, slow);
        let flags = MemFlags::trusted();
        let word = self.builder.ins().load(
            types::I64,
            flags,
            chunk,
            slot + zun_gc::STATIC_SLOT_OWNER_OFFSET as i32,
        );
        let low = self.builder.ins().ireduce(types::I32, word);
        let same_owner = self
            .builder
            .ins()
            .icmp_imm(IntCC::Equal, low, owner as i64);
        let high = self.builder.ins().ushr_imm(word, 32);
        let claimed = self.builder.ins().icmp_imm(IntCC::NotEqual, high, 0);
        let ok = self.builder.ins().band(same_owner, claimed);
        let fast = self.builder.create_block();
        self.builder.ins().brif(ok, fast, &[], slow, &[]);

        self.builder.switch_to_block(fast);
        self.builder.seal_block(fast);
        let tag = self
            .builder
            .ins()
            .iconst(types::I8, zun_core::AnyTag::Float as u8 as i64);
        self.builder.ins().store(
            flags,
            tag,
            chunk,
            slot + zun_gc::STATIC_SLOT_TAG_OFFSET as i32,
        );
        self.builder.ins().store(
            flags,
            value,
            chunk,
            slot + zun_gc::STATIC_SLOT_PAYLOAD_OFFSET as i32,
        );
        let set = self
            .builder
            .ins()
            .iconst(types::I32, zun_gc::STATIC_SLOT_SET as i64);
        self.builder.ins().store(
            flags,
            set,
            chunk,
            slot + zun_gc::STATIC_SLOT_STATE_OFFSET as i32,
        );
        self.builder.ins().jump(done, &[]);

        self.builder.switch_to_block(slow);
        self.builder.seal_block(slow);
        let kind = JitKind::Num(NumKind::F64);
        let raw = crate::shim::pack_result(&mut self.builder, value, kind);
        let set_fn = self.static_set_fref();
        let owner_c = self.builder.ins().iconst(types::I64, owner as i64);
        let field_c = self.builder.ins().iconst(types::I64, field as i64);
        let kind_c = self.builder.ins().iconst(types::I64, kind.to_byte() as i64);
        self.builder
            .ins()
            .call(set_fn, &[owner_c, field_c, raw, kind_c]);
        self.builder.ins().jump(done, &[]);

        self.builder.switch_to_block(done);
        self.builder.seal_block(done);
    }

    pub(crate) fn alloc_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let conv = self.module.isa().default_call_conv();
        let mut sig = cranelift_codegen::ir::Signature::new(conv);
        sig.params.push(AbiParam::new(types::I32));
        sig.params.push(AbiParam::new(types::I64));
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

/// What a null check guards, for the `TypeError` text (shared with Tier 0).
#[derive(Clone, Copy)]
pub(crate) enum NullDeref {
    /// `ldfld` (`storing: false`) or `stfld` (`storing: true`) of this field token.
    Field { rid: u32, storing: bool },
    /// `callvirt` of this method token.
    Call { rid: u32 },
}

impl NullDeref {
    /// The `(kind, rid)` pair `zun_jit_null_deref` takes.
    fn operands(self) -> (i64, i64) {
        match self {
            NullDeref::Field { rid, storing } => (i64::from(storing), i64::from(rid)),
            NullDeref::Call { rid } => (2, i64::from(rid)),
        }
    }
}

/// The RID of a field-token operand.
pub(crate) fn field_token_rid(operand: &Operand) -> Result<u32, String> {
    match operand {
        Operand::Token(token) => Ok(token.rid()),
        _ => Err("field operand".into()),
    }
}

/// `null` is a zero `Object` handle, so an `Object` value and a non-`Object` slot
/// (or the reverse) never agree: refusing the pair keeps a null out of an `any`,
/// array or string slot, where zero is a different thing.
pub(crate) fn require_reference_agreement(
    got: JitKind,
    want: JitKind,
    what: &str,
) -> Result<(), String> {
    if (got == JitKind::Object) != (want == JitKind::Object) {
        return Err(format!(
            "{what}: {got:?} value meets a {want:?} slot (null reference cannot be represented)"
        ));
    }
    Ok(())
}

/// A field or method receiver must be a managed reference.
pub(crate) fn require_reference_receiver(got: JitKind, what: &str) -> Result<(), String> {
    if matches!(got, JitKind::Object | JitKind::Array) {
        Ok(())
    } else {
        Err(format!("{what} receiver is a {got:?} value"))
    }
}

impl Translator<'_> {
    /// Guard a dereference of `receiver`: a zero handle is `null`, which raises the
    /// JS `TypeError` Tier 0 raises instead of reading address 0. The raise goes
    /// through the host (`zun_jit_null_deref` leaves the exception pending) and the
    /// usual after-call routing sends it to a catch in this frame or out of it. The
    /// check is a compare and a cold branch; execution continues in the new block.
    ///
    /// Without a bridge (AOT, the codegen-only tests) there is no host to build the
    /// error, so the cold path traps. A null receiver cannot be reported there, and
    /// reading address 0 would not be better.
    pub(crate) fn emit_null_check(
        &mut self,
        receiver: Value,
        what: NullDeref,
        stack: &[Slot],
        block_sigs: &mut HashMap<Block, Vec<JitKind>>,
        call_offset: usize,
        index_of_offset: &BTreeMap<usize, usize>,
    ) -> Result<(), String> {
        let proceed = self.builder.create_block();
        let slow = self.builder.create_block();
        self.builder.set_cold_block(slow);
        self.builder.ins().brif(receiver, proceed, &[], slow, &[]);

        self.builder.switch_to_block(slow);
        if self.bridge_enabled {
            let (kind, rid) = what.operands();
            let raise = self.null_deref_fref();
            let kind = self.builder.ins().iconst(types::I64, kind);
            let rid = self.builder.ins().iconst(types::I64, rid);
            let caller_sp = self.builder.ins().get_stack_pointer(types::I64);
            let caller_fp = self.builder.ins().get_frame_pointer(types::I64);
            self.builder
                .ins()
                .call(raise, &[kind, rid, caller_sp, caller_fp]);
            // The throw is pending now: route it exactly like a callee's. Its
            // continuation block is unreachable, so it only needs a terminator.
            let mut after = stack.to_vec();
            self.route_pending_check(block_sigs, &mut after, call_offset, index_of_offset, true)?;
        }
        self.builder
            .ins()
            .trap(cranelift_codegen::ir::TrapCode::unwrap_user(1));

        self.builder.switch_to_block(proceed);
        Ok(())
    }

    fn null_deref_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        if let Some(fref) = self.null_deref_ref {
            return fref;
        }
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64)); // caller stack pointer
        sig.params.push(AbiParam::new(types::I64)); // caller frame pointer
        let id = self
            .module
            .declare_function("zun_jit_null_deref", Linkage::Import, &sig)
            .expect("declare zun_jit_null_deref");
        let fref = self.module.declare_func_in_func(id, self.builder.func);
        self.null_deref_ref = Some(fref);
        fref
    }
}
