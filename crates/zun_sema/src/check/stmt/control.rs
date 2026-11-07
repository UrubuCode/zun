use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_if_stmt(
        &mut self,
        cx: &mut FunctionCx,
        stmt: &ast::Stmt,
    ) -> hir::Stmt {
        let ast::Stmt::If {
            cond,
            then_block,
            else_block,
            ..
        } = stmt
        else {
            unreachable!()
        };
        let narrowing = self.type_guard_narrowing(cx, cond);
        // A typed shadow copy would swallow writes to the guarded variable
        // (`if (Array.isArray(c)) c = c[0]`), so assigned names stay untyped.
        let narrowing = narrowing.filter(|(name, _, _)| {
            let mut assigned = HashSet::new();
            self.collect_mutated_names(&then_block.statements, &mut assigned);
            !assigned.contains(name)
        });
        let cond = self.check_condition(cx, cond);
        let then_body = match narrowing {
            // Inside the guarded branch the untyped local is shadowed by
            // a typed copy (`typeof x === "string"`, `Array.isArray(x)`).
            Some((name, slot, narrowed)) => {
                cx.scopes.push(Scope::new());
                let read = hir::Expr {
                    kind: hir::ExprKind::Local(slot),
                    ty: Ty::Any,
                    span: then_block.span,
                };
                let init = hir::Expr {
                    kind: hir::ExprKind::Convert {
                        value: Box::new(read),
                    },
                    ty: narrowed,
                    span: then_block.span,
                };
                let shadow = cx.declare(&name, narrowed);
                let mut body = vec![hir::Stmt::Let { slot: shadow, init }];
                body.extend(self.check_block(cx, then_block));
                cx.scopes.pop();
                body
            }
            None => self.check_block(cx, then_block),
        };
        let else_body = else_block
            .as_ref()
            .map(|b| self.check_block(cx, b))
            .unwrap_or_default();
        hir::Stmt::If {
            cond,
            then_body,
            else_body,
        }
    }

    pub(in crate::check) fn check_while_stmt(
        &mut self,
        cx: &mut FunctionCx,
        stmt: &ast::Stmt,
    ) -> hir::Stmt {
        let ast::Stmt::While { cond, body, .. } = stmt else {
            unreachable!()
        };
        let cond = self.check_condition(cx, cond);
        cx.loop_depth += 1;
        let body = self.check_block(cx, body);
        cx.loop_depth -= 1;
        hir::Stmt::While { cond, body }
    }

    pub(in crate::check) fn check_do_while_stmt(
        &mut self,
        cx: &mut FunctionCx,
        stmt: &ast::Stmt,
    ) -> hir::Stmt {
        let ast::Stmt::DoWhile { body, cond, .. } = stmt else {
            unreachable!()
        };
        cx.loop_depth += 1;
        let body = self.check_block(cx, body);
        cx.loop_depth -= 1;
        let cond = self.check_condition(cx, cond);
        hir::Stmt::DoWhile { body, cond }
    }

    pub(in crate::check) fn check_labeled_stmt(
        &mut self,
        cx: &mut FunctionCx,
        stmt: &ast::Stmt,
    ) -> hir::Stmt {
        let ast::Stmt::Labeled {
            label, statement, ..
        } = stmt
        else {
            unreachable!()
        };
        let label_text = self.text(label).to_string();
        let labels_loop = matches!(
            statement.as_ref(),
            ast::Stmt::While { .. }
                | ast::Stmt::DoWhile { .. }
                | ast::Stmt::For { .. }
                | ast::Stmt::ForOf { .. }
                | ast::Stmt::ForIn { .. }
        );
        if labels_loop {
            cx.loop_labels.push(label_text.clone());
        } else {
            cx.block_labels.push(label_text.clone());
        }
        let body = self.check_stmt(cx, statement);
        if labels_loop {
            cx.loop_labels.pop();
        } else {
            cx.block_labels.pop();
        }
        hir::Stmt::Labeled {
            label: label_text,
            body: Box::new(body),
        }
    }

    pub(in crate::check) fn check_break_stmt(
        &mut self,
        cx: &mut FunctionCx,
        stmt: &ast::Stmt,
    ) -> hir::Stmt {
        let ast::Stmt::Break { label, span } = stmt else {
            unreachable!()
        };
        match label {
            Some(label) => {
                let name = self.text(label);
                if !cx.loop_labels.iter().any(|c| c == name)
                    && !cx.block_labels.iter().any(|c| c == name)
                {
                    self.error(*span, format!("unknown break label `{name}`"));
                }
            }
            None => {
                if cx.loop_depth == 0 && cx.switch_depth == 0 {
                    self.error(*span, "`break` is only valid inside a loop or switch");
                }
            }
        }
        hir::Stmt::Break(label.map(|name| self.text(&name).to_string()))
    }

    pub(in crate::check) fn check_continue_stmt(
        &mut self,
        cx: &mut FunctionCx,
        stmt: &ast::Stmt,
    ) -> hir::Stmt {
        let ast::Stmt::Continue { label, span } = stmt else {
            unreachable!()
        };
        match label {
            Some(label) => {
                let name = self.text(label);
                if !cx.loop_labels.iter().any(|c| c == name) {
                    self.error(*span, format!("unknown continue label `{name}`"));
                }
            }
            None => {
                if cx.loop_depth == 0 {
                    self.error(*span, "`continue` is only valid inside a loop");
                }
            }
        }
        hir::Stmt::Continue(label.map(|name| self.text(&name).to_string()))
    }

    pub(in crate::check) fn check_return_stmt(
        &mut self,
        cx: &mut FunctionCx,
        stmt: &ast::Stmt,
    ) -> hir::Stmt {
        let ast::Stmt::Return { value, span } = stmt else {
            unreachable!()
        };
        let value = value.as_ref().map(|v| {
            let mut e = self.check_expr(cx, v);
            coerce_numeric_literal(&mut e, cx.return_ty);
            let e = Self::unbox_to(e, cx.return_ty);
            // Implicit widening of an integer value to `number`.
            if matches!(e.ty, Ty::Int(_)) && cx.return_ty == Ty::F64 {
                let span = e.span;
                hir::Expr {
                    kind: hir::ExprKind::Convert { value: Box::new(e) },
                    ty: Ty::F64,
                    span,
                }
            } else {
                e
            }
        });
        let value = if value.is_none() && cx.return_ty == Ty::Any {
            Some(self.check_expr(
                cx,
                &ast::Expr::Unary {
                    op: ast::UnaryOp::Void,
                    operand: Box::new(ast::Expr::Number {
                        value: 0.0,
                        span: *span,
                    }),
                    span: *span,
                },
            ))
        } else {
            value
        };
        match (&value, cx.return_ty) {
            (None, Ty::Void) => {}
            // A bare `return;` in a generator (or an `any`-returning
            // function) yields `undefined`.
            (None, Ty::Any) => {}
            (None, expected) if !expected.is_error() => {
                self.error(*span, format!("expected a {expected:?} return value"));
            }
            (Some(v), expected)
                if !expected.is_error() && !v.ty.is_error() && !self.is_assignable(v.ty, expected) => {
                    self.error(
                        v.span,
                        format!("return type {:?} does not match {expected:?}", v.ty),
                    );
                }
            _ => {}
        }
        // Inside a `try` with a `finally`, the return value is computed
        // first and held in a temporary while the cleanup runs.
        if cx.finally_depth > 0 {
            if let Some(v) = value {
                if !matches!(
                    v.kind,
                    hir::ExprKind::Number(_)
                        | hir::ExprKind::String(_)
                        | hir::ExprKind::Boolean(_)
                        | hir::ExprKind::Null
                ) && !v.ty.is_error()
                {
                    let span = v.span;
                    let ty = v.ty;
                    let (slot, _) = self.synthetic_local(cx, ty);
                    return hir::Stmt::Group(vec![
                        hir::Stmt::Let { slot, init: v },
                        hir::Stmt::Return(Some(hir::Expr {
                            kind: hir::ExprKind::Local(slot),
                            ty,
                            span,
                        })),
                    ]);
                }
                return hir::Stmt::Return(Some(v));
            }
        }
        hir::Stmt::Return(value)
    }

    pub(in crate::check) fn check_try_stmt(
        &mut self,
        cx: &mut FunctionCx,
        stmt: &ast::Stmt,
    ) -> hir::Stmt {
        let ast::Stmt::Try {
            body,
            catches,
            finally,
            ..
        } = stmt
        else {
            unreachable!()
        };
        let has_finally = finally.is_some();
        if has_finally {
            cx.finally_depth += 1;
        }
        let body = self.check_block(cx, body);
        let catches = catches
            .iter()
            .map(|catch| {
                let ty = self.resolve_annotation(&catch.ty);
                cx.scopes.push(Scope::new());
                let slot = cx.declare(self.text(&catch.binding), ty);
                let mut body = self.check_block(cx, &catch.body);
                // A catch binding captured and mutated by a closure lives
                // in a cell: the handler receives the exception in a raw
                // slot and the body starts by moving it into the cell.
                let slot = if cx.boxed.contains(&slot) {
                    let raw = cx.declare("__catch_exception", ty);
                    body.insert(
                        0,
                        hir::Stmt::Let {
                            slot,
                            init: hir::Expr {
                                kind: hir::ExprKind::Local(raw),
                                ty,
                                span: catch.body.span,
                            },
                        },
                    );
                    raw
                } else {
                    slot
                };
                cx.scopes.pop();
                hir::CatchClause { slot, ty, body }
            })
            .collect();
        if has_finally {
            cx.finally_depth -= 1;
        }
        let finally_body = finally.as_ref().map(|block| self.check_block(cx, block));
        hir::Stmt::Try {
            body,
            catches,
            finally_body,
        }
    }
}
