use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_call(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::Call { callee, args, .. } = expr else { unreachable!() };
        let name = self.text(callee).to_string();
        if name == "super" {
            return self.check_super_call(cx, None, args, span);
        }
        // The Error family is callable without `new` (`TypeError("x")`).
        if matches!(
            name.as_str(),
            "Error" | "EvalError" | "RangeError" | "ReferenceError" | "SyntaxError"
                | "TypeError" | "URIError" | "AggregateError"
        ) && self.classes_by_name.contains_key(&name)
            && !self.by_name.contains_key(&name)
            && cx.lookup(&name).is_none()
        {
            return self.check_expr(
                cx,
                &ast::Expr::New { class: *callee, args: args.clone(), span },
            );
        }
        // `Array(n)` behaves exactly like `new Array(n)`.
        if name == "Array"
            && !self.by_name.contains_key(&name)
            && cx.lookup(&name).is_none()
        {
            return self.check_expr(
                cx,
                &ast::Expr::New { class: *callee, args: args.clone(), span },
            );
        }
        // Internal: the value of the `generator.return(v)` request being
        // unwound (used by the `yield*` desugaring to forward it).
        if name == "__generatorReturnValue" && args.is_empty() {
            if let Some((id, _)) = zun_natives::resolve("collections", "generatorReturnValue") {
                return hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: Vec::new() }, ty: Ty::Any, span };
            }
        }
        // Global `parseInt`/`parseFloat` are the same as their `Number.*`
        // statics; route to that implementation when no user binding
        // shadows the name.
        if matches!(name.as_str(), "parseInt" | "parseFloat")
            && !self.by_name.contains_key(&name)
        {
            return self.check_number_static(cx, &name, callee.span, args, span);
        }
        // `Object(x)`: primitives become their wrapper objects, anything
        // already an object is returned unchanged.
        if name == "Object"
            && !self.by_name.contains_key(&name)
            && cx.lookup(&name).is_none()
            && args.len() == 1
        {
            let arg = self.check_expr(cx, &args[0]);
            let wrapper_name = match arg.ty {
                Ty::F64 => Some("NumberObject"),
                Ty::Boolean => Some("BooleanObject"),
                _ => None,
            };
            if let Some(wrapper) =
                wrapper_name.and_then(|name| self.classes_by_name.get(name).copied())
            {
                return hir::Expr {
                    kind: hir::ExprKind::New { class: wrapper, args: vec![arg] },
                    ty: Ty::Instance(wrapper),
                    span,
                };
            }
            return arg;
        }
        // Global `isNaN`/`isFinite` coerce with ToNumber first (unlike
        // the strict `Number.*` predicates).
        if matches!(name.as_str(), "isNaN" | "isFinite")
            && !self.by_name.contains_key(&name)
            && cx.lookup(&name).is_none()
        {
            if args.len() != 1 {
                self.error(span, format!("`{name}` expects 1 argument, found {}", args.len()));
                return self.poison(span);
            }
            let Some((id, _)) = zun_natives::resolve("number", &name) else {
                self.error(span, format!("missing Number intrinsic `{name}`"));
                return self.poison(span);
            };
            let mut arg = self.check_expr(cx, &args[0]);
            coerce_numeric_literal(&mut arg, Ty::F64);
            let arg = self.to_number_for_arg(arg);
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                ty: Ty::Boolean,
                span,
            };
        }
        if matches!(name.as_str(), "encodeURIComponent" | "decodeURIComponent")
            && !self.by_name.contains_key(&name)
            && cx.lookup(&name).is_none()
            && args.len() == 1
        {
            let Some((id, _)) = zun_natives::resolve("string", &name) else {
                self.error(span, format!("missing string intrinsic `{name}`"));
                return self.poison(span);
            };
            let arg = self.check_expr(cx, &args[0]);
            let arg = if arg.ty == Ty::String { arg } else { self.to_string_for_parse(arg) };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                ty: Ty::String,
                span,
            };
        }
        // `Function(p1, ..., body)` behaves exactly like `new Function(...)`.
        if name == "Function"
            && !self.by_name.contains_key(&name)
            && !self.classes_by_name.contains_key(&name)
        {
            return self.lower_function_construct(cx, args, span);
        }
        if let Some(ast::Expr::Spread { expr, .. }) = args.last() {
            if let Some(&func) = self.by_name.get(&name) {
                let rest_param = self.signatures[func as usize].rest_param;
                if rest_param == Some(args.len() - 1) {
                    let mut fixed_args = Vec::with_capacity(args.len() - 1);
                    for argument in &args[..args.len() - 1] {
                        fixed_args.push(self.check_expr(cx, argument));
                    }
                    let array = self.check_expr(cx, expr);
                    let array = self.spread_source_to_array(array);
                    if array.ty != Ty::Array && !array.ty.is_error() {
                        self.error(array.span, "spread call requires an Array value");
                        return self.poison(span);
                    }
                    let params = self.signatures[func as usize].params.clone();
                    for (argument, &expected) in fixed_args.iter_mut().zip(&params) {
                        coerce_numeric_literal(argument, expected);
                        if !expected.is_error()
                            && !argument.ty.is_error()
                            && !self.is_assignable(argument.ty, expected)
                        {
                            self.error(
                                argument.span,
                                format!("argument type {:?} does not match {expected:?}", argument.ty),
                            );
                        }
                    }
                    let return_ty = self.signatures[func as usize].return_ty;
                    return hir::Expr {
                        kind: if fixed_args.is_empty() {
                            hir::ExprKind::CallRestArray {
                                func,
                                args: Box::new(array),
                            }
                        } else {
                            hir::ExprKind::CallRestArrayFixed {
                                func,
                                fixed_args,
                                args: Box::new(array),
                            }
                        },
                        ty: return_ty,
                        span,
                    };
                }
            }
        }
        if args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
            && (self.by_name.contains_key(&name) || cx.lookup(&name).is_some() || cx.captures.contains_key(&name))
        {
            // A named function whose only parameter is the rest array
            // takes the flattened argument array directly.
            if let Some(&func) = self.by_name.get(&name) {
                if cx.lookup(&name).is_none()
                    && self.signatures[func as usize].rest_param == Some(0)
                {
                    let elements = args
                        .iter()
                        .map(|arg| match arg {
                            ast::Expr::Spread { expr, .. } => {
                                ast::ArrayElement::Spread((**expr).clone())
                            }
                            other => ast::ArrayElement::Expr(other.clone()),
                        })
                        .collect();
                    let literal = ast::Expr::ArrayLiteral { elements, span };
                    let array = self.check_expr(cx, &literal);
                    let return_ty = self.signatures[func as usize].return_ty;
                    return hir::Expr {
                        kind: hir::ExprKind::CallRestArray { func, args: Box::new(array) },
                        ty: return_ty,
                        span,
                    };
                }
            }
            // `f(a, ...xs)` on a named function with fixed parameters
            // followed by a rest parameter: flatten every argument into
            // one array, then split it into the fixed prefix and a
            // fresh rest array.
            if let Some(&func) = self.by_name.get(&name) {
                let rest_index = self.signatures[func as usize].rest_param;
                if let (true, Some(rest_index)) = (cx.lookup(&name).is_none(), rest_index) {
                    if rest_index > 0 && self.signatures[func as usize].params.len() == rest_index + 1 {
                        let elements = args
                            .iter()
                            .map(|arg| match arg {
                                ast::Expr::Spread { expr, .. } => {
                                    ast::ArrayElement::Spread((**expr).clone())
                                }
                                other => ast::ArrayElement::Expr(other.clone()),
                            })
                            .collect();
                        let literal = ast::Expr::ArrayLiteral { elements, span };
                        let flat = self.check_expr(cx, &literal);
                        if flat.ty == Ty::Array {
                            let temp = cx.declare(&self.fresh_temp_name(cx), Ty::Array);
                            let params = self.signatures[func as usize].params.clone();
                            let mut fixed_args = Vec::with_capacity(rest_index);
                            for (index, &expected) in params.iter().take(rest_index).enumerate() {
                                let _ = expected;
                                let read = self.array_element_read(temp, Ty::Array, index, span);
                                fixed_args.push(read);
                            }
                            let rest = self.array_rest_read(temp, Ty::Array, rest_index, span);
                            let return_ty = self.signatures[func as usize].return_ty;
                            return hir::Expr {
                                kind: hir::ExprKind::Sequence {
                                    effect: Box::new(hir::Expr {
                                        kind: hir::ExprKind::Assign { slot: temp, value: Box::new(flat) },
                                        ty: Ty::Array,
                                        span,
                                    }),
                                    value: Box::new(hir::Expr {
                                        kind: hir::ExprKind::CallRestArrayFixed {
                                            func,
                                            fixed_args,
                                            args: Box::new(rest),
                                        },
                                        ty: return_ty,
                                        span,
                                    }),
                                },
                                ty: return_ty,
                                span,
                            };
                        }
                    }
                }
            }
            let callee_expr = ast::Expr::Ident(*callee);
            if let Some(call) = self.check_spread_call(cx, &callee_expr, args, span) {
                return call;
            }
        }
        let mut args: Vec<hir::Expr> =
            args.iter().map(|a| self.check_expr(cx, a)).collect();

        // `BigInt(x)` coercion when called as a plain function (JS
        // BigInt is not constructed with `new`). A string argument
        // parses (decimal or `0x`/`0o`/`0b`); a number must be
        // integer-valued (non-integers are a runtime RangeError).
        if name == "BigInt" && !self.by_name.contains_key(&name) {
            if args.len() != 1 {
                self.error(span, "`BigInt` expects one argument");
                return self.poison(span);
            }
            let arg = args.into_iter().next().expect("one argument");
            let native_name = if arg.ty.is_int() || arg.ty.is_float() {
                "bigIntFromNumber"
            } else if arg.ty == Ty::String {
                "bigIntNew"
            } else if arg.ty.is_error() {
                return self.poison(span);
            } else {
                self.error(arg.span, "`BigInt` expects a string or number argument");
                return self.poison(span);
            };
            let Some((id, _)) = zun_natives::resolve("collections", native_name) else {
                self.error(span, format!("missing collection intrinsic `{native_name}`"));
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                ty: Ty::BigInt,
                span,
            };
        }

        // `Boolean(x)` coercion when called as a plain function:
        // returns the JS truthiness of `x` as a `boolean` primitive
        // (not a wrapper object). The `truthy` native mirrors the
        // interpreter's `is_truthy`, so coercion matches `if`/`&&`.
        if name == "Boolean" && !self.by_name.contains_key(&name) {
            if args.len() != 1 {
                self.error(span, "`Boolean` expects one argument");
                return self.poison(span);
            }
            let arg = args.into_iter().next().expect("one argument");
            if arg.ty.is_error() {
                return self.poison(span);
            }
            // A value already typed `boolean` is its own truthiness.
            if arg.ty == Ty::Boolean {
                return arg;
            }
            let Some((id, _)) = zun_natives::resolve("boolean", "booleanCoerce") else {
                self.error(span, "missing Boolean intrinsic `booleanCoerce`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                ty: Ty::Boolean,
                span,
            };
        }

        // `String(x)` coercion when called as a plain function (JS
        // `String` is a coercion, not a constructor, called bare): the
        // ECMAScript ToString of `x` as a `string` primitive. A value
        // already typed `string` is returned unchanged; every other
        // operand is boxed into `Any` and run through the `stringCoerce`
        // native, which performs object→primitive coercion
        // (`[Symbol.toPrimitive]` / `[Symbol.toStringTag]`) at run time.
        if name == "String" && !self.by_name.contains_key(&name) {
            if args.len() != 1 {
                self.error(span, "`String` expects one argument");
                return self.poison(span);
            }
            let arg = args.into_iter().next().expect("one argument");
            if arg.ty.is_error() {
                return self.poison(span);
            }
            // A value already typed `string` is its own ToString.
            if arg.ty == Ty::String {
                return arg;
            }
            let Some((id, _)) = zun_natives::resolve("collections", "stringCoerce") else {
                self.error(span, "missing `stringCoerce` intrinsic");
                return self.poison(span);
            };
            // The native reads its operand as `any`, so box a non-`any`
            // operand (primitive or object) into an `Any` carrier.
            let arg = if arg.ty == Ty::Any {
                arg
            } else {
                hir::Expr {
                    span: arg.span,
                    kind: hir::ExprKind::BoxAny { value: Box::new(arg) },
                    ty: Ty::Any,
                }
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                ty: Ty::String,
                span,
            };
        }

        // `Number(x)` coercion when called as a plain function (JS
        // `Number` as a coercion, not a constructor): the ECMAScript
        // ToNumber of `x` as a `number` primitive. A value already typed
        // numeric is returned unchanged; every other operand is boxed
        // into `Any` and run through the `numberCoerce` native, which
        // performs string parsing and object→primitive coercion
        // (`[Symbol.toPrimitive]("number")`) at run time.
        if name == "Number" && !self.by_name.contains_key(&name) {
            // `Number()` with no argument is `+0`.
            if args.is_empty() {
                return hir::Expr { kind: hir::ExprKind::Number(0.0), ty: Ty::F64, span };
            }
            if args.len() != 1 {
                self.error(span, "`Number` expects one argument");
                return self.poison(span);
            }
            let arg = args.into_iter().next().expect("one argument");
            if arg.ty.is_error() {
                return self.poison(span);
            }
            // A value already typed `number`/`float` is its own ToNumber.
            if arg.ty == Ty::F64 {
                return arg;
            }
            let Some((id, _)) = zun_natives::resolve("collections", "numberCoerce") else {
                self.error(span, "missing `numberCoerce` intrinsic");
                return self.poison(span);
            };
            // The native reads its operand as `any`, so box a non-`any`
            // operand (primitive or object) into an `Any` carrier.
            let arg = if arg.ty == Ty::Any {
                arg
            } else {
                hir::Expr {
                    span: arg.span,
                    kind: hir::ExprKind::BoxAny { value: Box::new(arg) },
                    ty: Ty::Any,
                }
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                ty: Ty::F64,
                span,
            };
        }

        if let Some(&func) = self.by_name.get(&name) {
            if self.signatures[func as usize].is_generator {
                let Some((native_id, _)) = zun_natives::resolve("buffer", "generatorIteratorNewArgs") else {
                    self.error(span, "missing generator iterator intrinsic");
                    return self.poison(span);
                };
                // A rest parameter collects the trailing arguments into one array.
                let mut args = args;
                if let Some(rest_index) = self.signatures[func as usize].rest_param {
                    let param_tys = self.signatures[func as usize].params.clone();
                    while args.len() < rest_index {
                        let expected = param_tys.get(args.len()).copied().unwrap_or(Ty::Any);
                        args.push(default_args::missing_arg(expected, span));
                    }
                    let rest: Vec<Option<hir::Expr>> =
                        args.drain(rest_index..).map(Some).collect();
                    let mut packed = hir::Expr {
                        kind: hir::ExprKind::ArrayLiteral { elements: rest },
                        ty: Ty::Array,
                        span,
                    };
                    self.box_function_array_elements(&mut packed);
                    args.push(packed);
                }
                return hir::Expr {
                    kind: hir::ExprKind::GeneratorCall { func, args, native_id },
                    ty: Ty::Any,
                    span,
                };
            }
        }

        // A local shadows a capture of the same name. Inside a closure a captured
        // callable is read from the environment instead of a local slot.
        if let Some(slot) = cx.lookup(&name) {
            if cx.locals[slot as usize] == Ty::Function(hir::DYNAMIC_FUNCTION)
                && self.reads_through_global(cx, &name, slot)
            {
                let callee = self.global_get_expr(&name, span);
                return hir::Expr {
                    kind: hir::ExprKind::IndirectCall {
                        callee: Box::new(callee),
                        args,
                        func: hir::DYNAMIC_FUNCTION,
                    },
                    ty: Ty::Any,
                    span,
                };
            }
        }
        let callee_source: Option<(hir::ExprKind, Ty)> = match cx.lookup(&name) {
            Some(slot) if cx.boxed.contains(&slot) => {
                Some((hir::ExprKind::CellLoad { slot }, cx.locals[slot as usize]))
            }
            Some(slot) => Some((hir::ExprKind::Local(slot), cx.locals[slot as usize])),
            None => cx.captures.get(&name).map(|capture| {
                (
                    hir::ExprKind::CaptureLoad {
                        environment_slot: capture.environment_slot,
                        ty: capture.ty,
                        mode: capture.mode,
                    },
                    capture.ty,
                )
            }),
        };
        if let Some((callee_proto, function_ty)) = callee_source {
            let callee_kind = || match &callee_proto {
                hir::ExprKind::CaptureLoad { environment_slot, ty, mode } => {
                    hir::ExprKind::CaptureLoad {
                        environment_slot: *environment_slot,
                        ty: *ty,
                        mode: *mode,
                    }
                }
                hir::ExprKind::Local(slot) => hir::ExprKind::Local(*slot),
                hir::ExprKind::CellLoad { slot } => hir::ExprKind::CellLoad { slot: *slot },
                _ => unreachable!(),
            };

            // Calling an `any`-typed local performs a dynamic dispatch:
            // the value must be callable at run time (e.g. a function
            // read from an object's function slot via destructuring, as
            // `const { resolve } = Promise.withResolvers()` produces).
            // Lowered like a generic `Function` callback (single arg).
            if function_ty == Ty::Any {
                // A dynamic callee may be invoked with any argument count
                // (JS); the counted indirect-call form carries the arity.
                let callee_expr = hir::Expr {
                    kind: callee_kind(),
                    ty: function_ty,
                    span,
                };
                return hir::Expr {
                    kind: hir::ExprKind::IndirectCall {
                        callee: Box::new(callee_expr),
                        args,
                        func: hir::DYNAMIC_FUNCTION,
                    },
                    ty: Ty::Any,
                    span,
                };
            }
            // Calling a proxy value: `proxy(...args)` routes through the
            // handler's `apply` trap (NUCLEUS #10). The arguments are
            // packed into an array (each boxed to `Any` by `StoreArray`)
            // and dispatched to `proxyApply` with an `undefined` `this`.
            if function_ty == Ty::Proxy {
                let callee_expr = hir::Expr {
                    kind: callee_kind(),
                    ty: function_ty,
                    span,
                };
                let args_array = hir::Expr {
                    kind: hir::ExprKind::ArrayLiteral {
                        elements: args.into_iter().map(Some).collect(),
                    },
                    ty: Ty::Array,
                    span,
                };
                let this_arg = hir::Expr { kind: hir::ExprKind::Null, ty: Ty::Any, span };
                let Some((id, _)) = zun_natives::resolve("collections", "proxyApply") else {
                    self.error(span, "missing `proxyApply` intrinsic");
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![callee_expr, this_arg, args_array],
                    },
                    ty: Ty::Any,
                    span,
                };
            }
            if let Ty::Function(func) = function_ty {
                let return_ty = if func == hir::DYNAMIC_FUNCTION {
                    // Any arity: a bound function (`f.bind(..)`) forwards N call
                    // arguments (lowered to `call.indirect.n` when N != 1).
                    Ty::Any
                } else if func == hir::DYNAMIC_PREDICATE {
                    if args.len() != 1 {
                        self.error(span, format!("generic Predicate callback expects exactly one argument, found {}", args.len()));
                    }
                    Ty::Boolean
                } else if func == hir::DYNAMIC_REDUCER {
                    if args.len() != 2 {
                        self.error(span, format!("generic Reducer callback expects exactly two arguments, found {}", args.len()));
                    }
                    Ty::Any
                } else {
                    let (param_tys, return_ty, optional_count, signature_name) = {
                        let sig = &self.signatures[func as usize];
                        (
                            sig.params.clone(),
                            sig.return_ty,
                            sig.optional_count,
                            sig.name.clone(),
                        )
                    };
                    let sig = Signature {
                        name: signature_name,
                        params: param_tys.clone(),
                        return_ty,
                        optional_count,
                        rest_param: self.signatures[func as usize].rest_param,
                        is_generator: self.signatures[func as usize].is_generator,
                        is_static: false,
                    };
                    if let Some(rest_index) = sig.rest_param {
                        if args.len() < rest_index {
                            self.error(
                                span,
                                format!(
                                    "function value expects at least {} argument(s), found {}",
                                    rest_index,
                                    args.len()
                                ),
                            );
                        }
                        for (arg, &expected) in args.iter_mut().zip(&param_tys).take(rest_index) {
                            coerce_numeric_literal(arg, expected);
                            if !expected.is_error()
                                && !arg.ty.is_error()
                                && !self.is_assignable(arg.ty, expected)
                            {
                                self.error(
                                    arg.span,
                                    format!("argument type {:?} does not match {expected:?}", arg.ty),
                                );
                            }
                        }
                        // Function values packed into the rest array ride in functionBox records.
                        let mut args = args;
                        for index in rest_index..args.len() {
                            if matches!(args[index].ty, Ty::Function(_)) {
                                let element = std::mem::replace(&mut args[index], self.poison(span));
                                args[index] = self.box_function_argument(element);
                            }
                        }
                        return hir::Expr {
                            kind: hir::ExprKind::IndirectCallRest {
                                callee: Box::new(hir::Expr {
                                    kind: callee_kind(),
                                    ty: function_ty,
                                    span,
                                }),
                                args,
                                func,
                            },
                            ty: return_ty,
                            span,
                        };
                    }
                    // JS allows calling with fewer arguments: missing ones read
                    // as `undefined`.
                    let sig = Signature { optional_count: param_tys.len(), ..sig };
                    if args.len() > param_tys.len()
                    {
                        self.error(
                            span,
                            format!(
                                "function value expects {} to {} argument(s), found {}",
                                param_tys.len().saturating_sub(optional_count),
                                param_tys.len(),
                                args.len()
                            ),
                        );
                    } else {
                        self.pad_default_args(&mut args, &sig, span);
                        for (arg, &expected) in args.iter_mut().zip(&param_tys) {
                            coerce_numeric_literal(arg, expected);
                            if !expected.is_error()
                                && !arg.ty.is_error()
                                && !self.is_assignable(arg.ty, expected)
                            {
                                self.error(
                                    arg.span,
                                    format!(
                                        "argument type {:?} does not match {expected:?}",
                                        arg.ty
                                    ),
                                );
                            }
                        }
                    }
                    return_ty
                };
                let callee_expr = hir::Expr {
                    kind: callee_kind(),
                    ty: function_ty,
                    span,
                };
                return hir::Expr {
                    kind: hir::ExprKind::IndirectCall {
                        callee: Box::new(callee_expr),
                        args,
                        func,
                    },
                    ty: return_ty,
                    span,
                };
            }
        }

        // `print` is the one implicit intrinsic: one argument of any
        // primitive type, returns void.
        if name == "print" {
            if args.len() != 1 {
                self.error(span, "print takes exactly one argument");
            }
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: zun_natives::PRINT,
                    args,
                },
                ty: Ty::Void,
                span,
            };
        }

        // The `timers` macrotask surface (ASYNC slice 4): bare global
        // functions on the VM's unified event loop. Resolved here like
        // `print`, but only when the name is not a user symbol (a local
        // or top-level function of the same name shadows it, handled
        // above). Each lowers to the interpreter's timer floor.
        if matches!(
            name.as_str(),
            "setTimeout"
                | "setInterval"
                | "clearTimeout"
                | "clearInterval"
                | "queueMicrotask"
                | "setImmediate"
        ) && !self.by_name.contains_key(&name)
            && cx.lookup(&name).is_none()
        {
            let Some((id, _)) = zun_natives::resolve("timers", &name) else {
                self.error(span, format!("missing timer intrinsic `{name}`"));
                return self.poison(span);
            };
            let number = |span| hir::Expr {
                kind: hir::ExprKind::Number(0.0),
                ty: Ty::F64,
                span,
            };
            let is_callback = |arg: &hir::Expr| {
                matches!(arg.ty, Ty::Function(_)) || arg.ty.is_error()
            };
            let (built_args, ret) = match name.as_str() {
                "setTimeout" | "setInterval" => {
                    if args.is_empty() {
                        self.error(span, format!("`{name}` expects a callback and an optional delay"));
                        return self.poison(span);
                    }
                    let mut it = args.into_iter();
                    let cb = it.next().expect("callback");
                    if !is_callback(&cb) {
                        self.error(cb.span, format!("`{name}` expects a function callback"));
                    }
                    let mut ms = it.next().unwrap_or_else(|| number(span));
                    // Accept any numeric delay; a non-number is an error.
                    if !ms.ty.is_int() && !ms.ty.is_float() && !ms.ty.is_error() {
                        self.error(ms.span, format!("`{name}` delay must be a number"));
                    }
                    coerce_numeric_literal(&mut ms, Ty::F64);
                    let extra: Vec<hir::Expr> = it.collect();
                    if extra.is_empty() {
                        (vec![cb, ms], Ty::F64)
                    } else {
                        // Extra arguments are forwarded to the callback: pass them
                        // as a boxed array to the `*Args` variant.
                        let args_name = format!("{name}Args");
                        let Some((args_id, _)) = zun_natives::resolve("timers", &args_name) else {
                            self.error(span, format!("missing timer intrinsic `{args_name}`"));
                            return self.poison(span);
                        };
                        let elements = extra
                            .into_iter()
                            .map(|value| Some(Self::box_to_any(value)))
                            .collect();
                        let array = hir::Expr {
                            kind: hir::ExprKind::ArrayLiteral { elements },
                            ty: Ty::Array,
                            span,
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id: args_id, args: vec![cb, ms, array] },
                            ty: Ty::F64,
                            span,
                        };
                    }
                }
                "setImmediate" | "queueMicrotask" => {
                    if args.len() != 1 {
                        self.error(span, format!("`{name}` expects a single callback"));
                        return self.poison(span);
                    }
                    let cb = args.into_iter().next().expect("callback");
                    if !is_callback(&cb) {
                        self.error(cb.span, format!("`{name}` expects a function callback"));
                    }
                    let ret = if name == "setImmediate" { Ty::F64 } else { Ty::Void };
                    (vec![cb], ret)
                }
                // clearTimeout / clearInterval
                _ => {
                    if args.len() != 1 {
                        self.error(span, format!("`{name}` expects a timer handle"));
                        return self.poison(span);
                    }
                    let mut handle = args.into_iter().next().expect("handle");
                    if !handle.ty.is_int() && !handle.ty.is_float() && !handle.ty.is_error() {
                        self.error(handle.span, format!("`{name}` expects a numeric handle"));
                    }
                    coerce_numeric_literal(&mut handle, Ty::F64);
                    (vec![handle], Ty::Void)
                }
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: built_args },
                ty: ret,
                span,
            };
        }

        if matches!(
            name.as_str(),
            "decoratorAddInstanceInitializer" | "decoratorAccessGet" | "decoratorAccessSet"
        ) && !self.by_name.contains_key(&name)
        {
            // Prelude plumbing for `DecoratorContext` / `DecoratorAccess`
            // (builtin/decorators.zun): checked against the native signature.
            let Some((id, native)) = zun_natives::resolve("collections", &name) else {
                return self.poison(span);
            };
            if args.len() != native.params.len() {
                self.error(span, format!("`{name}` has an invalid argument list"));
                return self.poison(span);
            }
            for (arg, &param) in args.iter_mut().zip(native.params) {
                let expected = native_ty(param);
                coerce_numeric_literal(arg, expected);
                if !arg.ty.is_error()
                    && !(matches!(expected, Ty::Function(_)) && matches!(arg.ty, Ty::Function(_)))
                    && !self.is_assignable(arg.ty, expected)
                {
                    self.error(arg.span, format!("`{name}` argument does not match {expected:?}"));
                }
            }
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args },
                ty: native_ty(native.ret),
                span,
            };
        }

        if name == "decoratorAddInitializer" || name == "decoratorRunInitializers" {
            // Prelude plumbing for the typed `DecoratorContext`
            // (builtin/decorators.zun).
            let expected = usize::from(name == "decoratorAddInitializer");
            if args.len() != expected
                || (expected == 1 && !matches!(args[0].ty, Ty::Function(_)))
            {
                self.error(span, format!("`{name}` has an invalid argument list"));
                return self.poison(span);
            }
            let Some((id, native)) = zun_natives::resolve("collections", &name) else {
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args },
                ty: native_ty(native.ret),
                span,
            };
        }

        if name == "dynamicSetFunction" {
            if args.len() != 3 {
                self.error(span, "dynamicSetFunction expects an object, name, and function");
                return self.poison(span);
            }
            let Some((id, native)) = zun_natives::resolve("collections", &name) else {
                return self.poison(span);
            };
            if args[0].ty != Ty::DynamicObject
                || args[1].ty != Ty::String
                || !matches!(args[2].ty, Ty::Function(_))
            {
                self.error(span, "dynamicSetFunction expects (dynamic, string, function)");
                return self.poison(span);
            }
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args },
                ty: native_ty(native.ret),
                span,
            };
        }

        if name == "dynamicSetSymbolFunction" {
            if args.len() != 3 {
                self.error(span, "dynamicSetSymbolFunction expects an object, symbol, and function");
                return self.poison(span);
            }
            let Some((id, native)) = zun_natives::resolve("collections", &name) else {
                return self.poison(span);
            };
            if args[0].ty != Ty::DynamicObject
                || args[1].ty != Ty::Symbol
                || !matches!(args[2].ty, Ty::Function(_))
            {
                self.error(span, "dynamicSetSymbolFunction expects (dynamic, symbol, function)");
                return self.poison(span);
            }
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args },
                ty: native_ty(native.ret),
                span,
            };
        }

        if name == "dynamic_new" {
            if !args.is_empty() {
                self.error(span, "dynamic_new takes no arguments");
            }
            return hir::Expr {
                kind: hir::ExprKind::DynamicNew,
                ty: Ty::DynamicObject,
                span,
            };
        }

        if name == "box_any" {
            if args.len() != 1 {
                self.error(span, "box_any takes exactly one argument");
                return self.poison(span);
            }
            let value = args.pop().unwrap();
            return hir::Expr {
                kind: hir::ExprKind::BoxAny {
                    value: Box::new(value),
                },
                ty: Ty::Any,
                span,
            };
        }

        // `Symbol(description?)` mints a fresh, unique symbol. The
        // description is optional; when present it must be a string.
        if name == "Symbol" && !self.by_name.contains_key("Symbol") {
            if args.len() > 1 {
                self.error(span, "`Symbol` takes an optional description argument");
                return self.poison(span);
            }
            let Some((id, _)) = zun_natives::resolve("collections", "symbolNew") else {
                self.error(span, "missing `symbolNew` intrinsic");
                return self.poison(span);
            };
            let description = match args.pop() {
                Some(mut value) => {
                    if value.ty != Ty::String && value.ty != Ty::Any && !value.ty.is_error() {
                        // Box non-string descriptions to `undefined` at
                        // runtime by passing them through as `Any`.
                        value = hir::Expr {
                            span: value.span,
                            kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                            ty: Ty::Any,
                        };
                    }
                    value
                }
                None => hir::Expr { kind: hir::ExprKind::Null, ty: Ty::Any, span },
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![description] },
                ty: Ty::Symbol,
                span,
            };
        }

        let dynamic_property = |checker: &mut Checker<'_>,
                                args: &mut Vec<hir::Expr>,
                                span: Span| {
            if args.len() != 2
                || args[0].ty != Ty::DynamicObject
                || args[1].ty != Ty::String
            {
                checker.error(span, "dynamic property access expects (dynamic, string)");
                return None;
            }
            let name = match &args[1].kind {
                hir::ExprKind::String(name) => name.clone(),
                _ => {
                    checker.error(
                        args[1].span,
                        "dynamic property name must be a string literal",
                    );
                    return None;
                }
            };
            Some((args.remove(0), name))
        };

        if name == "dynamic_get" || name == "reflect_get" {
            if name == "reflect_get"
                && args.len() == 2
                && args[0].ty == Ty::DynamicObject
                && args[1].ty == Ty::String
                && !matches!(args[1].kind, hir::ExprKind::String(_))
            {
                let key = args.pop().unwrap();
                let object = args.pop().unwrap();
                return hir::Expr {
                    kind: hir::ExprKind::DynamicGetValue {
                        object: Box::new(object),
                        key: Box::new(key),
                    },
                    ty: Ty::Any,
                    span,
                };
            }
            let Some((object, property)) = dynamic_property(self, &mut args, span) else {
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::DynamicGet {
                    object: Box::new(object),
                    name: property,
                },
                ty: Ty::Any,
                span,
            };
        }

        if name == "dynamic_set" || name == "reflect_set" {
            if name == "reflect_set"
                && args.len() == 3
                && args[0].ty == Ty::DynamicObject
                && args[1].ty == Ty::String
                && args[2].ty == Ty::Any
                && !matches!(args[1].kind, hir::ExprKind::String(_))
            {
                let value = args.pop().unwrap();
                let key = args.pop().unwrap();
                let object = args.pop().unwrap();
                return hir::Expr {
                    kind: hir::ExprKind::DynamicSetValue {
                        object: Box::new(object),
                        key: Box::new(key),
                        value: Box::new(value),
                    },
                    ty: Ty::Any,
                    span,
                };
            }
            if args.len() != 3
                || args[0].ty != Ty::DynamicObject
                || args[1].ty != Ty::String
                || args[2].ty != Ty::Any
            {
                self.error(
                    span,
                    "dynamic property assignment expects (dynamic, string, any)",
                );
                return self.poison(span);
            }
            let property = match &args[1].kind {
                hir::ExprKind::String(name) => name.clone(),
                _ => {
                    self.error(
                        args[1].span,
                        "dynamic property name must be a string literal",
                    );
                    return self.poison(span);
                }
            };
            let object = args.remove(0);
            args.remove(0);
            let value = args.remove(0);
            return hir::Expr {
                kind: hir::ExprKind::DynamicSet {
                    object: Box::new(object),
                    name: property,
                    value: Box::new(value),
                },
                ty: Ty::Any,
                span,
            };
        }

        // An imported intrinsic: check against its native signature.
        if let Some((intrinsic_id, intrinsic)) = self.imports.get(&name).copied() {
            let param_tys: Vec<Ty> =
                intrinsic.params.iter().map(|&t| native_ty(t)).collect();
            if args.len() != param_tys.len() {
                self.error(
                    span,
                    format!(
                        "`{name}` expects {} argument(s), found {}",
                        param_tys.len(),
                        args.len()
                    ),
                );
            } else {
                for (arg, &expected) in args.iter_mut().zip(&param_tys) {
                    coerce_numeric_literal(arg, expected);
                    if !expected.is_error()
                        && !arg.ty.is_error()
                        && !self.is_assignable(arg.ty, expected)
                    {
                        self.error(
                            arg.span,
                            format!(
                                "argument type {:?} does not match {expected:?}",
                                arg.ty
                            ),
                        );
                    }
                }
            }
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: intrinsic_id,
                    args,
                },
                ty: native_ty(intrinsic.ret),
                span,
            };
        }

        if let Some(external) = self.external_functions.get(&name).cloned() {
            if args.len() != external.params.len() {
                self.error(
                    span,
                    format!(
                        "`{name}` expects {} argument(s), found {}",
                        external.params.len(),
                        args.len()
                    ),
                );
            } else {
                for (arg, &expected) in args.iter_mut().zip(&external.params) {
                    coerce_numeric_literal(arg, expected);
                    if !expected.is_error()
                        && !arg.ty.is_error()
                        && !self.is_assignable(arg.ty, expected)
                    {
                        self.error(
                            arg.span,
                            format!(
                                "argument type {:?} does not match {expected:?}",
                                arg.ty
                            ),
                        );
                    }
                }
            }
            return hir::Expr {
                kind: hir::ExprKind::ExternalCall {
                    module: external.module,
                    name: external.name,
                    args,
                },
                ty: external.return_ty,
                span,
            };
        }

        if self.module_globals.contains(&name) {
            // A module-level binding called from another function:
            // read it from the realm store and call it dynamically.
            let callee_value = self.global_get_expr(&name, callee.span);
            return hir::Expr {
                kind: hir::ExprKind::IndirectCall {
                    callee: Box::new(callee_value),
                    args,
                    func: hir::DYNAMIC_FUNCTION,
                },
                ty: Ty::Any,
                span,
            };
        }
        let Some(&func) = self.by_name.get(&name) else {
            // An undeclared callee is a run-time error (it may sit in dead
            // code): `undefined()` is a TypeError, any other undeclared
            // name a ReferenceError raised by reading it.
            let callee_value = if name == "undefined" {
                Some(self.check_expr(
                    cx,
                    &ast::Expr::Unary {
                        op: ast::UnaryOp::Void,
                        operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                        span,
                    },
                ))
            } else {
                zun_natives::resolve("collections", "globalRead").map(|(id, _)| hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![Self::string_literal(name.clone(), span)],
                    },
                    ty: Ty::Any,
                    span,
                })
            };
            if let Some(callee_value) = callee_value {
                return hir::Expr {
                    kind: hir::ExprKind::IndirectCall {
                        callee: Box::new(callee_value),
                        args,
                        func: hir::DYNAMIC_FUNCTION,
                    },
                    ty: Ty::Any,
                    span,
                };
            }
            self.error(callee.span, format!("unknown function `{name}`"));
            return self.poison(span);
        };
        let (param_tys, return_ty, optional_count, signature_name) = {
            let sig = &self.signatures[func as usize];
            (
                sig.params.clone(),
                sig.return_ty,
                sig.optional_count,
                sig.name.clone(),
            )
        };
        let sig = Signature {
            name: signature_name,
            params: param_tys.clone(),
            return_ty,
            optional_count,
            rest_param: self.signatures[func as usize].rest_param,
            is_generator: self.signatures[func as usize].is_generator,
            is_static: false,
        };
        let actual_argc = args.len();
        let rest_index = sig.rest_param;
        if let Some(rest_index) = rest_index {
            let rest_required = rest_index.saturating_sub(optional_count);
            if args.len() < rest_required {
                self.error(
                    span,
                    format!(
                        "`{name}` expects at least {} argument(s), found {}",
                        rest_required,
                        args.len()
                    ),
                );
            } else {
                while args.len() < rest_index {
                    args.push(default_args::missing_arg(param_tys[args.len()], span));
                }
                // Function values packed into the rest array ride in functionBox records.
                for index in rest_index..args.len() {
                    if matches!(args[index].ty, Ty::Function(_)) {
                        let element = std::mem::replace(&mut args[index], self.poison(span));
                        args[index] = self.box_function_argument(element);
                    }
                }
                for (arg, &expected) in args.iter_mut().take(rest_index).zip(&param_tys) {
                    coerce_numeric_literal(arg, expected);
                    if !expected.is_error()
                        && !arg.ty.is_error()
                        && !self.is_assignable(arg.ty, expected)
                    {
                        self.error(
                            arg.span,
                            format!("argument type {:?} does not match {expected:?}", arg.ty),
                        );
                    }
                }
            }
        } else if args.len() < param_tys.len().saturating_sub(optional_count)
            || args.len() > param_tys.len()
        {
            self.error(
                span,
                format!(
                    "`{name}` expects {} to {} argument(s), found {}",
                    param_tys.len().saturating_sub(optional_count),
                    param_tys.len(),
                    args.len()
                ),
            );
        } else {
            self.pad_default_args(&mut args, &sig, span);
            for (arg, &expected) in args.iter_mut().zip(&param_tys) {
                coerce_numeric_literal(arg, expected);
                if !expected.is_error()
                    && !arg.ty.is_error()
                    && !self.is_assignable(arg.ty, expected)
                {
                    self.error(
                        arg.span,
                        format!("argument type {:?} does not match {expected:?}", arg.ty),
                    );
                }
            }
        }
        // A callee that reads `arguments` learns the real argument count
        // (before default padding) through the last argument's evaluation.
        if self.arguments_functions.contains(&func) {
            if let (Some(last), Some((mark_id, _))) =
                (args.pop(), zun_natives::resolve("collections", "setCallArgc"))
            {
                let (ty, last_span) = (last.ty, last.span);
                args.push(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: mark_id,
                        args: vec![
                            last,
                            hir::Expr {
                                kind: hir::ExprKind::Number(actual_argc as f64),
                                ty: Ty::F64,
                                span: last_span,
                            },
                        ],
                    },
                    ty,
                    span: last_span,
                });
            }
        }
        hir::Expr {
            kind: if rest_index.is_some() {
                hir::ExprKind::CallRest { func, args }
            } else {
                hir::ExprKind::Call { func, args }
            },
            ty: return_ty,
            span,
        }
    }

    pub(in crate::check) fn check_static_method_call(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::StaticMethodCall { class, method, args, .. } = expr else { unreachable!() };
        let class_name = self.text(class).to_string();
        if let Some(&namespace) = self.core_namespaces.get(&class_name).filter(|_| cx.lookup(&class_name).is_none()) {
            let Some((id, native)) = zun_natives::resolve_core(namespace, self.text(method)) else {
                self.error(method.span, format!("`zun:core` namespace `{}` has no member `{}`", namespace, self.text(method)));
                return self.poison(span);
            };
            if native.params.len() != args.len() {
                self.error(span, format!("`{}` expects {} argument(s), found {}", self.text(method), native.params.len(), args.len()));
                return self.poison(span);
            }
            let mut checked = Vec::with_capacity(args.len());
            for (arg, expected) in args.iter().zip(native.params.iter().map(|ty| native_ty(*ty))) {
                let mut value = self.check_expr(cx, arg);
                coerce_numeric_literal(&mut value, expected);
                if !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                    self.error(value.span, format!("argument type {:?} does not match {expected:?}", value.ty));
                }
                checked.push(value);
            }
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: checked },
                ty: native_ty(native.ret),
                span,
            };
        }
        if let Some(module) = self.namespace_imports.get(&class_name).cloned() {
            let Some((id, native)) = zun_natives::resolve(&module, self.text(method)) else {
                self.error(method.span, format!("`{}` has no export named `{}`", module, self.text(method)));
                return self.poison(span);
            };
            if native.params.len() != args.len() {
                self.error(span, format!("`{}` expects {} argument(s), found {}", self.text(method), native.params.len(), args.len()));
                return self.poison(span);
            }
            let mut checked = Vec::with_capacity(args.len());
            for (arg, expected) in args.iter().zip(native.params.iter().map(|ty| native_ty(*ty))) {
                let mut value = self.check_expr(cx, arg);
                coerce_numeric_literal(&mut value, expected);
                if !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                    self.error(value.span, format!("argument type {:?} does not match {expected:?}", value.ty));
                }
                checked.push(value);
            }
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: checked },
                ty: native_ty(native.ret),
                span,
            };
        }
        let Some(&class_index) = self.classes_by_name.get(&class_name) else {
            self.error(class.span, format!("unknown class `{class_name}`"));
            return self.poison(span);
        };
        let Some(func) = self.method_of(class_index, self.text(method)) else {
            self.error(method.span, format!("unknown static method `{}`", self.text(method)));
            return self.poison(span);
        };
        let (param_tys, return_ty, optional_count) = {
            let sig = &self.signatures[func as usize];
            (sig.params.clone(), sig.return_ty, sig.optional_count)
        };
        let required = param_tys.len().saturating_sub(optional_count);
        if args.len() > param_tys.len() || args.len() < required {
            self.error(span, format!("static method `{}` expects {} arguments", self.text(method), param_tys.len()));
            return self.poison(span);
        }
        let mut checked = Vec::with_capacity(args.len());
        for (arg, &expected) in args.iter().zip(&param_tys) {
            let mut value = self.check_expr(cx, arg);
            // Instance and free calls narrow a bare numeric literal to
            // the parameter type; static calls did not, so
            // `Date.UTC(2020, 0, 1, ...)` failed on literals that the
            // same arguments would accept through any other call
            // shape.
            coerce_numeric_literal(&mut value, expected);
            if !expected.is_error() && !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                self.error(value.span, format!("argument type {:?} does not match {expected:?}", value.ty));
            }
            checked.push(value);
        }
        // Omitted optional arguments are passed as null sentinels; the
        // callee prologue materialises its declared defaults.
        if checked.len() < param_tys.len() {
            let sig = Signature {
                name: self.text(method).to_string(),
                params: param_tys.clone(),
                return_ty,
                optional_count,
                rest_param: None,
                is_generator: false,
                is_static: true,
            };
            self.pad_default_args(&mut checked, &sig, span);
        }
        hir::Expr {
            kind: hir::ExprKind::StaticCall { func, args: checked },
            ty: return_ty,
            span,
        }
    }

    pub(in crate::check) fn check_regex_symbol_call_expr(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let ast::Expr::ComputedCall { callee, args, span: call_span } = expr else { unreachable!() };
        // `/re/[Symbol.split](str, ...rest)` is `str.split(/re/, ...rest)`.
        let ast::Expr::Index { object, index, .. } = callee.as_ref() else { unreachable!() };
        let method = match index.as_ref() {
            ast::Expr::Field { field, .. } => *field,
            ast::Expr::StaticField { field, .. } => *field,
            _ => unreachable!(),
        };
        // A non-string subject is coerced with ToString first.
        let subject = self.check_expr(cx, &args[0]);
        let direct_match_all =
            matches!(self.well_known_symbol_of_key(index), Some(zun_core::WellKnownSymbol::MatchAll));
        if (subject.ty != Ty::String && !subject.ty.is_error()) || direct_match_all {
            let kind = match self.well_known_symbol_of_key(index) {
                Some(zun_core::WellKnownSymbol::Match) => 0.0,
                Some(zun_core::WellKnownSymbol::Replace) => 1.0,
                Some(zun_core::WellKnownSymbol::Search) => 2.0,
                Some(zun_core::WellKnownSymbol::Split) => 3.0,
                _ => 6.0,
            };
            let subject = self.to_string_for_parse(subject);
            let regex = self.check_expr(cx, object);
            let regex = Self::box_to_any(regex);
            let extra = args.get(1).map(|arg| self.check_expr(cx, arg));
            return self.string_symbol_dispatch(*call_span, kind, subject, regex, extra);
        }
        let mut rest = vec![(**object).clone()];
        rest.extend(args.iter().skip(1).cloned());
        let call = ast::Expr::MethodCall {
            receiver: Box::new(args[0].clone()),
            method,
            args: rest,
            span: *call_span,
        };
        self.check_expr(cx, &call)
    }

    pub(in crate::check) fn check_computed_call(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::ComputedCall { callee, args, .. } = expr else { unreachable!() };
        if args.iter().any(|a| matches!(a, ast::Expr::Spread { .. })) {
            // `(arr.method as any)(...args)` on a typed array keeps its receiver:
            // lower it as the method call it denotes.
            let mut inner = callee.as_ref();
            while let ast::Expr::Cast { expr, .. } = inner {
                inner = expr.as_ref();
            }
            if let ast::Expr::Field { object, field, .. } = inner {
                let probe = self.check_expr(cx, object);
                if matches!(probe.ty, Ty::Any | Ty::DynamicObject) {
                    let elements = args
                        .iter()
                        .map(|arg| match arg {
                            ast::Expr::Spread { expr, .. } => ast::ArrayElement::Spread((**expr).clone()),
                            other => ast::ArrayElement::Expr(other.clone()),
                        })
                        .collect();
                    let literal = ast::Expr::ArrayLiteral { elements, span };
                    let list = self.check_expr(cx, &literal);
                    if let Some((id, _)) = zun_natives::resolve("array", "methodCallSpread") {
                        let name = hir::Expr {
                            kind: hir::ExprKind::String(self.text(field).to_string()),
                            ty: Ty::String,
                            span,
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id,
                                args: vec![Self::box_to_any(probe), Self::box_to_any(name), list],
                            },
                            ty: Ty::Any,
                            span,
                        };
                    }
                }
                if probe.ty == Ty::Array {
                    let call = ast::Expr::MethodCall {
                        receiver: object.clone(),
                        method: *field,
                        args: args.clone(),
                        span,
                    };
                    return self.check_expr(cx, &call);
                }
            }
            if let Some(call) = self.check_spread_call(cx, callee, args, span) {
                return call;
            }
        }
        let is_well_known_iterator = matches!(callee.as_ref(), ast::Expr::Index {
            index,
            ..
        } if matches!(index.as_ref(), ast::Expr::Field { object, field, .. }
            if matches!(object.as_ref(), ast::Expr::Ident(name) if self.text(name) == "Symbol")
                && self.text(field) == "iterator"));
        if is_well_known_iterator && args.is_empty() {
            let ast::Expr::Index { object, .. } = callee.as_ref() else {
                unreachable!();
            };
            let object = self.check_expr(cx, object);
            let native_name = match object.ty {
                Ty::Array => "arrayIteratorNew",
                Ty::Map => "mapIteratorNew",
                Ty::Set => "setIteratorNew",
                Ty::String => "stringIteratorNew",
                Ty::Error => "arrayIteratorNew",
                Ty::Any | Ty::DynamicObject => "symbolIteratorCall",
                Ty::BufferView(kind) => match crate::typed_arrays::iterator_new(kind) {
                    Some(new_name) => new_name,
                    None => {
                        self.error(
                            object.span,
                            "a DataView has no Symbol.iterator".to_string(),
                        );
                        return self.poison(span);
                    }
                },
                other => {
                    self.error(
                        object.span,
                        format!("type {other:?} has no Symbol.iterator"),
                    );
                    return self.poison(span);
                }
            };
            let Some((id, _)) = zun_natives::resolve("buffer", native_name) else {
                self.error(span, "missing Symbol.iterator intrinsic");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![object],
                },
                ty: Ty::Any,
                span,
            };
        }
        // `console[key](args)` with a runtime key: dispatch on the key over
        // the console methods (the key is evaluated once into a temp).
        if let ast::Expr::Index { object: recv_ast, index: key_ast, .. } = callee.as_ref() {
            let namespace_methods: &[(&str, usize, usize)] = match recv_ast.as_ref() {
                ast::Expr::Ident(id) if cx.lookup(self.text(id)).is_none() => match self.text(id) {
                    "console" => &[
                        ("debug", 0, 99), ("info", 0, 99), ("warn", 0, 99), ("error", 0, 99), ("log", 0, 99),
                    ],
                    "JSON" => &[("stringify", 1, 3), ("parse", 1, 2)],
                    "Math" => &[
                        ("abs", 1, 1), ("floor", 1, 1), ("ceil", 1, 1), ("round", 1, 1), ("max", 0, 99),
                        ("min", 0, 99), ("sqrt", 1, 1), ("pow", 2, 2), ("trunc", 1, 1), ("sign", 1, 1),
                    ],
                    "Object" => &[
                        ("keys", 1, 1), ("values", 1, 1), ("entries", 1, 1), ("assign", 1, 99),
                        ("freeze", 1, 1), ("isFrozen", 1, 1), ("create", 1, 1), ("getPrototypeOf", 1, 1),
                        ("defineProperty", 3, 3), ("getOwnPropertyDescriptor", 2, 2),
                        ("getOwnPropertyNames", 1, 1), ("fromEntries", 1, 1), ("groupBy", 2, 2), ("seal", 1, 1),
                        ("isSealed", 1, 1), ("isExtensible", 1, 1), ("preventExtensions", 1, 1),
                        ("setPrototypeOf", 2, 2), ("hasOwn", 2, 2), ("getOwnPropertySymbols", 1, 1),
                    ],
                    "Reflect" => &[
                        ("get", 2, 3), ("set", 3, 4), ("has", 2, 2), ("ownKeys", 1, 1),
                        ("defineProperty", 3, 3), ("getPrototypeOf", 1, 1), ("deleteProperty", 2, 2),
                    ],
                    "Array" => &[("isArray", 1, 1), ("from", 1, 3), ("of", 0, 99)],
                    "Number" => &[
                        ("isInteger", 1, 1), ("isNaN", 1, 1), ("isFinite", 1, 1), ("parseFloat", 1, 1),
                        ("parseInt", 1, 2),
                    ],
                    "String" => &[("fromCharCode", 0, 99)],
                    _ => &[],
                },
                ast::Expr::RegexLiteral { .. } => &[("exec", 1, 1), ("test", 1, 1)],
                // A typed collection/string held in a local: dispatch over its methods.
                ast::Expr::Ident(id) => match cx.lookup(self.text(id)).map(|slot| cx.locals[slot as usize]) {
                    Some(Ty::Map) => &[
                        ("entries", 0, 0), ("keys", 0, 0), ("values", 0, 0), ("get", 1, 1), ("set", 2, 2),
                        ("has", 1, 1), ("delete", 1, 1), ("clear", 0, 0),
                    ],
                    Some(Ty::Set) => &[
                        ("entries", 0, 0), ("keys", 0, 0), ("values", 0, 0), ("add", 1, 1),
                        ("has", 1, 1), ("delete", 1, 1), ("clear", 0, 0),
                    ],
                    Some(Ty::String) => &[
                        ("toUpperCase", 0, 0), ("toLowerCase", 0, 0), ("trim", 0, 0), ("charAt", 1, 1),
                        ("charCodeAt", 1, 1), ("slice", 1, 2), ("substring", 1, 2), ("indexOf", 1, 2),
                        ("split", 1, 2), ("includes", 1, 2), ("startsWith", 1, 2), ("endsWith", 1, 2),
                        ("repeat", 1, 1), ("padStart", 1, 2), ("padEnd", 1, 2), ("concat", 0, 99),
                    ],
                    _ => &[],
                },
                _ => &[],
            };
            if !namespace_methods.is_empty()
                && !matches!(key_ast.as_ref(), ast::Expr::String { .. })
            {
                let key_hir = self.check_expr(cx, key_ast);
                if !key_hir.ty.is_error() {
                    let key_hir = self.to_string_for_parse(key_hir);
                    let (slot, tmp) = self.synthetic_local(cx, Ty::String);
                    let mut chain = ast::Expr::Unary {
                        op: ast::UnaryOp::Void,
                        operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                        span,
                    };
                    for (method, min, max) in namespace_methods.iter().copied() {
                        if args.len() < min || args.len() > max {
                            continue;
                        }
                        chain = ast::Expr::Conditional {
                            condition: Box::new(ast::Expr::Binary {
                                op: ast::BinaryOp::Eq,
                                lhs: Box::new(ast::Expr::Ident(tmp)),
                                rhs: Box::new(ast::Expr::String { value: method.to_string(), span }),
                                span,
                            }),
                            when_true: Box::new(ast::Expr::MethodCall {
                                receiver: recv_ast.clone(),
                                method: ast::synthetic_ident(method),
                                args: args.clone(),
                                span,
                            }),
                            when_false: Box::new(chain),
                            span,
                        };
                    }
                    let body = self.check_expr(cx, &chain);
                    let ty = body.ty;
                    return hir::Expr {
                        kind: hir::ExprKind::Sequence {
                            effect: Box::new(hir::Expr {
                                kind: hir::ExprKind::Assign { slot, value: Box::new(key_hir) },
                                ty: Ty::String,
                                span,
                            }),
                            value: Box::new(body),
                        },
                        ty,
                        span,
                    };
                }
            }
        }
        // `recv[key](args)` on a dynamic receiver: a member call (keeps
        // `this`), dispatched by the stringified key at run time.
        if let ast::Expr::Index { object: recv_ast, index: key_ast, .. } = callee.as_ref() {
            if args.len() <= 6 && !matches!(key_ast.as_ref(), ast::Expr::Spread { .. }) {
                let recv_hir = self.check_expr(cx, recv_ast);
                if matches!(recv_hir.ty, Ty::Any | Ty::DynamicObject | Ty::Array | Ty::Map | Ty::Set | Ty::String) {
                    let key_hir = self.check_expr(cx, key_ast);
                    let array_ok = !matches!(recv_hir.ty, Ty::Array | Ty::Map | Ty::Set | Ty::String)
                        || matches!(key_hir.ty, Ty::String | Ty::Any);
                    // A key of unknown type may be a Symbol: split on its run-time type;
                    // a symbol key reads the member and calls it with the receiver as
                    // `this`, any other key keeps the stringified member dispatch.
                    if array_ok
                        && !self.symbol_call_guard
                        && matches!(key_hir.ty, Ty::Any | Ty::Symbol)
                        && matches!(recv_hir.ty, Ty::Any | Ty::DynamicObject)
                    {
                        let (recv_slot, recv_tmp) = self.synthetic_local(cx, Ty::Any);
                        let (key_slot, key_tmp) = self.synthetic_local(cx, Ty::Any);
                        let is_symbol = ast::Expr::Binary {
                            op: ast::BinaryOp::Eq,
                            lhs: Box::new(ast::Expr::Unary {
                                op: ast::UnaryOp::Typeof,
                                operand: Box::new(ast::Expr::Ident(key_tmp)),
                                span,
                            }),
                            rhs: Box::new(ast::Expr::String { value: "symbol".to_string(), span }),
                            span,
                        };
                        let mut symbol_args = vec![ast::Expr::Ident(recv_tmp)];
                        symbol_args.extend(args.iter().cloned());
                        let symbol_call = ast::Expr::MethodCall {
                            receiver: Box::new(ast::Expr::Index {
                                object: Box::new(ast::Expr::Ident(recv_tmp)),
                                index: Box::new(ast::Expr::Ident(key_tmp)),
                                span,
                            }),
                            method: ast::synthetic_ident("call"),
                            args: symbol_args,
                            span,
                        };
                        let string_call = ast::Expr::ComputedCall {
                            callee: Box::new(ast::Expr::Index {
                                object: Box::new(ast::Expr::Ident(recv_tmp)),
                                index: Box::new(ast::Expr::Ident(key_tmp)),
                                span,
                            }),
                            args: args.clone(),
                            span,
                        };
                        let chain = ast::Expr::Conditional {
                            condition: Box::new(is_symbol),
                            when_true: Box::new(symbol_call),
                            when_false: Box::new(string_call),
                            span,
                        };
                        self.symbol_call_guard = true;
                        let body = self.check_expr(cx, &chain);
                        self.symbol_call_guard = false;
                        let ty = body.ty;
                        let assign_key = hir::Expr {
                            kind: hir::ExprKind::Assign { slot: key_slot, value: Box::new(Self::box_to_any(key_hir)) },
                            ty: Ty::Any,
                            span,
                        };
                        let inner = hir::Expr {
                            kind: hir::ExprKind::Sequence { effect: Box::new(assign_key), value: Box::new(body) },
                            ty,
                            span,
                        };
                        let assign_recv = hir::Expr {
                            kind: hir::ExprKind::Assign { slot: recv_slot, value: Box::new(Self::box_to_any(recv_hir)) },
                            ty: Ty::Any,
                            span,
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::Sequence { effect: Box::new(assign_recv), value: Box::new(inner) },
                            ty,
                            span,
                        };
                    }
                    if array_ok
                        && (matches!(key_hir.ty, Ty::String | Ty::F64 | Ty::Any) || key_hir.ty.is_int())
                    {
                        let key_hir = self.to_string_for_parse(key_hir);
                        let native_name = [
                            "dynamicCall0",
                            "dynamicCall1",
                            "dynamicCall2",
                            "dynamicCall3",
                            "dynamicCall4",
                            "dynamicCall5",
                            "dynamicCall6",
                        ][args.len()];
                        if let Some((id, _)) = zun_natives::resolve("collections", native_name) {
                            let mut call_args = vec![recv_hir, key_hir];
                            for arg in args {
                                call_args.push(self.check_expr(cx, arg));
                            }
                            return hir::Expr {
                                kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                                ty: Ty::Any,
                                span,
                            };
                        }
                    }
                }
            }
        }
        // `recv?.[key](args)`: receiver-guarded member call.
        if let ast::Expr::OptionalIndex { object: recv_ast, index: key_ast, .. } = callee.as_ref() {
            let recv_hir = self.check_expr(cx, recv_ast);
            if matches!(recv_hir.ty, Ty::Any | Ty::DynamicObject) {
                let (slot, tmp) = self.synthetic_local(cx, Ty::Any);
                let tmp_expr = ast::Expr::Ident(tmp);
                let call = ast::Expr::ComputedCall {
                    callee: Box::new(ast::Expr::Index {
                        object: Box::new(tmp_expr.clone()),
                        index: key_ast.clone(),
                        span,
                    }),
                    args: args.clone(),
                    span,
                };
                return self.optional_guarded(cx, slot, Self::box_to_any(recv_hir), tmp_expr, call, span);
            }
        }
        let callee = self.check_expr(cx, callee);
        if let Ty::Function(func) = callee.ty {
            let mut args: Vec<hir::Expr> =
                args.iter().map(|argument| self.check_expr(cx, argument)).collect();
            if matches!(
                func,
                hir::DYNAMIC_FUNCTION | hir::DYNAMIC_PREDICATE | hir::DYNAMIC_REDUCER
            ) {
                let (expected, return_ty) = match func {
                    hir::DYNAMIC_REDUCER => (2, Ty::Any),
                    hir::DYNAMIC_PREDICATE => (1, Ty::Boolean),
                    _ => (1, Ty::Any),
                };
                // A plain dynamic function value accepts any argument
                // count (JS); only the fixed-shape predicate/reducer
                // callbacks enforce their arity.
                if func != hir::DYNAMIC_FUNCTION && args.len() != expected {
                    self.error(
                        span,
                        format!("dynamic function value expects {expected} argument(s), found {}", args.len()),
                    );
                }
                return hir::Expr {
                    kind: hir::ExprKind::IndirectCall {
                        callee: Box::new(callee),
                        args,
                        func,
                    },
                    ty: return_ty,
                    span,
                };
            }
            let (param_tys, optional_count, rest_param, return_ty, signature_name) = {
                let signature = &self.signatures[func as usize];
                (
                    signature.params.clone(),
                    signature.optional_count,
                    signature.rest_param,
                    signature.return_ty,
                    signature.name.clone(),
                )
            };
            if let Some(rest_index) = rest_param {
                if args.len() < rest_index {
                    self.error(
                        span,
                        format!("function value expects at least {} argument(s), found {}", rest_index, args.len()),
                    );
                }
                for (arg, &expected) in args.iter_mut().zip(&param_tys).take(rest_index) {
                    coerce_numeric_literal(arg, expected);
                    if !expected.is_error()
                        && !arg.ty.is_error()
                        && !self.is_assignable(arg.ty, expected)
                    {
                        self.error(
                            arg.span,
                            format!("argument type {:?} does not match {expected:?}", arg.ty),
                        );
                    }
                }
                return hir::Expr {
                    kind: hir::ExprKind::IndirectCallRest {
                        callee: Box::new(callee),
                        args,
                        func,
                    },
                    ty: return_ty,
                    span,
                };
            }
            if args.len() < param_tys.len().saturating_sub(optional_count)
                || args.len() > param_tys.len()
            {
                self.error(
                    span,
                    format!(
                        "function value expects {} to {} argument(s), found {}",
                        param_tys.len().saturating_sub(optional_count),
                        param_tys.len(),
                        args.len()
                    ),
                );
            } else {
                let signature = Signature {
                    name: signature_name,
                    params: param_tys.clone(),
                    return_ty,
                    optional_count,
                    rest_param,
                    is_generator: false,
                    is_static: false,
                };
                self.pad_default_args(&mut args, &signature, span);
                for (arg, &expected) in args.iter_mut().zip(&param_tys) {
                    coerce_numeric_literal(arg, expected);
                    if !expected.is_error()
                        && !arg.ty.is_error()
                        && !self.is_assignable(arg.ty, expected)
                    {
                        self.error(
                            arg.span,
                            format!("argument type {:?} does not match {expected:?}", arg.ty),
                        );
                    }
                }
            }
            return hir::Expr {
                kind: hir::ExprKind::IndirectCall {
                    callee: Box::new(callee),
                    args,
                    func,
                },
                ty: return_ty,
                span,
            };
        }
        // A dynamic (`any`) callee is invoked through the counted
        // dynamic-call form, which accepts any argument count.
        if matches!(callee.ty, Ty::Any | Ty::DynamicObject) {
            let args: Vec<hir::Expr> =
                args.iter().map(|argument| self.check_expr(cx, argument)).collect();
            return hir::Expr {
                kind: hir::ExprKind::IndirectCall {
                    callee: Box::new(callee),
                    args,
                    func: hir::DYNAMIC_FUNCTION,
                },
                ty: Ty::Any,
                span,
            };
        }
        if callee.ty.is_error() {
            return self.poison(span);
        }
        self.error(span, "computed calls are not supported for this callee");
        self.poison(span)
    }
}
