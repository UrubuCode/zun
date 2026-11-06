use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_method_call(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::MethodCall { receiver, method, args, .. } = expr else { unreachable!() };
        // `recv.method(...args)` on a dynamic receiver: dispatch at run time.
        if args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
            && !matches!(self.text(method), "call" | "apply" | "bind")
            && !matches!(receiver.as_ref(), ast::Expr::Ident(_) | ast::Expr::Field { .. } | ast::Expr::StaticField { .. } if false)
        {
            let is_namespace = matches!(receiver.as_ref(), ast::Expr::Ident(n) if cx.lookup(self.text(n)).is_none());
            if !is_namespace {
                let probe = self.check_expr(cx, receiver);
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
                            kind: hir::ExprKind::String(self.text(method).to_string()),
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
            }
        }
        // `Object.prototype.hasOwnProperty.call(obj, key)` is `Object.hasOwn(obj, key)`.
        if self.text(method) == "call" && args.len() == 2 && !self.classes_by_name.contains_key("Object") {
            let is_has_own_call = match receiver.as_ref() {
                ast::Expr::Field { object: proto, field: member, .. } => {
                    self.text(member) == "hasOwnProperty"
                        && (matches!(proto.as_ref(),
                            ast::Expr::Field { object: base, field: p, .. }
                                if self.text(p) == "prototype"
                                    && matches!(base.as_ref(), ast::Expr::Ident(n) if self.text(n) == "Object"))
                            || matches!(proto.as_ref(),
                                ast::Expr::StaticField { class, field: p, .. }
                                    if self.text(p) == "prototype" && self.text(class) == "Object"))
                }
                _ => false,
            };
            if is_has_own_call {
                // `Object.prototype.hasOwnProperty.call(Ctor.prototype, "name")` on a
                // built-in constructor: answered from the built-in method table.
                let proto_owner = match &args[0] {
                    ast::Expr::Field { object: base, field: p, .. } if self.text(p) == "prototype" => match base.as_ref() {
                        ast::Expr::Ident(n) => Some(self.text(n).to_string()),
                        _ => None,
                    },
                    ast::Expr::StaticField { class, field: p, .. } if self.text(p) == "prototype" => {
                        Some(self.text(class).to_string())
                    }
                    _ => None,
                };
                if let (Some(owner), ast::Expr::String { value, .. }) = (&proto_owner, &args[1]) {
                    // A user class: its prototype owns exactly the declared
                    // instance methods and accessors (plus `constructor`).
                    if let Some(&class_idx) = self.classes_by_name.get(owner) {
                        let own = &self.methods_by_class[class_idx as usize];
                        let present = value == "constructor"
                            || [value.clone(), format!("get_{value}"), format!("set_{value}")]
                                .iter()
                                .any(|n| {
                                    own.get(n).is_some_and(|&f| !self.signatures[f as usize].is_static)
                                });
                        return hir::Expr { kind: hir::ExprKind::Boolean(present), ty: Ty::Boolean, span };
                    }
                }
                // A well-known Symbol key (`Symbol.toPrimitive`) on a user class
                // prototype: present when the class declares that symbol method.
                if let (Some(owner), ast::Expr::Field { .. }) = (&proto_owner, &args[1]) {
                    if let Some(&class_idx) = self.classes_by_name.get(owner) {
                        if let Some(symbol) = self.well_known_symbol_of_key(&args[1]) {
                            let present = self
                                .class_symbol_methods
                                .iter()
                                .any(|&(c, s, _)| c == class_idx && s == symbol.id().get());
                            return hir::Expr { kind: hir::ExprKind::Boolean(present), ty: Ty::Boolean, span };
                        }
                    }
                }
                let proto_owner = proto_owner;
                if let (Some(owner), ast::Expr::String { .. }) = (proto_owner, &args[1]) {
                    if !self.classes_by_name.contains_key(&owner) {
                        if let ast::Expr::String { value, .. } = &args[1] {
                            if builtin_function_length(&[owner.as_str(), "prototype", value.as_str()]).is_some() {
                                return hir::Expr { kind: hir::ExprKind::Boolean(true), ty: Ty::Boolean, span };
                            }
                        }
                    }
                }
                return self.check_object_static(cx, "hasOwn", method.span, args, span);
            }
        }
        // `Array.prototype.<m>.call(arrayLike, ...rest)` on a non-mutating
        // method: convert the array-like with `Array.from` and call `<m>`.
        if self.text(method) == "call" && !args.is_empty() && !self.classes_by_name.contains_key("Array") {
            let array_method = match receiver.as_ref() {
                ast::Expr::Field { object: proto, field: member, .. }
                    if matches!(proto.as_ref(),
                        ast::Expr::Field { object: base, field: p, .. }
                            if self.text(p) == "prototype"
                                && matches!(base.as_ref(), ast::Expr::Ident(n) if self.text(n) == "Array"))
                        || matches!(proto.as_ref(),
                            ast::Expr::StaticField { class, field: p, .. }
                                if self.text(p) == "prototype" && self.text(class) == "Array") =>
                {
                    Some(self.text(member).to_string())
                }
                _ => None,
            };
            const READ_ONLY: &[&str] = &[
                "reduce", "reduceRight", "map", "filter", "forEach", "some", "every", "find",
                "findIndex", "findLast", "findLastIndex", "indexOf", "lastIndexOf", "includes",
                "join", "slice", "concat", "at", "flat", "flatMap", "entries", "keys", "values",
            ];
            if let Some(name) = array_method {
                // `Array.prototype.push/pop.call(receiver, ...)` mutates an Array
                // or an array-like through the generic algorithm.
                if (name == "push" || name == "pop" || name == "fill" || name == "reverse")
                    && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
                {
                    let native = match name.as_str() {
                        "push" => "arrayLikePush",
                        "fill" => "arrayLikeFill",
                        "reverse" => "arrayLikeReverse",
                        _ => "arrayLikePop",
                    };
                    if let Some((id, _)) = zun_natives::resolve("array", native) {
                        let source = self.check_expr(cx, &args[0]);
                        let mut call_args = vec![Self::box_to_any(source)];
                        if name == "push" || name == "fill" || name == "reverse" {
                            let literal = ast::Expr::ArrayLiteral {
                                elements: args[1..].iter().cloned().map(ast::ArrayElement::Expr).collect(),
                                span,
                            };
                            let list = self.check_expr(cx, &literal);
                            call_args.push(list);
                        }
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                            ty: Ty::Any,
                            span,
                        };
                    }
                }
                if READ_ONLY.contains(&name.as_str()) && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. })) {
                    if let Some((from_id, _)) = zun_natives::resolve("array", "arrayFromArrayLike") {
                        let source = self.check_expr(cx, &args[0]);
                        let (slot, tmp) = self.synthetic_local(cx, Ty::Array);
                        let call = ast::Expr::MethodCall {
                            receiver: Box::new(ast::Expr::Ident(tmp)),
                            method: ast::synthetic_ident(&name),
                            args: args[1..].to_vec(),
                            span,
                        };
                        let body = self.check_expr(cx, &call);
                        let ty = body.ty;
                        return hir::Expr {
                            kind: hir::ExprKind::Sequence {
                                effect: Box::new(hir::Expr {
                                    kind: hir::ExprKind::Assign {
                                        slot,
                                        value: Box::new(hir::Expr {
                                            kind: hir::ExprKind::CallBuiltin {
                                                id: from_id,
                                                args: vec![Self::box_to_any(source)],
                                            },
                                            ty: Ty::Array,
                                            span,
                                        }),
                                    },
                                    ty: Ty::Array,
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
        }
        // `String.prototype.concat.apply(base, args)`.
        if self.text(method) == "apply" && args.len() == 2 && !self.classes_by_name.contains_key("String") {
            let is_string_concat = match receiver.as_ref() {
                ast::Expr::Field { object: proto, field: member, .. } => {
                    self.text(member) == "concat"
                        && matches!(proto.as_ref(),
                            ast::Expr::Field { object: base, field: p, .. }
                                if self.text(p) == "prototype"
                                    && matches!(base.as_ref(), ast::Expr::Ident(n) if self.text(n) == "String"))
                }
                _ => false,
            };
            if is_string_concat {
                if let Some(&func) = self.by_name.get("__zunStringConcatArgs") {
                    let base = self.check_expr(cx, &args[0]);
                    let base = if base.ty == Ty::String {
                        base
                    } else {
                        self.to_string_for_parse(base)
                    };
                    let list = self.check_expr(cx, &args[1]);
                    let list = self.spread_source_to_array(list);
                    return hir::Expr {
                        kind: hir::ExprKind::Call { func, args: vec![base, list] },
                        ty: Ty::String,
                        span,
                    };
                }
            }
        }
        // `Error.prototype.toString.call(value)`.
        if self.text(method) == "call" && args.len() == 1 {
            let is_error_proto_to_string = match receiver.as_ref() {
                ast::Expr::Field { object: proto, field: member, .. } => {
                    self.text(member) == "toString"
                        && matches!(proto.as_ref(),
                            ast::Expr::Field { object: base, field: p, .. }
                                if self.text(p) == "prototype"
                                    && matches!(base.as_ref(), ast::Expr::Ident(n) if self.text(n) == "Error"))
                        || matches!(proto.as_ref(),
                            ast::Expr::StaticField { class, field: p, .. }
                                if self.text(p) == "prototype" && self.text(class) == "Error")
                }
                _ => false,
            };
            if is_error_proto_to_string {
                if let Some(&func) = self.by_name.get("__zunErrorProtoToString") {
                    let value = self.check_expr(cx, &args[0]);
                    return hir::Expr {
                        kind: hir::ExprKind::Call { func, args: vec![Self::box_to_any(value)] },
                        ty: Ty::String,
                        span,
                    };
                }
            }
        }
        // `Object.prototype.toString.call(value)`.
        if self.text(method) == "call" && args.len() == 1 && !self.classes_by_name.contains_key("Object") {
            let is_proto_to_string = match receiver.as_ref() {
                ast::Expr::Field { object: proto, field: member, .. } => {
                    self.text(member) == "toString"
                        && match proto.as_ref() {
                            ast::Expr::Field { object: base, field: p, .. } => {
                                self.text(p) == "prototype"
                                    && matches!(base.as_ref(), ast::Expr::Ident(n) if self.text(n) == "Object")
                            }
                            ast::Expr::StaticField { class, field: p, .. } => {
                                self.text(p) == "prototype" && self.text(class) == "Object"
                            }
                            _ => false,
                        }
                }
                _ => false,
            };
            if is_proto_to_string {
                if let Some((id, _)) = zun_natives::resolve_name("objectToStringTag") {
                    let value = self.check_expr(cx, &args[0]);
                    let value = if value.ty == Ty::Any {
                        value
                    } else {
                        hir::Expr {
                            span: value.span,
                            kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                            ty: Ty::Any,
                        }
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![value] },
                        ty: Ty::String,
                        span,
                    };
                }
            }
        }
        // `Math.<fn>.apply(thisArg, array)` / `.call(thisArg, ...args)`.
        if let ast::Expr::Field { object: inner_obj, field: inner_field, .. } = receiver.as_ref() {
            if let ast::Expr::Ident(m) = inner_obj.as_ref() {
                let m_name = self.text(m);
                let verb = self.text(method).to_string();
                let fn_name = self.text(inner_field).to_string();
                if m_name == "Math"
                    && !self.classes_by_name.contains_key("Math")
                    && math_function_length(&fn_name).is_some()
                    && (verb == "apply" || verb == "call")
                {
                    if verb == "apply" {
                        if let Some(array) = args.get(1) {
                            return self.math_apply_expr(cx, &fn_name, array, span);
                        }
                        return self.check_math_static(cx, &fn_name, method.span, &[], span);
                    }
                    let rest = if args.is_empty() { &args[..] } else { &args[1..] };
                    return self.check_math_static(cx, &fn_name, method.span, rest, span);
                }
            }
        }
        if let ast::Expr::Ident(class) = receiver.as_ref() {
            let class_name = self.text(class).to_string();
            if class_name == "super" {
                return self.check_super_call(cx, Some(method), args, span);
            }
            if class_name == "Map"
                && self.text(method) == "groupBy"
                && !self.classes_by_name.contains_key("Map")
                && args.len() == 2
            {
                if let Some(&func) = self.by_name.get("__zunMapGroupBy") {
                    let items = self.check_expr(cx, &args[0]);
                    let callback = self.check_expr(cx, &args[1]);
                    return hir::Expr {
                        kind: hir::ExprKind::Call {
                            func,
                            args: vec![Self::box_to_any(items), callback],
                        },
                        ty: Ty::Map,
                        span,
                    };
                }
            }
            if class_name == "Object" && !self.classes_by_name.contains_key("Object") {
                return self.check_object_static(
                    cx,
                    &self.text(method).to_string(),
                    method.span,
                    args,
                    span,
                );
            }
            // `console.log/info/debug/error/warn(...)` are variadic in JS
            // (space-separated, each argument Node-formatted). The
            // prelude `console` method takes a single value, so intercept
            // here to format every argument through `ConsoleFormat.top`,
            // join with spaces, and print once.
            if class_name == "console" {
                if let Some(result) =
                    self.check_console_variadic(cx, &self.text(method).to_string(), args, span)
                {
                    return result;
                }
            }
            // `Reflect.*` reflection statics (`get set has deleteProperty
            // ownKeys`). Surface-only namespace, recognized by receiver
            // name; a program may still declare its own `Reflect` class.
            if class_name == "Reflect" && !self.classes_by_name.contains_key("Reflect") {
                return self.check_reflect_static(
                    cx,
                    &self.text(method).to_string(),
                    method.span,
                    args,
                    span,
                );
            }
            // `Buffer.*` statics (`from alloc allocUnsafe concat
            // isBuffer byteLength`). Recognized by receiver name; a
            // program may still declare its own `Buffer` class.
            if class_name == "Buffer" && !self.classes_by_name.contains_key("Buffer") {
                return self.check_buffer_static(
                    cx,
                    &self.text(method).to_string(),
                    method.span,
                    args,
                    span,
                );
            }
            // `Number.*` statics (`isNaN isFinite isInteger isSafeInteger
            // parseFloat parseInt`). Unlike `Object`, the `Number` class
            // exists (its instance helpers), so recognition keys on the
            // receiver name alone — a value never names `Number`.
            if class_name == "Number" {
                return self.check_number_static(
                    cx,
                    &self.text(method).to_string(),
                    method.span,
                    args,
                    span,
                );
            }
            // `Math.<fn>(...)` static math, routed to the `math` native
            // group's f64 helpers. Recognized by receiver name (a value
            // never names `Math`); a program may still declare its own.
            if class_name == "Math" && !self.classes_by_name.contains_key("Math") {
                return self.check_math_static(
                    cx,
                    &self.text(method).to_string(),
                    method.span,
                    args,
                    span,
                );
            }
            // `String.fromCharCode(...)` / `String.fromCodePoint(...)`.
            // Recognized by receiver name only for the known statics;
            // anything else on `String` falls through to class dispatch.
            if class_name == "String"
                && matches!(self.text(method), "fromCharCode" | "fromCodePoint")
            {
                return self.check_string_static(
                    cx,
                    &self.text(method).to_string(),
                    method.span,
                    args,
                    span,
                );
            }
            // `Array.isArray(x)` / `Array.of(...)` / `Array.from(src,
            // mapFn?)`. Recognized by receiver name for the known statics
            // (the `Array` class exists for its instance surface, like
            // `Number`); anything else on `Array` falls through.
            if class_name == "Array"
                && matches!(self.text(method), "isArray" | "of" | "from")
            {
                return self.check_array_static(
                    cx,
                    &self.text(method).to_string(),
                    method.span,
                    args,
                    span,
                );
            }
            // `Symbol.for(key)` / `Symbol.keyFor(symbol)` — the global
            // cross-realm registry.
            if class_name == "Symbol" && !self.classes_by_name.contains_key("Symbol") {
                let member = self.text(method).to_string();
                let (native, arg_ty, ret_ty) = match member.as_str() {
                    "for" => ("symbolFor", Ty::String, Ty::Symbol),
                    "keyFor" => ("symbolKeyFor", Ty::Symbol, Ty::Any),
                    other => {
                        self.error(method.span, format!("`Symbol.{other}` is not a function"));
                        return self.poison(span);
                    }
                };
                if args.len() != 1 {
                    self.error(span, format!("`Symbol.{member}` expects one argument"));
                    return self.poison(span);
                }
                let Some((id, _)) = zun_natives::resolve("collections", native) else {
                    self.error(span, format!("missing `{native}` intrinsic"));
                    return self.poison(span);
                };
                let mut arg = self.check_expr(cx, &args[0]);
                if !arg.ty.is_error() && !self.is_assignable(arg.ty, arg_ty) {
                    self.error(arg.span, format!("`Symbol.{member}` expects a {arg_ty:?} argument"));
                }
                coerce_numeric_literal(&mut arg, arg_ty);
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                    ty: ret_ty,
                    span,
                };
            }
            // `BigInt.asIntN(bits, value)` / `BigInt.asUintN(bits,
            // value)`. The `BigInt` class exists (instance helpers), so
            // recognition keys on the receiver name and method pair; any
            // other member falls through to class dispatch.
            if class_name == "BigInt"
                && matches!(self.text(method), "asIntN" | "asUintN")
            {
                let member = self.text(method).to_string();
                let native = if member == "asIntN" {
                    "bigIntAsIntN"
                } else {
                    "bigIntAsUintN"
                };
                if args.len() != 2 {
                    self.error(span, format!("`BigInt.{member}` expects two arguments"));
                    return self.poison(span);
                }
                let Some((id, _)) = zun_natives::resolve("collections", native) else {
                    self.error(span, format!("missing `{native}` intrinsic"));
                    return self.poison(span);
                };
                let mut bits = self.check_expr(cx, &args[0]);
                coerce_numeric_literal(&mut bits, Ty::Int(IntKind::I64));
                if bits.ty == Ty::F64 {
                    // A `number` width is truncated (ToIndex).
                    let bits_span = bits.span;
                    bits = hir::Expr {
                        kind: hir::ExprKind::Convert { value: Box::new(bits) },
                        ty: Ty::Int(IntKind::I64),
                        span: bits_span,
                    };
                }
                if !bits.ty.is_error() && !bits.ty.is_int() {
                    self.error(bits.span, format!("`BigInt.{member}` bit width must be an integer"));
                }
                let value = self.check_expr(cx, &args[1]);
                if !value.ty.is_error() && value.ty != Ty::BigInt {
                    self.error(value.span, format!("`BigInt.{member}` expects a BigInt value"));
                }
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![bits, value] },
                    ty: Ty::BigInt,
                    span,
                };
            }
            // `Promise.*` statics (`resolve reject all allSettled race
            // any`). Recognized by receiver name when no user class
            // shadows `Promise`; a Promise *value* is dispatched by type.
            if class_name == "Promise" && !self.classes_by_name.contains_key("Promise") {
                return self.check_promise_static(
                    cx,
                    &self.text(method).to_string(),
                    method.span,
                    args,
                    span,
                );
            }
            // `ArrayBuffer.isView(x)` — true for a DataView or typed
            // array, false for anything else. Accepts any value; a
            // non-`any` operand is boxed, so a plain value answers
            // `false` rather than failing to type-check.
            if class_name == "ArrayBuffer"
                && self.text(method) == "isView"
                && !self.classes_by_name.contains_key("ArrayBuffer")
            {
                if args.len() != 1 {
                    self.error(span, "`ArrayBuffer.isView` expects one argument".to_string());
                    return self.poison(span);
                }
                let Some((id, _)) = zun_natives::resolve("buffer", "isView") else {
                    self.error(span, "missing `isView` intrinsic".to_string());
                    return self.poison(span);
                };
                let mut arg = self.check_expr(cx, &args[0]);
                if arg.ty != Ty::Any && !arg.ty.is_error() {
                    arg = hir::Expr {
                        span: arg.span,
                        kind: hir::ExprKind::BoxAny { value: Box::new(arg) },
                        ty: Ty::Any,
                    };
                }
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                    ty: Ty::Boolean,
                    span,
                };
            }
            // `Int32Array.from(src, mapFn?)` / `Int32Array.of(...items)`
            // and the other ten kinds. Recognized by the constructor
            // name when no user class shadows it; the target element
            // kind travels to the kind-agnostic native as a table-order
            // discriminant argument.
            if let Some(view_kind) = crate::typed_arrays::view_kind(&class_name) {
                if matches!(self.text(method), "from" | "of")
                    && !self.classes_by_name.contains_key(&class_name)
                {
                    return self.check_typed_array_static(
                        cx,
                        view_kind,
                        &self.text(method).to_string(),
                        args,
                        span,
                    );
                }
            }
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
                if zun_natives::module(&module).is_none() {
                    // Source-module namespaces are handled below.
                } else {
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
            }
            if let Some(module) = self.namespace_imports.get(&class_name).cloned() {
                if let Some(external) = self
                    .external_namespace_functions
                    .get(&(module.clone(), self.text(method).to_string()))
                    .cloned()
                {
                    if external.params.len() != args.len() {
                        self.error(span, format!("`{}` expects {} argument(s), found {}", self.text(method), external.params.len(), args.len()));
                        return self.poison(span);
                    }
                    let mut checked = Vec::with_capacity(args.len());
                    for (arg, &expected) in args.iter().zip(&external.params) {
                        let mut value = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut value, expected);
                        if !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                            self.error(value.span, format!("argument type {:?} does not match {expected:?}", value.ty));
                        }
                        checked.push(value);
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::ExternalCall {
                            module: external.module,
                            name: external.name,
                            args: checked,
                        },
                        ty: external.return_ty,
                        span,
                    };
                }
            }
            if let Some(&class_index) = self.classes_by_name.get(&class_name) {
                if let Some(func) = self.method_of(class_index, self.text(method)) {
                    // A static taking the receiving class value hidden-first.
                    let hidden_class = self.static_this.contains_key(&func);
                    let (param_tys, return_ty) = {
                        let sig = &self.signatures[func as usize];
                        (sig.params[hidden_class as usize..].to_vec(), sig.return_ty)
                    };
                    if !self.signatures[func as usize].is_static {
                        self.error(
                            method.span,
                            format!("instance method `{}` must be called through an object", self.text(method)),
                        );
                        return self.poison(span);
                    }
                    let optional_count = self.signatures[func as usize].optional_count;
                    // JS ignores surplus arguments: they are type-checked and dropped.
                    if args.len() >= param_tys.len().saturating_sub(optional_count) {
                        let mut checked = Vec::with_capacity(param_tys.len());
                        for extra in args.iter().skip(param_tys.len()) {
                            let _ = self.check_expr(cx, extra);
                        }
                        for (arg, &expected) in args.iter().zip(&param_tys) {
                            let mut value = self.check_expr(cx, arg);
                            // The only argument-checking site that
                            // skipped this, so a bare literal failed
                            // here while the identical argument was
                            // accepted through every other call shape.
                            coerce_numeric_literal(&mut value, expected);
                            if !expected.is_error()
                                && !value.ty.is_error()
                                && !self.is_assignable(value.ty, expected)
                            {
                                self.error(
                                    value.span,
                                    format!("argument type {:?} does not match {expected:?}", value.ty),
                                );
                            }
                            checked.push(value);
                        }
                        // Omitted optional arguments are null sentinels;
                        // the callee prologue materialises the defaults.
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
                        if hidden_class {
                            let token = zun_natives::USER_CLASS_FLAG | (class_index + 1);
                            checked.insert(0, self.realm_descriptor_expr(token, span));
                        }
                        return hir::Expr {
                            kind: hir::ExprKind::StaticCall { func, args: checked },
                            ty: return_ty,
                            span,
                        };
                    }
                }
            }
        }
        // Static-regex fast path (the `[GeneratedRegex]` analog): a
        // regex literal or `new RegExp(const, const)` used directly by
        // `.test` / `.isMatch`. The pattern is known at compile time, so
        // it is recorded for `zun_ilgen` to synthesize a ZIL matcher
        // (which tiers up to native like any loop). `zun_ilgen` falls
        // back to the runtime engine for patterns outside the compilable
        // subset, so this never changes observable behavior. `exec` and
        // the string `match`/`search`/etc. paths are left untouched.
        {
            let mname_peek = self.text(method).to_string();
            if (mname_peek == "test" || mname_peek == "isMatch") && args.len() == 1 {
                if let Some((pattern, flags)) = self.static_regex(receiver.as_ref()) {
                    let mut subject = self.check_expr(cx, &args[0]);
                    coerce_numeric_literal(&mut subject, Ty::String);
                    if !subject.ty.is_error() && !self.is_assignable(subject.ty, Ty::String)
                    {
                        self.error(
                            subject.span,
                            format!(
                                "`{mname_peek}` expects a string argument, found {:?}",
                                subject.ty
                            ),
                        );
                    }
                    let matcher = self.regex_matchers.len() as u32;
                    self.regex_matchers.push(hir::CompiledRegex { pattern, flags });
                    return hir::Expr {
                        kind: hir::ExprKind::RegexMatch { matcher, args: vec![subject] },
                        ty: Ty::Boolean,
                        span,
                    };
                }
            }
        }
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
        if receiver.ty == Ty::String && mname == "normalize" {
            if args.len() > 1 {
                self.error(
                    span,
                    format!("`normalize` expects 0 or 1 argument(s), found {}", args.len()),
                );
                return self.poison(span);
            }
            let form = match args.first() {
                Some(arg) => {
                    let checked = self.check_expr(cx, arg);
                    if !checked.ty.is_error() && checked.ty != Ty::String {
                        self.error(checked.span, "normalize form must be a string");
                    }
                    checked
                }
                None => hir::Expr {
                    kind: hir::ExprKind::String("NFC".to_string()),
                    ty: Ty::String,
                    span,
                },
            };
            let Some((id, _)) = zun_natives::resolve("string", "stringNormalize") else {
                self.error(span, "missing string intrinsic `stringNormalize`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, form] },
                ty: Ty::String,
                span,
            };
        }
        let typed_array_method = match receiver.ty {
            Ty::BufferView(kind) => crate::typed_arrays::method(kind, mname.as_str()),
            _ => None,
        };
        let managed_method = typed_array_method.or_else(|| match (receiver.ty, mname.as_str()) {
            // Standard `Array.prototype` methods routed to natives. The
            // callback methods invoke the dynamic function value with the
            // ECMAScript `(value, index, array)` signature (extra
            // arguments dropped to the callback's declared arity).
            (Ty::Array, "forEach") => {
                Some(("arrayForEach", Ty::Void, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
            }
            (Ty::Array, "map") => {
                Some(("arrayMap", Ty::Array, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
            }
            (Ty::Array, "filter") => {
                Some(("arrayFilter", Ty::Array, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
            }
            (Ty::Array, "find") => {
                Some(("arrayFind", Ty::Any, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
            }
            (Ty::Array, "findIndex") => Some((
                "arrayFindIndex",
                Ty::Int(IntKind::I32),
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::Array, "some") => {
                Some(("arraySome", Ty::Boolean, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
            }
            (Ty::Array, "every") => {
                Some(("arrayEvery", Ty::Boolean, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
            }
            (Ty::Array, "join") => {
                Some(("arrayJoin", Ty::String, vec![Ty::String]))
            }
            (Ty::Array, "shift") => Some(("arrayShift", Ty::Any, vec![])),
            (Ty::Array, "unshift") => {
                Some(("arrayUnshift", Ty::Int(IntKind::U64), vec![Ty::Any]))
            }
            // `at(index)` with relative/undefined semantics (native).
            (Ty::Array, "at") => Some(("arrayAt", Ty::Any, vec![Ty::F64])),
            (Ty::Array, "findLast") => Some((
                "arrayFindLast",
                Ty::Any,
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::Array, "findLastIndex") => Some((
                "arrayFindLastIndex",
                Ty::Int(IntKind::I32),
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::Array, "flatMap") => Some((
                "arrayFlatMap",
                Ty::Array,
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::Array, "group") => Some((
                "arrayGroup",
                Ty::DynamicObject,
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::Array, "groupToMap") => Some((
                "arrayGroupToMap",
                Ty::Map,
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::Array, "keys") => Some(("arrayKeys", Ty::Any, vec![])),
            (Ty::Array, "values") => Some(("arrayValues", Ty::Any, vec![])),
            (Ty::Array, "entries") => Some(("arrayEntries", Ty::Any, vec![])),
            (Ty::Map, "set") => Some(("mapSetChain", Ty::Map, vec![Ty::Any, Ty::Any])),
            (Ty::Map, "get") => Some(("mapGet", Ty::Any, vec![Ty::Any])),
            (Ty::Map, "has") => Some(("mapHas", Ty::Boolean, vec![Ty::Any])),
            (Ty::Map, "size") => Some(("mapSize", Ty::Int(IntKind::U64), vec![])),
            (Ty::Map, "delete") => Some(("mapDelete", Ty::Boolean, vec![Ty::Any])),
            (Ty::Map, "clear") => Some(("mapClear", Ty::Void, vec![])),
            (Ty::Map, "values") => Some(("mapValues", Ty::Any, vec![])),
            (Ty::Map, "keys") => Some(("mapKeys", Ty::Any, vec![])),
            (Ty::Map, "entries") => Some(("mapEntries", Ty::Any, vec![])),
            (Ty::Map, "forEach") => Some((
                "mapForEach",
                Ty::Void,
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::Set, "add") => Some(("setAddChain", Ty::Set, vec![Ty::Any])),
            (Ty::Set, "has") => Some(("setHas", Ty::Boolean, vec![Ty::Any])),
            (Ty::Set, "size") => Some(("setSize", Ty::Int(IntKind::U64), vec![])),
            (Ty::Set, "delete") => Some(("setDelete", Ty::Boolean, vec![Ty::Any])),
            (Ty::Set, "clear") => Some(("setClear", Ty::Void, vec![])),
            (Ty::Set, "values") => Some(("setValues", Ty::Any, vec![])),
            (Ty::Set, "keys") => Some(("setValues", Ty::Any, vec![])),
            (Ty::Set, "entries") => Some(("setEntries", Ty::Any, vec![])),
            (Ty::Set, "forEach") => Some((
                "setForEach",
                Ty::Void,
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::WeakMap, "set") => {
                Some(("weakMapSet", Ty::WeakMap, vec![Ty::Any, Ty::Any]))
            }
            (Ty::WeakMap, "get") => Some(("weakMapGet", Ty::Any, vec![Ty::Any])),
            (Ty::WeakMap, "has") => Some(("weakMapHas", Ty::Boolean, vec![Ty::Any])),
            (Ty::WeakMap, "delete") => Some(("weakMapDelete", Ty::Boolean, vec![Ty::Any])),
            (Ty::WeakSet, "add") => Some(("weakSetAdd", Ty::WeakSet, vec![Ty::Any])),
            (Ty::WeakSet, "has") => Some(("weakSetHas", Ty::Boolean, vec![Ty::Any])),
            (Ty::WeakSet, "delete") => Some(("weakSetDelete", Ty::Boolean, vec![Ty::Any])),
            (Ty::WeakRef, "deref") => Some(("weakRefDeref", Ty::Any, vec![])),
            (Ty::RegExp, "isMatch") => Some(("regexpIsMatch", Ty::Boolean, vec![Ty::String])),
            (Ty::RegExp, "test") => Some(("regexpTest", Ty::Boolean, vec![Ty::String])),
            (Ty::RegExp, "exec") => Some(("regexpExec", Ty::Array, vec![Ty::String])),
            (Ty::RegExp, "source") => Some(("regexpSource", Ty::String, vec![])),
            (Ty::BigInt, "add") => Some(("bigIntAdd", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "sub") => Some(("bigIntSub", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "mul") => Some(("bigIntMul", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "div") => Some(("bigIntDiv", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "rem") => Some(("bigIntRem", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "pow") => Some(("bigIntPow", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "neg") => Some(("bigIntNeg", Ty::BigInt, vec![])),
            (Ty::BigInt, "and") => Some(("bigIntAnd", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "or") => Some(("bigIntOr", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "xor") => Some(("bigIntXor", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "not") => Some(("bigIntNot", Ty::BigInt, vec![])),
            (Ty::BigInt, "shl") => Some(("bigIntShl", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "shr") => Some(("bigIntShr", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "compare") => Some(("bigIntCompare", Ty::Int(IntKind::I32), vec![Ty::BigInt])),
            (Ty::BigInt, "toString") => Some(("bigIntToString", Ty::String, vec![])),
            (Ty::Promise, "state") => Some(("promiseState", Ty::Int(IntKind::U8), vec![])),
            (Ty::Promise, "result") => Some(("promiseResult", Ty::Any, vec![])),
            (Ty::Promise, "isReady") => Some(("promiseIsReady", Ty::Boolean, vec![])),
            (Ty::Promise, "resolve") => Some(("promiseResolve", Ty::Boolean, vec![Ty::Any])),
            (Ty::Promise, "reject") => Some(("promiseReject", Ty::Boolean, vec![Ty::String])),
            (Ty::String, "toInt") => Some(("toInt", Ty::Int(IntKind::I32), vec![])),
            (Ty::String, "toFloat") => Some(("toFloat", Ty::F64, vec![])),
            (Ty::String, "toBoolean") => Some(("toBoolean", Ty::Boolean, vec![])),
            (Ty::String, "reverse") => Some(("reverse", Ty::String, vec![])),
            (Ty::String, "length") => Some(("length", Ty::Int(IntKind::U64), vec![])),
            (Ty::String, "toUpperCase") => Some(("toUpperCase", Ty::String, vec![])),
            (Ty::String, "toLowerCase") => Some(("toLowerCase", Ty::String, vec![])),
            (Ty::String, "trim") => Some(("trim", Ty::String, vec![])),
            (Ty::String, "trimStart") => Some(("trimStart", Ty::String, vec![])),
            (Ty::String, "trimEnd") => Some(("trimEnd", Ty::String, vec![])),
            (Ty::String, "charCodeAt") => {
                Some(("charCodeAt", Ty::Int(IntKind::U32), vec![Ty::Int(IntKind::U64)]))
            }
            (Ty::String, "indexOf") => {
                Some(("indexOf", Ty::Int(IntKind::I32), vec![Ty::String]))
            }
            (Ty::String, "lastIndexOf") => {
                Some(("lastIndexOf", Ty::Int(IntKind::I32), vec![Ty::String]))
            }
            (Ty::String, "substring") => Some((
                "substring",
                Ty::String,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U64)],
            )),
            (Ty::String, "repeat") => {
                Some(("repeat", Ty::String, vec![Ty::Int(IntKind::U64)]))
            }
            (Ty::String, "startsWith") => {
                Some(("startsWith", Ty::Boolean, vec![Ty::String]))
            }
            (Ty::String, "endsWith") => Some(("endsWith", Ty::Boolean, vec![Ty::String])),
            // ECMA-402 locale surface (NUCLEUS #1), ICU4X-backed.
            // `localeCompare` returns the ECMAScript -1/0/1; v1 uses
            // root collation and the root (`und`) locale for casing.
            (Ty::String, "includes") => Some(("includes", Ty::Boolean, vec![Ty::String])),
            // Index is a `number` so a negative value (from the end) can
            // cross the boundary; out of range yields `undefined`.
            (Ty::String, "at") => Some(("at", Ty::String, vec![Ty::F64])),
            (Ty::String, "codePointAt") => {
                Some(("codePointAt", Ty::Int(IntKind::U32), vec![Ty::Int(IntKind::U64)]))
            }
            (Ty::String, "replaceAll") => {
                Some(("replaceAll", Ty::String, vec![Ty::String, Ty::String]))
            }
            (Ty::BufferView(hir::BufferViewKind::DataView), "getUint32") => Some((
                "dataViewGetU32",
                Ty::Int(IntKind::U32),
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setUint32") => Some((
                "dataViewSetU32",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U32), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getUint16") => Some((
                "dataViewGetU16",
                Ty::Int(IntKind::U16),
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setUint16") => Some((
                "dataViewSetU16",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U16), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getFloat64") => Some((
                "dataViewGetF64",
                Ty::F64,
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setFloat64") => Some((
                "dataViewSetF64",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::F64, Ty::Boolean],
            )),
            // The remaining DataView element widths. JS names
            // 64-bit access `getBigInt64`/`getBigUint64` and returns a
            // BigInt; Zun has native 64-bit integers, so these return
            // `i64`/`u64` and are named without the Big.
            (Ty::BufferView(hir::BufferViewKind::DataView), "getInt8") => Some((
                "dataViewGetInt8",
                Ty::Int(IntKind::I8),
                vec![Ty::Int(IntKind::U64)],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setInt8") => Some((
                "dataViewSetInt8",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::I8)],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getUint8") => Some((
                "dataViewGetUint8",
                Ty::Int(IntKind::U8),
                vec![Ty::Int(IntKind::U64)],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setUint8") => Some((
                "dataViewSetUint8",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U8)],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getInt16") => Some((
                "dataViewGetInt16",
                Ty::Int(IntKind::I16),
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setInt16") => Some((
                "dataViewSetInt16",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::I16), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getInt32") => Some((
                "dataViewGetInt32",
                Ty::Int(IntKind::I32),
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setInt32") => Some((
                "dataViewSetInt32",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::I32), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getFloat32") => Some((
                "dataViewGetFloat32",
                Ty::F32,
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setFloat32") => Some((
                "dataViewSetFloat32",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::F32, Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getInt64") => Some((
                "dataViewGetInt64",
                Ty::Int(IntKind::I64),
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setInt64") => Some((
                "dataViewSetInt64",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::I64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getUint64") => Some((
                "dataViewGetUint64",
                Ty::Int(IntKind::U64),
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setUint64") => Some((
                "dataViewSetUint64",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "byteLength") => Some((
                "dataViewByteLength",
                Ty::Int(IntKind::U64),
                vec![],
            )),
            // `byteOffset`/`buffer`: the view's position in its
            // backing buffer and a reference to that `ArrayBuffer`.
            (Ty::BufferView(hir::BufferViewKind::DataView), "byteOffset") => Some((
                "dataViewByteOffset",
                Ty::Int(IntKind::U64),
                vec![],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "buffer") => Some((
                "dataViewBuffer",
                Ty::Buffer,
                vec![],
            )),
            // `ArrayBuffer`, which is what `Ty::Buffer` denotes. These
            // are methods rather than the properties JS has, matching
            // `String.length()` and `Map.size()` here; the divergence
            // is recorded in NUCLEUS rather than special-cased.
            (Ty::Buffer, "byteLength") => {
                Some(("bufferLength", Ty::Int(IntKind::U64), vec![]))
            }
            (Ty::Buffer, "slice") => Some((
                "bufferSlice",
                Ty::Buffer,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U64)],
            )),
            // ES2024 resizable ArrayBuffer. As with `byteLength`, these
            // JS properties are methods here (the divergence is in
            // NUCLEUS). `resize` returns a boolean rather than JS's
            // `undefined`; a rejected resize is a runtime error.
            (Ty::Buffer, "maxByteLength") => {
                Some(("bufferMaxByteLength", Ty::Int(IntKind::U64), vec![]))
            }
            (Ty::Buffer, "resizable") => {
                Some(("bufferResizable", Ty::Boolean, vec![]))
            }
            (Ty::Buffer, "detached") => {
                Some(("bufferDetached", Ty::Boolean, vec![]))
            }
            (Ty::Buffer, "resize") => Some((
                "bufferResize",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64)],
            )),
            _ => None,
        });
        if let Some((native_name, return_ty, expected)) = managed_method {
            // A callback method's optional `thisArg` is bound onto the
            // callback (`cb.bind(thisArg)`) before the native runs.
            let undefined_ast = ast::Expr::Unary { op: ast::UnaryOp::Void, operand: Box::new(ast::Expr::Number { value: 0.0, span }), span };
            let this_arg_ast = if matches!(receiver.ty, Ty::Array | Ty::Map | Ty::Set)
                && expected.len() == 1
                && args.len() == 2
                && expected[0] == Ty::Function(hir::DYNAMIC_FUNCTION)
            {
                args.get(1)
            } else if matches!(receiver.ty, Ty::Array | Ty::Map | Ty::Set)
                && expected.len() == 1
                && args.len() == 1
                && expected[0] == Ty::Function(hir::DYNAMIC_FUNCTION)
                && self.ast_function_takes_this(&args[0])
            {
                // A callback with an explicit `this` parameter and no thisArg
                // is called with `this` undefined.
                Some(&undefined_ast)
            } else {
                None
            };
            let args = if matches!(receiver.ty, Ty::Array | Ty::Map | Ty::Set)
                && expected.len() == 1
                && args.len() == 2
                && expected[0] == Ty::Function(hir::DYNAMIC_FUNCTION)
            {
                &args[..1]
            } else {
                args
            };
            if args.len() != expected.len() {
                self.error(
                    span,
                    format!(
                        "`{mname}` expects {} argument(s), found {}",
                        expected.len(),
                        args.len()
                    ),
                );
                return self.poison(span);
            }
            let receiver_is_string = receiver.ty == Ty::String;
            let receiver_is_regexp = receiver.ty == Ty::RegExp;
            let mut call_args = vec![receiver];
            let callback_receiver = call_args[0].ty == Ty::Array;
            for (arg, expected_ty) in args.into_iter().zip(expected) {
                // A JS callback `(value, index, collection)` has dynamic
                // parameters, so hint unannotated callback params to
                // `any` (whether the receiver is an Array, Map, or Set).
                let _ = callback_receiver;
                if expected_ty == Ty::Function(hir::DYNAMIC_FUNCTION) {
                    self.closure_param_hint = Some(vec![Ty::Any, Ty::Any, Ty::Any]);
                }
                let mut value = self.check_expr(cx, arg);
                self.closure_param_hint = None;
                if expected_ty == Ty::Function(hir::DYNAMIC_FUNCTION) && matches!(value.ty, Ty::Function(_)) {
                    if let (Some(this_ast), Some((bind_id, _))) =
                        (this_arg_ast, zun_natives::resolve("collections", "functionBind"))
                    {
                        let checked = self.check_expr(cx, this_ast);
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
                // `re.exec(x)` / `re.test(x)` run ToString on a non-string subject.
                if receiver_is_regexp
                    && expected_ty == Ty::String
                    && value.ty != Ty::String
                    && !value.ty.is_error()
                {
                    value = self.to_string_for_parse(value);
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
                // A function value flowing into an `any` parameter
                // (`set.add(fn)`) must be boxed like any other value.
                if expected_ty == Ty::Any && matches!(value.ty, Ty::Function(_)) {
                    value = Self::box_to_any(value);
                }
                // `any` flowing into a numeric array-method parameter runs ToNumber.
                if callback_receiver && expected_ty == Ty::F64 && value.ty == Ty::Any {
                    value = self.to_number_expr(value);
                }
                call_args.push(value);
            }
            let module = if receiver_is_string {
                "string"
            } else if native_name.starts_with("dataView")
                || native_name.starts_with("buffer")
                // The typed-array natives, by the prefixes the master
                // table gives them. Matching the method suffix
                // instead (`ArrayGet`, `ArraySet`) missed
                // `u32ArrayLength` and friends, which then resolved
                // against `zun:collections` and were reported
                // missing; a hand-written list of prefixes missed
                // every kind added after it was written.
                || crate::typed_arrays::NATIVE_PREFIXES
                    .iter()
                    .any(|prefix| native_name.starts_with(prefix))
            {
                "buffer"
            } else if native_name.starts_with("promise") {
                "promise"
            } else if matches!(
                native_name,
                "arrayAt"
                    | "arrayFindLast"
                    | "arrayFindLastIndex"
                    | "arrayFlat"
                    | "arrayFlatMap"
                    | "arrayGroup"
                    | "arrayGroupToMap"
                    | "arrayKeys"
                    | "arrayValues"
                    | "arrayEntries"
                    | "arraySpliceInsert"
                    | "arrayReduceNoInit"
                    | "arrayReduceRightNoInit"
                    | "arrayIsArray"
                    | "arrayFrom"
                    | "arrayFromMapped"
            ) {
                "array"
            } else {
                "collections"
            };
            let Some((id, _)) = zun_natives::resolve(module, native_name) else {
                self.error(
                    span,
                    format!("missing collection intrinsic `{native_name}`"),
                );
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: call_args,
                },
                ty: return_ty,
                span,
            };
        }
        if let Ty::Function(func) = receiver.ty {
            return self.lower_function_reflective_call(
                cx, receiver, &mname, func, args, span, method.span,
            );
        }
        // JS-conformant `toFixed` / `toExponential` / `toPrecision` /
        // `toString(radix)`: the raw argument is passed as `any` and
        // coerced, range-checked (managed `RangeError`) and formatted
        // exactly on the interpreter floor.
        if receiver.ty == Ty::F64
            && matches!(
                mname.as_str(),
                "toFixed" | "toExponential" | "toPrecision" | "toString"
            )
            && args.len() <= 1
        {
            let native_name = match mname.as_str() {
                "toFixed" => "numberToFixedJs",
                "toExponential" => "numberToExponentialJs",
                "toPrecision" => "numberToPrecisionJs",
                _ => "numberToStringRadixJs",
            };
            let Some((id, _)) = zun_natives::resolve("number", native_name) else {
                self.error(span, format!("missing Number intrinsic `{native_name}`"));
                return self.poison(span);
            };
            let arg = match args.first() {
                Some(arg) => {
                    let value = self.check_expr(cx, arg);
                    Self::box_to_any(value)
                }
                None => hir::Expr {
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
                },
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, arg] },
                ty: Ty::String,
                span,
            };
        }
        // `number.toString(radix?)` — base-`radix` (2..=36) rendering,
        // default 10. Intercepted before class dispatch so it supersedes
        // the no-arg extern `toString` for the radix form and the
        // no-arg form alike.
        if receiver.ty == Ty::F64 && mname == "toString" && args.len() <= 1 {
            let Some((id, _)) = zun_natives::resolve("number", "toStringRadix") else {
                self.error(span, "missing Number intrinsic `toStringRadix`");
                return self.poison(span);
            };
            let radix = match args.first() {
                Some(arg) => {
                    let mut radix = self.check_expr(cx, arg);
                    coerce_numeric_literal(&mut radix, Ty::Int(IntKind::I32));
                    if !radix.ty.is_error()
                        && !self.is_assignable(radix.ty, Ty::Int(IntKind::I32))
                    {
                        self.error(radix.span, "`toString` radix must be an integer");
                    }
                    radix
                }
                None => hir::Expr {
                    kind: hir::ExprKind::Number(10.0),
                    ty: Ty::Int(IntKind::I32),
                    span,
                },
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, radix] },
                ty: Ty::String,
                span,
            };
        }
        // `number.toExponential(fractionDigits?)` /
        // `number.toPrecision(precision?)` — instance formatting,
        // intercepted before class dispatch like `toString(radix?)`.
        // The optional count is lowered to an out-of-range sentinel for
        // the native: `-1` ("absent") for `toExponential`, `0` for
        // `toPrecision`.
        if receiver.ty == Ty::F64
            && matches!(mname.as_str(), "toExponential" | "toPrecision")
            && args.len() <= 1
        {
            let native = if mname == "toExponential" {
                "toExponential"
            } else {
                "toPrecision"
            };
            let sentinel = if mname == "toExponential" { -1.0 } else { 0.0 };
            let Some((id, _)) = zun_natives::resolve("number", native) else {
                self.error(span, format!("missing Number intrinsic `{native}`"));
                return self.poison(span);
            };
            let count = match args.first() {
                Some(arg) => {
                    let mut count = self.check_expr(cx, arg);
                    coerce_numeric_literal(&mut count, Ty::Int(IntKind::I32));
                    if !count.ty.is_error()
                        && !self.is_assignable(count.ty, Ty::Int(IntKind::I32))
                    {
                        self.error(
                            count.span,
                            format!("`{mname}` argument must be an integer"),
                        );
                    }
                    count
                }
                None => hir::Expr {
                    kind: hir::ExprKind::Number(sentinel),
                    ty: Ty::Int(IntKind::I32),
                    span,
                },
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, count] },
                ty: Ty::String,
                span,
            };
        }
        // `number.toFixed(digits?)` — fixed-point formatting, digits
        // default 0 (range 0..=100 enforced by the native). Hand-lowered
        // before class dispatch like `toExponential`/`toPrecision`.
        if receiver.ty == Ty::F64 && mname == "toFixed" && args.len() <= 1 {
            let Some((id, _)) = zun_natives::resolve("number", "toFixed") else {
                self.error(span, "missing Number intrinsic `toFixed`");
                return self.poison(span);
            };
            let digits = match args.first() {
                Some(arg) => {
                    let mut digits = self.check_expr(cx, arg);
                    coerce_numeric_literal(&mut digits, Ty::Int(IntKind::I32));
                    if !digits.ty.is_error()
                        && !self.is_assignable(digits.ty, Ty::Int(IntKind::I32))
                    {
                        self.error(digits.span, "`toFixed` argument must be an integer");
                    }
                    digits
                }
                None => hir::Expr {
                    kind: hir::ExprKind::Number(0.0),
                    ty: Ty::Int(IntKind::I32),
                    span,
                },
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, digits] },
                ty: Ty::String,
                span,
            };
        }
        // Optional trailing BCP-47 `locale` (ECMA-402) for the root-locale
        // `toLocale*` family. Hand-lowered because `managed_method` is
        // fixed-arity: an omitted locale is the sentinel `""` (root).
        if (receiver.ty == Ty::String
            && matches!(mname.as_str(), "localeCompare" | "toLocaleLowerCase" | "toLocaleUpperCase"))
            || (receiver.ty == Ty::F64 && mname == "toLocaleString")
        {
            let (module, native, leading) = match (receiver.ty, mname.as_str()) {
                (Ty::String, "localeCompare") => ("string", "localeCompare", 1usize),
                (Ty::String, "toLocaleLowerCase") => ("string", "toLocaleLowerCase", 0),
                (Ty::String, _) => ("string", "toLocaleUpperCase", 0),
                _ => ("number", "intlNumberFormat", 0),
            };
            if args.len() < leading || args.len() > leading + 1 {
                self.error(
                    span,
                    format!("`{mname}` expects {leading} or {} argument(s), found {}", leading + 1, args.len()),
                );
                return self.poison(span);
            }
            let mut call_args = Vec::new();
            if leading == 1 {
                let that = self.check_expr(cx, &args[0]);
                if !that.ty.is_error() && that.ty != Ty::String {
                    self.error(that.span, "`localeCompare` argument must be a string");
                }
                call_args.push(that);
            }
            let locale = match args.get(leading) {
                Some(arg) => {
                    let checked = self.check_expr(cx, arg);
                    if !checked.ty.is_error() && checked.ty != Ty::String {
                        self.error(checked.span, format!("`{mname}` locale must be a string"));
                    }
                    checked
                }
                None => hir::Expr {
                    kind: hir::ExprKind::String(String::new()),
                    ty: Ty::String,
                    span,
                },
            };
            let Some((id, _)) = zun_natives::resolve(module, native) else {
                self.error(span, format!("missing intrinsic `{native}`"));
                return self.poison(span);
            };
            // `intlNumberFormat(locale, n)` takes the locale first; the
            // string natives take `(receiver, [that,] locale)`.
            let mut all = Vec::new();
            if receiver.ty == Ty::F64 {
                all.push(locale);
                all.push(receiver);
            } else {
                all.push(receiver);
                all.extend(call_args);
                all.push(locale);
            }
            let ty = if native == "localeCompare" { Ty::Int(IntKind::I32) } else { Ty::String };
            return hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: all }, ty, span };
        }
        // `number.valueOf()` / `string.valueOf()` — identity, returning
        // the primitive itself. No native needed: the checked receiver
        // expression already is the value.
        if matches!(receiver.ty, Ty::F64 | Ty::String | Ty::BigInt)
            && mname == "valueOf"
            && args.is_empty()
        {
            return receiver;
        }
        // `String.prototype.matchAll(regexp)` — all matches of a global
        // regexp as an array of match arrays (each like `exec`). The `g`
        // flag is required (checked natively); the array-of-arrays shape
        // is the same pragmatic choice as `keys()/values()/entries()`
        // (no distinct iterator object yet).
        if receiver.ty == Ty::String && mname == "matchAll" && args.len() == 1 {
            let first = self.check_expr(cx, &args[0]);
            if first.ty.is_error() {
                return self.poison(span);
            }
            // Every argument shape (RegExp, string, any, object carrying
            // `Symbol.matchAll`) goes through the runtime dispatch, which
            // returns the lazy match iterator.
            let first = Self::box_to_any(first);
            return self.string_symbol_dispatch(span, 4.0, receiver, first, None);
        }
        // `boolean.toString()` → `"true"` / `"false"`. Intercepted
        // before class dispatch, like `number.toString(radix?)`, since
        // the primitive `boolean` has no backing class.
        if receiver.ty == Ty::Boolean && mname == "toString" && args.is_empty() {
            let Some((id, _)) = zun_natives::resolve("boolean", "booleanToString") else {
                self.error(span, "missing Boolean intrinsic `booleanToString`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver] },
                ty: Ty::String,
                span,
            };
        }
        // String/Number/Array are primitives with no concatenated class
        // (see PRELUDE_CLASSES): resolve a method not already handled by
        // the tables above directly against its native group by name —
        // covering every native method without re-listing each.
        let primitive_group = match receiver.ty {
            Ty::String => Some(("string", "String")),
            Ty::F64 => Some(("number", "Number")),
            Ty::Array => Some(("array", "Array")),
            _ => None,
        };
        if receiver.ty == Ty::Array
            && matches!(
                mname.as_str(),
                "slice" | "indexOf" | "lastIndexOf" | "includes" | "reverse"
            )
            && !self.classes_by_name.contains_key("Array")
        {
            if mname == "reverse" {
                let Some((id, _)) = zun_natives::resolve("array", "arrayReverse") else {
                    self.error(span, "missing array intrinsic `arrayReverse`");
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver] },
                    ty: Ty::Array,
                    span,
                };
            }
            let native_name = match mname.as_str() {
                "slice" => "arraySliceRange",
                "indexOf" => "arrayIndexOf",
                "lastIndexOf" => "arrayLastIndexOf",
                _ => "arrayIncludes",
            };
            let Some((id, native)) = zun_natives::resolve("array", native_name) else {
                self.error(span, format!("missing array intrinsic `{native_name}`"));
                return self.poison(span);
            };
            let is_slice = mname == "slice";
            let max_args = if is_slice { 2 } else { 2 };
            let min_args = if is_slice { 0 } else { 1 };
            if args.len() < min_args || args.len() > max_args {
                self.error(span, format!("`{mname}` called with a wrong number of arguments"));
                return self.poison(span);
            }
            let mut call_args = vec![receiver];
            let mut rest = args.iter();
            if !is_slice {
                let needle = self.check_expr(cx, rest.next().expect("needle"));
                call_args.push(Self::box_to_any(needle));
            }
            let defaults = if is_slice {
                [0.0, f64::INFINITY]
            } else if mname == "lastIndexOf" {
                [f64::INFINITY, 0.0]
            } else {
                [0.0, 0.0]
            };
            let wanted = native.params.len() - call_args.len();
            for position in 0..wanted {
                let value = match rest.next() {
                    Some(arg) => {
                        let mut value = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut value, Ty::F64);
                        self.to_number_for_arg(value)
                    }
                    None => hir::Expr {
                        kind: hir::ExprKind::Number(defaults[position]),
                        ty: Ty::F64,
                        span,
                    },
                };
                call_args.push(value);
            }
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                ty: native_ty(native.ret),
                span,
            };
        }
        if let Some((group, class_name)) = primitive_group {
            if !self.classes_by_name.contains_key(class_name) {
                // `string`/`number` natives are bare (`toUpperCase`);
                // `array` natives carry an `array` prefix (`arraySlice`).
                let native_name = if group == "array" {
                    let mut chars = mname.chars();
                    match chars.next() {
                        Some(first) => format!("array{}{}", first.to_ascii_uppercase(), chars.as_str()),
                        None => mname.clone(),
                    }
                } else {
                    mname.clone()
                };
                if let Some((id, native)) = zun_natives::resolve(group, &native_name) {
                    if native.params.len() == args.len() + 1 {
                        let mut call_args = vec![receiver];
                        for (arg, expected) in args
                            .iter()
                            .zip(native.params[1..].iter().map(|t| native_ty(*t)))
                        {
                            let mut value = self.check_expr(cx, arg);
                            coerce_numeric_literal(&mut value, expected);
                            call_args.push(value);
                        }
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                            ty: native_ty(native.ret),
                            span,
                        };
                    }
                }
                self.error(
                    method.span,
                    format!("no method `{mname}` on {class_name}"),
                );
                return self.poison(span);
            }
        }
        let class = match receiver.ty {
            Ty::String => self.classes_by_name["String"],
            Ty::F64 => self.classes_by_name["Number"],
            Ty::Array => self.classes_by_name["Array"],
            Ty::Instance(c) => c,
            Ty::Error => return self.poison(span),
            other => {
                self.error(receiver.span, format!("type {other:?} has no methods"));
                return self.poison(span);
            }
        };
        // `addEventListener` / `removeEventListener` accept either a
        // function or a `{ handleEvent }` object. A function cannot be
        // stored in `any`, so the object form routes to a sibling
        // `<name>Object` method taking an `any` listener.
        let mut mname = mname;
        if matches!(mname.as_str(), "addEventListener" | "removeEventListener")
            && args.len() >= 2
            && !matches!(args[1], ast::Expr::Function(_))
            && self.method_of(class, &format!("{mname}Object")).is_some()
        {
            let probe = self.check_expr(cx, &args[1]);
            if matches!(probe.ty, Ty::DynamicObject | Ty::Any | Ty::Instance(_)) {
                mname = format!("{mname}Object");
            }
        }
        // Private methods (`#m`) bind lexically to the class that declares
        // the call site, so a same-named subclass `#m` never overrides them.
        let mut private_direct = false;
        let lexical = match cx.this_class {
            Some(tc) if mname.starts_with('#') && self.is_subclass(class, tc) => {
                self.method_of(tc, &mname).inspect(|_| private_direct = true)
            }
            _ => None,
        };
        let Some(func) = lexical.or_else(|| self.method_of(class, &mname)) else {
            self.error(method.span, format!("no method `{mname}` on this type"));
            return self.poison(span);
        };
        // Static signatures omit the implicit receiver. Reaching this
        // instance-call path with that arity means `value.method()`
        // attempted to invoke a static member through an object.
        if self.signatures[func as usize].is_static {
            self.error(
                method.span,
                format!("static method `{mname}` must be called through its class"),
            );
            return self.poison(span);
        }
        if let Some(ast::Expr::Spread { expr, .. }) = args.last() {
            let rest_index = self.signatures[func as usize].rest_param;
            let fixed_count = args.len().saturating_sub(1);
            if rest_index == Some(fixed_count + 1) {
                let mut fixed_args = Vec::with_capacity(fixed_count);
                for argument in &args[..fixed_count] {
                    fixed_args.push(self.check_expr(cx, argument));
                }
                let array = self.check_expr(cx, expr);
                let array = self.spread_source_to_array(array);
                if array.ty != Ty::Array && !array.ty.is_error() {
                    self.error(array.span, "spread method call requires an Array value");
                    return self.poison(span);
                }
                let params = self.signatures[func as usize].params.clone();
                for (argument, &expected) in fixed_args.iter_mut().zip(params.iter().skip(1)) {
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
                        hir::ExprKind::MethodCallRestArray {
                            func,
                            receiver: Box::new(receiver),
                            args: Box::new(array),
                        }
                    } else {
                        hir::ExprKind::MethodCallRestArrayFixed {
                            func,
                            receiver: Box::new(receiver),
                            fixed_args,
                            args: Box::new(array),
                        }
                    },
                    ty: return_ty,
                    span,
                };
            }
        }
        let (param_tys, return_ty, optional_count, signature_name) = {
            let sig = &self.signatures[func as usize];
            (
                sig.params.clone(),
                sig.return_ty,
                sig.optional_count,
                sig.name.clone(),
            )
        };
        // param_tys[0] is `this`; the remaining are the declared params.
        let mut call_args = vec![receiver];
        let listener_hint = if mname == "addEventListener" {
            self.classes_by_name.get("Event").map(|&event| Ty::Instance(event))
        } else {
            None
        };
        for (arg_index, a) in args.iter().enumerate() {
            // Contextual typing: an unannotated listener closure
            // receives the `Event`.
            if arg_index == 1 && matches!(a, ast::Expr::Function(_)) {
                if let Some(event_ty) = listener_hint {
                    self.closure_param_hint = Some(vec![event_ty]);
                }
            }
            call_args.push(self.check_expr(cx, a));
            self.closure_param_hint = None;
        }
        let sig = Signature {
            name: signature_name,
            params: param_tys.clone(),
            return_ty,
            optional_count,
            rest_param: self.signatures[func as usize].rest_param,
            is_generator: self.signatures[func as usize].is_generator,
            is_static: false,
        };
        let rest_index = sig.rest_param;
        if rest_index.is_some() && call_args.len() < rest_index.unwrap() {
            self.error(
                span,
                format!("`{mname}` expects at least {} argument(s)", rest_index.unwrap() - 1),
            );
        } else if rest_index.is_none()
            && (call_args.len() < param_tys.len().saturating_sub(optional_count)
                || call_args.len() > param_tys.len())
        {
            self.error(
                span,
                format!(
                    "`{mname}` expects {} to {} argument(s), found {}",
                    param_tys.len() - 1 - optional_count,
                    param_tys.len() - 1,
                    call_args.len() - 1
                ),
            );
        } else {
            if rest_index.is_none() {
                self.pad_default_args(&mut call_args, &sig, span);
            }
            for (arg, &expected) in call_args
                .iter_mut()
                .zip(&param_tys)
                .skip(1)
                .take(rest_index.unwrap_or(param_tys.len()) - 1)
            {
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
        if rest_index.is_none() {
            if let Some(&owner) = self.routed_methods.get(&func) {
                if let Some(routed) =
                    self.routed_method_call(func, owner, &mut call_args, return_ty, span)
                {
                    return routed;
                }
            }
        }
        hir::Expr {
            kind: if rest_index.is_some() {
                hir::ExprKind::MethodCallRest { func, args: call_args }
            } else if private_direct {
                hir::ExprKind::StaticCall { func, args: call_args }
            } else {
                hir::ExprKind::MethodCall { func, args: call_args }
            },
            ty: return_ty,
            span,
        }
    }
}
