//! First-class `any` values in compiled frames.
//!
//! An `any` is two machine words at every boundary (`tag`, `payload`, see
//! `any_abi`), but inside a compiled frame it is **three SSA values / Cranelift
//! variables, never a stack slot**:
//!
//! - `tag`: the [`zun_core::AnyTag`] byte, widened to `i64`;
//! - `bits`: the payload when the tag is not a managed reference, else `0`;
//! - `obj`: the payload when the tag IS a managed reference
//!   ([`zun_core::AnyTag::MANAGED_REFERENCE`]), else `0`. This value is declared
//!   as needing a stack map, so a moving collection rewrites it like a typed
//!   object reference; a zero `obj` is skipped by the root scan exactly like a
//!   null typed reference.
//!
//! The payload is `bits | obj`. The collector only ever learns about a compiled
//! frame through stack maps, which can hold SSA values that ARE references; a
//! value whose reference-ness depends on its tag cannot be scanned
//! conservatively because the collector moves objects. Splitting the payload by
//! tag at the point where an `any` is created makes the question static.
//!
//! On the operand stack an `any` is a [`Slot`] `(tag, JitKind::Other)`; the
//! other two values are found through `Translator::any_parts`, keyed by the tag
//! value. Every stack entry carries a **fresh** tag value (see
//! [`Translator::fresh_tag`]) so the key is never shared by two entries with
//! different parts.

use super::*;

/// The three SSA values of an `any` (see the module documentation).
#[derive(Clone, Copy, Debug)]
pub(crate) struct AnyVal {
    pub(crate) tag: Value,
    pub(crate) bits: Value,
    pub(crate) obj: Value,
}

/// A statically known `any` constant (`null` or `undefined`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AnyConst {
    Null,
    Undef,
}

const OBJECT_TAG: i64 = zun_core::AnyTag::MANAGED_REFERENCE as i64;

