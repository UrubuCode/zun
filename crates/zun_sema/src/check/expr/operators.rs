use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_await(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let ast::Expr::Await { operand, .. } = expr else {
            unreachable!()
        };
        let checked = self.check_expr(cx, operand);
        // `await` accepts any value: a non-Promise resumes on the next tick.
        let span = checked.span;
        hir::Expr {
            kind: hir::ExprKind::Await {
                operand: Box::new(checked),
            },
            ty: Ty::Any,
            span,
        }
    }

    pub(in crate::check) fn check_import_call(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::ImportCall { specifier, .. } = expr else {
            unreachable!()
        };
        let mut checked = self.check_expr(cx, specifier);
        if !checked.ty.is_error() && checked.ty != Ty::String {
            self.error(checked.span, "import() specifier must be a string");
            checked = self.poison(span);
        }
        // v1 types every dynamic import as `Promise<any>`; the resolved
        // namespace is an opt-in DynamicObject accessed dynamically.
        hir::Expr {
            kind: hir::ExprKind::ImportDynamic {
                specifier: Box::new(checked),
            },
            ty: Ty::Promise,
            span,
        }
    }

    pub(in crate::check) fn check_yield(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::Yield { value, .. } = expr else {
            unreachable!()
        };
        if !cx.is_generator {
            self.error(span, "yield is only valid inside a generator function");
        }
        let checked = self.check_expr(cx, value);
        hir::Expr {
            kind: hir::ExprKind::Yield {
                value: Box::new(checked),
            },
            ty: Ty::Any,
            span,
        }
    }

    pub(in crate::check) fn check_binary_expr(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::Binary { op, lhs, rhs, .. } = expr else {
            unreachable!()
        };
        if let ast::BinaryOp::In = op {
            return self.check_in_operator(cx, lhs, rhs, span);
        }
        if let ast::BinaryOp::Instanceof = op {
            return self.check_instanceof_operator(cx, lhs, rhs, span);
        }
        // The `x++`/`x--` desugar (`x = x ± 1`) carries a numeric literal
        // whose span equals the whole expression's; on a BigInt operand
        // that literal is `1n`.
        let update_step = matches!(
            rhs.as_ref(),
            ast::Expr::Number { span: s, .. } if *s == span
        ) && matches!(op, ast::BinaryOp::Add | ast::BinaryOp::Sub);
        let step_is_one = matches!(rhs.as_ref(), ast::Expr::Number { value, .. } if *value == 1.0);
        let mut lhs = self.check_expr(cx, lhs);
        let mut rhs = self.check_expr(cx, rhs);
        // `++`/`--` on a dynamic operand is ToNumeric(x) ± 1, never concatenation.
        if update_step && step_is_one && lhs.ty == Ty::Any {
            lhs = self.to_number_expr(lhs);
        }
        if update_step && lhs.ty == Ty::BigInt {
            if let Some((id, native)) = zun_natives::resolve("collections", "bigIntNew") {
                rhs = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![hir::Expr {
                            kind: hir::ExprKind::String("1".to_string()),
                            ty: Ty::String,
                            span,
                        }],
                    },
                    ty: native_ty(native.ret),
                    span,
                };
            }
        }
        // Let an integral number literal take on an Int sibling's type
        // so `x + 1` type-checks when `x: int` (and vice versa).
        unify_numeric_literal(&mut lhs, &mut rhs);
        // A JS `number` (f64) meeting a fixed-width integer (e.g. a
        // string `.length`) widens the integer to `number`.
        if matches!(
            *op,
            ast::BinaryOp::Add
                | ast::BinaryOp::Sub
                | ast::BinaryOp::Mul
                | ast::BinaryOp::Div
                | ast::BinaryOp::Rem
                | ast::BinaryOp::Lt
                | ast::BinaryOp::Le
                | ast::BinaryOp::Gt
                | ast::BinaryOp::Ge
                | ast::BinaryOp::Eq
                | ast::BinaryOp::Ne
        ) {
            let widen = |e: &mut hir::Expr| {
                let span = e.span;
                let inner = std::mem::replace(
                    e,
                    hir::Expr {
                        kind: hir::ExprKind::Null,
                        ty: Ty::Any,
                        span,
                    },
                );
                *e = hir::Expr {
                    kind: hir::ExprKind::Convert {
                        value: Box::new(inner),
                    },
                    ty: Ty::F64,
                    span,
                };
            };
            // Only the `u64` of `.length` widens implicitly; other
            // fixed-width integers still require an explicit conversion.
            let length_ty = Ty::Int(IntKind::U64);
            if lhs.ty == Ty::F64 && rhs.ty == length_ty {
                widen(&mut rhs);
            } else if rhs.ty == Ty::F64 && lhs.ty == length_ty {
                widen(&mut lhs);
            }
        }
        // JS loose equality (`==` / `!=` from the OXC front end).
        // Operands of one primitive type compare like `===`; anything
        // else (mixed types, `any`, objects) goes through the runtime
        // IsLooselyEqual algorithm.
        let strict_op;
        let op = if matches!(op, ast::BinaryOp::LooseEq | ast::BinaryOp::LooseNe) {
            let negate = matches!(op, ast::BinaryOp::LooseNe);
            let simple = lhs.ty == rhs.ty
                && (lhs.ty == Ty::F64
                    || lhs.ty == Ty::String
                    || lhs.ty == Ty::Boolean
                    || lhs.ty == Ty::BigInt
                    || lhs.ty.is_int()
                    || lhs.ty.is_float());
            if lhs.ty.is_error() || rhs.ty.is_error() {
                return self.poison(span);
            }
            if !simple {
                let Some((id, _)) = zun_natives::resolve("collections", "looseEquals") else {
                    self.error(span, "missing `looseEquals` intrinsic");
                    return self.poison(span);
                };
                let call = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![Self::box_to_any(lhs), Self::box_to_any(rhs)],
                    },
                    ty: Ty::Boolean,
                    span,
                };
                return if negate {
                    hir::Expr {
                        kind: hir::ExprKind::Unary {
                            op: ast::UnaryOp::Not,
                            operand: Box::new(call),
                        },
                        ty: Ty::Boolean,
                        span,
                    }
                } else {
                    call
                };
            }
            strict_op = if negate {
                ast::BinaryOp::Ne
            } else {
                ast::BinaryOp::Eq
            };
            &strict_op
        } else {
            op
        };
        // Strict equality across different static types is simply
        // `false` in JS (never an error): compare through `any`, where
        // the runtime sees two unequal kinds.
        if matches!(op, ast::BinaryOp::Eq | ast::BinaryOp::Ne)
            && lhs.ty != rhs.ty
            && !lhs.ty.is_error()
            && !rhs.ty.is_error()
            && lhs.ty != Ty::Any
            && rhs.ty != Ty::Any
            && !(matches!(lhs.ty, Ty::Function(_)) && matches!(rhs.ty, Ty::Function(_)))
            && !(matches!(lhs.ty, Ty::Instance(_)) && matches!(rhs.ty, Ty::Instance(_)))
        {
            lhs = Self::box_to_any(lhs);
            rhs = Self::box_to_any(rhs);
        }
        if lhs.ty == Ty::BigInt && rhs.ty == Ty::BigInt {
            // `>>>` has no BigInt form in ECMAScript: it is a TypeError.
            if let ast::BinaryOp::UShr = op {
                if let Some((id, native)) = zun_natives::resolve("collections", "bigIntUshr") {
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![lhs, rhs],
                        },
                        ty: native_ty(native.ret),
                        span,
                    };
                }
                self.error(span, "BigInts have no unsigned right shift (`>>>`)");
                return self.poison(span);
            }
            let intrinsic = match op {
                ast::BinaryOp::Add => Some("bigIntAdd"),
                ast::BinaryOp::Sub => Some("bigIntSub"),
                ast::BinaryOp::Mul => Some("bigIntMul"),
                ast::BinaryOp::Div => Some("bigIntDiv"),
                ast::BinaryOp::Rem => Some("bigIntRem"),
                ast::BinaryOp::Pow => Some("bigIntPow"),
                ast::BinaryOp::BitAnd => Some("bigIntAnd"),
                ast::BinaryOp::BitOr => Some("bigIntOr"),
                ast::BinaryOp::BitXor => Some("bigIntXor"),
                ast::BinaryOp::Shl => Some("bigIntShl"),
                ast::BinaryOp::Shr => Some("bigIntShr"),
                ast::BinaryOp::Eq => Some("bigIntEq"),
                ast::BinaryOp::Ne => Some("bigIntNe"),
                ast::BinaryOp::Lt => Some("bigIntLt"),
                ast::BinaryOp::Le => Some("bigIntLe"),
                ast::BinaryOp::Gt => Some("bigIntGt"),
                ast::BinaryOp::Ge => Some("bigIntGe"),
                _ => None,
            };
            if let Some(name) = intrinsic {
                if let Some((id, native)) = zun_natives::resolve("collections", name) {
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![lhs, rhs],
                        },
                        ty: native_ty(native.ret),
                        span,
                    };
                }
            }
            self.error(span, "BigInt supports arithmetic and comparison operators");
            return self.poison(span);
        }
        // `number ** number` routes to the `Math.pow` intrinsic (JS
        // exponent semantics, NaN/Infinity edges included). Integer `**`
        // stays an IL opcode (`pow.i32`), lowered below via binary_result.
        if let ast::BinaryOp::Pow = op {
            // `number ** number`, and any `**` with a dynamic operand,
            // route to `Math.pow` (the dynamic side coerces to f64 at
            // run time). Pure-int `**` stays the `pow.i32` opcode below.
            let numberish = |t: Ty| t == Ty::F64 || t == Ty::Any;
            if numberish(lhs.ty) && numberish(rhs.ty) {
                if let Some((id, native)) = zun_natives::resolve("math", "powNumber") {
                    let lhs = self.to_number_expr_strict(lhs);
                    let rhs = self.to_number_expr_strict(rhs);
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![lhs, rhs],
                        },
                        ty: native_ty(native.ret),
                        span,
                    };
                }
            }
        }
        // `+` with an object/any operand: ECMAScript ToPrimitive(default)
        // on both sides, then string concat or numeric add, at run time.
        if let ast::BinaryOp::Add = op {
            let objectish = |t: Ty| matches!(t, Ty::Any | Ty::DynamicObject | Ty::Instance(_));
            let operable = |t: Ty| {
                objectish(t)
                    || t == Ty::String
                    || t == Ty::F64
                    || t == Ty::Boolean
                    || t == Ty::BigInt
                    || t == Ty::Array
                    || t == Ty::Void
                    || t == Ty::Symbol
                    || matches!(t, Ty::Int(_) | Ty::Float(_) | Ty::BufferView(_) | Ty::Buffer)
            };
            let numeric_ty = |t: Ty| t.is_int() || t.is_float();
            // Route through the runtime `addAny` (ECMAScript
            // ToPrimitive + string concat / numeric add) whenever an
            // operand is object/any, or a string meets a non-string
            // primitive (`"x" + 1`, `"x" + true`). Same-type numeric and
            // string+string keep their direct lowerings below.
            let involves_string = lhs.ty == Ty::String || rhs.ty == Ty::String;
            if operable(lhs.ty)
                && operable(rhs.ty)
                && (objectish(lhs.ty)
                    || objectish(rhs.ty)
                    || (involves_string && lhs.ty != rhs.ty)
                    || (lhs.ty == Ty::Boolean && rhs.ty == Ty::Boolean)
                    || (lhs.ty == Ty::Array && rhs.ty == Ty::Array)
                    || (lhs.ty != rhs.ty
                        && !(numeric_ty(lhs.ty) && numeric_ty(rhs.ty))
                        && lhs.ty != Ty::BigInt
                        && rhs.ty != Ty::BigInt))
            {
                if let Some((id, _)) = zun_natives::resolve("collections", "addAny") {
                    let to_any = |e: hir::Expr| {
                        if e.ty == Ty::Any {
                            e
                        } else {
                            hir::Expr {
                                span: e.span,
                                kind: hir::ExprKind::BoxAny { value: Box::new(e) },
                                ty: Ty::Any,
                            }
                        }
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![to_any(lhs), to_any(rhs)],
                        },
                        ty: Ty::Any,
                        span,
                    };
                }
            }
        }
        // Dynamic numeric operators (`-`, `*`, `/`, `%`) apply
        // ECMAScript ToNumber to both operands when a non-numeric or
        // dynamic operand is involved (`"5" * 2`, `+x`, `any - 1`), then
        // run the ordinary f64 op. Same-type numeric operands keep their
        // direct lowering below.
        if matches!(
            *op,
            ast::BinaryOp::Sub
                | ast::BinaryOp::Mul
                | ast::BinaryOp::Div
                | ast::BinaryOp::Rem
                | ast::BinaryOp::BitAnd
                | ast::BinaryOp::BitOr
                | ast::BinaryOp::BitXor
                | ast::BinaryOp::Shl
                | ast::BinaryOp::Shr
                | ast::BinaryOp::UShr
        ) {
            let numericish =
                |t: Ty| matches!(t, Ty::Int(_)) || matches!(t, Ty::Float(_)) || t == Ty::F64;
            let needs_coerce = !lhs.ty.is_error()
                && !rhs.ty.is_error()
                && !(numericish(lhs.ty) && numericish(rhs.ty))
                && lhs.ty != Ty::BigInt
                && rhs.ty != Ty::BigInt;
            if needs_coerce {
                if let Some((id, _)) = zun_natives::resolve("collections", "numberCoerceStrict") {
                    let coerce = |e: hir::Expr| hir::Expr {
                        span: e.span,
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![Self::box_to_any(e)],
                        },
                        ty: Ty::F64,
                    };
                    let lhs = coerce(lhs);
                    let rhs = coerce(rhs);
                    return hir::Expr {
                        kind: hir::ExprKind::Binary {
                            op: *op,
                            lhs: Box::new(lhs),
                            rhs: Box::new(rhs),
                        },
                        ty: Ty::F64,
                        span,
                    };
                }
            }
        }
        // Relational comparison of class instances (`dateA < dateB`)
        // applies ToPrimitive (`valueOf`) then compares numerically.
        if matches!(
            *op,
            ast::BinaryOp::Lt | ast::BinaryOp::Le | ast::BinaryOp::Gt | ast::BinaryOp::Ge
        ) && (matches!(lhs.ty, Ty::Instance(_)) || matches!(rhs.ty, Ty::Instance(_)))
            && !lhs.ty.is_error()
            && !rhs.ty.is_error()
        {
            if let Some((id, _)) = zun_natives::resolve("collections", "numberCoerce") {
                let coerce = |e: hir::Expr| hir::Expr {
                    span: e.span,
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![Self::box_to_any(e)],
                    },
                    ty: Ty::F64,
                };
                return hir::Expr {
                    kind: hir::ExprKind::Binary {
                        op: *op,
                        lhs: Box::new(coerce(lhs)),
                        rhs: Box::new(coerce(rhs)),
                    },
                    ty: Ty::Boolean,
                    span,
                };
            }
        }
        // Relational comparison of arrays / plain objects: ToPrimitive at run
        // time, through the dynamic comparison.
        let (lhs, rhs) = if matches!(
            *op,
            ast::BinaryOp::Lt | ast::BinaryOp::Le | ast::BinaryOp::Gt | ast::BinaryOp::Ge
        ) && (matches!(lhs.ty, Ty::Array | Ty::DynamicObject)
            || matches!(rhs.ty, Ty::Array | Ty::DynamicObject))
            && !lhs.ty.is_error()
            && !rhs.ty.is_error()
        {
            (Self::box_to_any(lhs), Self::box_to_any(rhs))
        } else {
            (lhs, rhs)
        };
        let ty = self.binary_result(*op, &lhs, &rhs, span);
        // A logical/nullish operator whose result is dynamic keeps the
        // deciding operand's value (value-preserving short-circuit), so
        // both operands must be boxed to a uniform `any` carrier.
        let (lhs, rhs) = if ty == Ty::Any
            && matches!(
                *op,
                ast::BinaryOp::And | ast::BinaryOp::Or | ast::BinaryOp::Nullish
            ) {
            (Self::box_to_any(lhs), Self::box_to_any(rhs))
        } else {
            (lhs, rhs)
        };
        hir::Expr {
            kind: hir::ExprKind::Binary {
                op: *op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
            ty,
            span,
        }
    }

    pub(in crate::check) fn check_conditional(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::Conditional {
            condition,
            when_true,
            when_false,
            ..
        } = expr
        else {
            unreachable!()
        };
        let condition = self.check_condition(cx, condition);
        let mut when_true = self.check_expr(cx, when_true);
        let mut when_false = self.check_expr(cx, when_false);
        let ty = if when_true.ty == when_false.ty {
            when_true.ty
        } else if matches!(when_true.ty, Ty::Function(_))
            && matches!(when_false.ty, Ty::Function(_))
        {
            // Two function-typed branches (e.g. `cond ? inc : dec`). Each
            // function is a distinct nominal `Ty::Function(index)`. A
            // *concrete* function type lowers to a monomorphic
            // `call.indirect` whose token pins the callee to that one
            // method (the interpreter rejects a value whose `method_rid`
            // differs — see `CallIndirect` in zun_interp), so a join to
            // either branch's index would be unsound: the other branch's
            // value would fail the pin at run time. The sound join is the
            // dynamic any-arity function type, which dispatches purely on
            // the runtime value (token rid 0) and yields `any`.
            Ty::Function(hir::DYNAMIC_FUNCTION)
        } else {
            // Branches of differing types form a union, erased to a
            // dynamic value. Box both sides so the stack value is a
            // proper `any` carrier regardless of which branch runs.
            when_true = Self::box_to_any(when_true);
            when_false = Self::box_to_any(when_false);
            Ty::Any
        };
        hir::Expr {
            kind: hir::ExprKind::Conditional {
                condition: Box::new(condition),
                when_true: Box::new(when_true),
                when_false: Box::new(when_false),
            },
            ty,
            span,
        }
    }

    pub(in crate::check) fn check_unary(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::Unary { op, operand, .. } = expr else {
            unreachable!()
        };
        match op {
            ast::UnaryOp::Typeof => return self.check_typeof_operator(cx, operand, span),
            ast::UnaryOp::Delete => return self.check_delete_operator(cx, operand, span),
            ast::UnaryOp::Neg | ast::UnaryOp::Not | ast::UnaryOp::BitNot | ast::UnaryOp::Void => {}
        }
        // Fold `-<number literal>` into a single negative number literal
        // so it stays coercible to an int context (e.g. `abs(-7)`).
        if let ast::UnaryOp::Neg = op {
            if let ast::Expr::Number { value, .. } = operand.as_ref() {
                return hir::Expr {
                    kind: hir::ExprKind::Number(-value),
                    ty: Ty::F64,
                    span,
                };
            }
        }
        let operand = self.check_expr(cx, operand);
        if operand.ty == Ty::BigInt && matches!(op, ast::UnaryOp::Neg | ast::UnaryOp::BitNot) {
            let native_name = match op {
                ast::UnaryOp::Neg => "bigIntNeg",
                ast::UnaryOp::BitNot => "bigIntNot",
                _ => unreachable!(),
            };
            if let Some((id, native)) = zun_natives::resolve("collections", native_name) {
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![operand],
                    },
                    ty: native_ty(native.ret),
                    span,
                };
            }
        }
        // `!x` on a non-boolean operand: JS ToBoolean first.
        let operand = if matches!(op, ast::UnaryOp::Not)
            && operand.ty != Ty::Boolean
            && !operand.ty.is_error()
        {
            match zun_natives::resolve("boolean", "booleanCoerce") {
                Some((id, _)) => {
                    let operand_span = operand.span;
                    hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![operand],
                        },
                        ty: Ty::Boolean,
                        span: operand_span,
                    }
                }
                None => operand,
            }
        } else {
            operand
        };
        // `-x` / `~x` on a non-numeric operand: JS ToNumber first.
        let operand = if matches!(op, ast::UnaryOp::Neg | ast::UnaryOp::BitNot)
            && !operand.ty.is_int()
            && !operand.ty.is_float()
            && operand.ty != Ty::BigInt
        {
            self.to_number_expr(operand)
        } else {
            operand
        };
        let ty = match op {
            // Negation keeps the operand's numeric type; `!` needs bool.
            ast::UnaryOp::Neg if operand.ty.is_int() || operand.ty.is_float() => operand.ty,
            ast::UnaryOp::Neg => {
                self.error(operand.span, format!("cannot negate {:?}", operand.ty));
                Ty::Error
            }
            ast::UnaryOp::Not => self.expect_ty(&operand, Ty::Boolean),
            // `~` keeps the operand's numeric type (int, or `number` via
            // the int32 coercion lowered in ilgen).
            ast::UnaryOp::BitNot if operand.ty.is_int() || operand.ty == Ty::F64 => operand.ty,
            ast::UnaryOp::BitNot => {
                self.error(
                    operand.span,
                    format!("cannot apply `~` to {:?}", operand.ty),
                );
                Ty::Error
            }
            // `void x` evaluates its operand and yields `undefined`.
            ast::UnaryOp::Void => Ty::Any,
            ast::UnaryOp::Typeof | ast::UnaryOp::Delete => {
                unreachable!("handled before operand type-checking")
            }
        };
        hir::Expr {
            kind: hir::ExprKind::Unary {
                op: *op,
                operand: Box::new(operand),
            },
            ty,
            span,
        }
    }

    pub(in crate::check) fn check_cast(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::Cast { expr, ty, .. } = expr else {
            unreachable!()
        };
        let value = self.check_expr(cx, expr);
        let target = self.resolve_annotation(ty);
        // `x as any` is a widening assertion: box a concrete value into
        // the dynamic carrier (a value already `any` passes through).
        if target == Ty::Any {
            return if value.ty == Ty::Any || value.ty.is_error() {
                value
            } else {
                hir::Expr {
                    span,
                    kind: hir::ExprKind::BoxAny {
                        value: Box::new(value),
                    },
                    ty: Ty::Any,
                }
            };
        }
        // Numeric conversions may also explicitly unbox an `any` value.
        let numeric = |t: Ty| t.is_int() || t.is_float();
        if !value.ty.is_error()
            && value.ty != target
            && !((numeric(value.ty) && numeric(target))
                || (value.ty == Ty::Any
                    && (numeric(target)
                        || target == Ty::Boolean
                        || target == Ty::Array
                        || target == Ty::String
                        || matches!(
                            target,
                            Ty::Instance(_)
                                | Ty::BufferView(_)
                                | Ty::Map
                                | Ty::Set
                                | Ty::Promise
                                | Ty::DynamicObject
                                | Ty::Buffer
                        ))))
        {
            self.error(span, format!("cannot convert {:?} to {target:?}", value.ty));
            return self.poison(span);
        }
        hir::Expr {
            kind: hir::ExprKind::Convert {
                value: Box::new(value),
            },
            ty: target,
            span,
        }
    }
}
