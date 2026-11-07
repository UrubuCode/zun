use crate::check::*;

mod control;
mod declarations;
mod loops;

impl Checker<'_> {
    pub(in crate::check) fn check_stmt(
        &mut self,
        cx: &mut FunctionCx,
        stmt: &ast::Stmt,
    ) -> hir::Stmt {
        match stmt {
            ast::Stmt::Destructure { .. } => self.check_destructure_stmt(cx, stmt),
            ast::Stmt::Let { .. } => self.check_let_stmt(cx, stmt),
            ast::Stmt::Expr(e) => hir::Stmt::Expr(self.check_expr(cx, e)),
            ast::Stmt::If { .. } => self.check_if_stmt(cx, stmt),
            ast::Stmt::While { .. } => self.check_while_stmt(cx, stmt),
            ast::Stmt::DoWhile { .. } => self.check_do_while_stmt(cx, stmt),
            ast::Stmt::For { .. } => self.check_for_stmt(cx, stmt),
            ast::Stmt::ForOf { .. } => self.check_for_of_stmt(cx, stmt),
            ast::Stmt::ForIn { .. } => self.check_for_in_stmt(cx, stmt),
            ast::Stmt::Labeled { .. } => self.check_labeled_stmt(cx, stmt),
            ast::Stmt::Switch {
                discriminant,
                cases,
                span,
            } => self.check_switch(cx, discriminant, cases, *span),
            ast::Stmt::Break { .. } => self.check_break_stmt(cx, stmt),
            ast::Stmt::Continue { .. } => self.check_continue_stmt(cx, stmt),
            ast::Stmt::Return { .. } => self.check_return_stmt(cx, stmt),
            ast::Stmt::Throw { value, .. } => hir::Stmt::Throw(self.check_expr(cx, value)),
            ast::Stmt::Try { .. } => self.check_try_stmt(cx, stmt),
        }
    }

    /// Lower a `Number.<method>(...)` static call. The strict predicates accept
    /// any argument type (JS does not coerce — a non-number is simply `false`);
    /// `parseFloat`/`parseInt` take a string and an optional radix.
    /// `Math.<fn>(...)` — route a JS Math call to the `math` native group's
    /// f64 helper of the matching name (`floor` → `floorNumber`, `pow` →
    /// `powNumber`, …). Arity and argument coercion follow the native's declared
    /// signature; the result type is the native's return type.
    /// Shared lowering for `for-of`/`for-in`: given the iterable (or key array)
    /// and the iterator-protocol intrinsic names, drive the iterator and bind
    /// each value to the loop pattern.
    pub(in crate::check) fn lower_for_iteration(
        &mut self,
        cx: &mut FunctionCx,
        iterable_expr: hir::Expr,
        new_name: &str,
        done_name: &str,
        next_name: &str,
        pattern: &ast::Pattern,
        body: &ast::Block,
    ) -> hir::Stmt {
        let span = iterable_expr.span;
        let Some((new_id, _)) = zun_natives::resolve("buffer", new_name) else {
            self.error(span, "missing iterator constructor intrinsic");
            return hir::Stmt::Expr(self.poison(span));
        };
        let Some((done_id, _)) = zun_natives::resolve("buffer", done_name) else {
            self.error(span, "missing iterator completion intrinsic");
            return hir::Stmt::Expr(self.poison(span));
        };
        let Some((next_id, _)) = zun_natives::resolve("buffer", next_name) else {
            self.error(span, "missing iterator next intrinsic");
            return hir::Stmt::Expr(self.poison(span));
        };
        let iterator = hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id: new_id,
                args: vec![iterable_expr],
            },
            ty: Ty::Any,
            span,
        };
        let iterator_slot = cx.declare("__for_of_iterator", Ty::Any);
        let done = hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id: done_id,
                args: vec![hir::Expr {
                    kind: hir::ExprKind::Local(iterator_slot),
                    ty: Ty::Any,
                    span,
                }],
            },
            ty: Ty::Boolean,
            span,
        };
        let next = hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id: next_id,
                args: vec![hir::Expr {
                    kind: hir::ExprKind::Local(iterator_slot),
                    ty: Ty::Any,
                    span,
                }],
            },
            ty: Ty::Any,
            span,
        };
        cx.scopes.push(Scope::new());
        let (binding, prelude) = match pattern {
            ast::Pattern::Ident { name, ty } => {
                let declared = ty
                    .as_ref()
                    .map(|t| self.resolve_annotation(t))
                    .unwrap_or(Ty::Any);
                let name_text = self.text(name).to_string();
                let slot = cx.declare(&name_text, declared);
                // A loop variable that a closure captures and that is assigned
                // lives in a per-iteration cell from the start of the body.
                if cx.mutated_names.contains(&name_text)
                    && cx.mutated_names.contains(&format!("{name_text}"))
                    && !matches!(declared, Ty::Function(_))
                {
                    cx.boxed.insert(slot);
                }
                (slot, Vec::new())
            }
            _ => {
                let element = cx.declare(&self.fresh_temp_name(cx), Ty::Any);
                let mut prelude = Vec::new();
                let element_expr = hir::Expr {
                    kind: hir::ExprKind::Local(element),
                    ty: Ty::Any,
                    span,
                };
                self.bind_pattern(cx, pattern, element_expr, &mut prelude);
                (element, prelude)
            }
        };
        cx.loop_depth += 1;
        let mut body = self.check_block(cx, body);
        cx.loop_depth -= 1;
        if !prelude.is_empty() {
            let mut combined = prelude;
            combined.append(&mut body);
            body = combined;
        }
        cx.scopes.pop();
        hir::Stmt::ForOf {
            iterator,
            iterator_slot,
            done,
            next,
            binding,
            body,
        }
    }

    /// Lower a `switch` with fall-through: the discriminant is evaluated once
    /// into a temp, the first matching case index is computed lazily with a
    /// nested conditional (default index otherwise), and each clause body runs
    /// when `index <= clause`. `break` exits the enclosing synthetic label.
    pub(in crate::check) fn check_switch(
        &mut self,
        cx: &mut FunctionCx,
        discriminant: &ast::Expr,
        cases: &[ast::SwitchCase],
        span: Span,
    ) -> hir::Stmt {
        let disc = self.check_expr(cx, discriminant);
        let disc = Self::box_to_any(disc);
        cx.scopes.push(Scope::new());
        let disc_slot = cx.declare(&self.fresh_temp_name(cx), Ty::Any);
        let idx_slot = cx.declare(&self.fresh_temp_name(cx), Ty::F64);
        let num = |value: f64| hir::Expr {
            kind: hir::ExprKind::Number(value),
            ty: Ty::F64,
            span,
        };
        let none_index = cases.len() as f64;
        let default_index = cases
            .iter()
            .position(|c| c.test.is_none())
            .map_or(none_index, |p| p as f64);
        let mut index_expr = num(default_index);
        let mut tests = Vec::new();
        for (position, case) in cases.iter().enumerate() {
            if let Some(test) = &case.test {
                let test = self.check_expr(cx, test);
                tests.push((position, Self::box_to_any(test)));
            }
        }
        for (position, test) in tests.into_iter().rev() {
            let cond = hir::Expr {
                kind: hir::ExprKind::Binary {
                    op: ast::BinaryOp::Eq,
                    lhs: Box::new(hir::Expr {
                        kind: hir::ExprKind::Local(disc_slot),
                        ty: Ty::Any,
                        span,
                    }),
                    rhs: Box::new(test),
                },
                ty: Ty::Boolean,
                span,
            };
            index_expr = hir::Expr {
                kind: hir::ExprKind::Conditional {
                    condition: Box::new(cond),
                    when_true: Box::new(num(position as f64)),
                    when_false: Box::new(index_expr),
                },
                ty: Ty::F64,
                span,
            };
        }
        let mut out = vec![
            hir::Stmt::Let {
                slot: disc_slot,
                init: disc,
            },
            hir::Stmt::Let {
                slot: idx_slot,
                init: index_expr,
            },
        ];
        cx.switch_depth += 1;
        for (position, case) in cases.iter().enumerate() {
            if case.body.statements.is_empty() {
                continue;
            }
            // All case bodies share the switch's one lexical scope, so a `let`
            // declared in one case is visible to the cases after it.
            let body: Vec<hir::Stmt> = case
                .body
                .statements
                .iter()
                .map(|statement| self.check_stmt(cx, statement))
                .collect();
            let cond = hir::Expr {
                kind: hir::ExprKind::Binary {
                    op: ast::BinaryOp::Le,
                    lhs: Box::new(hir::Expr {
                        kind: hir::ExprKind::Local(idx_slot),
                        ty: Ty::F64,
                        span,
                    }),
                    rhs: Box::new(num(position as f64)),
                },
                ty: Ty::Boolean,
                span,
            };
            out.push(hir::Stmt::If {
                cond,
                then_body: body,
                else_body: Vec::new(),
            });
        }
        cx.switch_depth -= 1;
        cx.scopes.pop();
        hir::Stmt::Labeled {
            label: "\0switch".to_string(),
            body: Box::new(hir::Stmt::Group(out)),
        }
    }
}
