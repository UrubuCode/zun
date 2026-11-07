use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_for_stmt(
        &mut self,
        cx: &mut FunctionCx,
        stmt: &ast::Stmt,
    ) -> hir::Stmt {
        let ast::Stmt::For {
            init,
            extra_init,
            cond,
            update,
            body,
            ..
        } = stmt
        else {
            unreachable!()
        };
        // The init clause's `let` is scoped to the loop.
        cx.scopes.push(Scope::new());
        let init = init.as_ref().map(|stmt| {
            let first = self.check_stmt(cx, stmt);
            if extra_init.is_empty() {
                Box::new(first)
            } else {
                let mut group = vec![first];
                for extra in extra_init {
                    group.push(self.check_stmt(cx, extra));
                }
                Box::new(hir::Stmt::Group(group))
            }
        });
        let cond = cond
            .as_ref()
            .map(|expr| self.check_condition(cx, expr))
            .unwrap_or(hir::Expr {
                kind: hir::ExprKind::Boolean(true),
                ty: Ty::Boolean,
                span: body.span,
            });
        let update = update.as_ref().map(|expr| self.check_expr(cx, expr));
        cx.loop_depth += 1;
        let body = self.check_block(cx, body);
        cx.loop_depth -= 1;
        cx.scopes.pop();
        hir::Stmt::For {
            init,
            cond,
            update,
            body,
        }
    }

    pub(in crate::check) fn check_for_of_stmt(
        &mut self,
        cx: &mut FunctionCx,
        stmt: &ast::Stmt,
    ) -> hir::Stmt {
        let ast::Stmt::ForOf {
            pattern,
            iterable,
            body,
            span: _,
        } = stmt
        else {
            unreachable!()
        };
        let iterable_expr = self.check_expr(cx, iterable);
        let (new_name, done_name, next_name) = match iterable_expr.ty {
            Ty::Array => ("arrayIteratorNew", "arrayIteratorDone", "arrayIteratorNext"),
            Ty::Map => ("mapIteratorNew", "mapIteratorDone", "mapIteratorNext"),
            Ty::Set => ("setIteratorNew", "setIteratorDone", "setIteratorNext"),
            // A string iterates by code point; reuses the array
            // iterator's `done`/`next` over a code-point array.
            Ty::String => (
                "stringIteratorNew",
                "arrayIteratorDone",
                "arrayIteratorNext",
            ),
            Ty::Any => (
                "generatorIteratorNew",
                "generatorIteratorDone",
                "generatorIteratorNext",
            ),
            // A user object implementing `[Symbol.iterator]`: a dynamic
            // object literal's symbol-function slot, or a typed class's
            // computed-symbol method. Both route through the generic
            // iteration protocol, which calls `[Symbol.iterator]()` and
            // drives the returned iterator's `next()`.
            Ty::DynamicObject => (
                "generatorIteratorNew",
                "generatorIteratorDone",
                "generatorIteratorNext",
            ),
            Ty::Instance(idx) if self.class_has_iterator(idx) => (
                "generatorIteratorNew",
                "generatorIteratorDone",
                "generatorIteratorNext",
            ),
            Ty::Error => ("arrayIteratorNew", "arrayIteratorDone", "arrayIteratorNext"),
            // A typed array iterates its elements; it builds an element
            // sequence and reuses the array iterator's `done`/`next`,
            // as a string iterator does. `DataView` is not iterable.
            Ty::BufferView(kind) => match crate::typed_arrays::iterator_new(kind) {
                Some(new_name) => (new_name, "arrayIteratorDone", "arrayIteratorNext"),
                None => {
                    self.error(iterable_expr.span, "a DataView is not iterable".to_string());
                    ("arrayIteratorNew", "arrayIteratorDone", "arrayIteratorNext")
                }
            },
            other => {
                self.error(
                    iterable_expr.span,
                    format!("type {other:?} is not iterable"),
                );
                ("arrayIteratorNew", "arrayIteratorDone", "arrayIteratorNext")
            }
        };
        self.lower_for_iteration(
            cx,
            iterable_expr,
            new_name,
            done_name,
            next_name,
            pattern,
            body,
        )
    }

    pub(in crate::check) fn check_for_in_stmt(
        &mut self,
        cx: &mut FunctionCx,
        stmt: &ast::Stmt,
    ) -> hir::Stmt {
        let ast::Stmt::ForIn {
            pattern,
            object,
            body,
            ..
        } = stmt
        else {
            unreachable!()
        };
        // `for (k in obj)` iterates the object's enumerable keys: build
        // `objectKeys(obj)` (an Array) and run the array-iterator loop.
        let object_expr = self.check_expr(cx, object);
        let span = object_expr.span;
        let keys_arg = if object_expr.ty == Ty::Any {
            object_expr
        } else {
            hir::Expr {
                span,
                kind: hir::ExprKind::BoxAny {
                    value: Box::new(object_expr),
                },
                ty: Ty::Any,
            }
        };
        let Some((keys_id, _)) = zun_natives::resolve("collections", "objectForInKeys") else {
            self.error(span, "missing `objectKeys` intrinsic");
            return hir::Stmt::Expr(self.poison(span));
        };
        let Some((has_id, _)) = zun_natives::resolve("collections", "forInHas") else {
            self.error(span, "missing `forInHas` intrinsic");
            return hir::Stmt::Expr(self.poison(span));
        };
        // The enumerated object is evaluated once; every key is
        // re-checked against it before its iteration runs (for-in is live:
        // a key deleted earlier in the loop is skipped).
        let subject = cx.declare(&self.fresh_temp_name(cx), Ty::Any);
        let subject_read = hir::Expr {
            kind: hir::ExprKind::Local(subject),
            ty: Ty::Any,
            span,
        };
        let keys = hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id: keys_id,
                args: vec![hir::Expr {
                    kind: hir::ExprKind::Local(subject),
                    ty: Ty::Any,
                    span,
                }],
            },
            ty: Ty::Array,
            span,
        };
        let lowered = self.lower_for_iteration(
            cx,
            keys,
            "arrayIteratorNew",
            "arrayIteratorDone",
            "arrayIteratorNext",
            pattern,
            body,
        );
        match lowered {
            hir::Stmt::ForOf {
                iterator,
                iterator_slot,
                done,
                next,
                binding,
                body,
            } => {
                let key_read = hir::Expr {
                    kind: hir::ExprKind::Local(binding),
                    ty: cx.locals[binding as usize],
                    span,
                };
                let key_arg = if key_read.ty == Ty::Any || key_read.ty == Ty::String {
                    key_read
                } else {
                    Self::box_to_any(key_read)
                };
                let still_there = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: has_id,
                        args: vec![subject_read, key_arg],
                    },
                    ty: Ty::Boolean,
                    span,
                };
                hir::Stmt::Group(vec![
                    hir::Stmt::Let {
                        slot: subject,
                        init: keys_arg,
                    },
                    hir::Stmt::ForOf {
                        iterator,
                        iterator_slot,
                        done,
                        next,
                        binding,
                        body: vec![hir::Stmt::If {
                            cond: still_there,
                            then_body: body,
                            else_body: Vec::new(),
                        }],
                    },
                ])
            }
            other => other,
        }
    }
}
