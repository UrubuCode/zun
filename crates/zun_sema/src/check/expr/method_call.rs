use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_method_call(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::MethodCall { receiver, method, args, .. } = expr else { unreachable!() };
        // AST-phase interceptors (the `.call`/`.apply` prototype forms,
        // identifier-receiver namespace statics, and the static-regex fast
        // path), in their original order (see `mcall_statics.rs`).
        if let Some(e) = self.try_mcall_spread_dynamic(cx, receiver.as_ref(), method, args, span) { return e; }
        if let Some(e) = self.try_mcall_object_hasown(cx, receiver.as_ref(), method, args, span) { return e; }
        if let Some(e) = self.try_mcall_array_proto_call(cx, receiver.as_ref(), method, args, span) { return e; }
        if let Some(e) = self.try_mcall_proto_apply_tostring(cx, receiver.as_ref(), method, args, span) { return e; }
        if let Some(e) = self.try_mcall_math_apply(cx, receiver.as_ref(), method, args, span) { return e; }
        if let Some(e) = self.try_mcall_ident_receiver(cx, receiver.as_ref(), method, args, span) { return e; }
        if let Some(e) = self.try_mcall_static_regex(cx, receiver.as_ref(), method, args, span) { return e; }
        let mut receiver = self.check_expr(cx, receiver);
        let mname = self.text(method).to_string();
        // An integer-typed receiver (`s.codePointAt(0)!`) is a JS number:
        // widen it so the `Number.prototype` methods apply.
        if receiver.ty.is_int()
            && matches!(
                mname.as_str(),
                "toString" | "toFixed" | "toExponential" | "toPrecision" | "toLocaleString" | "valueOf"
            )
        {
            let receiver_span = receiver.span;
            receiver = hir::Expr {
                kind: hir::ExprKind::Convert { value: Box::new(receiver) },
                ty: Ty::F64,
                span: receiver_span,
            };
        }
        // `Object.prototype` instance reflectors over typed, dynamic, or
        // `any` receivers. Checked before the generic `any` member-call
        // path so they are not swallowed by dynamic dispatch.
        if (mname == "hasOwnProperty"
            || mname == "isPrototypeOf"
            || mname == "propertyIsEnumerable")
            && args.len() == 1
            && matches!(
                receiver.ty,
                Ty::DynamicObject | Ty::Any | Ty::Instance(_)
            )
        {
            let argument = self.check_expr(cx, &args[0]);
            let (name, ty) = if mname == "hasOwnProperty" {
                ("objectHasOwn", Ty::Boolean)
            } else if mname == "propertyIsEnumerable" {
                ("objectPropertyIsEnumerable", Ty::Boolean)
            } else {
                ("objectIsPrototypeOf", Ty::Boolean)
            };
            let Some((id, _)) = zun_natives::resolve("collections", name) else {
                self.error(span, format!("missing Object intrinsic `{name}`"));
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, argument] },
                ty,
                span,
            };
        }
        if receiver.ty == Ty::Array && mname == "pop" {
            if !args.is_empty() {
                self.error(span, "Array.pop expects no arguments");
            }
            return hir::Expr {
                kind: hir::ExprKind::ArrayPop {
                    object: Box::new(receiver),
                },
                ty: Ty::Any,
                span,
            };
        }
        // `push()` / `unshift()` with no values only report the length.
        if receiver.ty == Ty::Array && matches!(mname.as_str(), "push" | "unshift") && args.is_empty() {
            return hir::Expr {
                kind: hir::ExprKind::ArrayLength { object: Box::new(receiver) },
                ty: Ty::Int(IntKind::U64),
                span,
            };
        }
        // `unshift(a, b, c)` prepends each value from the last to the first so
        // the final order is `a, b, c`; the result is the final length.
        if receiver.ty == Ty::Array
            && mname == "unshift"
            && args.len() > 1
            && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
        {
            let Some((id, _)) = zun_natives::resolve("collections", "arrayUnshift") else {
                self.error(span, "missing Array intrinsic `arrayUnshift`");
                return self.poison(span);
            };
            let name = self.fresh_temp_name(cx);
            let slot = cx.declare(&name, Ty::Array);
            let local = |span| hir::Expr { kind: hir::ExprKind::Local(slot), ty: Ty::Array, span };
            let mut calls = Vec::with_capacity(args.len());
            for arg in args.iter().rev() {
                let value = self.check_expr(cx, arg);
                calls.push(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![local(span), value] },
                    ty: Ty::Int(IntKind::U64),
                    span,
                });
            }
            let mut result = calls.pop().expect("at least two unshifts");
            while let Some(effect) = calls.pop() {
                result = hir::Expr {
                    kind: hir::ExprKind::Sequence { effect: Box::new(effect), value: Box::new(result) },
                    ty: Ty::Int(IntKind::U64),
                    span,
                };
            }
            return hir::Expr {
                kind: hir::ExprKind::Sequence {
                    effect: Box::new(hir::Expr {
                        kind: hir::ExprKind::Assign { slot, value: Box::new(receiver) },
                        ty: Ty::Array,
                        span,
                    }),
                    value: Box::new(result),
                },
                ty: Ty::Int(IntKind::U64),
                span,
            };
        }
        // `slice(...args)`: a runtime argument list.
        if receiver.ty == Ty::Array
            && mname == "slice"
            && args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
        {
            let elements = args
                .iter()
                .map(|arg| match arg {
                    ast::Expr::Spread { expr, .. } => ast::ArrayElement::Spread((**expr).clone()),
                    other => ast::ArrayElement::Expr(other.clone()),
                })
                .collect();
            let literal = ast::Expr::ArrayLiteral { elements, span };
            let list = self.check_expr(cx, &literal);
            let Some((id, _)) = zun_natives::resolve("array", "arraySliceArgs") else {
                self.error(span, "missing Array intrinsic `arraySliceArgs`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, list] },
                ty: Ty::Array,
                span,
            };
        }
        // `fill(value, start?, end?)` and `copyWithin(target, start, end?)`.
        if receiver.ty == Ty::Array
            && matches!(mname.as_str(), "fill" | "copyWithin")
            && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
        {
            let (min, max, native) = if mname == "fill" { (1, 3, "arrayFill") } else { (1, 3, "arrayCopyWithin") };
            if args.len() < min || args.len() > max {
                self.error(span, format!("`{mname}` expects {min} to {max} argument(s), found {}", args.len()));
                return self.poison(span);
            }
            let Some((id, _)) = zun_natives::resolve("array", native) else {
                self.error(span, format!("missing Array intrinsic `{native}`"));
                return self.poison(span);
            };
            let mut call_args = vec![receiver];
            for arg in args.iter() {
                let value = self.check_expr(cx, arg);
                call_args.push(Self::box_to_any(value));
            }
            while call_args.len() < 4 {
                call_args.push(hir::Expr {
                    kind: hir::ExprKind::Unary {
                        op: hir::UnaryOp::Void,
                        operand: Box::new(hir::Expr { kind: hir::ExprKind::Null, ty: Ty::Any, span }),
                    },
                    ty: Ty::Any,
                    span,
                });
            }
            return hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: call_args }, ty: Ty::Array, span };
        }
        // Strings hold valid UTF-8 only, so they are always well formed.
        if receiver.ty == Ty::String && args.is_empty() && matches!(mname.as_str(), "isWellFormed" | "toWellFormed") {
            let Some((id, _)) = zun_natives::resolve("string", mname.as_str()) else {
                self.error(span, format!("missing String intrinsic `{mname}`"));
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver] },
                ty: if mname == "toWellFormed" { Ty::String } else { Ty::Boolean },
                span,
            };
        }
        // `join()` defaults the separator to a comma; `toString()` is `join()`.
        if receiver.ty == Ty::Array && matches!(mname.as_str(), "join" | "toString") && args.is_empty() {
            let Some((id, _)) = zun_natives::resolve("collections", "arrayJoin") else {
                self.error(span, "missing Array intrinsic `arrayJoin`");
                return self.poison(span);
            };
            let separator = hir::Expr { kind: hir::ExprKind::String(",".to_string()), ty: Ty::String, span };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, separator] },
                ty: Ty::String,
                span,
            };
        }
        if receiver.ty == Ty::Array && mname == "push" {
            if args.len() > 1 && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. })) {
                // `push(a, b, c)`: evaluate the receiver once into a temp and
                // push each value in order; the result is the final length.
                let name = self.fresh_temp_name(cx);
                let slot = cx.declare(&name, Ty::Array);
                let local = |span| hir::Expr { kind: hir::ExprKind::Local(slot), ty: Ty::Array, span };
                let mut pushes = Vec::with_capacity(args.len());
                for arg in args.iter() {
                    let value = self.check_expr(cx, arg);
                    pushes.push(hir::Expr {
                        kind: hir::ExprKind::ArrayPush { object: Box::new(local(span)), value: Box::new(value) },
                        ty: Ty::Int(IntKind::U64),
                        span,
                    });
                }
                let mut result = pushes.pop().expect("at least two pushes");
                while let Some(effect) = pushes.pop() {
                    result = hir::Expr {
                        kind: hir::ExprKind::Sequence { effect: Box::new(effect), value: Box::new(result) },
                        ty: Ty::Int(IntKind::U64),
                        span,
                    };
                }
                return hir::Expr {
                    kind: hir::ExprKind::Sequence {
                        effect: Box::new(hir::Expr {
                            kind: hir::ExprKind::Assign { slot, value: Box::new(receiver) },
                            ty: Ty::Array,
                            span,
                        }),
                        value: Box::new(result),
                    },
                    ty: Ty::Int(IntKind::U64),
                    span,
                };
            }
            if args.len() != 1 {
                self.error(span, "Array.push expects one argument");
                return self.poison(span);
            }
            let value = self.check_expr(cx, &args[0]);
            return hir::Expr {
                kind: hir::ExprKind::ArrayPush {
                    object: Box::new(receiver),
                    value: Box::new(value),
                },
                ty: Ty::Int(IntKind::U64),
                span,
            };
        }
        if receiver.ty == Ty::Array && mname == "splice" {
            // `splice(start, deleteCount, ...items)`. The inserted items
            // are gathered into a managed array (an array literal), so a
            // single fixed-arity native carries the variadic inserts.
            if args.len() < 2 || args.iter().any(|a| matches!(a, ast::Expr::Spread { .. })) {
                // A runtime argument list (`splice(start)`, `splice(...args)`).
                let elements = args
                    .iter()
                    .map(|arg| match arg {
                        ast::Expr::Spread { expr, .. } => ast::ArrayElement::Spread((**expr).clone()),
                        other => ast::ArrayElement::Expr(other.clone()),
                    })
                    .collect();
                let literal = ast::Expr::ArrayLiteral { elements, span };
                let list = self.check_expr(cx, &literal);
                let Some((id, _)) = zun_natives::resolve("array", "arraySpliceArgs") else {
                    self.error(span, "missing Array intrinsic `arraySpliceArgs`");
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, list] },
                    ty: Ty::Array,
                    span,
                };
            }
            let mut start = self.check_expr(cx, &args[0]);
            coerce_numeric_literal(&mut start, Ty::Int(IntKind::U64));
            let mut delete_count = self.check_expr(cx, &args[1]);
            coerce_numeric_literal(&mut delete_count, Ty::Int(IntKind::U64));
            let static_ints = (start.ty.is_error() || self.is_assignable(start.ty, Ty::Int(IntKind::U64)))
                && (delete_count.ty.is_error() || self.is_assignable(delete_count.ty, Ty::Int(IntKind::U64)));
            if !static_ints {
                // Non-integer (`number` / dynamic) bounds: the runtime argument
                // list applies the full ToIntegerOrInfinity clamping.
                let elements = args
                    .iter()
                    .map(|arg| ast::ArrayElement::Expr(arg.clone()))
                    .collect();
                let literal = ast::Expr::ArrayLiteral { elements, span };
                let list = self.check_expr(cx, &literal);
                let Some((id, _)) = zun_natives::resolve("array", "arraySpliceArgs") else {
                    self.error(span, "missing Array intrinsic `arraySpliceArgs`");
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, list] },
                    ty: Ty::Array,
                    span,
                };
            }
            let items: Vec<Option<hir::Expr>> =
                args[2..].iter().map(|item| Some(self.check_expr(cx, item))).collect();
            let items = hir::Expr {
                kind: hir::ExprKind::ArrayLiteral { elements: items },
                ty: Ty::Array,
                span,
            };
            let Some((id, _)) = zun_natives::resolve("array", "arraySpliceInsert") else {
                self.error(span, "missing Array intrinsic `arraySpliceInsert`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![receiver, start, delete_count, items],
                },
                ty: Ty::Array,
                span,
            };
        }
        if receiver.ty == Ty::Array && mname == "toReversed" {
            if !args.is_empty() {
                self.error(
                    span,
                    format!("`Array.toReversed` expects 0 arguments, found {}", args.len()),
                );
                return self.poison(span);
            }
            let Some((id, _)) = zun_natives::resolve("array", "arrayToReversed") else {
                self.error(span, "missing Array intrinsic `arrayToReversed`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver] },
                ty: Ty::Array,
                span,
            };
        }
        if receiver.ty == Ty::Array && mname == "toSorted" {
            // `toSorted(cmp?)`: a sorted copy, optionally by comparator
            // (default ascending string order). The receiver is untouched.
            if args.len() > 1 {
                self.error(
                    span,
                    format!("`Array.toSorted` accepts at most 1 argument, found {}", args.len()),
                );
                return self.poison(span);
            }
            let (native_name, call_args) = match args.first() {
                Some(arg) => {
                    let value = self.check_expr(cx, arg);
                    if !value.ty.is_error()
                        && !self.is_assignable(value.ty, Ty::Function(hir::DYNAMIC_FUNCTION))
                    {
                        self.error(value.span, "`Array.toSorted` comparator must be a function");
                    }
                    ("arrayToSortedBy", vec![receiver, value])
                }
                None => ("arrayToSorted", vec![receiver]),
            };
            let Some((id, _)) = zun_natives::resolve("array", native_name) else {
                self.error(span, format!("missing Array intrinsic `{native_name}`"));
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                ty: Ty::Array,
                span,
            };
        }
        if receiver.ty == Ty::Array && mname == "with" {
            // `with(index, value)`: a copy with `index` replaced; the
            // index is a (possibly negative) integer, the value any.
            if args.len() != 2 {
                self.error(
                    span,
                    format!("`Array.with` expects 2 arguments, found {}", args.len()),
                );
                return self.poison(span);
            }
            let mut index = self.check_expr(cx, &args[0]);
            coerce_numeric_literal(&mut index, Ty::Int(IntKind::I64));
            if !index.ty.is_error() && !self.is_assignable(index.ty, Ty::Int(IntKind::I64)) {
                self.error(index.span, "`Array.with` index must be an integer");
            }
            let value = self.check_expr(cx, &args[1]);
            let value = if value.ty == Ty::Any || value.ty.is_error() {
                value
            } else {
                let vspan = value.span;
                hir::Expr {
                    kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                    ty: Ty::Any,
                    span: vspan,
                }
            };
            let Some((id, _)) = zun_natives::resolve("array", "arrayWith") else {
                self.error(span, "missing Array intrinsic `arrayWith`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, index, value] },
                ty: Ty::Array,
                span,
            };
        }
        if receiver.ty == Ty::Array && mname == "toSpliced" {
            // `toSpliced(start, deleteCount, ...items)`: a copy with the
            // splice applied, mirroring `splice`'s index resolution but
            // leaving the receiver untouched.
            if args.len() < 2 || args.iter().any(|a| matches!(a, ast::Expr::Spread { .. })) {
                // A runtime argument list: splice a copy with JS clamping.
                let elements = args
                    .iter()
                    .map(|arg| match arg {
                        ast::Expr::Spread { expr, .. } => ast::ArrayElement::Spread((**expr).clone()),
                        other => ast::ArrayElement::Expr(other.clone()),
                    })
                    .collect();
                let literal = ast::Expr::ArrayLiteral { elements, span };
                let list = self.check_expr(cx, &literal);
                let (Some((slice_id, _)), Some((splice_id, _))) = (
                    zun_natives::resolve("array", "arraySlice"),
                    zun_natives::resolve("array", "arraySpliceArgs"),
                ) else {
                    self.error(span, "missing Array intrinsics for `toSpliced`");
                    return self.poison(span);
                };
                let name = self.fresh_temp_name(cx);
                let slot = cx.declare(&name, Ty::Array);
                let local = |span| hir::Expr { kind: hir::ExprKind::Local(slot), ty: Ty::Array, span };
                let zero = hir::Expr { kind: hir::ExprKind::Number(0.0), ty: Ty::Int(IntKind::U64), span };
                let copy = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id: slice_id, args: vec![receiver, zero] },
                    ty: Ty::Array,
                    span,
                };
                let splice = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id: splice_id, args: vec![local(span), list] },
                    ty: Ty::Array,
                    span,
                };
                return hir::Expr {
                    kind: hir::ExprKind::Sequence {
                        effect: Box::new(hir::Expr {
                            kind: hir::ExprKind::Assign { slot, value: Box::new(copy) },
                            ty: Ty::Array,
                            span,
                        }),
                        value: Box::new(hir::Expr {
                            kind: hir::ExprKind::Sequence { effect: Box::new(splice), value: Box::new(local(span)) },
                            ty: Ty::Array,
                            span,
                        }),
                    },
                    ty: Ty::Array,
                    span,
                };
            }
            let mut start = self.check_expr(cx, &args[0]);
            coerce_numeric_literal(&mut start, Ty::Int(IntKind::U64));
            if !start.ty.is_error() && !self.is_assignable(start.ty, Ty::Int(IntKind::U64)) {
                self.error(start.span, "`Array.toSpliced` start must be an integer");
            }
            let mut delete_count = self.check_expr(cx, &args[1]);
            coerce_numeric_literal(&mut delete_count, Ty::Int(IntKind::U64));
            if !delete_count.ty.is_error()
                && !self.is_assignable(delete_count.ty, Ty::Int(IntKind::U64))
            {
                self.error(delete_count.span, "`Array.toSpliced` deleteCount must be an integer");
            }
            let items: Vec<Option<hir::Expr>> =
                args[2..].iter().map(|item| Some(self.check_expr(cx, item))).collect();
            let items = hir::Expr {
                kind: hir::ExprKind::ArrayLiteral { elements: items },
                ty: Ty::Array,
                span,
            };
            let Some((id, _)) = zun_natives::resolve("array", "arrayToSpliced") else {
                self.error(span, "missing Array intrinsic `arrayToSpliced`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![receiver, start, delete_count, items],
                },
                ty: Ty::Array,
                span,
            };
        }
        if receiver.ty == Ty::Array && mname == "concat" {
            // `concat(...items)`: the variadic items are gathered into
            // a managed Array; the native spreads or appends each.
            let items: Vec<Option<hir::Expr>> =
                args.iter().map(|item| Some(self.check_expr(cx, item))).collect();
            let items = hir::Expr {
                kind: hir::ExprKind::ArrayLiteral { elements: items },
                ty: Ty::Array,
                span,
            };
            let Some((id, _)) = zun_natives::resolve("array", "arrayConcat") else {
                self.error(span, "missing Array intrinsic `arrayConcat`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, items] },
                ty: Ty::Array,
                span,
            };
        }
        if receiver.ty == Ty::Array && mname == "flat" {
            // `flat(depth=1)` — the depth defaults to 1 when omitted.
            if args.len() > 1 {
                self.error(
                    span,
                    format!("`Array.flat` accepts at most 1 argument, found {}", args.len()),
                );
                return self.poison(span);
            }
            let depth = match args.first() {
                Some(arg) => {
                    let mut value = self.check_expr(cx, arg);
                    coerce_numeric_literal(&mut value, Ty::F64);
                    if !value.ty.is_error() && !self.is_assignable(value.ty, Ty::F64) {
                        self.error(value.span, "`Array.flat` depth must be a number");
                    }
                    value
                }
                None => hir::Expr {
                    kind: hir::ExprKind::Number(1.0),
                    ty: Ty::F64,
                    span,
                },
            };
            let Some((id, _)) = zun_natives::resolve("array", "arrayFlat") else {
                self.error(span, "missing Array intrinsic `arrayFlat`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, depth] },
                ty: Ty::Array,
                span,
            };
        }
        if receiver.ty == Ty::Array
            && matches!(mname.as_str(), "sort" | "reduce" | "reduceRight")
        {
            // Variable-arity standard Array methods. `sort` takes an
            // optional comparator (default is ascending string order);
            // `reduce`/`reduceRight` require an initial accumulator on
            // this floor (the no-initial form is deferred). The callback
            // is a dynamic function value invoked by the native with the
            // ECMAScript argument shape.
            let (native_name, return_ty, expected): (&str, Ty, Vec<Ty>) =
                match (mname.as_str(), args.len()) {
                    ("sort", 0) => ("arraySortDefault", Ty::Array, vec![]),
                    ("sort", 1) => (
                        "arraySortBy",
                        Ty::Array,
                        vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
                    ),
                    ("reduce", 2) => (
                        "arrayReduce",
                        Ty::Any,
                        vec![Ty::Function(hir::DYNAMIC_FUNCTION), Ty::Any],
                    ),
                    ("reduceRight", 2) => (
                        "arrayReduceRight",
                        Ty::Any,
                        vec![Ty::Function(hir::DYNAMIC_FUNCTION), Ty::Any],
                    ),
                    // No-initial-value form: the first/last element seeds
                    // the accumulator; an empty array throws a TypeError.
                    ("reduce", 1) => (
                        "arrayReduceNoInit",
                        Ty::Any,
                        vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
                    ),
                    ("reduceRight", 1) => (
                        "arrayReduceRightNoInit",
                        Ty::Any,
                        vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
                    ),
                    (other, found) => {
                        self.error(
                            span,
                            format!(
                                "`Array.{other}` does not accept {found} argument(s)"
                            ),
                        );
                        return self.poison(span);
                    }
                };
            let mut call_args = vec![receiver];
            let undefined_ast = ast::Expr::Unary { op: ast::UnaryOp::Void, operand: Box::new(ast::Expr::Number { value: 0.0, span }), span };
            for (arg, expected_ty) in args.iter().zip(expected) {
                let mut value = self.check_expr(cx, arg);
                if matches!(value.ty, Ty::Function(_)) && self.ast_function_takes_this(arg) {
                    if let Some((bind_id, _)) = zun_natives::resolve("collections", "functionBind") {
                        let checked = self.check_expr(cx, &undefined_ast);
                        let bound_this = self.box_function_argument(checked);
                        value = hir::Expr {
                            span: value.span,
                            kind: hir::ExprKind::FunctionBind {
                                callee: Box::new(value),
                                args: vec![bound_this],
                                native_id: bind_id,
                            },
                            ty: Ty::Function(hir::DYNAMIC_FUNCTION),
                        };
                    }
                }
                coerce_numeric_literal(&mut value, expected_ty);
                if !value.ty.is_error() && !self.is_assignable(value.ty, expected_ty) {
                    self.error(
                        value.span,
                        format!(
                            "argument type {:?} does not match {expected_ty:?}",
                            value.ty
                        ),
                    );
                }
                call_args.push(value);
            }
            let Some((id, _)) = zun_natives::resolve("collections", native_name)
                .or_else(|| zun_natives::resolve("array", native_name))
            else {
                self.error(span, format!("missing collection intrinsic `{native_name}`"));
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                ty: return_ty,
                span,
            };
        }
        if receiver.ty == Ty::Promise
            && matches!(mname.as_str(), "then" | "catch" | "finally")
        {
            // `then`/`catch`/`finally` take optional function callbacks
            // and return a NEW Promise settled from the callback's result
            // (adopting a returned Promise). Absent callback slots lower
            // to `null`, which the floor reads as "no handler". The
            // callback re-enters the interpreter, so this stays Tier 0.
            let (native_name, max_callbacks) = match mname.as_str() {
                "then" => ("promiseThen", 2usize),
                "catch" => ("promiseCatch", 1usize),
                _ => ("promiseFinally", 1usize),
            };
            if args.len() > max_callbacks {
                self.error(
                    span,
                    format!(
                        "`Promise.{mname}` accepts at most {max_callbacks} argument(s), found {}",
                        args.len()
                    ),
                );
                return self.poison(span);
            }
            let mut call_args = vec![receiver];
            for index in 0..max_callbacks {
                match args.get(index) {
                    Some(arg) => {
                        let value = self.check_expr(cx, arg);
                        if !value.ty.is_error()
                            && !self.is_assignable(
                                value.ty,
                                Ty::Function(hir::DYNAMIC_FUNCTION),
                            )
                        {
                            self.error(
                                value.span,
                                format!("`Promise.{mname}` expects a function callback"),
                            );
                        }
                        call_args.push(value);
                    }
                    None => call_args.push(hir::Expr {
                        kind: hir::ExprKind::Null,
                        ty: Ty::Any,
                        span,
                    }),
                }
            }
            let Some((id, _)) = zun_natives::resolve("promise", native_name) else {
                self.error(span, format!("missing Promise intrinsic `{native_name}`"));
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                ty: Ty::Promise,
                span,
            };
        }
        if matches!(receiver.ty, Ty::Any | Ty::DynamicObject) && args.len() <= 6 {
            // Dynamic member call on an `any`/dynamic receiver (e.g. a
            // namespace from `import()`, or an object-literal method in
            // the function slot): dispatched by name at run time.
            let native_name = [
                "dynamicCall0",
                "dynamicCall1",
                "dynamicCall2",
                "dynamicCall3",
                "dynamicCall4",
                "dynamicCall5",
                "dynamicCall6",
            ][args.len()];
            let Some((id, _)) = zun_natives::resolve("collections", native_name) else {
                self.error(span, format!("missing collection intrinsic `{native_name}`"));
                return self.poison(span);
            };
            let mut call_args = vec![
                receiver,
                hir::Expr { kind: hir::ExprKind::String(mname.clone()), ty: Ty::String, span: method.span },
            ];
            for arg in args {
                call_args.push(self.check_expr(cx, arg));
            }
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                ty: Ty::Any,
                span,
            };
        }
        // `String.prototype.concat(...strs)` — variadic. Folded into a
        // chain of the two-argument string `concat` native; zero
        // arguments return the receiver (as `"x".concat() === "x"`).
        if receiver.ty == Ty::String
            && mname == "concat"
            && args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
        {
            if let Some(&func) = self.by_name.get("__zunStringConcatArgs") {
                let elements = args
                    .iter()
                    .map(|arg| match arg {
                        ast::Expr::Spread { expr, .. } => ast::ArrayElement::Spread((**expr).clone()),
                        other => ast::ArrayElement::Expr(other.clone()),
                    })
                    .collect();
                let list = self.check_expr(cx, &ast::Expr::ArrayLiteral { elements, span });
                return hir::Expr {
                    kind: hir::ExprKind::Call { func, args: vec![receiver, list] },
                    ty: Ty::String,
                    span,
                };
            }
        }
        if receiver.ty == Ty::String && mname == "concat" {
            let Some((id, _)) = zun_natives::resolve("string", "concat") else {
                self.error(span, "missing String intrinsic `concat`");
                return self.poison(span);
            };
            let mut acc = receiver;
            for arg in args {
                let mut value = self.check_expr(cx, arg);
                // Non-string arguments are converted with ToString.
                if !value.ty.is_error() && value.ty != Ty::String {
                    if let Some((coerce_id, _)) =
                        zun_natives::resolve("collections", "stringCoerceStrict")
                    {
                        let value_span = value.span;
                        value = hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id: coerce_id,
                                args: vec![Self::box_to_any(value)],
                            },
                            ty: Ty::String,
                            span: value_span,
                        };
                    }
                }
                if !value.ty.is_error() && !self.is_assignable(value.ty, Ty::String) {
                    self.error(value.span, "`concat` expects string arguments");
                }
                acc = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![acc, value] },
                    ty: Ty::String,
                    span,
                };
            }
            return acc;
        }
        // String methods called with JS-style arguments (optional,
        // `undefined`, fractional/negative, untyped): the statically
        // typed natives only accept exact integer/string arguments, so
        // anything else goes through the dynamic `stringMethod` native,
        // which applies full ToIntegerOrInfinity / ToString coercion.
        if receiver.ty == Ty::String
            && matches!(
                mname.as_str(),
                "substring" | "substr" | "slice" | "indexOf" | "lastIndexOf" | "includes"
                    | "startsWith" | "endsWith" | "padStart" | "padEnd" | "charAt" | "repeat"
                    | "charCodeAt" | "codePointAt" | "at"
            )
            && args.len() <= 2
        {
            let checked: Vec<hir::Expr> = args.iter().map(|a| self.check_expr(cx, a)).collect();
            let natural_literal = |e: &hir::Expr| {
                matches!(e.kind, hir::ExprKind::Number(v) if e.ty == Ty::F64 && v.fract() == 0.0 && v >= 0.0)
            };
            let static_ok = checked.iter().all(|e| !e.ty.is_error())
                && match mname.as_str() {
                    "substring" => {
                        checked.len() == 2
                            && checked.iter().all(|e| e.ty.is_int() || natural_literal(e))
                    }
                    "slice" => checked.iter().all(|e| e.ty == Ty::F64),
                    // A literal index may be out of range (NaN / undefined results),
                    // which the integer-typed native cannot express.
                    "charCodeAt" | "codePointAt" => checked.len() == 1 && checked[0].ty.is_int(),
                    "repeat" => {
                        checked.len() == 1 && (checked[0].ty.is_int() || natural_literal(&checked[0]))
                    }
                    "at" => checked.len() == 1 && checked[0].ty == Ty::F64,
                    "indexOf" | "lastIndexOf" | "includes" | "startsWith" | "endsWith" => {
                        checked.len() == 1 && checked[0].ty == Ty::String
                    }
                    // `padStart(target, fill)` with an integer target and
                    // a string fill keeps the statically typed (JIT-able)
                    // managed-string ABI; other shapes go dynamic.
                    "padStart" | "padEnd" => {
                        checked.len() == 2
                            && (checked[0].ty.is_int() || natural_literal(&checked[0]))
                            && checked[1].ty == Ty::String
                    }
                    _ => false,
                };
            if !static_ok && checked.iter().all(|e| !e.ty.is_error()) {
                let Some((id, _)) = zun_natives::resolve("collections", "stringMethod") else {
                    self.error(span, "missing collection intrinsic `stringMethod`");
                    return self.poison(span);
                };
                let undefined = |span| hir::Expr {
                    kind: hir::ExprKind::Unary {
                        op: ast::UnaryOp::Void,
                        operand: Box::new(hir::Expr {
                            kind: hir::ExprKind::Number(0.0),
                            ty: Ty::F64,
                            span,
                        }),
                    },
                    ty: Ty::Any,
                    span,
                };
                let mut call_args = vec![
                    receiver,
                    hir::Expr {
                        kind: hir::ExprKind::String(mname.clone()),
                        ty: Ty::String,
                        span: method.span,
                    },
                ];
                let mut it = checked.into_iter();
                for _ in 0..2 {
                    call_args.push(it.next().unwrap_or_else(|| undefined(span)));
                }
                let result_ty = match mname.as_str() {
                    "indexOf" | "lastIndexOf" | "charCodeAt" => Ty::F64,
                    // `undefined` past the end, so the result stays dynamic.
                    "codePointAt" => Ty::Any,
                    "includes" | "startsWith" | "endsWith" => Ty::Boolean,
                    _ => Ty::String,
                };
                let call = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                    ty: Ty::Any,
                    span,
                };
                if result_ty == Ty::Any {
                    return call;
                }
                return hir::Expr {
                    kind: hir::ExprKind::Convert { value: Box::new(call) },
                    ty: result_ty,
                    span,
                };
            }
        }
        // `String.prototype.slice(start?, end?)` — negative indices
        // count from the end (distinct from `substring`). An absent
        // argument defaults to the whole range; `end` is passed as a
        // value past any length so it clamps to the length natively.
        if receiver.ty == Ty::String && mname == "slice" {
            if args.len() > 2 {
                self.error(span, "`slice` expects at most 2 arguments");
                return self.poison(span);
            }
            let Some((id, _)) = zun_natives::resolve("string", "slice") else {
                self.error(span, "missing String intrinsic `slice`");
                return self.poison(span);
            };
            let mut bound = |checker: &mut Self, arg: Option<&ast::Expr>, default: f64| {
                match arg {
                    Some(arg) => {
                        let mut value = checker.check_expr(cx, arg);
                        coerce_numeric_literal(&mut value, Ty::F64);
                        if !value.ty.is_error() && !checker.is_assignable(value.ty, Ty::F64) {
                            checker.error(value.span, "`slice` index must be a number");
                        }
                        value
                    }
                    None => hir::Expr { kind: hir::ExprKind::Number(default), ty: Ty::F64, span },
                }
            };
            let start = bound(self, args.first(), 0.0);
            // 2^53: past any representable string length, so it clamps to
            // the length and means "to the end".
            let end = bound(self, args.get(1), 9_007_199_254_740_992.0);
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, start, end] },
                ty: Ty::String,
                span,
            };
        }
        // `String.prototype.split(separator, limit?)` — dispatch by the
        // separator's type: a RegExp stays on the regex engine, a string
        // uses the string-separator native (with an optional limit).
        if receiver.ty == Ty::String && mname == "split" {
            if args.len() > 2 {
                self.error(span, "`split` expects 1 or 2 arguments");
                return self.poison(span);
            }
            let first = match args.first() {
                Some(arg) => self.check_expr(cx, arg),
                // `split()` behaves as `split(undefined)`.
                None => self.check_expr(
                    cx,
                    &ast::Expr::Unary {
                        op: ast::UnaryOp::Void,
                        operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                        span,
                    },
                ),
            };
            if matches!(first.ty, Ty::Any | Ty::DynamicObject) {
                let limit = args.get(1).map(|arg| self.check_expr(cx, arg));
                return self.string_symbol_dispatch(span, 3.0, receiver, first, limit);
            }
            if first.ty == Ty::RegExp {
                if args.len() == 2 {
                    let mut limit = self.check_expr(cx, &args[1]);
                    coerce_numeric_literal(&mut limit, Ty::Int(IntKind::I64));
                    limit = self.split_limit_expr(limit);
                    let Some((id, _)) = zun_natives::resolve("collections", "regexpSplitLimit") else {
                        self.error(span, "missing collection intrinsic `regexpSplitLimit`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![first, receiver, limit] },
                        ty: Ty::Array,
                        span,
                    };
                }
                let Some((id, _)) = zun_natives::resolve("collections", "regexpSplit") else {
                    self.error(span, "missing collection intrinsic `regexpSplit`");
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![first, receiver] },
                    ty: Ty::Array,
                    span,
                };
            }
            if !first.ty.is_error() && !self.is_assignable(first.ty, Ty::String) {
                self.error(first.span, "`split` expects a string or RegExp separator");
                return self.poison(span);
            }
            let limit = match args.get(1) {
                Some(arg) => {
                    let mut value = self.check_expr(cx, arg);
                    coerce_numeric_literal(&mut value, Ty::Int(IntKind::I64));
                    value = self.split_limit_expr(value);
                    if !value.ty.is_error() && !self.is_assignable(value.ty, Ty::Int(IntKind::I64)) {
                        self.error(value.span, "`split` limit must be an integer");
                    }
                    value
                }
                // Absent limit → negative sentinel meaning unbounded.
                None => hir::Expr {
                    kind: hir::ExprKind::Number(-1.0),
                    ty: Ty::Int(IntKind::I64),
                    span,
                },
            };
            let Some((id, _)) = zun_natives::resolve("collections", "stringSplit") else {
                self.error(span, "missing collection intrinsic `stringSplit`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, first, limit] },
                ty: Ty::Array,
                span,
            };
        }
        // `String.prototype.replace(search, replacement)` — dispatch by
        // the search's type: a RegExp uses the regex engine, a string
        // replaces the first occurrence via the string native (so it no
        // longer needs the `String` class in scope).
        // `replaceAll(/re/g, replacement)`: a regex search replaces every
        // match through the same natives as `replace`.
        if receiver.ty == Ty::String
            && mname == "replaceAll"
            && args.len() == 2
        {
            let first = self.check_expr(cx, &args[0]);
            let replacement = self.check_expr(cx, &args[1]);
            if matches!(first.ty, Ty::Any | Ty::DynamicObject) {
                return self.string_symbol_dispatch(span, 5.0, receiver, first, Some(replacement));
            }
            // A dynamic (`any`) replacement is a function or ToString'd at run time.
            if replacement.ty == Ty::Any && matches!(first.ty, Ty::RegExp | Ty::String) {
                let first = Self::box_to_any(first);
                return self.string_symbol_dispatch(span, 5.0, receiver, first, Some(replacement));
            }
            // A RegExp search must carry the `g` flag (checked at run time).
            let first = if first.ty == Ty::RegExp {
                let Some((guard, _)) = zun_natives::resolve("collections", "regexpReplaceAllGuard") else {
                    self.error(span, "missing collection intrinsic `regexpReplaceAllGuard`");
                    return self.poison(span);
                };
                hir::Expr {
                    span: first.span,
                    kind: hir::ExprKind::CallBuiltin { id: guard, args: vec![first] },
                    ty: Ty::RegExp,
                }
            } else {
                first
            };
            if matches!(replacement.ty, Ty::Function(_)) {
                return self.replace_with_function(span, first, receiver, replacement, true);
            }
            let (first, replacement) = if first.ty != Ty::RegExp {
                let first = if first.ty == Ty::String { first } else { self.to_string_for_parse(first) };
                let replacement = if replacement.ty == Ty::String { replacement } else { self.to_string_for_parse(replacement) };
                (first, replacement)
            } else {
                (first, replacement)
            };
            if first.ty == Ty::String && replacement.ty == Ty::String {
                let Some((id, _)) = zun_natives::resolve("string", "replaceAll") else {
                    self.error(span, "missing String intrinsic `replaceAll`");
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, first, replacement] },
                    ty: Ty::String,
                    span,
                };
            }
            if replacement.ty != Ty::String && !replacement.ty.is_error() {
                self.error(replacement.span, "`replaceAll` replacement must be a string");
                return self.poison(span);
            }
            let Some((id, _)) = zun_natives::resolve("collections", "regexpReplace") else {
                self.error(span, "missing collection intrinsic `regexpReplace`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![first, receiver, replacement] },
                ty: Ty::String,
                span,
            };
        }
        if receiver.ty == Ty::String && mname == "replace" && args.len() == 2 {
            let first = self.check_expr(cx, &args[0]);
            let replacement = self.check_expr(cx, &args[1]);
            if matches!(first.ty, Ty::Any | Ty::DynamicObject) {
                return self.string_symbol_dispatch(span, 1.0, receiver, first, Some(replacement));
            }
            if matches!(replacement.ty, Ty::Function(_))
                && matches!(first.ty, Ty::RegExp | Ty::String)
            {
                return self.replace_with_function(span, first, receiver, replacement, false);
            }
            // A dynamic (`any`) replacement is a function or ToString'd at run time.
            if replacement.ty == Ty::Any && matches!(first.ty, Ty::RegExp | Ty::String) {
                let first = Self::box_to_any(first);
                return self.string_symbol_dispatch(span, 1.0, receiver, first, Some(replacement));
            }
            if replacement.ty != Ty::String && !replacement.ty.is_error() {
                self.error(
                    replacement.span,
                    "`replace` replacement must be a string",
                );
                return self.poison(span);
            }
            if first.ty == Ty::RegExp {
                let Some((id, _)) = zun_natives::resolve("collections", "regexpReplace") else {
                    self.error(span, "missing collection intrinsic `regexpReplace`");
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![first, receiver, replacement] },
                    ty: Ty::String,
                    span,
                };
            }
            if !first.ty.is_error() && !self.is_assignable(first.ty, Ty::String) {
                self.error(first.span, "`replace` expects a string or RegExp search");
                return self.poison(span);
            }
            let Some((id, _)) = zun_natives::resolve("string", "replace") else {
                self.error(span, "missing String intrinsic `replace`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, first, replacement] },
                ty: Ty::String,
                span,
            };
        }
        // String methods taking a RegExp: match/search.
        if receiver.ty == Ty::String
            && matches!(mname.as_str(), "match" | "search")
        {
            let expected_args = 1;
            if args.len() == expected_args {
                let first = self.check_expr(cx, &args[0]);
                if matches!(first.ty, Ty::Any | Ty::DynamicObject) {
                    let kind = if mname == "match" { 0.0 } else { 2.0 };
                    return self.string_symbol_dispatch(span, kind, receiver, first, None);
                }
                {
                    if first.ty != Ty::RegExp && !first.ty.is_error() {
                        self.error(
                            first.span,
                            format!("`{mname}` expects a RegExp argument"),
                        );
                        return self.poison(span);
                    }
                    let (native_name, return_ty) = match mname.as_str() {
                        "match" => ("regexpMatch", Ty::Array),
                        _ => ("regexpSearch", Ty::Int(IntKind::I32)),
                    };
                    let call_args = vec![first, receiver];
                    let Some((id, _)) =
                        zun_natives::resolve("collections", native_name)
                    else {
                        self.error(
                            span,
                            format!("missing collection intrinsic `{native_name}`"),
                        );
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                        ty: return_ty,
                        span,
                    };
                }
            }
        }
        // `length`/`byteLength`/`get`/`set` on a typed-array view come
        // from the master table in `zun_core`, so every kind the
        // language accepts has the same reachable method surface.
        // `FinalizationRegistry.register(target, heldValue, unregisterToken?)`
        // and `unregister(unregisterToken)`. Hand-lowered rather than
        // routed through the fixed-arity `managed_method` table because
        // `register`'s unregister token is optional: when omitted, a
        // `null` sentinel is synthesized so the registration carries no
        // token and can never be unregistered (per the ECMAScript spec).
        if receiver.ty == Ty::FinalizationRegistry
            && (mname == "register" || mname == "unregister")
        {
            let null_sentinel = |span| hir::Expr {
                kind: hir::ExprKind::Null,
                ty: Ty::Any,
                span,
            };
            if mname == "register" {
                if args.len() != 2 && args.len() != 3 {
                    self.error(
                        span,
                        format!("`register` expects 2 or 3 argument(s), found {}", args.len()),
                    );
                    return self.poison(span);
                }
                let target = self.check_expr(cx, &args[0]);
                let held = self.check_expr(cx, &args[1]);
                if held.ty.is_error() {
                    return self.poison(span);
                }
                // The held value is arbitrary: it is boxed into `Any`,
                // GC-traced while the registration is pending, and
                // delivered to the cleanup callback on collection.
                let held = if held.ty == Ty::Any {
                    held
                } else {
                    hir::Expr {
                        span: held.span,
                        kind: hir::ExprKind::BoxAny { value: Box::new(held) },
                        ty: Ty::Any,
                    }
                };
                let token = match args.get(2) {
                    Some(arg) => self.check_expr(cx, arg),
                    None => null_sentinel(span),
                };
                let Some((id, _)) =
                    zun_natives::resolve("collections", "finalizationRegister")
                else {
                    self.error(span, "missing intrinsic `finalizationRegister`");
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![receiver, target, held, token],
                    },
                    ty: Ty::Void,
                    span,
                };
            }
            // `unregister(unregisterToken)`.
            if args.len() != 1 {
                self.error(
                    span,
                    format!("`unregister` expects 1 argument, found {}", args.len()),
                );
                return self.poison(span);
            }
            let token = self.check_expr(cx, &args[0]);
            let Some((id, _)) =
                zun_natives::resolve("collections", "finalizationUnregister")
            else {
                self.error(span, "missing intrinsic `finalizationUnregister`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![receiver, token],
                },
                ty: Ty::Boolean,
                span,
            };
        }
        // Node `Buffer` instance surface. A Buffer is a `U8` byte view,
        // so these methods live on that kind; they take optional
        // arguments (defaults applied here) and route to the
        // `nodeBuffer` natives, which operate over the shared byte
        // floor. Placed before the generated typed-array dispatch so
        // `slice`/`subarray`/`fill` get the Buffer (optional-arg)
        // semantics rather than the typed-array ones.
        // A Buffer is a `U8` view, so it shares its type with a plain
        // `Uint8Array`. Only intercept the Buffer-unique method shapes;
        // `fill`/`subarray`/`get`/`set`/`length`/`byteLength` keep their
        // typed-array semantics (generic dispatch below). `fill(value)`
        // with a single argument is the one Node form added here (the
        // 3-arg typed-array `fill` still falls through).
        if receiver.ty == Ty::BufferView(hir::BufferViewKind::U8) {
            let intercept = match mname.as_str() {
                "toString" | "write" | "slice" | "equals" | "copy" => true,
                // Node Buffer search accepts number / string / Buffer
                // needles (richer than the numeric typed-array form).
                "indexOf" | "includes" | "lastIndexOf" => true,
                "swap16" | "swap32" | "swap64" => true,
                "fill" => args.len() == 1,
                other => other.starts_with("read") || other.starts_with("write"),
            };
            if intercept {
                return self.check_buffer_method(cx, receiver, &mname, args, span);
            }
        }
        // The second-generation typed-array surface (bulk `set`,
        // `slice`/`copyWithin`, the functional methods). These are
        // kind-agnostic natives that need optional-argument handling
        // and `set` arity/type overloading, so they are lowered by a
        // dedicated routine rather than the generic method table. Not
        // for `DataView`, and `slice` on a `U8` view is already taken
        // by the Node Buffer intercept above (Buffer semantics).
        if let Ty::BufferView(kind) = receiver.ty {
            if kind != hir::BufferViewKind::DataView
                && matches!(
                    mname.as_str(),
                    "set"
                        | "slice"
                        | "copyWithin"
                        | "indexOf"
                        | "includes"
                        | "join"
                        | "forEach"
                        | "map"
                        | "filter"
                        | "reduce"
                        | "reduceRight"
                        | "find"
                        | "findIndex"
                        | "some"
                        | "every"
                )
            {
                return self.check_typed_array_method(
                    cx,
                    receiver,
                    kind,
                    mname.as_str(),
                    args,
                    span,
                );
            }
        }
        // `DataView` element accessors, with an **optional** little-
        // endian flag that defaults to `false` (big-endian) as the
        // spec requires. Previously the flag was mandatory, so
        // `view.getUint32(0)` failed to type-check; the single-byte
        // accessors never take one.
        if receiver.ty == Ty::BufferView(hir::BufferViewKind::DataView)
            && data_view_accessor(mname.as_str())
        {
            return self.check_data_view_method(cx, receiver, mname.as_str(), args, span);
        }
        // ES2025 Set methods lower to the prelude `__zunSetOp*` helpers,
        // which follow the set-like protocol on the argument.
        if receiver.ty == Ty::Set
            && args.len() == 1
            && matches!(
                mname.as_str(),
                "union" | "intersection" | "difference" | "symmetricDifference"
                    | "isSubsetOf" | "isSupersetOf" | "isDisjointFrom"
            )
        {
            let predicate = mname.starts_with("is");
            let helper = if predicate { "__zunSetOpBool" } else { "__zunSetOpSet" };
            if let Some(&func) = self.by_name.get(helper) {
                let argument = self.check_expr(cx, &args[0]);
                let op = hir::Expr {
                    kind: hir::ExprKind::String(mname.clone()),
                    ty: Ty::String,
                    span,
                };
                return hir::Expr {
                    kind: hir::ExprKind::Call {
                        func,
                        args: vec![receiver, op, Self::box_to_any(argument)],
                    },
                    ty: if predicate { Ty::Boolean } else { Ty::Set },
                    span,
                };
            }
        }
        // `ArrayBuffer.prototype.transfer(newLength?)` and
        // `transferToFixedLength(newLength?)` (NUCLEUS #17). Hand-lowered
        // rather than routed through the fixed-arity `managed_method`
        // table because `newLength` is optional: when omitted, a `-1`
        // sentinel is synthesized meaning "keep the source length".
        // Both detach the source and yield a fresh `Buffer`.
        if receiver.ty == Ty::Buffer
            && (mname == "transfer" || mname == "transferToFixedLength")
        {
            if args.len() > 1 {
                self.error(
                    span,
                    format!("`{mname}` expects 0 or 1 argument(s), found {}", args.len()),
                );
                return self.poison(span);
            }
            let new_length = match args.first() {
                Some(arg) => {
                    let mut length = self.check_expr(cx, arg);
                    coerce_numeric_literal(&mut length, Ty::Int(IntKind::I64));
                    if !length.ty.is_error() && !length.ty.is_int() {
                        self.error(length.span, "transfer length must be an integer");
                    }
                    length
                }
                None => hir::Expr {
                    kind: hir::ExprKind::Number(-1.0),
                    ty: Ty::Int(IntKind::I64),
                    span,
                },
            };
            let native = if mname == "transfer" {
                "bufferTransfer"
            } else {
                "bufferTransferFixed"
            };
            let Some((id, _)) = zun_natives::resolve("buffer", native) else {
                self.error(span, format!("missing buffer intrinsic `{native}`"));
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, new_length] },
                ty: Ty::Buffer,
                span,
            };
        }
        // `String.prototype.normalize(form?)` (NUCLEUS #1). Hand-lowered
        // because `form` is optional: the ECMAScript default `"NFC"` is
        // synthesized when it is omitted. The ICU4X-backed
        // `stringNormalize` native validates the form at run time.
        let receiver = match self.mcall_string_normalize(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_managed(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_number_a(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_number_b(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_number_c(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_primitive_tail(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        self.mcall_instance_dispatch(cx, receiver, mname, method, args, span)
    }
}