impl Translator<'_> {
    /// Import a host helper once per function (the `FuncRef` is only valid in the
    /// function being built, so the cache lives on the translator).
    pub(crate) fn import(
        &mut self,
        name: &'static str,
        params: &[Type],
        rets: &[Type],
    ) -> cranelift_codegen::ir::FuncRef {
        if let Some(&fref) = self.imports.get(name) {
            return fref;
        }
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        for &ty in params {
            sig.params.push(AbiParam::new(ty));
        }
        for &ty in rets {
            sig.returns.push(AbiParam::new(ty));
        }
        let id = self
            .module
            .declare_function(name, Linkage::Import, &sig)
            .unwrap_or_else(|e| panic!("declare {name}: {e}"));
        let fref = self.module.declare_func_in_func(id, self.builder.func);
        self.imports.insert(name, fref);
        fref
    }

    /// A tag value no other stack entry shares.
    fn fresh_tag(&mut self, tag: Value) -> Value {
        self.builder.ins().iadd_imm(tag, 0)
    }

    /// Split a boundary `(tag, payload)` pair into `(bits, obj)`. This is the
    /// only place where the tag decides whether a payload is a reference.
    pub(crate) fn split_payload(&mut self, tag: Value, payload: Value) -> (Value, Value) {
        split_payload_in(&mut self.builder, tag, payload)
    }

    /// The boundary payload word of an `any`: `bits | obj`.
    pub(crate) fn join_payload(&mut self, any: AnyVal) -> Value {
        self.builder.ins().bor(any.bits, any.obj)
    }

    /// Build an [`AnyVal`] from a boundary `(tag, payload)` pair.
    pub(crate) fn any_from_words(&mut self, tag: Value, payload: Value) -> AnyVal {
        let (bits, obj) = self.split_payload(tag, payload);
        AnyVal { tag, bits, obj }
    }

    /// Push an `any` onto the operand stack.
    pub(crate) fn push_any(&mut self, stack: &mut Vec<Slot>, any: AnyVal) {
        let tag = self.fresh_tag(any.tag);
        self.builder.declare_value_needs_stack_map(any.obj);
        self.any_parts.insert(tag, (any.bits, any.obj));
        stack.push((tag, JitKind::Other));
    }

    /// Push a statically known `null` / `undefined`.
    pub(crate) fn push_any_const(&mut self, stack: &mut Vec<Slot>, what: AnyConst) {
        let tag = match what {
            AnyConst::Null => zun_core::AnyTag::Null,
            AnyConst::Undef => zun_core::AnyTag::Void,
        };
        let tag = self.builder.ins().iconst(types::I64, tag as i64);
        let zero = self.builder.ins().iconst(types::I64, 0);
        self.push_any(
            stack,
            AnyVal {
                tag,
                bits: zero,
                obj: zero,
            },
        );
        let pushed = stack.last().expect("just pushed").0;
        self.any_consts.insert(pushed, what);
    }

    /// The parts of an `any` stack entry.
    pub(crate) fn any_of(&self, slot: Slot) -> Result<AnyVal, String> {
        if slot.1 != JitKind::Other {
            return Err(format!("expected an any value, found {:?}", slot.1));
        }
        let &(bits, obj) = self
            .any_parts
            .get(&slot.0)
            .ok_or("an any stack entry has no parts")?;
        Ok(AnyVal {
            tag: slot.0,
            bits,
            obj,
        })
    }

    /// Read an `any` local's current parts.
    pub(crate) fn load_any_local(&mut self, local: usize) -> Result<AnyVal, String> {
        let (bits_var, obj_var) = self.any_vars[local].ok_or("not an any local")?;
        let tag = self.builder.use_var(self.vars[local]);
        let bits = self.builder.use_var(bits_var);
        let obj = self.builder.use_var(obj_var);
        Ok(AnyVal { tag, bits, obj })
    }

    /// Write an `any` into a local's three variables.
    pub(crate) fn store_any_local(&mut self, local: usize, any: AnyVal) -> Result<(), String> {
        let (bits_var, obj_var) = self.any_vars[local].ok_or("not an any local")?;
        self.builder.def_var(self.vars[local], any.tag);
        self.builder.def_var(bits_var, any.bits);
        self.builder.def_var(obj_var, any.obj);
        Ok(())
    }

    /// Read a result carrier written by a helper **immediately** after the call,
    /// with no safepoint in between, and turn it into an [`AnyVal`]. The carrier
    /// is dead afterwards: the caller continues with the SSA triple, which the
    /// stack map keeps current.
    pub(crate) fn load_carrier(&mut self, base: Value) -> AnyVal {
        let tag = self.builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            base,
            crate::any_abi::TAG_OFFSET,
        );
        let payload = self.builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            base,
            crate::any_abi::PAYLOAD_OFFSET,
        );
        self.any_from_words(tag, payload)
    }

    /// A fresh carrier-sized outgoing area (used only around one helper call).
    pub(crate) fn carrier_area(&mut self) -> Value {
        let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            crate::any_abi::CARRIER_SIZE,
            crate::any_abi::CARRIER_ALIGN_SHIFT,
        ));
        self.builder.ins().stack_addr(types::I64, slot, 0)
    }

    /// `box.any`: Tier 0 boxes with `Value::box_any` (`Int` -> `Any::int`, `Float`
    /// -> `Any::float`, `Bool`, a non-null object -> `Any::object`, `Null` ->
    /// `Any::null`, an `any` unchanged).
    pub(crate) fn op_box_any(&mut self, stack: &mut Vec<Slot>) -> Result<(), String> {
        let slot = stack.pop().ok_or("stack underflow")?;
        if slot.1 == JitKind::Other {
            stack.push(slot);
            return Ok(());
        }
        let any = self.box_to_any(slot.0, slot.1)?;
        self.push_any(stack, any);
        // A boxed number or bool stays decidable at a comparison (`switch` boxes both
        // sides of every `case` test).
        if matches!(slot.1, JitKind::Num(_) | JitKind::Bool) {
            let pushed = stack.last().expect("just pushed").0;
            self.any_boxed.insert(pushed, slot);
        }
        Ok(())
    }

    /// Box a typed (non-`any`) value into the triple, as `Value::box_any` does.
    pub(crate) fn box_to_any(&mut self, value: Value, kind: JitKind) -> Result<AnyVal, String> {
        let zero = self.builder.ins().iconst(types::I64, 0);
        let any = match kind {
            JitKind::Other => return Err("box_to_any of an any".into()),
            JitKind::Num(NumKind::F64) => {
                let bits = self
                    .builder
                    .ins()
                    .bitcast(types::I64, MemFlags::new(), value);
                let tag = self
                    .builder
                    .ins()
                    .iconst(types::I64, zun_core::AnyTag::Float as i64);
                AnyVal {
                    tag,
                    bits,
                    obj: zero,
                }
            }
            JitKind::Num(NumKind::F32) => {
                let wide = self.builder.ins().fpromote(types::F64, value);
                let bits = self
                    .builder
                    .ins()
                    .bitcast(types::I64, MemFlags::new(), wide);
                let tag = self
                    .builder
                    .ins()
                    .iconst(types::I64, zun_core::AnyTag::Float as i64);
                AnyVal {
                    tag,
                    bits,
                    obj: zero,
                }
            }
            JitKind::Num(_) => {
                let bits = widen_to_i64(&mut self.builder, value, kind);
                let tag = self
                    .builder
                    .ins()
                    .iconst(types::I64, zun_core::AnyTag::Int as i64);
                AnyVal {
                    tag,
                    bits,
                    obj: zero,
                }
            }
            JitKind::Bool => {
                let bits = self.builder.ins().uextend(types::I64, value);
                let tag = self
                    .builder
                    .ins()
                    .iconst(types::I64, zun_core::AnyTag::Bool as i64);
                AnyVal {
                    tag,
                    bits,
                    obj: zero,
                }
            }
            JitKind::Object | JitKind::Array => {
                // A zero handle is `null`: `Value::Null` boxes to `Any::null`.
                self.builder.declare_value_needs_stack_map(value);
                let is_null = self.builder.ins().icmp_imm(IntCC::Equal, value, 0);
                let object = self.builder.ins().iconst(types::I64, OBJECT_TAG);
                let null = self
                    .builder
                    .ins()
                    .iconst(types::I64, zun_core::AnyTag::Null as i64);
                let tag = self.builder.ins().select(is_null, null, object);
                AnyVal {
                    tag,
                    bits: zero,
                    obj: value,
                }
            }
            // A string is a handle into the heap's append-only string table, which is
            // exactly what `Any::string` carries (no copy, no interning, never moved).
            JitKind::String => {
                let tag = self
                    .builder
                    .ins()
                    .iconst(types::I64, zun_core::AnyTag::String as i64);
                AnyVal {
                    tag,
                    bits: value,
                    obj: zero,
                }
            }
            JitKind::Function => {
                return Err("box.any of a Function value is not lowered".into());
            }
        };
        Ok(any)
    }

    /// `unbox.any`. The result's static kind is only known from its use, so the
    /// translator reads the instruction after it:
    ///
    /// - `conv f64`: a number. `Int` converts, `Float` is its bits, `undefined`
    ///   is `NaN` (all as Tier 0's `convert`); every other tag is a
    ///   `RunError::Malformed("cannot convert ...")` fault in Tier 0, which the
    ///   compiled code reproduces as a trap in a cold block.
    /// - `ret` / `stloc` into a scalar slot, `brtrue` / `brfalse`: see
    ///   [`UnboxUse`].
    ///
    /// A non-`any` operand passes through (Tier 0 pushes it as is).
    pub(crate) fn op_unbox_any(
        &mut self,
        stack: &mut Vec<Slot>,
        next: Option<&zun_il::Instruction>,
        conv_pc: usize,
    ) -> Result<(), String> {
        let slot = stack.pop().ok_or("stack underflow")?;
        if slot.1 != JitKind::Other {
            stack.push(slot);
            return Ok(());
        }
        let any = self.any_of(slot)?;
        match self.unbox_use(next)? {
            UnboxUse::Num(target) => {
                let value = self.any_to_num(any, target, conv_pc);
                stack.push((value, JitKind::Num(target)));
            }
            UnboxUse::Truthiness => {
                // `brtrue`/`brfalse` run `is_truthy`, which unboxes first, so the
                // `any` itself is the operand.
                stack.push(slot);
            }
        }
        Ok(())
    }

    /// What the instruction after an `unbox.any` does with its result.
    pub(crate) fn unbox_use(
        &self,
        next: Option<&zun_il::Instruction>,
    ) -> Result<UnboxUse, String> {
        let Some(next) = next else {
            return Err("unbox.any is the last instruction".into());
        };
        match (next.opcode, next.operand) {
            (Opcode::Conv, Operand::Imm32(imm)) if NumKind::from_imm(imm).is_some() => {
                Ok(UnboxUse::Num(NumKind::from_imm(imm).expect("checked")))
            }
            (Opcode::BranchTrue | Opcode::BranchFalse, _) => Ok(UnboxUse::Truthiness),
            // A bare `unbox.any; ret` / `stloc` hands the unboxed value on unchanged (no
            // conversion, no check), which a typed slot cannot represent: not lowered.
            _ => Err(format!(
                "unbox.any followed by {:?} has no lowered form",
                next.opcode
            )),
        }
    }

    /// The arm a tag takes that Tier 0's `convert` cannot convert (`Bool`, `Null`, a
    /// string, an object, ...): Tier 0 fails the whole run with an uncatchable
    /// `RunError::Malformed("cannot convert <value>")` framed at the `conv`. With a
    /// bridge the host rebuilds that exact error (`zun_jit_unbox_fault`) and the frame
    /// leaves through the same exceptional exit a null dereference uses; without one
    /// (AOT, `Jit::new`) there is no host to build it, and the arm traps, as the null
    /// dereference check does there.
    fn emit_unbox_fault(&mut self, any: AnyVal, conv_pc: usize) {
        if self.bridge_enabled {
            let payload = self.join_payload(any);
            let rid = self.builder.ins().iconst(types::I64, i64::from(self.method_rid));
            let pc = self.builder.ins().iconst(types::I64, conv_pc as i64);
            let fault = self.import(
                "zun_jit_unbox_fault",
                &[types::I64, types::I64, types::I64, types::I64],
                &[],
            );
            self.builder.ins().call(fault, &[rid, pc, any.tag, payload]);
            let exit = self.null_exit_block();
            self.builder.ins().jump(exit, &[]);
        } else {
            self.builder
                .ins()
                .trap(cranelift_codegen::ir::TrapCode::unwrap_user(2));
        }
    }

    /// `unbox.any; conv <target>`: Tier 0 `convert` of the unboxed value. `Int`
    /// and `Float` convert (a float to an integer wraps like `to_int_wrapping`, 64-bit
    /// targets saturate), `undefined` is `NaN` (`0` for an integer target), every
    /// other tag is a fault there and a trap here.
    fn any_to_num(&mut self, any: AnyVal, target: NumKind, conv_pc: usize) -> Value {
        if matches!(target, NumKind::F64 | NumKind::F32) {
            let wide = self.any_to_f64(any, conv_pc);
            return self.convert(wide, JitKind::Num(NumKind::F64), JitKind::Num(target));
        }
        let to = JitKind::Num(target);
        let merge = self.builder.create_block();
        self.builder.append_block_param(merge, clif_ty(to));
        let cold = self.builder.create_block();
        self.builder.set_cold_block(cold);
        let is_int = self.builder.ins().icmp_imm(IntCC::Equal, any.tag, zun_core::AnyTag::Int as i64);
        let from_int = self.convert(any.bits, JitKind::Num(NumKind::I64), to);
        let not_int = self.builder.create_block();
        self.builder.ins().brif(is_int, merge, &[BlockArg::from(from_int)], not_int, &[]);
        self.builder.switch_to_block(not_int);
        self.builder.seal_block(not_int);
        let is_float = self.builder.ins().icmp_imm(IntCC::Equal, any.tag, zun_core::AnyTag::Float as i64);
        let as_float = self.builder.ins().bitcast(types::F64, MemFlags::new(), any.bits);
        let from_float = self.convert(as_float, JitKind::Num(NumKind::F64), to);
        let not_float = self.builder.create_block();
        self.builder.ins().brif(is_float, merge, &[BlockArg::from(from_float)], not_float, &[]);
        self.builder.switch_to_block(not_float);
        self.builder.seal_block(not_float);
        let is_void = self.builder.ins().icmp_imm(IntCC::Equal, any.tag, zun_core::AnyTag::Void as i64);
        let zero = typed_zero(&mut self.builder, to);
        self.builder.ins().brif(is_void, merge, &[BlockArg::from(zero)], cold, &[]);
        self.builder.switch_to_block(cold);
        self.builder.seal_block(cold);
        self.emit_unbox_fault(any, conv_pc);
        self.builder.switch_to_block(merge);
        self.builder.seal_block(merge);
        self.builder.block_params(merge)[0]
    }

    fn any_to_f64(&mut self, any: AnyVal, conv_pc: usize) -> Value {
        let merge = self.builder.create_block();
        self.builder.append_block_param(merge, types::F64);
        let cold = self.builder.create_block();
        self.builder.set_cold_block(cold);
        let is_float = self.builder.ins().icmp_imm(
            IntCC::Equal,
            any.tag,
            zun_core::AnyTag::Float as i64,
        );
        let as_float = self
            .builder
            .ins()
            .bitcast(types::F64, MemFlags::new(), any.bits);
        let not_float = self.builder.create_block();
        self.builder
            .ins()
            .brif(is_float, merge, &[BlockArg::from(as_float)], not_float, &[]);
        self.builder.switch_to_block(not_float);
        self.builder.seal_block(not_float);
        let is_int = self.builder.ins().icmp_imm(
            IntCC::Equal,
            any.tag,
            zun_core::AnyTag::Int as i64,
        );
        let as_int = self.builder.ins().fcvt_from_sint(types::F64, any.bits);
        let not_int = self.builder.create_block();
        self.builder
            .ins()
            .brif(is_int, merge, &[BlockArg::from(as_int)], not_int, &[]);
        self.builder.switch_to_block(not_int);
        self.builder.seal_block(not_int);
        let is_void = self.builder.ins().icmp_imm(
            IntCC::Equal,
            any.tag,
            zun_core::AnyTag::Void as i64,
        );
        let nan = self.builder.ins().f64const(f64::NAN);
        self.builder
            .ins()
            .brif(is_void, merge, &[BlockArg::from(nan)], cold, &[]);
        self.builder.switch_to_block(cold);
        self.builder.seal_block(cold);
        self.emit_unbox_fault(any, conv_pc);
        self.builder.switch_to_block(merge);
        self.builder.seal_block(merge);
        self.builder.block_params(merge)[0]
    }

    /// Truthiness of an `any`, as the interpreter's `is_truthy` (which unboxes
    /// first): `false`, `0`, `-0`, `NaN`, `""`, `null` and `undefined` are falsy;
    /// everything else, including every object but a zero BigInt, is truthy.
    /// Returns an `I8` flag.
    pub(crate) fn any_truthy(&mut self, any: AnyVal) -> Value {
        let merge = self.builder.create_block();
        self.builder.append_block_param(merge, types::I8);
        let fast = self.builder.create_block();
        let slow = self.builder.create_block();
        self.builder.set_cold_block(slow);
        // Int, Float and Bool are the tags 0..=2.
        let is_scalar = self.builder.ins().icmp_imm(
            IntCC::UnsignedLessThanOrEqual,
            any.tag,
            zun_core::AnyTag::Bool as i64,
        );
        self.builder.ins().brif(is_scalar, fast, &[], slow, &[]);

        self.builder.switch_to_block(fast);
        self.builder.seal_block(fast);
        let as_float = self
            .builder
            .ins()
            .bitcast(types::F64, MemFlags::new(), any.bits);
        let zero_f = self.builder.ins().f64const(0.0);
        let float_truthy = self
            .builder
            .ins()
            .fcmp(FloatCC::OrderedNotEqual, as_float, zero_f);
        let bits_truthy = self.builder.ins().icmp_imm(IntCC::NotEqual, any.bits, 0);
        let is_float = self.builder.ins().icmp_imm(
            IntCC::Equal,
            any.tag,
            zun_core::AnyTag::Float as i64,
        );
        let scalar = self.builder.ins().select(is_float, float_truthy, bits_truthy);
        self.builder.ins().jump(merge, &[BlockArg::from(scalar)]);

        self.builder.switch_to_block(slow);
        self.builder.seal_block(slow);
        let object_block = self.builder.create_block();
        let not_object = self.builder.create_block();
        let string_block = self.builder.create_block();
        let immediate = self.builder.create_block();
        let is_object = self.builder.ins().icmp_imm(IntCC::Equal, any.tag, OBJECT_TAG);
        self.builder
            .ins()
            .brif(is_object, object_block, &[], not_object, &[]);

        self.builder.switch_to_block(object_block);
        self.builder.seal_block(object_block);
        let truthy_fn = self.import("zun_jit_any_object_truthy", &[types::I64], &[types::I32]);
        let call = self.builder.ins().call(truthy_fn, &[any.obj]);
        let flag = self.builder.inst_results(call)[0];
        let flag = self.builder.ins().icmp_imm(IntCC::NotEqual, flag, 0);
        self.builder.ins().jump(merge, &[BlockArg::from(flag)]);

        self.builder.switch_to_block(not_object);
        self.builder.seal_block(not_object);
        let is_string = self.builder.ins().icmp_imm(
            IntCC::Equal,
            any.tag,
            zun_core::AnyTag::String as i64,
        );
        self.builder
            .ins()
            .brif(is_string, string_block, &[], immediate, &[]);

        self.builder.switch_to_block(string_block);
        self.builder.seal_block(string_block);
        let length_fn = self.import("zun_jit_string_length", &[types::I64], &[types::I64]);
        let call = self.builder.ins().call(length_fn, &[any.bits]);
        let length = self.builder.inst_results(call)[0];
        let non_empty = self.builder.ins().icmp_imm(IntCC::SignedGreaterThan, length, 0);
        self.builder
            .ins()
            .jump(merge, &[BlockArg::from(non_empty)]);

        // `null` and `undefined` are falsy; a class descriptor and a symbol are
        // truthy (`Value::Any(_) => true`).
        self.builder.switch_to_block(immediate);
        self.builder.seal_block(immediate);
        let truthy = self.builder.ins().icmp_imm(
            IntCC::UnsignedGreaterThanOrEqual,
            any.tag,
            zun_core::AnyTag::ClassDescriptor as i64,
        );
        self.builder.ins().jump(merge, &[BlockArg::from(truthy)]);

        self.builder.switch_to_block(merge);
        self.builder.seal_block(merge);
        self.builder.block_params(merge)[0]
    }

    /// Strict `===` / `!==` where at least one operand is an `any`.
    ///
    /// Lowered without a helper only where Tier 0's `cmp` is decidable from tag
    /// and payload: against a literal `null` / `undefined` (`tag ==`), and
    /// against an `f64` or `bool` operand (`Float`/`Int`/`Bool` payload compare,
    /// every other tag unequal). Two non-constant `any`s can be objects whose
    /// equality goes through function boxes and constructor records, or strings
    /// whose handles are not canonical (equal text can have different handles),
    /// so that form is not lowered.
    pub(crate) fn compare_any(
        &mut self,
        a: Slot,
        b: Slot,
        equal: bool,
    ) -> Result<Value, String> {
        // The operand that is decidable on its own: a literal `null` / `undefined`, or
        // a typed number / bool (also one that was just boxed). The other operand is
        // the `any` whose tag and payload are inspected.
        let typed_view = |this: &Self, slot: Slot| -> Option<Slot> {
            if slot.1 != JitKind::Other {
                Some(slot)
            } else {
                this.any_boxed.get(&slot.0).copied()
            }
        };
        let is_const = |this: &Self, slot: Slot| this.any_consts.contains_key(&slot.0);
        let (any_slot, other) = if a.1 == JitKind::Other && is_const(self, b) {
            (a, b)
        } else if b.1 == JitKind::Other && is_const(self, a) {
            (b, a)
        } else if a.1 == JitKind::Other && typed_view(self, b).is_some() {
            (a, typed_view(self, b).expect("checked"))
        } else if b.1 == JitKind::Other && typed_view(self, a).is_some() {
            (b, typed_view(self, a).expect("checked"))
        } else {
            return Err(
                "comparison of two any values that are not a literal null/undefined or a number/bool"
                    .into(),
            );
        };
        let any = self.any_of(any_slot)?;
        let same = if let Some(&constant) = self.any_consts.get(&other.0) {
            let tag = match constant {
                AnyConst::Null => zun_core::AnyTag::Null,
                AnyConst::Undef => zun_core::AnyTag::Void,
            };
            self.builder
                .ins()
                .icmp_imm(IntCC::Equal, any.tag, tag as i64)
        } else {
            match other.1 {
                JitKind::Num(NumKind::F64 | NumKind::F32) => {
                    let operand = if other.1 == JitKind::Num(NumKind::F32) {
                        self.builder.ins().fpromote(types::F64, other.0)
                    } else {
                        other.0
                    };
                    let as_float = self
                        .builder
                        .ins()
                        .bitcast(types::F64, MemFlags::new(), any.bits);
                    let as_int = self.builder.ins().fcvt_from_sint(types::F64, any.bits);
                    let float_eq = self.builder.ins().fcmp(FloatCC::Equal, as_float, operand);
                    let int_eq = self.builder.ins().fcmp(FloatCC::Equal, as_int, operand);
                    let is_float = self.builder.ins().icmp_imm(
                        IntCC::Equal,
                        any.tag,
                        zun_core::AnyTag::Float as i64,
                    );
                    let is_int = self.builder.ins().icmp_imm(
                        IntCC::Equal,
                        any.tag,
                        zun_core::AnyTag::Int as i64,
                    );
                    let float_hit = self.builder.ins().band(is_float, float_eq);
                    let int_hit = self.builder.ins().band(is_int, int_eq);
                    self.builder.ins().bor(float_hit, int_hit)
                }
                JitKind::Num(_) => {
                    // An integer container: `Int` compares exactly, `Float` by value.
                    let widened = widen_to_i64(&mut self.builder, other.0, other.1);
                    let as_float = self
                        .builder
                        .ins()
                        .bitcast(types::F64, MemFlags::new(), any.bits);
                    let wide_float = self.builder.ins().fcvt_from_sint(types::F64, widened);
                    let float_eq = self.builder.ins().fcmp(FloatCC::Equal, as_float, wide_float);
                    let int_eq = self.builder.ins().icmp(IntCC::Equal, any.bits, widened);
                    let is_float = self.builder.ins().icmp_imm(
                        IntCC::Equal,
                        any.tag,
                        zun_core::AnyTag::Float as i64,
                    );
                    let is_int = self.builder.ins().icmp_imm(
                        IntCC::Equal,
                        any.tag,
                        zun_core::AnyTag::Int as i64,
                    );
                    let float_hit = self.builder.ins().band(is_float, float_eq);
                    let int_hit = self.builder.ins().band(is_int, int_eq);
                    self.builder.ins().bor(float_hit, int_hit)
                }
                JitKind::Bool => {
                    let is_bool = self.builder.ins().icmp_imm(
                        IntCC::Equal,
                        any.tag,
                        zun_core::AnyTag::Bool as i64,
                    );
                    let flag = self.builder.ins().uextend(types::I64, other.0);
                    let bits_equal = self.builder.ins().icmp(IntCC::Equal, any.bits, flag);
                    // Tier 0 reads a boxed bool as `payload != 0`; a stored
                    // payload is only ever 0 or 1.
                    self.builder.ins().band(is_bool, bits_equal)
                }
                kind => return Err(format!("comparison of an any with a {kind:?} value")),
            }
        };
        let result = if equal {
            same
        } else {
            self.builder.ins().bxor_imm(same, 1)
        };
        Ok(self.builder.ins().uextend(types::I32, result))
    }

    /// `ldelem` into an `any`: the helper writes a carrier, which is read back
    /// right after the call (the helper does not allocate or collect).
    pub(crate) fn emit_array_load_any(
        &mut self,
        stack: &mut Vec<Slot>,
        array: Value,
        index: Value,
    ) {
        let carrier = self.carrier_area();
        // An out-of-bounds read leaves the area untouched; seed it as `undefined`.
        let void_tag = self
            .builder
            .ins()
            .iconst(types::I64, zun_core::AnyTag::Void as i64);
        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().store(
            MemFlags::trusted(),
            void_tag,
            carrier,
            crate::any_abi::TAG_OFFSET,
        );
        self.builder.ins().store(
            MemFlags::trusted(),
            zero,
            carrier,
            crate::any_abi::PAYLOAD_OFFSET,
        );
        let load = self.import(
            "zun_jit_array_load_any",
            &[types::I64, types::I64, types::I64],
            &[types::I32],
        );
        self.builder.ins().call(load, &[array, index, carrier]);
        let any = self.load_carrier(carrier);
        self.push_any(stack, any);
    }

    /// `Map.get` with an `any` key and an `any` result.
    pub(crate) fn emit_map_get_any(
        &mut self,
        stack: &mut Vec<Slot>,
        map: Value,
        key: AnyVal,
    ) {
        let key_payload = self.join_payload(key);
        let out = self.carrier_area();
        let helper = self.import(
            "zun_jit_map_get_any",
            &[types::I64, types::I64, types::I64, types::I64],
            &[types::I32],
        );
        self.builder
            .ins()
            .call(helper, &[map, key.tag, key_payload, out]);
        let any = self.load_carrier(out);
        self.push_any(stack, any);
    }
}

