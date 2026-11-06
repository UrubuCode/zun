use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_call(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::Call { callee, args, .. } = expr else { unreachable!() };
        let name = self.text(callee).to_string();
        // AST-phase interceptors (name-based sugar and spread lowering).
        if let Some(e) = self.try_call_name_sugar(cx, callee, &name, args, span) {
            return e;
        }
        if let Some(e) = self.try_call_spread(cx, callee, &name, args, span) {
            return e;
        }
        let mut args: Vec<hir::Expr> =
            args.iter().map(|a| self.check_expr(cx, a)).collect();
        // Value-phase interceptors over the checked arguments, in order.
        if let Some(e) = self.try_call_coerce(&name, &mut args, span) {
            return e;
        }
        if let Some(e) = self.try_call_generator(&name, &mut args, span) {
            return e;
        }
        if let Some(e) = self.try_call_local_indirect(cx, &name, &mut args, span) {
            return e;
        }
        if let Some(e) = self.try_call_print_timers(cx, &name, &mut args, span) {
            return e;
        }
        if let Some(e) = self.try_call_decorators(&name, &mut args, span) {
            return e;
        }
        if let Some(e) = self.try_call_dynamic_misc(&name, &mut args, span) {
            return e;
        }
        if let Some(e) = self.try_call_dynamic_reflect(&name, &mut args, span) {
            return e;
        }
        if let Some(e) = self.try_call_imports_external(callee, &name, &mut args, span) {
            return e;
        }
        // The final named-function dispatch (and unknown-name fallback).
        self.call_named_dispatch(cx, callee, &name, args, span)
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
