use crate::check::*;

impl Checker<'_> {
    /// `f(a, ...xs, b)` on a function value: collect the arguments into one
    /// array literal (spreads flattened) and invoke through `functionCallSpread`.
    pub(in crate::check) fn check_spread_call(
        &mut self,
        cx: &mut FunctionCx,
        callee: &ast::Expr,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        let callee_hir = self.check_expr(cx, callee);
        if !matches!(callee_hir.ty, Ty::Function(_) | Ty::Any) {
            return None;
        }
        let elements = args
            .iter()
            .map(|arg| match arg {
                ast::Expr::Spread { expr, .. } => ast::ArrayElement::Spread((**expr).clone()),
                other => ast::ArrayElement::Expr(other.clone()),
            })
            .collect();
        let literal = ast::Expr::ArrayLiteral { elements, span };
        let mut array = self.check_expr(cx, &literal);
        self.box_function_array_elements(&mut array);
        let (id, _) = zun_natives::resolve("collections", "functionCallSpread")?;
        Some(hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![callee_hir, array],
            },
            ty: Ty::Any,
            span,
        })
    }

    /// `{ slot = bound; test == null ? undefined : call }`: the shared shape of
    /// optional calls on dynamic receivers (`bound` evaluated exactly once).
    pub(in crate::check) fn optional_guarded(
        &mut self,
        cx: &mut FunctionCx,
        slot: hir::LocalSlot,
        bound: hir::Expr,
        test: ast::Expr,
        call: ast::Expr,
        span: Span,
    ) -> hir::Expr {
        self.optional_guarded_with(cx, slot, bound, test, call, None, span)
    }

    /// Like [`Self::optional_guarded`], with an explicit expression for the
    /// nullish case (default: `undefined`).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::check) fn optional_guarded_with(
        &mut self,
        cx: &mut FunctionCx,
        slot: hir::LocalSlot,
        bound: hir::Expr,
        test: ast::Expr,
        call: ast::Expr,
        nullish: Option<ast::Expr>,
        span: Span,
    ) -> hir::Expr {
        if let Some(probe) = nullish {
            // Nullish member: only a managed receiver (Map, class instance, Date ...)
            // may still resolve the method through the `?`-prefixed dynamic call;
            // anything else short-circuits to `undefined` without evaluating the
            // arguments.
            let Some((probe_id, _)) = zun_natives::resolve("collections", "optionalProbeOk") else {
                return self.poison(span);
            };
            let tmp_name = match &test {
                ast::Expr::Assign { value, .. } => match value.as_ref() {
                    ast::Expr::Field { object, .. } => object.as_ref().clone(),
                    _ => return self.poison(span),
                },
                _ => return self.poison(span),
            };
            let receiver_hir = self.check_expr(cx, &tmp_name);
            let ok = hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: probe_id,
                    args: vec![Self::box_to_any(receiver_hir)],
                },
                ty: Ty::Boolean,
                span,
            };
            let probe_hir = self.check_expr(cx, &probe);
            let undefined_hir = self.check_expr(
                cx,
                &ast::Expr::Unary {
                    op: ast::UnaryOp::Void,
                    operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                    span,
                },
            );
            let nullish_hir = hir::Expr {
                kind: hir::ExprKind::Conditional {
                    condition: Box::new(ok),
                    when_true: Box::new(Self::box_to_any(probe_hir)),
                    when_false: Box::new(Self::box_to_any(undefined_hir)),
                },
                ty: Ty::Any,
                span,
            };
            let cond_ast = ast::Expr::Binary {
                op: ast::BinaryOp::LooseEq,
                lhs: Box::new(test),
                rhs: Box::new(ast::Expr::Null { span }),
                span,
            };
            let condition = self.check_condition(cx, &cond_ast);
            let call_hir = self.check_expr(cx, &call);
            let value = hir::Expr {
                kind: hir::ExprKind::Conditional {
                    condition: Box::new(condition),
                    when_true: Box::new(nullish_hir),
                    when_false: Box::new(Self::box_to_any(call_hir)),
                },
                ty: Ty::Any,
                span,
            };
            return hir::Expr {
                kind: hir::ExprKind::Sequence {
                    effect: Box::new(hir::Expr {
                        kind: hir::ExprKind::Assign {
                            slot,
                            value: Box::new(bound),
                        },
                        ty: Ty::Any,
                        span,
                    }),
                    value: Box::new(value),
                },
                ty: Ty::Any,
                span,
            };
        }
        let cond = ast::Expr::Conditional {
            condition: Box::new(ast::Expr::Binary {
                op: ast::BinaryOp::LooseEq,
                lhs: Box::new(test),
                rhs: Box::new(ast::Expr::Null { span }),
                span,
            }),
            when_true: Box::new(ast::Expr::Unary {
                op: ast::UnaryOp::Void,
                operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                span,
            }),
            when_false: Box::new(call),
            span,
        };
        let value = self.check_expr(cx, &cond);
        let ty = value.ty;
        hir::Expr {
            kind: hir::ExprKind::Sequence {
                effect: Box::new(hir::Expr {
                    kind: hir::ExprKind::Assign {
                        slot,
                        value: Box::new(bound),
                    },
                    ty: Ty::Any,
                    span,
                }),
                value: Box::new(value),
            },
            ty,
            span,
        }
    }
}