/// How the result of an `unbox.any` is consumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnboxUse {
    Num(NumKind),
    Truthiness,
}

/// Convert an integer or bool scalar to the `i64` an `Any::int` carries (sign or
/// zero extended by the source kind, as `Value::Int`'s `i64` container is).
pub(crate) fn widen_to_i64(builder: &mut FunctionBuilder, value: Value, kind: JitKind) -> Value {
    let ty = builder.func.dfg.value_type(value);
    if ty == types::I64 {
        value
    } else if is_signed(kind) {
        builder.ins().sextend(types::I64, value)
    } else {
        builder.ins().uextend(types::I64, value)
    }
}

/// Split a boundary `(tag, payload)` pair into `(bits, obj)` with a builder that
/// is not (yet) owned by a [`Translator`] (the function prologue). Together with
/// [`Translator::join_payload`] this is the only place where the tag decides
/// whether a payload is a reference, derived from
/// [`zun_core::AnyTag::MANAGED_REFERENCE`].
pub(crate) fn split_payload_in(
    builder: &mut FunctionBuilder,
    tag: Value,
    payload: Value,
) -> (Value, Value) {
    let object_tag = builder.ins().iconst(types::I64, OBJECT_TAG);
    let is_object = builder.ins().icmp(IntCC::Equal, tag, object_tag);
    let zero = builder.ins().iconst(types::I64, 0);
    let obj = builder.ins().select(is_object, payload, zero);
    let bits = builder.ins().select(is_object, zero, payload);
    (bits, obj)
}

