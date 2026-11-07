use super::*;

impl<'a, 'b> FnLowerer<'a, 'b> {
    /// An await: suspends like a yield inside async bodies, else a plain await.
    pub(super) fn lower_await(&mut self, operand: &hir::Expr) {
        self.expr(operand);
        if self.is_async {
            // Inside an `async` body an await suspends like a yield: the
            // coroutine driver parks the frame on the awaited value and
            // resumes with the settlement (or throws at this point).
            if self.is_async_gen {
                let (mark_id, _) = zun_natives::resolve("collections", "asyncAwaitMark")
                    .expect("asyncAwaitMark intrinsic");
                self.emit(Instruction::new(
                    Opcode::CallBuiltin,
                    Operand::Imm32(mark_id as i32),
                ));
            }
            self.emit(Instruction::simple(Opcode::Yield));
            let (sent_id, _) = zun_natives::resolve("collections", "generatorSent")
                .expect("generatorSent intrinsic");
            self.emit(Instruction::new(
                Opcode::CallBuiltin,
                Operand::Imm32(sent_id as i32),
            ));
        } else {
            self.emit(Instruction::simple(Opcode::Await));
        }
    }

    /// A yield, followed by reading back the value sent in by the caller.
    pub(super) fn lower_yield(&mut self, value: &hir::Expr) {
        self.expr(value);
        self.emit(Instruction::simple(Opcode::Yield));
        // The value of a `yield` expression is what `next(v)` sends back in
        // (undefined when nothing was sent).
        let (sent_id, _) =
            zun_natives::resolve("collections", "generatorSent").expect("generatorSent intrinsic");
        self.emit(Instruction::new(
            Opcode::CallBuiltin,
            Operand::Imm32(sent_id as i32),
        ));
    }

