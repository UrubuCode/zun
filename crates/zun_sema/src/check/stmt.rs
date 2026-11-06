use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_stmt(&mut self, cx: &mut FunctionCx, stmt: &ast::Stmt) -> hir::Stmt {
        match stmt {
            ast::Stmt::Destructure { pattern, init, .. } => {
                let value = self.check_expr(cx, init);
                let mut out = Vec::new();
                self.bind_pattern(cx, pattern, value, &mut out);
                hir::Stmt::Group(out)
            }
            ast::Stmt::Let { name, ty, init, .. } => {
                if matches!(init, ast::Expr::Function(_)) && !cx.mutated_names.contains(self.text(name)) {
                    self.closure_self_name = Some(self.text(name).to_string());
                }
                // The adapter's injected `arguments = [p1, ..., ...rest]` (name and
                // literal share one span): trimmed to the real call-site count.
                let injected_arguments = self.text(name) == "arguments"
                    && matches!(init, ast::Expr::ArrayLiteral { span: literal, .. } if *literal == name.span);
                let mut init = self.check_expr(cx, init);
                self.closure_self_name = None;
                if injected_arguments && init.ty == Ty::Array {
                    if let Some((trim_id, _)) = zun_natives::resolve("collections", "argumentsObject") {
                        let (span_of, ty) = (init.span, init.ty);
                        init = hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id: trim_id, args: vec![init] },
                            ty,
                            span: span_of,
                        };
                    }
                }
                let declared = ty.as_ref().map(|t| self.resolve_annotation(t));
                // `let x: int = 1` — retype an integral number literal to the
                // declared numeric type.
                if let Some(d) = declared {
                    coerce_numeric_literal(&mut init, d);
                }
                let var_ty = match declared {
                    Some(declared) => {
                        if !declared.is_error()
                            && !init.ty.is_error()
                            && !self.is_assignable(init.ty, declared)
                        {
                            self.error(
                                init.span,
                                format!(
                                    "initializer type {:?} does not match declared type {declared:?}",
                                    init.ty
                                ),
                            );
                        }
                        init = Self::unbox_to(init, declared);
                        declared
                    }
                    // An unannotated reassigned variable initialised from an integer-typed
                    // value (`let hi = arr.length - 1`) is a JS `number`: widen to f64 so
                    // later float assignments type-check.
                    None if init.ty.is_int() && cx.mutated_names.contains(self.text(name)) => {
                        let span = init.span;
                        init = hir::Expr { kind: hir::ExprKind::Convert { value: Box::new(init) }, ty: Ty::F64, span };
                        Ty::F64
                    }
                    None => init.ty,
                };
                let name_text = self.text(name).to_string();
                // A function-valued variable that is reassigned somewhere holds
                // an arbitrary function: calls go through the dynamic path.
                let reassigned_fn = declared.is_none()
                    && matches!(var_ty, Ty::Function(_))
                    && cx.mutated_names.contains(&name_text)
                    && cx.mutated_names.contains(&format!("{name_text}"));
                let boxed_fn = reassigned_fn
                    && cx.mutated_names.contains(&name_text)
                    && cx.mutated_names.contains(&format!("{name_text}"));
                if reassigned_fn {
                    self.reassigned_fn_names.insert(name_text.clone());
                }
                let var_ty = if reassigned_fn { Ty::Function(hir::DYNAMIC_FUNCTION) } else { var_ty };
                let slot = cx.declare(&name_text, var_ty);
                // A variable that a closure may capture and that is assigned
                // somewhere is boxed from its declaration, so every access
                // lowered before the capturing closure already uses the cell.
                if cx.mutated_names.contains(&name_text)
                    && cx.mutated_names.contains(&format!("\u{1}{name_text}"))
                    && (boxed_fn || !matches!(var_ty, Ty::Function(_)))
                {
                    cx.boxed.insert(slot);
                }
                if cx.boxed.contains(&slot) {
                    hir::Stmt::BoxedLet { slot, init }
                } else {
                    hir::Stmt::Let { slot, init }
                }
            }
            ast::Stmt::Expr(e) => hir::Stmt::Expr(self.check_expr(cx, e)),
            ast::Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
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
                            kind: hir::ExprKind::Convert { value: Box::new(read) },
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
            ast::Stmt::While { cond, body, .. } => {
                let cond = self.check_condition(cx, cond);
                cx.loop_depth += 1;
                let body = self.check_block(cx, body);
                cx.loop_depth -= 1;
                hir::Stmt::While { cond, body }
            }
            ast::Stmt::DoWhile { body, cond, .. } => {
                cx.loop_depth += 1;
                let body = self.check_block(cx, body);
                cx.loop_depth -= 1;
                let cond = self.check_condition(cx, cond);
                hir::Stmt::DoWhile { body, cond }
            }
            ast::Stmt::For {
                init,
                extra_init,
                cond,
                update,
                body,
                ..
            } => {
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
            ast::Stmt::ForOf {
                pattern,
                iterable,
                body,
                span: _,
            } => {
                let iterable_expr = self.check_expr(cx, iterable);
                let (new_name, done_name, next_name) = match iterable_expr.ty {
                    Ty::Array => ("arrayIteratorNew", "arrayIteratorDone", "arrayIteratorNext"),
                    Ty::Map => ("mapIteratorNew", "mapIteratorDone", "mapIteratorNext"),
                    Ty::Set => ("setIteratorNew", "setIteratorDone", "setIteratorNext"),
                    // A string iterates by code point; reuses the array
                    // iterator's `done`/`next` over a code-point array.
                    Ty::String => ("stringIteratorNew", "arrayIteratorDone", "arrayIteratorNext"),
                    Ty::Any => ("generatorIteratorNew", "generatorIteratorDone", "generatorIteratorNext"),
                    // A user object implementing `[Symbol.iterator]`: a dynamic
                    // object literal's symbol-function slot, or a typed class's
                    // computed-symbol method. Both route through the generic
                    // iteration protocol, which calls `[Symbol.iterator]()` and
                    // drives the returned iterator's `next()`.
                    Ty::DynamicObject => {
                        ("generatorIteratorNew", "generatorIteratorDone", "generatorIteratorNext")
                    }
                    Ty::Instance(idx) if self.class_has_iterator(idx) => {
                        ("generatorIteratorNew", "generatorIteratorDone", "generatorIteratorNext")
                    }
                    Ty::Error => ("arrayIteratorNew", "arrayIteratorDone", "arrayIteratorNext"),
                    // A typed array iterates its elements; it builds an element
                    // sequence and reuses the array iterator's `done`/`next`,
                    // as a string iterator does. `DataView` is not iterable.
                    Ty::BufferView(kind) => match crate::typed_arrays::iterator_new(kind) {
                        Some(new_name) => (new_name, "arrayIteratorDone", "arrayIteratorNext"),
                        None => {
                            self.error(
                                iterable_expr.span,
                                "a DataView is not iterable".to_string(),
                            );
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
            ast::Stmt::ForIn { pattern, object, body, .. } => {
                // `for (k in obj)` iterates the object's enumerable keys: build
                // `objectKeys(obj)` (an Array) and run the array-iterator loop.
                let object_expr = self.check_expr(cx, object);
                let span = object_expr.span;
                let keys_arg = if object_expr.ty == Ty::Any {
                    object_expr
                } else {
                    hir::Expr {
                        span,
                        kind: hir::ExprKind::BoxAny { value: Box::new(object_expr) },
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
                let subject_read = hir::Expr { kind: hir::ExprKind::Local(subject), ty: Ty::Any, span };
                let keys = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id: keys_id, args: vec![hir::Expr { kind: hir::ExprKind::Local(subject), ty: Ty::Any, span }] },
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
                    hir::Stmt::ForOf { iterator, iterator_slot, done, next, binding, body } => {
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
                            kind: hir::ExprKind::CallBuiltin { id: has_id, args: vec![subject_read, key_arg] },
                            ty: Ty::Boolean,
                            span,
                        };
                        hir::Stmt::Group(vec![
                            hir::Stmt::Let { slot: subject, init: keys_arg },
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
            ast::Stmt::Labeled { label, statement, .. } => {
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
            ast::Stmt::Switch { discriminant, cases, span } => {
                self.check_switch(cx, discriminant, cases, *span)
            }
            ast::Stmt::Break { label, span } => {
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
            ast::Stmt::Continue { label, span } => {
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
            ast::Stmt::Return { value, span } => {
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
                            operand: Box::new(ast::Expr::Number { value: 0.0, span: *span }),
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
                    (Some(v), expected) => {
                        if !expected.is_error()
                            && !v.ty.is_error()
                            && !self.is_assignable(v.ty, expected)
                        {
                            self.error(
                                v.span,
                                format!("return type {:?} does not match {expected:?}", v.ty),
                            );
                        }
                    }
                    _ => {}
                }
                // Inside a `try` with a `finally`, the return value is computed
                // first and held in a temporary while the cleanup runs.
                if cx.finally_depth > 0 {
                    if let Some(v) = value {
                        if !matches!(v.kind, hir::ExprKind::Number(_) | hir::ExprKind::String(_) | hir::ExprKind::Boolean(_) | hir::ExprKind::Null) && !v.ty.is_error() {
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
            ast::Stmt::Throw { value, .. } => hir::Stmt::Throw(self.check_expr(cx, value)),
            ast::Stmt::Try {
                body,
                catches,
                finally,
                ..
            } => {
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
                let finally_body = finally.as_ref().map(|block| {
                    self.check_block(cx, block)
                });
                hir::Stmt::Try {
                    body,
                    catches,
                    finally_body,
                }
            }
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
            kind: hir::ExprKind::CallBuiltin { id: new_id, args: vec![iterable_expr] },
            ty: Ty::Any,
            span,
        };
        let iterator_slot = cx.declare("__for_of_iterator", Ty::Any);
        let done = hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id: done_id,
                args: vec![hir::Expr { kind: hir::ExprKind::Local(iterator_slot), ty: Ty::Any, span }],
            },
            ty: Ty::Boolean,
            span,
        };
        let next = hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id: next_id,
                args: vec![hir::Expr { kind: hir::ExprKind::Local(iterator_slot), ty: Ty::Any, span }],
            },
            ty: Ty::Any,
            span,
        };
        cx.scopes.push(Scope::new());
        let (binding, prelude) = match pattern {
            ast::Pattern::Ident { name, ty } => {
                let declared = ty.as_ref().map(|t| self.resolve_annotation(t)).unwrap_or(Ty::Any);
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
                let element_expr = hir::Expr { kind: hir::ExprKind::Local(element), ty: Ty::Any, span };
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
        hir::Stmt::ForOf { iterator, iterator_slot, done, next, binding, body }
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
        let num = |value: f64| hir::Expr { kind: hir::ExprKind::Number(value), ty: Ty::F64, span };
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
                    lhs: Box::new(hir::Expr { kind: hir::ExprKind::Local(disc_slot), ty: Ty::Any, span }),
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
            hir::Stmt::Let { slot: disc_slot, init: disc },
            hir::Stmt::Let { slot: idx_slot, init: index_expr },
        ];
        cx.switch_depth += 1;
        for (position, case) in cases.iter().enumerate() {
            if case.body.statements.is_empty() {
                continue;
            }
            // All case bodies share the switch's one lexical scope, so a `let`
            // declared in one case is visible to the cases after it.
            let body: Vec<hir::Stmt> =
                case.body.statements.iter().map(|statement| self.check_stmt(cx, statement)).collect();
            let cond = hir::Expr {
                kind: hir::ExprKind::Binary {
                    op: ast::BinaryOp::Le,
                    lhs: Box::new(hir::Expr { kind: hir::ExprKind::Local(idx_slot), ty: Ty::F64, span }),
                    rhs: Box::new(num(position as f64)),
                },
                ty: Ty::Boolean,
                span,
            };
            out.push(hir::Stmt::If { cond, then_body: body, else_body: Vec::new() });
        }
        cx.switch_depth -= 1;
        cx.scopes.pop();
        hir::Stmt::Labeled {
            label: "\0switch".to_string(),
            body: Box::new(hir::Stmt::Group(out)),
        }
    }
}