impl Translator<'_> {
    /// Pop a branch condition. An `any` runs the interpreter's `is_truthy`; any
    /// other kind is already a flag.
    pub(crate) fn branch_condition(&mut self, stack: &mut Vec<Slot>) -> Result<Value, String> {
        let slot = stack.pop().ok_or("stack underflow")?;
        match slot.1 {
            JitKind::Other => {
                let any = self.any_of(slot)?;
                Ok(self.any_truthy(any))
            }
            // A number is truthy unless it is 0, -0 or NaN, as Tier 0's `is_truthy`.
            JitKind::Num(NumKind::F64 | NumKind::F32) => {
                let zero = if slot.1 == JitKind::Num(NumKind::F32) {
                    self.builder.ins().f32const(0.0)
                } else {
                    self.builder.ins().f64const(0.0)
                };
                Ok(self
                    .builder
                    .ins()
                    .fcmp(FloatCC::OrderedNotEqual, slot.0, zero))
            }
            // A string is truthy when it is not empty (a handle is never the test).
            JitKind::String => {
                let length_fn = self.import("zun_jit_string_length", &[types::I64], &[types::I64]);
                let call = self.builder.ins().call(length_fn, &[slot.0]);
                let length = self.builder.inst_results(call)[0];
                Ok(self
                    .builder
                    .ins()
                    .icmp_imm(IntCC::SignedGreaterThan, length, 0))
            }
            _ => Ok(slot.0),
        }
    }
}