    /// Binary operators, including short-circuit, nullish and JS-number bitwise forms.
    pub(super) fn lower_binary(&mut self, op: &BinaryOp, lhs: &hir::Expr, rhs: &hir::Expr) {
        if *op == BinaryOp::Nullish {
            self.expr(lhs);
            self.emit(Instruction::simple(Opcode::Dup));
            self.emit(Instruction::simple(Opcode::ConstNull));
            self.emit(Instruction::simple(Opcode::CmpEq));
            let use_rhs = self.emit_branch(Opcode::BranchTrue);
            self.emit(Instruction::simple(Opcode::Dup));
            self.emit(Instruction::simple(Opcode::ConstUndef));
            self.emit(Instruction::simple(Opcode::CmpEq));
            let use_rhs_undef = self.emit_branch(Opcode::BranchTrue);
            let end = self.emit_branch(Opcode::Branch);
            self.patch_to_here(use_rhs);
            self.patch_to_here(use_rhs_undef);
            self.emit(Instruction::simple(Opcode::Pop));
            self.expr(rhs);
            self.patch_to_here(end);
            return;
        }
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            // Value-preserving short-circuit (JS semantics): the result
            // is the operand that decided the outcome, not a boolean.
            //   a || b: keep a when a is truthy, else evaluate b.
            //   a && b: keep a when a is falsy, else evaluate b.
            // Branch opcodes test truthiness, so this is correct for both
            // boolean and dynamic operands.
            self.expr(lhs);
            self.emit(Instruction::simple(Opcode::Dup));
            let keep = self.emit_branch(if *op == BinaryOp::And {
                Opcode::BranchFalse
            } else {
                Opcode::BranchTrue
            });
            self.emit(Instruction::simple(Opcode::Pop));
            self.expr(rhs);
            self.patch_to_here(keep);
            return;
        }
        // JS-number bitwise/shift: both operands are `number` (f64).
        // Coerce to int32 (uint32 for the `>>>` left operand), run the
        // integer op, then widen the result back to a `number`.
        let is_int = lhs.ty.is_int();
        if !is_int
            && matches!(
                op,
                BinaryOp::BitAnd
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::Shl
                    | BinaryOp::Shr
                    | BinaryOp::UShr
            )
        {
            let is_ushr = *op == BinaryOp::UShr;
            self.expr(lhs);
            self.emit_to_int32(is_ushr);
            self.expr(rhs);
            self.emit_to_int32(false);
            let opcode = match op {
                BinaryOp::BitAnd => Opcode::AndI32,
                BinaryOp::BitOr => Opcode::OrI32,
                BinaryOp::BitXor => Opcode::XorI32,
                BinaryOp::Shl => Opcode::ShlI32,
                BinaryOp::Shr => Opcode::ShrI32,
                BinaryOp::UShr => Opcode::UShrI32,
                _ => unreachable!(),
            };
            self.emit(Instruction::simple(opcode));
            // The int result (int32, or uint32 for `>>>`) widens back to
            // a JS `number`.
            self.emit_conv_kind(NumKind::F64);
            return;
        }
        self.expr(lhs);
        self.expr(rhs);
        // Pick the opcode family from the operand type: int operands use
        // the i32 ops, floats the f64 ops, strings concatenate.
        let opcode = match op {
            BinaryOp::Add if lhs.ty == Ty::String => Opcode::Concat,
            BinaryOp::Add => int_or_float(is_int, Opcode::AddI32, Opcode::AddF64),
            BinaryOp::Sub => int_or_float(is_int, Opcode::SubI32, Opcode::SubF64),
            BinaryOp::Mul => int_or_float(is_int, Opcode::MulI32, Opcode::MulF64),
            BinaryOp::Div => int_or_float(is_int, Opcode::DivI32, Opcode::DivF64),
            BinaryOp::Rem => int_or_float(is_int, Opcode::RemI32, Opcode::RemF64),
            // Integer `**` (number `**` is routed to `Math.pow` in sema).
            BinaryOp::Pow => Opcode::PowI32,
            // Integer bitwise/shift (the f64 forms returned above).
            BinaryOp::BitAnd => Opcode::AndI32,
            BinaryOp::BitOr => Opcode::OrI32,
            BinaryOp::BitXor => Opcode::XorI32,
            BinaryOp::Shl => Opcode::ShlI32,
            BinaryOp::Shr => Opcode::ShrI32,
            BinaryOp::UShr => Opcode::UShrI32,
            // Comparisons share one opcode set; the runtime compares by
            // the operands' actual kind.
            BinaryOp::Eq | BinaryOp::LooseEq => Opcode::CmpEq,
            BinaryOp::Ne | BinaryOp::LooseNe => Opcode::CmpNe,
            BinaryOp::Lt => Opcode::CmpLt,
            BinaryOp::Le => Opcode::CmpLe,
            BinaryOp::Gt => Opcode::CmpGt,
            BinaryOp::Ge => Opcode::CmpGe,
            BinaryOp::And | BinaryOp::Or | BinaryOp::Nullish => {
                unreachable!("logical operators lower through branches")
            }
            BinaryOp::In | BinaryOp::Instanceof => {
                unreachable!("`in`/`instanceof` are lowered in sema")
            }
        };
        self.emit(Instruction::simple(opcode));
    }

    /// The ternary conditional.
    pub(super) fn lower_conditional(
        &mut self,
        condition: &hir::Expr,
        when_true: &hir::Expr,
        when_false: &hir::Expr,
    ) {
        self.expr(condition);
        let to_false = self.emit_branch(Opcode::BranchFalse);
        self.expr(when_true);
        let to_end = self.emit_branch(Opcode::Branch);
        self.patch_to_here(to_false);
        self.expr(when_false);
        self.patch_to_here(to_end);
    }

    /// Unary operators.
    pub(super) fn lower_unary(&mut self, op: &UnaryOp, operand: &hir::Expr) {
        match op {
            UnaryOp::Neg if operand.ty.is_int() => {
                self.expr(operand);
                self.emit(Instruction::simple(Opcode::NegI32));
            }
            UnaryOp::Neg => {
                // `neg.f64` flips the sign bit. The previous `0 - x` was
                // wrong at zero: `0.0 - 0.0` is `+0.0`, while `-0.0` must
                // stay `-0.0` (observable as `1 / -x === -Infinity`).
                self.expr(operand);
                self.emit(Instruction::simple(Opcode::NegF64));
            }
            UnaryOp::Not => {
                // x == false
                self.expr(operand);
                self.emit(Instruction::simple(Opcode::ConstFalse));
                self.emit(Instruction::simple(Opcode::CmpEq));
            }
            UnaryOp::BitNot if operand.ty.is_int() => {
                self.expr(operand);
                self.emit(Instruction::simple(Opcode::NotI32));
            }
            UnaryOp::BitNot => {
                // `~` on a JS `number`: ToInt32, complement, widen back.
                self.expr(operand);
                self.emit_to_int32(false);
                self.emit(Instruction::simple(Opcode::NotI32));
                self.emit_conv_kind(NumKind::F64);
            }
            UnaryOp::Void => {
                // Evaluate for side effects, discard, yield `undefined`.
                self.expr(operand);
                self.emit(Instruction::simple(Opcode::Pop));
                self.emit(Instruction::simple(Opcode::ConstUndef));
            }
            UnaryOp::Typeof | UnaryOp::Delete => {
                unreachable!("`typeof`/`delete` are lowered in sema")
            }
        }
    }

    /// A type conversion: unbox/cast from Any, then numeric conversion.
    pub(super) fn lower_convert(&mut self, value: &hir::Expr, e: &hir::Expr) {
        if value.ty == Ty::Any {
            self.expr(value);
            if let Ty::Instance(index) = e.ty {
                self.emit(Instruction::new(
                    Opcode::CastClass,
                    Operand::Token(Token::new(TableKind::TypeDef, index + 1)),
                ));
            } else {
                self.emit(Instruction::simple(Opcode::UnboxAny));
            }
        } else {
            self.expr(value);
        }
        self.emit_conv_to(e.ty);
    }

    /// Evaluate `effect` for its side effects, then yield `value`.
    pub(super) fn lower_sequence(&mut self, effect: &hir::Expr, value: &hir::Expr) {
        self.expr(effect);
        self.emit(Instruction::simple(Opcode::Pop));
        self.expr(value);
    }

    /// A local assignment; the value stays on the stack (`dup` before `stloc`).
    pub(super) fn lower_assign(&mut self, slot: &hir::LocalSlot, value: &hir::Expr) {
        self.expr(value);
        self.emit(Instruction::simple(Opcode::Dup));
        self.emit(Instruction::new(Opcode::StoreLocal, Operand::Local(*slot)));
    }
}
