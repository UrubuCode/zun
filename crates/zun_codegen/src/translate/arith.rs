//! Arithmetic, comparison, and numeric-conversion lowering for the typed JIT.
//!
//! Each method pops its operands from the evaluation stack, emits the signed or
//! unsigned integer, float, or conversion instruction selected by the operand
//! kind, and pushes the result. Extracted verbatim from `translate/mod.rs`; the
//! single opcode dispatcher that calls these lives there.

use super::*;

impl Translator<'_> {
    pub(crate) fn int_bin(&mut self, stack: &mut Vec<Slot>, op: IntOp) -> Result<(), String> {
        let (b, _) = stack.pop().ok_or("stack underflow")?;
        let (a, k) = stack.pop().ok_or("stack underflow")?;
        let signed = is_signed(k);
        let ins = self.builder.ins();
        let r = match op {
            IntOp::Add => ins.iadd(a, b),
            IntOp::Sub => ins.isub(a, b),
            IntOp::Mul => ins.imul(a, b),
            IntOp::Div if signed => ins.sdiv(a, b),
            IntOp::Div => ins.udiv(a, b),
            IntOp::Rem if signed => ins.srem(a, b),
            IntOp::Rem => ins.urem(a, b),
            IntOp::And => ins.band(a, b),
            IntOp::Or => ins.bor(a, b),
            IntOp::Xor => ins.bxor(a, b),
            // Cranelift masks the shift amount to the controlling type's width,
            // matching JS's `& 31` for i32 and the Tier 0 masking.
            IntOp::Shl => ins.ishl(a, b),
            // `>>` is arithmetic for signed operands, logical for unsigned.
            IntOp::Shr if signed => ins.sshr(a, b),
            IntOp::Shr => ins.ushr(a, b),
            // `>>>` is always logical.
            IntOp::UShr => ins.ushr(a, b),
        };
        stack.push((r, k));
        Ok(())
    }

    /// Integer exponentiation `a ** b`, wrapping to the operand kind, as an
    /// inline loop (exponentiation by squaring). Wrapping multiply is a ring
    /// homomorphism, so this agrees with the Tier 0 `bin_int_pow` bit-for-bit
    /// regardless of the squaring order. A negative exponent yields 1.
    pub(crate) fn int_pow(&mut self, stack: &mut Vec<Slot>) -> Result<(), String> {
        let (b, _) = stack.pop().ok_or("stack underflow")?;
        let (a, k) = stack.pop().ok_or("stack underflow")?;
        let ty = self.builder.func.dfg.value_type(a);

        // The exponent as an unsigned i64 count: negative clamps to 0.
        let exp0 = if is_signed(k) {
            let wide = if ty == types::I64 {
                b
            } else {
                self.builder.ins().sextend(types::I64, b)
            };
            let zero = self.builder.ins().iconst(types::I64, 0);
            self.builder.ins().smax(wide, zero)
        } else if ty == types::I64 {
            b
        } else {
            self.builder.ins().uextend(types::I64, b)
        };
        let acc0 = self.builder.ins().iconst(ty, 1);

        let header = self.builder.create_block();
        let body = self.builder.create_block();
        let exit = self.builder.create_block();
        self.builder.append_block_param(header, ty); // acc
        self.builder.append_block_param(header, ty); // base
        self.builder.append_block_param(header, types::I64); // exp
        self.builder.append_block_param(exit, ty); // result

        self.builder.ins().jump(
            header,
            &[
                BlockArg::from(acc0),
                BlockArg::from(a),
                BlockArg::from(exp0),
            ],
        );

        self.builder.switch_to_block(header);
        let acc = self.builder.block_params(header)[0];
        let base = self.builder.block_params(header)[1];
        let exp = self.builder.block_params(header)[2];
        let zero64 = self.builder.ins().iconst(types::I64, 0);
        let done = self.builder.ins().icmp(IntCC::Equal, exp, zero64);
        self.builder
            .ins()
            .brif(done, exit, &[BlockArg::from(acc)], body, &[]);

        self.builder.switch_to_block(body);
        let one64 = self.builder.ins().iconst(types::I64, 1);
        let bit = self.builder.ins().band(exp, one64);
        let take = self.builder.ins().icmp(IntCC::NotEqual, bit, zero64);
        let acc_mul = self.builder.ins().imul(acc, base);
        let acc_next = self.builder.ins().select(take, acc_mul, acc);
        let base_next = self.builder.ins().imul(base, base);
        let exp_next = self.builder.ins().ushr(exp, one64);
        self.builder.ins().jump(
            header,
            &[
                BlockArg::from(acc_next),
                BlockArg::from(base_next),
                BlockArg::from(exp_next),
            ],
        );

        // The loop header is only fully known once the back-edge from `body`
        // is emitted; seal in dependency order.
        self.builder.seal_block(body);
        self.builder.seal_block(header);
        self.builder.seal_block(exit);

        self.builder.switch_to_block(exit);
        let result = self.builder.block_params(exit)[0];
        stack.push((result, k));
        Ok(())
    }

    pub(crate) fn float_bin(&mut self, stack: &mut Vec<Slot>, op: FloatOp) -> Result<(), String> {
        let (b, _) = stack.pop().ok_or("stack underflow")?;
        let (a, k) = stack.pop().ok_or("stack underflow")?;
        let ins = self.builder.ins();
        let r = match op {
            FloatOp::Add => ins.fadd(a, b),
            FloatOp::Sub => ins.fsub(a, b),
            FloatOp::Mul => ins.fmul(a, b),
            FloatOp::Div => ins.fdiv(a, b),
            // IEEE-754 2019 minimum/maximum: NaN propagates and -0.0 sorts
            // below +0.0, which is what `Math.min`/`Math.max` and the Tier 0
            // `js_min`/`js_max` do. The equivalence suite pins both edges.
            FloatOp::Min => ins.fmin(a, b),
            FloatOp::Max => ins.fmax(a, b),
        };
        stack.push((r, k));
        Ok(())
    }

    /// Float remainder (`RemF64`, JS `%`). Cranelift has no float-remainder
    /// instruction, and the inline identity `a - b*trunc(a/b)` diverges when the
    /// divisor is infinite (it yields NaN where C `fmod`, and the Tier 0 `a % b`,
    /// return `a`), so the remainder is a libcall to `zun_jit_math_fmod`.
    pub(crate) fn float_rem(&mut self, stack: &mut Vec<Slot>) -> Result<(), String> {
        let (b, _) = stack.pop().ok_or("stack underflow")?;
        let (a, k) = stack.pop().ok_or("stack underflow")?;
        let fref = self.math_fmod_fref();
        let inst = self.builder.ins().call(fref, &[a, b]);
        let r = self.builder.inst_results(inst)[0];
        stack.push((r, k));
        Ok(())
    }

    /// `min`/`max` over integers, picking the signed or unsigned instruction
    /// from the operand kind — the same rule `compare` uses for `icmp`.
    pub(crate) fn int_minmax(&mut self, stack: &mut Vec<Slot>, is_min: bool) -> Result<(), String> {
        let (b, _) = stack.pop().ok_or("stack underflow")?;
        let (a, k) = stack.pop().ok_or("stack underflow")?;
        let signed = is_signed(k);
        let ins = self.builder.ins();
        let r = match (is_min, signed) {
            (true, true) => ins.smin(a, b),
            (true, false) => ins.umin(a, b),
            (false, true) => ins.smax(a, b),
            (false, false) => ins.umax(a, b),
        };
        stack.push((r, k));
        Ok(())
    }

    /// `Math.round`, branchlessly.
    ///
    /// Mirrors the Tier 0 `js_round`: take the fractional part rather than
    /// adding 0.5, because `0.49999999999999994 + 0.5` rounds up to exactly
    /// `1.0` and would answer 1 where JS answers 0. NaN and the infinities
    /// need no special case — the subtraction yields NaN, the comparison is
    /// false, and `floor` already returned the right value.
    pub(crate) fn round_f64(&mut self, stack: &mut Vec<Slot>) -> Result<(), String> {
        let (x, k) = stack.pop().ok_or("stack underflow")?;
        let ty = self.builder.func.dfg.value_type(x);
        let half = self.fconst(ty, 0.5)?;
        let one = self.fconst(ty, 1.0)?;
        let zero = self.fconst(ty, 0.0)?;

        let truncated = self.builder.ins().floor(x);
        let fraction = self.builder.ins().fsub(x, truncated);
        let round_up = self
            .builder
            .ins()
            .fcmp(FloatCC::GreaterThanOrEqual, fraction, half);
        let incremented = self.builder.ins().fadd(truncated, one);
        let rounded = self.builder.ins().select(round_up, incremented, truncated);

        // A zero result takes the sign of the input: -0.4 rounds to -0.
        let is_zero = self.builder.ins().fcmp(FloatCC::Equal, rounded, zero);
        let signed_zero = self.builder.ins().fcopysign(zero, x);
        let result = self.builder.ins().select(is_zero, signed_zero, rounded);

        stack.push((result, k));
        Ok(())
    }

    /// `Math.sign`: NaN and both zeros pass through, everything else is ±1.
    pub(crate) fn sign_f64(&mut self, stack: &mut Vec<Slot>) -> Result<(), String> {
        let (x, k) = stack.pop().ok_or("stack underflow")?;
        let ty = self.builder.func.dfg.value_type(x);
        let one = self.fconst(ty, 1.0)?;
        let zero = self.fconst(ty, 0.0)?;

        // `Unordered(x, x)` is the NaN test; equality with zero catches both
        // signed zeros at once.
        let is_nan = self.builder.ins().fcmp(FloatCC::Unordered, x, x);
        let is_zero = self.builder.ins().fcmp(FloatCC::Equal, x, zero);
        let passthrough = self.builder.ins().bor(is_nan, is_zero);
        let magnitude = self.builder.ins().fcopysign(one, x);
        let result = self.builder.ins().select(passthrough, x, magnitude);

        stack.push((result, k));
        Ok(())
    }

    /// A float constant of `ty`, which is F32 or F64.
    pub(crate) fn fconst(&mut self, ty: Type, value: f64) -> Result<Value, String> {
        Ok(match ty {
            types::F32 => self.builder.ins().f32const(value as f32),
            types::F64 => self.builder.ins().f64const(value),
            other => return Err(format!("expected a float type, got {other}")),
        })
    }

    pub(crate) fn int_un(&mut self, stack: &mut Vec<Slot>, op: IntUnOp) -> Result<(), String> {
        let (a, k) = stack.pop().ok_or("stack underflow")?;
        let ins = self.builder.ins();
        let r = match op {
            IntUnOp::Clz => ins.clz(a),
            IntUnOp::Ctz => ins.ctz(a),
            IntUnOp::Popcnt => ins.popcnt(a),
        };
        stack.push((r, k));
        Ok(())
    }

    pub(crate) fn float_un(&mut self, stack: &mut Vec<Slot>, op: FloatUnOp) -> Result<(), String> {
        let (a, k) = stack.pop().ok_or("stack underflow")?;
        let ins = self.builder.ins();
        let r = match op {
            FloatUnOp::Sqrt => ins.sqrt(a),
            FloatUnOp::Abs => ins.fabs(a),
            FloatUnOp::Neg => ins.fneg(a),
            FloatUnOp::Floor => ins.floor(a),
            FloatUnOp::Ceil => ins.ceil(a),
            FloatUnOp::Trunc => ins.trunc(a),
        };
        stack.push((r, k));
        Ok(())
    }

    pub(crate) fn compare(&mut self, stack: &mut Vec<Slot>, cmp: Cmp) -> Result<(), String> {
        let (b, _) = stack.pop().ok_or("stack underflow")?;
        let (a, k) = stack.pop().ok_or("stack underflow")?;
        let flag = if is_float(k) {
            let cc = match cmp {
                Cmp::Eq => FloatCC::Equal,
                Cmp::Ne => FloatCC::NotEqual,
                Cmp::Lt => FloatCC::LessThan,
                Cmp::Le => FloatCC::LessThanOrEqual,
                Cmp::Gt => FloatCC::GreaterThan,
                Cmp::Ge => FloatCC::GreaterThanOrEqual,
            };
            self.builder.ins().fcmp(cc, a, b)
        } else {
            let signed = is_signed(k);
            let cc = match cmp {
                Cmp::Eq => IntCC::Equal,
                Cmp::Ne => IntCC::NotEqual,
                Cmp::Lt if signed => IntCC::SignedLessThan,
                Cmp::Lt => IntCC::UnsignedLessThan,
                Cmp::Le if signed => IntCC::SignedLessThanOrEqual,
                Cmp::Le => IntCC::UnsignedLessThanOrEqual,
                Cmp::Gt if signed => IntCC::SignedGreaterThan,
                Cmp::Gt => IntCC::UnsignedGreaterThan,
                Cmp::Ge if signed => IntCC::SignedGreaterThanOrEqual,
                Cmp::Ge => IntCC::UnsignedGreaterThanOrEqual,
            };
            self.builder.ins().icmp(cc, a, b)
        };
        // icmp/fcmp yield I8; widen to the I32 boolean representation.
        let as_i32 = self.builder.ins().uextend(types::I32, flag);
        stack.push((as_i32, JitKind::Bool));
        Ok(())
    }

    /// Convert `v` of kind `from` to kind `to`, emitting the right widening,
    /// narrowing, and int/float conversions.
    pub(crate) fn convert(&mut self, v: Value, from: JitKind, to: JitKind) -> Value {
        let from_ty = clif_ty(from);
        let to_ty = clif_ty(to);
        let from_float = is_float(from);
        let to_float = is_float(to);

        match (from_float, to_float) {
            (false, false) => {
                // int -> int: match widths, extending by source signedness.
                let fb = from_ty.bits();
                let tb = to_ty.bits();
                if tb == fb {
                    v
                } else if tb < fb {
                    self.builder.ins().ireduce(to_ty, v)
                } else if is_signed(from) {
                    self.builder.ins().sextend(to_ty, v)
                } else {
                    self.builder.ins().uextend(to_ty, v)
                }
            }
            (false, true) => {
                // int -> float. Widen narrow ints to I32 first for the convert.
                let wide = self.widen_for_convert(v, from);
                if is_signed(from) {
                    self.builder.ins().fcvt_from_sint(to_ty, wide)
                } else {
                    self.builder.ins().fcvt_from_uint(to_ty, wide)
                }
            }
            (true, false) => {
                // float -> int, saturating (never traps; JS-like), then narrow.
                let wide_ty = if to_ty.bits() < 32 { types::I32 } else { to_ty };
                let wide = if is_signed(to) {
                    self.builder.ins().fcvt_to_sint_sat(wide_ty, v)
                } else {
                    self.builder.ins().fcvt_to_uint_sat(wide_ty, v)
                };
                if wide_ty == to_ty {
                    wide
                } else {
                    self.builder.ins().ireduce(to_ty, wide)
                }
            }
            (true, true) => {
                if from_ty == to_ty {
                    v
                } else if to_ty == types::F64 {
                    self.builder.ins().fpromote(to_ty, v)
                } else {
                    self.builder.ins().fdemote(to_ty, v)
                }
            }
        }
    }

    /// Widen a narrow integer to I32 (for feeding int→float conversions).
    pub(crate) fn widen_for_convert(&mut self, v: Value, from: JitKind) -> Value {
        let ty = clif_ty(from);
        if ty.bits() >= 32 {
            v
        } else if is_signed(from) {
            self.builder.ins().sextend(types::I32, v)
        } else {
            self.builder.ins().uextend(types::I32, v)
        }
    }

    pub(crate) fn math_fmod_fref(&mut self) -> cranelift_codegen::ir::FuncRef {
        let mut sig = Signature::new(self.module.isa().default_call_conv());
        sig.params.push(AbiParam::new(types::F64));
        sig.params.push(AbiParam::new(types::F64));
        sig.returns.push(AbiParam::new(types::F64));
        let id = self
            .module
            .declare_function("zun_jit_math_fmod", Linkage::Import, &sig)
            .expect("declare zun_jit_math_fmod");
        self.module.declare_func_in_func(id, self.builder.func)
    }
}