impl Translator<'_> {
    /// Make `slot` fit a parameter / local of kind `want`: an `any` slot meets an
    /// `any` slot unchanged, a typed value meeting an `any` slot is boxed (as the
    /// interpreter's `box_any` would), an `any` meeting a typed slot is refused
    /// (it needs an `unbox.any`), and typed meets typed as before.
    pub(crate) fn coerce_slot(
        &mut self,
        slot: Slot,
        want: JitKind,
        what: &str,
    ) -> Result<Slot, String> {
        match (slot.1 == JitKind::Other, want == JitKind::Other) {
            (true, true) => Ok(slot),
            (false, true) => {
                let any = self.box_to_any(slot.0, slot.1)?;
                let mut tmp = Vec::with_capacity(1);
                self.push_any(&mut tmp, any);
                Ok(tmp[0])
            }
            (true, false) => Err(format!("{what}: an any value meets a {want:?} slot")),
            (false, false) => {
                require_reference_agreement(slot.1, want, what)?;
                Ok(slot)
            }
        }
    }
}

impl Translator<'_> {
    /// Refuse an opcode whose operands include an `any` unless the opcode has a
    /// lowered form for it. The IL is not strictly typed (an `any` can reach a
    /// number parameter, an arithmetic operand or a field store without an
    /// `unbox.any`), and an unlowered consumer would read the tag word as a
    /// number or an address.
    pub(crate) fn refuse_any_operands(
        &self,
        opcode: Opcode,
        stack: &[Slot],
    ) -> Result<(), String> {
        let handles_any = matches!(
            opcode,
            Opcode::Nop
                | Opcode::LoadLocal
                | Opcode::StoreLocal
                | Opcode::Pop
                | Opcode::Dup
                | Opcode::BranchTrue
                | Opcode::BranchFalse
                | Opcode::Branch
                | Opcode::Ret
                | Opcode::Call
                | Opcode::CallVirt
                | Opcode::CallIndirect
                | Opcode::CallBuiltin
                | Opcode::CmpEq
                | Opcode::CmpNe
                | Opcode::BoxAny
                | Opcode::UnboxAny
                | Opcode::LoadArray
                | Opcode::StoreArray
                | Opcode::ArrayPush
        );
        if handles_any {
            return Ok(());
        }
        let zun_il::StackEffect::Fixed { pops, .. } = opcode.stack_effect() else {
            return Ok(());
        };
        let top = stack.len().saturating_sub(pops as usize);
        if stack[top..].iter().any(|&(_, kind)| kind == JitKind::Other) {
            return Err(format!("{opcode:?} on an any operand has no lowered form"));
        }
        Ok(())
    }
}
