use super::function::capture_name;
use super::*;

impl<'a, 'b> FnLowerer<'a, 'b> {
    pub(super) fn expr(&mut self, e: &hir::Expr) {
        if !e.span.is_empty() {
            self.points.push((self.code.len() as u32, e.span));
        }
        match &e.kind {
            ExprKind::Number(v) => self.emit_numeric_const(*v, e.ty),
            ExprKind::String(s) => {
                let idx = self.module.strings.intern(s);
                self.emit(Instruction::new(
                    Opcode::ConstStr,
                    Operand::StringRef(idx.0),
                ));
            }
            ExprKind::Boolean(true) => self.emit(Instruction::simple(Opcode::ConstTrue)),
            ExprKind::Boolean(false) => self.emit(Instruction::simple(Opcode::ConstFalse)),
            ExprKind::Null => self.emit(Instruction::simple(Opcode::ConstNull)),
            ExprKind::Local(slot) => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(*slot)));
            }
            ExprKind::CellLoad { slot } => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(*slot)));
                self.emit_dynamic_load("$value");
                self.emit(Instruction::simple(Opcode::UnboxAny));
            }
            ExprKind::CellRef { slot } => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(*slot)));
            }
            ExprKind::SelfClosure => {
                let function = self.self_closure.expect("SelfClosure outside a closure");
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
                let token = Token::new(
                    TableKind::MethodDef,
                    self.closure_method_base + function + 1,
                );
                self.emit(Instruction::new(Opcode::MakeClosure, Operand::Token(token)));
            }
            ExprKind::CaptureRef { environment_slot } => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
                self.emit_dynamic_load(&capture_name(*environment_slot));
                self.emit(Instruction::simple(Opcode::UnboxAny));
            }
            ExprKind::CellStore { slot, value } => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(*slot)));
                self.expr(value);
                self.emit(Instruction::simple(Opcode::BoxAny));
                self.emit_dynamic_store("$value");
                self.emit(Instruction::simple(Opcode::UnboxAny));
            }
            ExprKind::FunctionRef { func } => {
                self.emit(Instruction::new(
                    Opcode::LoadFunction,
                    Operand::Token(Token::new(TableKind::MethodDef, *func + 1)),
                ));
            }
            ExprKind::Closure { function, captures } => {
                self.emit(Instruction::simple(Opcode::NewDynamic));
                for capture in captures {
                    self.emit(Instruction::simple(Opcode::Dup));
                    if capture.capture.mode == CaptureMode::ByCell {
                        match capture.capture.source {
                            CaptureSource::Local(slot) => {
                                self.emit(Instruction::new(
                                    Opcode::LoadLocal,
                                    Operand::Local(slot),
                                ));
                                self.emit(Instruction::simple(Opcode::BoxAny));
                            }
                            CaptureSource::Environment {
                                environment_slot, ..
                            } => {
                                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
                                self.emit_dynamic_load(&capture_name(environment_slot));
                            }
                        }
                    } else {
                        self.expr(&capture.value);
                        // A function value cannot box to `Any`; it stays raw and
                        // `StoreDynamic` routes it to the environment's function
                        // slot. (ByCell/mutated function captures stay deferred:
                        // the `$value` cell slot is an `Any` and cannot hold one.)
                        if !matches!(capture.capture.ty, Ty::Function(_)) {
                            self.emit(Instruction::simple(Opcode::BoxAny));
                        }
                    }
                    self.emit_dynamic_store(&capture_name(capture.capture.environment_slot));
                    self.emit(Instruction::simple(Opcode::Pop));
                }
                let token = Token::new(
                    TableKind::MethodDef,
                    self.closure_method_base + *function + 1,
                );
                self.emit(Instruction::new(Opcode::MakeClosure, Operand::Token(token)));
            }
            ExprKind::CaptureLoad {
                environment_slot,
                mode,
                ty,
            } => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
                self.emit_dynamic_load(&capture_name(*environment_slot));
                if *mode == CaptureMode::ByCell {
                    self.emit(Instruction::simple(Opcode::UnboxAny));
                    self.emit_dynamic_load("$value");
                }
                // A by-value function capture lives in the function slot and
                // `LoadDynamic` already yields `Value::Function`: no unbox.
                if *mode == CaptureMode::ByCell || !matches!(ty, Ty::Function(_)) {
                    self.emit(Instruction::simple(Opcode::UnboxAny));
                }
            }
            ExprKind::CaptureStore {
                environment_slot,
                mode,
                value,
                ..
            } => {
                self.emit(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
                if *mode == CaptureMode::ByCell {
                    self.emit_dynamic_load(&capture_name(*environment_slot));
                    self.emit(Instruction::simple(Opcode::UnboxAny));
                }
                self.expr(value);
                self.emit(Instruction::simple(Opcode::BoxAny));
                if *mode == CaptureMode::ByCell {
                    self.emit_dynamic_store("$value");
                } else {
                    self.emit_dynamic_store(&capture_name(*environment_slot));
                }
                self.emit(Instruction::simple(Opcode::UnboxAny));
            }
            ExprKind::Sequence { effect, value } => {
                self.expr(effect);
                self.emit(Instruction::simple(Opcode::Pop));
                self.expr(value);
            }
            ExprKind::Assign { slot, value } => {
                self.expr(value);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::new(Opcode::StoreLocal, Operand::Local(*slot)));
            }
            ExprKind::Await { operand } => {
                self.expr(operand);
                if self.is_async {
                    // Inside an `async` body an await suspends like a yield: the
                    // coroutine driver parks the frame on the awaited value and
                    // resumes with the settlement (or throws at this point).
                    if self.is_async_gen {
                        let (mark_id, _) = zun_natives::resolve("collections", "asyncAwaitMark")
                            .expect("asyncAwaitMark intrinsic");
                        self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(mark_id as i32)));
                    }
                    self.emit(Instruction::simple(Opcode::Yield));
                    let (sent_id, _) = zun_natives::resolve("collections", "generatorSent")
                        .expect("generatorSent intrinsic");
                    self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(sent_id as i32)));
                } else {
                    self.emit(Instruction::simple(Opcode::Await));
                }
            }
            ExprKind::ImportDynamic { specifier } => {
                self.expr(specifier);
                self.emit(Instruction::simple(Opcode::ImportDynamic));
            }
            ExprKind::Yield { value } => {
                self.expr(value);
                self.emit(Instruction::simple(Opcode::Yield));
                // The value of a `yield` expression is what `next(v)` sends back in
                // (undefined when nothing was sent).
                let (sent_id, _) = zun_natives::resolve("collections", "generatorSent")
                    .expect("generatorSent intrinsic");
                self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(sent_id as i32)));
            }
            ExprKind::Binary { op, lhs, rhs } => {
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
            ExprKind::Conditional {
                condition,
                when_true,
                when_false,
            } => {
                self.expr(condition);
                let to_false = self.emit_branch(Opcode::BranchFalse);
                self.expr(when_true);
                let to_end = self.emit_branch(Opcode::Branch);
                self.patch_to_here(to_false);
                self.expr(when_false);
                self.patch_to_here(to_end);
            }
            ExprKind::Unary { op, operand } => match op {
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
            },
            ExprKind::Call { func, args } => {
                for a in args {
                    self.expr(a);
                }
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(Opcode::Call, Operand::Token(token)));
            }
            ExprKind::CallRest { func, args } => {
                for arg in args {
                    self.expr(arg);
                }
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(
                    Opcode::CallRest,
                    Operand::TokenImm32(token, args.len() as i32),
                ));
            }
            ExprKind::CallRestArray { func, args } => {
                self.expr(args);
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(
                    Opcode::CallRestArray,
                    Operand::Token(token),
                ));
            }
            ExprKind::CallRestArrayFixed {
                func,
                fixed_args,
                args,
            } => {
                for arg in fixed_args {
                    self.expr(arg);
                }
                self.expr(args);
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(
                    Opcode::CallRestArrayFixed,
                    Operand::TokenImm32(token, fixed_args.len() as i32),
                ));
            }
            ExprKind::GeneratorCall { func, args, native_id } => {
                self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(*func as i32 + 1)));
                self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(args.len() as i32)));
                self.emit(Instruction::simple(Opcode::NewArray));
                for (index, arg) in args.iter().enumerate() {
                    self.emit(Instruction::simple(Opcode::Dup));
                    self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(index as i32)));
                    self.expr(arg);
                    self.emit(Instruction::simple(Opcode::StoreArray));
                    self.emit(Instruction::simple(Opcode::Pop));
                }
                self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(*native_id as i32)));
            }
            ExprKind::FunctionBind { callee, args, native_id } => {
                // Push the target function, then build the managed array of
                // prepended arguments, then invoke the `functionBind` builtin
                // (params `[Function, Array]`).
                self.expr(callee);
                self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(args.len() as i32)));
                self.emit(Instruction::simple(Opcode::NewArray));
                for (index, arg) in args.iter().enumerate() {
                    self.emit(Instruction::simple(Opcode::Dup));
                    self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(index as i32)));
                    self.expr(arg);
                    self.emit(Instruction::simple(Opcode::StoreArray));
                    self.emit(Instruction::simple(Opcode::Pop));
                }
                self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(*native_id as i32)));
            }
            ExprKind::IndirectCall { callee, args, func } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
                // A dynamic function value called with other than one argument
                // (e.g. a multi-argument bound call) carries its count.
                if *func == hir::DYNAMIC_FUNCTION && args.len() != 1 {
                    self.emit(Instruction::new(
                        Opcode::CallIndirectN,
                        Operand::Imm32(args.len() as i32),
                    ));
                } else {
                    let token = if *func == hir::DYNAMIC_FUNCTION || *func == hir::DYNAMIC_PREDICATE {
                        Token::null(TableKind::MethodDef)
                    } else if *func == hir::DYNAMIC_REDUCER {
                        Token::null(TableKind::MemberRef)
                    } else {
                        Token::new(TableKind::MethodDef, *func + 1)
                    };
                    self.emit(Instruction::new(
                        Opcode::CallIndirect,
                        Operand::Token(token),
                    ));
                }
            }
            ExprKind::IndirectCallRest { callee, args, func } => {
                self.expr(callee);
                for arg in args {
                    self.expr(arg);
                }
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(
                    Opcode::CallIndirectRest,
                    Operand::TokenImm32(token, args.len() as i32),
                ));
            }
            ExprKind::OptionalIndirectCall { callee, args, func } => {
                self.expr(callee);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstNull));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_null = self.emit_branch(Opcode::BranchTrue);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_undef = self.emit_branch(Opcode::BranchTrue);
                for arg in args { self.expr(arg); }
                if *func == hir::DYNAMIC_FUNCTION && args.len() != 1 {
                    self.emit(Instruction::new(Opcode::CallIndirectN, Operand::Imm32(args.len() as i32)));
                } else {
                    let token = if *func == hir::DYNAMIC_FUNCTION || *func == hir::DYNAMIC_PREDICATE {
                        Token::null(TableKind::MethodDef)
                    } else if *func == hir::DYNAMIC_REDUCER {
                        Token::null(TableKind::MemberRef)
                    } else { Token::new(TableKind::MethodDef, *func + 1) };
                    self.emit(Instruction::new(Opcode::CallIndirect, Operand::Token(token)));
                }
                let end = self.emit_branch(Opcode::Branch);
                self.patch_to_here(use_null);
                self.patch_to_here(use_undef);
                self.emit(Instruction::simple(Opcode::Pop));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.patch_to_here(end);
            }
            ExprKind::CallBuiltin { id, args } => {
                for a in args {
                    self.expr(a);
                }
                // A `zun:math` intrinsic that the machine can do in one
                // instruction is emitted as that instruction; everything else
                // keeps calling the native.
                match math_machine_op(*id, args.len()) {
                    Some(op) => self.emit(Instruction::simple(op)),
                    None => self.emit(Instruction::new(
                        Opcode::CallBuiltin,
                        Operand::Imm32(*id as i32),
                    )),
                }
            }
            ExprKind::RegexMatch { matcher, args } => {
                // `self.regex_plan` is a borrow independent of `self`; bind it
                // locally so the arm can also borrow `self` mutably to emit.
                let plan = self.regex_plan;
                match &plan[*matcher as usize] {
                    RegexPlan::Compiled { is_match_rid } => {
                        let rid = *is_match_rid;
                        self.expr(&args[0]); // subject string
                        self.emit(Instruction::new(
                            Opcode::Call,
                            Operand::Token(Token::new(TableKind::MethodDef, rid)),
                        ));
                    }
                    RegexPlan::Fallback { regexp_new_id, is_match_id, pattern, flags } => {
                        // regexpIsMatch(regexpNew(pattern, flags), subject) —
                        // identical to the ordinary (non-static) regex path.
                        let (new_id, match_id) = (*regexp_new_id, *is_match_id);
                        let p = self.module.strings.intern(pattern);
                        self.emit(Instruction::new(Opcode::ConstStr, Operand::StringRef(p.0)));
                        let f = self.module.strings.intern(flags);
                        self.emit(Instruction::new(Opcode::ConstStr, Operand::StringRef(f.0)));
                        self.emit(Instruction::new(
                            Opcode::CallBuiltin,
                            Operand::Imm32(new_id as i32),
                        ));
                        self.expr(&args[0]); // subject string
                        self.emit(Instruction::new(
                            Opcode::CallBuiltin,
                            Operand::Imm32(match_id as i32),
                        ));
                    }
                }
            }
            ExprKind::DynamicNew => {
                self.emit(Instruction::simple(Opcode::NewDynamic));
            }
            ExprKind::RealmGlobal => {
                self.emit(Instruction::simple(Opcode::RealmGlobal));
            }
            ExprKind::DynamicGet { object, name } => {
                self.expr(object);
                let property = self.module.strings.intern(name);
                self.emit(Instruction::new(
                    Opcode::LoadDynamic,
                    Operand::StringRef(property.0),
                ));
            }
            ExprKind::DynamicGetValue { object, key } => {
                self.expr(object);
                self.expr(key);
                self.emit(Instruction::simple(Opcode::LoadDynamicValue));
            }
            ExprKind::OptionalDynamicGet { object, name } => {
                self.expr(object);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstNull));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_null = self.emit_branch(Opcode::BranchTrue);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_undef = self.emit_branch(Opcode::BranchTrue);
                let property = self.module.strings.intern(name);
                self.emit(Instruction::new(
                    Opcode::LoadDynamic,
                    Operand::StringRef(property.0),
                ));
                let end = self.emit_branch(Opcode::Branch);
                self.patch_to_here(use_null);
                self.patch_to_here(use_undef);
                self.emit(Instruction::simple(Opcode::Pop));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.patch_to_here(end);
            }
            ExprKind::OptionalDynamicGetValue { object, key } => {
                self.expr(object);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstNull));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_null = self.emit_branch(Opcode::BranchTrue);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_undef = self.emit_branch(Opcode::BranchTrue);
                self.expr(key);
                self.emit(Instruction::simple(Opcode::LoadDynamicValue));
                let end = self.emit_branch(Opcode::Branch);
                self.patch_to_here(use_null);
                self.patch_to_here(use_undef);
                self.emit(Instruction::simple(Opcode::Pop));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.patch_to_here(end);
            }
            ExprKind::OptionalCallBuiltin { id, args } => {
                self.expr(&args[0]);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstNull));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_null = self.emit_branch(Opcode::BranchTrue);
                self.emit(Instruction::simple(Opcode::Dup));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.emit(Instruction::simple(Opcode::CmpEq));
                let use_undef = self.emit_branch(Opcode::BranchTrue);
                for arg in &args[1..] {
                    self.expr(arg);
                }
                self.emit(Instruction::new(
                    Opcode::CallBuiltin,
                    Operand::Imm32(*id as i32),
                ));
                let end = self.emit_branch(Opcode::Branch);
                self.patch_to_here(use_null);
                self.patch_to_here(use_undef);
                self.emit(Instruction::simple(Opcode::Pop));
                self.emit(Instruction::simple(Opcode::ConstUndef));
                self.patch_to_here(end);
            }
            ExprKind::DynamicSet {
                object,
                name,
                value,
            } => {
                self.expr(object);
                self.expr(value);
                let property = self.module.strings.intern(name);
                self.emit(Instruction::new(
                    Opcode::StoreDynamic,
                    Operand::StringRef(property.0),
                ));
            }
            ExprKind::DynamicSetValue { object, key, value } => {
                self.expr(object);
                self.expr(key);
                self.expr(value);
                self.emit(Instruction::simple(Opcode::StoreDynamicValue));
            }
            ExprKind::DynamicObjectLiteral { entries } => {
                // Allocate once, then set each property on a duplicated handle,
                // discarding the value the store leaves behind. The object stays
                // on the stack as the literal's result.
                self.emit(Instruction::simple(Opcode::NewDynamic));
                for entry in entries {
                    self.emit(Instruction::simple(Opcode::Dup));
                    match entry {
                        hir::ObjectEntry::Data { key, value } => match key {
                            hir::ObjectKey::Named(name) => {
                                self.expr(value);
                                let property = self.module.strings.intern(name);
                                self.emit(Instruction::new(
                                    Opcode::StoreDynamic,
                                    Operand::StringRef(property.0),
                                ));
                            }
                            hir::ObjectKey::Computed(key) => {
                                self.expr(key);
                                self.expr(value);
                                self.emit(Instruction::simple(Opcode::StoreDynamicValue));
                            }
                        },
                        hir::ObjectEntry::Install { builtin, args } => {
                            // The object is the dup'd handle (arg0); push the
                            // remaining args, call the native, and discard its
                            // result so the object stays as the literal's value.
                            for arg in args {
                                self.expr(arg);
                            }
                            self.emit(Instruction::new(
                                Opcode::CallBuiltin,
                                Operand::Imm32(*builtin as i32),
                            ));
                        }
                    }
                    self.emit(Instruction::simple(Opcode::Pop));
                }
            }
            ExprKind::BoxAny { value } => {
                self.expr(value);
                self.emit(Instruction::simple(Opcode::BoxAny));
            }
            ExprKind::ExternalCall { module, name, args } => {
                for a in args {
                    self.expr(a);
                }
                let module_name = self.module.strings.intern(module);
                let member_name = self.module.strings.intern(name);
                let module_ref = self
                    .module
                    .tables
                    .add_module_ref(ModuleRef { name: module_name });
                let arity = u8::try_from(args.len()).expect("external call arity exceeds metadata limit");
                // Keep arity first for the verifier, followed by scalar type
                // tags for the typed native-call boundary. Zero denotes a
                // managed/non-blittable type at the explicit stub boundary.
                let mut signature_bytes = Vec::with_capacity(args.len() + 2);
                signature_bytes.push(arity);
                signature_bytes.extend(args.iter().map(|arg| ffi_type_tag(arg.ty)));
                signature_bytes.push(ffi_type_tag(e.ty));
                let signature = self.module.blobs.intern(&signature_bytes);
                let member_ref = self.module.tables.add_member_ref(MemberRef {
                    parent: module_ref,
                    name: member_name,
                    signature,
                });
                self.emit(Instruction::new(Opcode::Call, Operand::Token(member_ref)));
            }
            ExprKind::MethodCall { func, args } => {
                // args[0] is the receiver; push it and the rest, then dispatch
                // virtually on the receiver's runtime class. The token names the
                // statically resolved method (for its name and arity).
                for a in args {
                    self.expr(a);
                }
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(Opcode::CallVirt, Operand::Token(token)));
            }
            ExprKind::StaticCall { func, args } => {
                for a in args {
                    self.expr(a);
                }
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(Opcode::Call, Operand::Token(token)));
            }
            ExprKind::MethodCallRest { func, args } => {
                for a in args {
                    self.expr(a);
                }
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(
                    Opcode::CallVirtRest,
                    Operand::TokenImm32(token, args.len() as i32),
                ));
            }
            ExprKind::MethodCallRestArray { func, receiver, args } => {
                self.expr(receiver);
                self.expr(args);
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(Opcode::CallVirtRestArray, Operand::Token(token)));
            }
            ExprKind::MethodCallRestArrayFixed { func, receiver, fixed_args, args } => {
                self.expr(receiver);
                for fixed in fixed_args {
                    self.expr(fixed);
                }
                self.expr(args);
                let token = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(
                    Opcode::CallVirtRestArrayFixed,
                    Operand::TokenImm32(token, fixed_args.len() as i32),
                ));
            }
            ExprKind::New { class, args } => {
                for a in args {
                    self.expr(a);
                }
                let token = self.layout.type_tokens[*class as usize];
                self.emit(Instruction::new(Opcode::NewObj, Operand::Token(token)));
            }
            ExprKind::NewWithConstructor { class, defaults, func, args } => {
                // Allocate with default field values, then run the constructor
                // on a duplicate of the instance handle. Every call leaves one
                // value (void included), so discard it; the instance remains.
                for d in defaults {
                    self.expr(d);
                }
                let token = self.layout.type_tokens[*class as usize];
                self.emit(Instruction::new(Opcode::NewObj, Operand::Token(token)));
                self.emit(Instruction::simple(Opcode::Dup));
                for a in args {
                    self.expr(a);
                }
                let ctor = Token::new(TableKind::MethodDef, *func + 1);
                self.emit(Instruction::new(Opcode::CallVirt, Operand::Token(ctor)));
                self.emit(Instruction::simple(Opcode::Pop));
            }
            ExprKind::ArrayNew { length } => {
                self.expr(length);
                self.emit(Instruction::simple(Opcode::NewArray));
            }
            ExprKind::ArrayLiteral { elements } => {
                // Allocate an array of fixed length, then store each element on
                // a duplicated handle at its positional index, discarding the
                // value `StoreArray` leaves behind. The array stays on the stack
                // as the literal's result. `StoreArray` boxes each value to Any.
                let length = i32::try_from(elements.len())
                    .expect("array literal length exceeds i32 range");
                self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(length)));
                self.emit(Instruction::simple(Opcode::NewArray));
                for (index, element) in elements.iter().enumerate() {
                    // A hole leaves the pre-allocated slot alone except that we
                    // overwrite the default fill with `undefined`, so a hole
                    // reads as `undefined` (not `0`) while still counting toward
                    // `length`.
                    let index = i32::try_from(index)
                        .expect("array literal index exceeds i32 range");
                    self.emit(Instruction::simple(Opcode::Dup));
                    self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(index)));
                    match element {
                        Some(element) => {
                            self.expr(element);
                            self.emit(Instruction::simple(Opcode::StoreArray));
                            self.emit(Instruction::simple(Opcode::Pop));
                        }
                        None => {
                            // An elision: mark the slot as a hole (reads as
                            // `undefined`, but is absent from own-key enumeration).
                            let (id, _) = zun_natives::resolve("collections", "arrayMakeHole")
                                .expect("arrayMakeHole intrinsic");
                            self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(id as i32)));
                            self.emit(Instruction::simple(Opcode::Pop));
                        }
                    }
                }
            }
            ExprKind::ArraySpreadLiteral { parts } => {
                // Build a growable array (length 0) and append each part in
                // source order: a plain element and a hole each push one value
                // (`undefined` for a hole); a spread splices the iterable
                // source's elements in via `arraySpreadAppend`.
                self.emit(Instruction::new(Opcode::ConstI32, Operand::Imm32(0)));
                self.emit(Instruction::simple(Opcode::NewArray));
                for part in parts {
                    self.emit(Instruction::simple(Opcode::Dup));
                    match part {
                        hir::ArrayPart::Element(element) => {
                            self.expr(element);
                            self.emit(Instruction::simple(Opcode::ArrayPush));
                        }
                        hir::ArrayPart::Hole => {
                            let (id, _) = zun_natives::resolve("collections", "arrayPushHole")
                                .expect("arrayPushHole intrinsic");
                            self.emit(Instruction::new(Opcode::CallBuiltin, Operand::Imm32(id as i32)));
                        }
                        hir::ArrayPart::Spread { builtin, source } => {
                            self.expr(source);
                            self.emit(Instruction::new(
                                Opcode::CallBuiltin,
                                Operand::Imm32(*builtin as i32),
                            ));
                        }
                    }
                    // Discard the per-part result (push length / native return),
                    // leaving the array as the literal's value.
                    self.emit(Instruction::simple(Opcode::Pop));
                }
            }
            ExprKind::FieldGet {
                object,
                class,
                field_slot,
            } => {
                self.expr(object);
                let token = self.layout.field_tokens[*class as usize][*field_slot as usize];
                self.emit(Instruction::new(Opcode::LoadField, Operand::Token(token)));
            }
            ExprKind::FieldSet {
                object,
                class,
                field_slot,
                value,
            } => {
                // [object, value] -> stfld -> [value]. stfld leaves the stored
                // value so the assignment yields it; in statement position the
                // always-pop rule discards it.
                self.expr(object);
                self.expr(value);
                let token = self.layout.field_tokens[*class as usize][*field_slot as usize];
                self.emit(Instruction::new(Opcode::StoreField, Operand::Token(token)));
            }
            ExprKind::StaticFieldGet { class, field_slot } => {
                let token = self.layout.field_tokens[*class as usize][*field_slot as usize];
                self.emit(Instruction::new(Opcode::LoadStatic, Operand::Token(token)));
            }
            ExprKind::StaticFieldSet { class, field_slot, value } => {
                self.expr(value);
                let token = self.layout.field_tokens[*class as usize][*field_slot as usize];
                self.emit(Instruction::new(Opcode::StoreStatic, Operand::Token(token)));
            }
            ExprKind::ArrayGet { object, index } => {
                self.expr(object);
                self.expr(index);
                self.emit(Instruction::simple(Opcode::LoadArray));
            }
            ExprKind::ArraySet {
                object,
                index,
                value,
            } => {
                self.expr(object);
                self.expr(index);
                self.expr(value);
                self.emit(Instruction::simple(Opcode::StoreArray));
            }
            ExprKind::ArrayLength { object } => {
                self.expr(object);
                self.emit(Instruction::simple(Opcode::ArrayLength));
            }
            ExprKind::ArrayPop { object } => {
                self.expr(object);
                self.emit(Instruction::simple(Opcode::ArrayPop));
            }
            ExprKind::ArrayPush { object, value } => {
                self.expr(object);
                self.expr(value);
                self.emit(Instruction::simple(Opcode::ArrayPush));
            }
            ExprKind::Convert { value } => {
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
            ExprKind::Error => unreachable!("ilgen runs only on error-free programs"),
        }
    }
}
