//! Identifier-receiver method calls for `check_method_call` (see
//! `method_call.rs` and `mcall_statics.rs`).
//!
//! When the receiver of a method call is a bare identifier, it may name a
//! namespace (`Math`, `Object`, `Reflect`), a constructor with statics
//! (`Array`, `Promise`, a typed array), an imported module namespace, or a
//! user class exposing a static method. `try_mcall_ident_receiver` dispatches
//! across these in the original order; each helper returns `Some` when it
//! handles the call.

use crate::check::*;

impl Checker<'_> {
    /// Method calls whose receiver is a bare identifier naming a namespace or
    /// constructor (`Math`, `Object`, `console`, `Array`, a `zun:core`
    /// namespace, an imported module, or a user class's static method).
    pub(in crate::check) fn try_mcall_ident_receiver(
        &mut self,
        cx: &mut FunctionCx,
        receiver: &ast::Expr,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        let ast::Expr::Ident(class) = receiver else {
            return None;
        };
        let class_name = self.text(class).to_string();
        if let Some(expr) = self.mcall_builtin_statics_1(cx, &class_name, method, args, span) {
            return Some(expr);
        }
        if let Some(expr) = self.mcall_builtin_statics_2(cx, &class_name, method, args, span) {
            return Some(expr);
        }
        if let Some(expr) = self.mcall_import_namespaces(cx, &class_name, method, args, span) {
            return Some(expr);
        }
        self.mcall_user_static(cx, &class_name, method, args, span)
    }

    /// Identifier-receiver statics, first group: `super`, `Map.groupBy`,
    /// `Object.*`, `console.*`, `Reflect.*`, `Buffer.*`, `Number.*`,
    /// `Math.*`, and `String.fromCharCode`/`fromCodePoint`.
    fn mcall_builtin_statics_1(
        &mut self,
        cx: &mut FunctionCx,
        class_name: &str,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        if class_name == "super" {
            return Some(self.check_super_call(cx, Some(method), args, span));
        }
        if class_name == "Map"
            && self.text(method) == "groupBy"
            && !self.classes_by_name.contains_key("Map")
            && args.len() == 2
        {
            if let Some(&func) = self.by_name.get("__zunMapGroupBy") {
                let items = self.check_expr(cx, &args[0]);
                let callback = self.check_expr(cx, &args[1]);
                return Some(hir::Expr {
                    kind: hir::ExprKind::Call {
                        func,
                        args: vec![Self::box_to_any(items), callback],
                    },
                    ty: Ty::Map,
                    span,
                });
            }
        }
        if class_name == "Object" && !self.classes_by_name.contains_key("Object") {
            return Some(self.check_object_static(
                cx,
                &self.text(method).to_string(),
                method.span,
                args,
                span,
            ));
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
                return Some(result);
            }
        }
        // `Reflect.*` reflection statics (`get set has deleteProperty
        // ownKeys`). Surface-only namespace, recognized by receiver
        // name; a program may still declare its own `Reflect` class.
        if class_name == "Reflect" && !self.classes_by_name.contains_key("Reflect") {
            return Some(self.check_reflect_static(
                cx,
                &self.text(method).to_string(),
                method.span,
                args,
                span,
            ));
        }
        // `Buffer.*` statics (`from alloc allocUnsafe concat
        // isBuffer byteLength`). Recognized by receiver name; a
        // program may still declare its own `Buffer` class.
        if class_name == "Buffer" && !self.classes_by_name.contains_key("Buffer") {
            return Some(self.check_buffer_static(
                cx,
                &self.text(method).to_string(),
                method.span,
                args,
                span,
            ));
        }
        // `Number.*` statics (`isNaN isFinite isInteger isSafeInteger
        // parseFloat parseInt`). Unlike `Object`, the `Number` class
        // exists (its instance helpers), so recognition keys on the
        // receiver name alone — a value never names `Number`.
        if class_name == "Number" {
            return Some(self.check_number_static(
                cx,
                &self.text(method).to_string(),
                method.span,
                args,
                span,
            ));
        }
        // `Math.<fn>(...)` static math, routed to the `math` native
        // group's f64 helpers. Recognized by receiver name (a value
        // never names `Math`); a program may still declare its own.
        if class_name == "Math" && !self.classes_by_name.contains_key("Math") {
            return Some(self.check_math_static(
                cx,
                &self.text(method).to_string(),
                method.span,
                args,
                span,
            ));
        }
        // `String.fromCharCode(...)` / `String.fromCodePoint(...)`.
        // Recognized by receiver name only for the known statics;
        // anything else on `String` falls through to class dispatch.
        if class_name == "String" && matches!(self.text(method), "fromCharCode" | "fromCodePoint") {
            return Some(self.check_string_static(
                cx,
                &self.text(method).to_string(),
                method.span,
                args,
                span,
            ));
        }
        None
    }

    /// Identifier-receiver statics, second group: `Array.isArray`/`of`/`from`,
    /// `Symbol.for`/`keyFor`, `BigInt.asIntN`/`asUintN`, `Promise.*`,
    /// `ArrayBuffer.isView`, and the typed-array `from`/`of`.
    fn mcall_builtin_statics_2(
        &mut self,
        cx: &mut FunctionCx,
        class_name: &str,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        // `Array.isArray(x)` / `Array.of(...)` / `Array.from(src,
        // mapFn?)`. Recognized by receiver name for the known statics
        // (the `Array` class exists for its instance surface, like
        // `Number`); anything else on `Array` falls through.
        if class_name == "Array" && matches!(self.text(method), "isArray" | "of" | "from") {
            return Some(self.check_array_static(
                cx,
                &self.text(method).to_string(),
                method.span,
                args,
                span,
            ));
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
                    return Some(self.poison(span));
                }
            };
            if args.len() != 1 {
                self.error(span, format!("`Symbol.{member}` expects one argument"));
                return Some(self.poison(span));
            }
            let Some((id, _)) = zun_natives::resolve("collections", native) else {
                self.error(span, format!("missing `{native}` intrinsic"));
                return Some(self.poison(span));
            };
            let mut arg = self.check_expr(cx, &args[0]);
            if !arg.ty.is_error() && !self.is_assignable(arg.ty, arg_ty) {
                self.error(
                    arg.span,
                    format!("`Symbol.{member}` expects a {arg_ty:?} argument"),
                );
            }
            coerce_numeric_literal(&mut arg, arg_ty);
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![arg],
                },
                ty: ret_ty,
                span,
            });
        }
        // `BigInt.asIntN(bits, value)` / `BigInt.asUintN(bits,
        // value)`. The `BigInt` class exists (instance helpers), so
        // recognition keys on the receiver name and method pair; any
        // other member falls through to class dispatch.
        if class_name == "BigInt" && matches!(self.text(method), "asIntN" | "asUintN") {
            let member = self.text(method).to_string();
            let native = if member == "asIntN" {
                "bigIntAsIntN"
            } else {
                "bigIntAsUintN"
            };
            if args.len() != 2 {
                self.error(span, format!("`BigInt.{member}` expects two arguments"));
                return Some(self.poison(span));
            }
            let Some((id, _)) = zun_natives::resolve("collections", native) else {
                self.error(span, format!("missing `{native}` intrinsic"));
                return Some(self.poison(span));
            };
            let mut bits = self.check_expr(cx, &args[0]);
            coerce_numeric_literal(&mut bits, Ty::Int(IntKind::I64));
            if bits.ty == Ty::F64 {
                // A `number` width is truncated (ToIndex).
                let bits_span = bits.span;
                bits = hir::Expr {
                    kind: hir::ExprKind::Convert {
                        value: Box::new(bits),
                    },
                    ty: Ty::Int(IntKind::I64),
                    span: bits_span,
                };
            }
            if !bits.ty.is_error() && !bits.ty.is_int() {
                self.error(
                    bits.span,
                    format!("`BigInt.{member}` bit width must be an integer"),
                );
            }
            let value = self.check_expr(cx, &args[1]);
            if !value.ty.is_error() && value.ty != Ty::BigInt {
                self.error(
                    value.span,
                    format!("`BigInt.{member}` expects a BigInt value"),
                );
            }
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![bits, value],
                },
                ty: Ty::BigInt,
                span,
            });
        }
        // `Promise.*` statics (`resolve reject all allSettled race
        // any`). Recognized by receiver name when no user class
        // shadows `Promise`; a Promise *value* is dispatched by type.
        if class_name == "Promise" && !self.classes_by_name.contains_key("Promise") {
            return Some(self.check_promise_static(
                cx,
                &self.text(method).to_string(),
                method.span,
                args,
                span,
            ));
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
                self.error(
                    span,
                    "`ArrayBuffer.isView` expects one argument".to_string(),
                );
                return Some(self.poison(span));
            }
            let Some((id, _)) = zun_natives::resolve("buffer", "isView") else {
                self.error(span, "missing `isView` intrinsic".to_string());
                return Some(self.poison(span));
            };
            let mut arg = self.check_expr(cx, &args[0]);
            if arg.ty != Ty::Any && !arg.ty.is_error() {
                arg = hir::Expr {
                    span: arg.span,
                    kind: hir::ExprKind::BoxAny {
                        value: Box::new(arg),
                    },
                    ty: Ty::Any,
                };
            }
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![arg],
                },
                ty: Ty::Boolean,
                span,
            });
        }
        // `Int32Array.from(src, mapFn?)` / `Int32Array.of(...items)`
        // and the other ten kinds. Recognized by the constructor
        // name when no user class shadows it; the target element
        // kind travels to the kind-agnostic native as a table-order
        // discriminant argument.
        if let Some(view_kind) = crate::typed_arrays::view_kind(class_name) {
            if matches!(self.text(method), "from" | "of")
                && !self.classes_by_name.contains_key(class_name)
            {
                return Some(self.check_typed_array_static(
                    cx,
                    view_kind,
                    &self.text(method).to_string(),
                    args,
                    span,
                ));
            }
        }
        None
    }

    /// Identifier-receiver calls into an imported namespace: a `zun:core`
    /// namespace, a native-module namespace import, or an external
    /// (FFI) namespace function.
    fn mcall_import_namespaces(
        &mut self,
        cx: &mut FunctionCx,
        class_name: &str,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        if let Some(&namespace) = self
            .core_namespaces
            .get(class_name)
            .filter(|_| cx.lookup(class_name).is_none())
        {
            let Some((id, native)) = zun_natives::resolve_core(namespace, self.text(method)) else {
                self.error(
                    method.span,
                    format!(
                        "`zun:core` namespace `{}` has no member `{}`",
                        namespace,
                        self.text(method)
                    ),
                );
                return Some(self.poison(span));
            };
            if native.params.len() != args.len() {
                self.error(
                    span,
                    format!(
                        "`{}` expects {} argument(s), found {}",
                        self.text(method),
                        native.params.len(),
                        args.len()
                    ),
                );
                return Some(self.poison(span));
            }
            let mut checked = Vec::with_capacity(args.len());
            for (arg, expected) in args
                .iter()
                .zip(native.params.iter().map(|ty| native_ty(*ty)))
            {
                let mut value = self.check_expr(cx, arg);
                coerce_numeric_literal(&mut value, expected);
                if !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                    self.error(
                        value.span,
                        format!("argument type {:?} does not match {expected:?}", value.ty),
                    );
                }
                checked.push(value);
            }
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: checked },
                ty: native_ty(native.ret),
                span,
            });
        }
        if let Some(module) = self.namespace_imports.get(class_name).cloned() {
            if zun_natives::module(&module).is_none() {
                // Source-module namespaces are handled below.
            } else {
                let Some((id, native)) = zun_natives::resolve(&module, self.text(method)) else {
                    self.error(
                        method.span,
                        format!("`{}` has no export named `{}`", module, self.text(method)),
                    );
                    return Some(self.poison(span));
                };
                if native.params.len() != args.len() {
                    self.error(
                        span,
                        format!(
                            "`{}` expects {} argument(s), found {}",
                            self.text(method),
                            native.params.len(),
                            args.len()
                        ),
                    );
                    return Some(self.poison(span));
                }
                let mut checked = Vec::with_capacity(args.len());
                for (arg, expected) in args
                    .iter()
                    .zip(native.params.iter().map(|ty| native_ty(*ty)))
                {
                    let mut value = self.check_expr(cx, arg);
                    coerce_numeric_literal(&mut value, expected);
                    if !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                        self.error(
                            value.span,
                            format!("argument type {:?} does not match {expected:?}", value.ty),
                        );
                    }
                    checked.push(value);
                }
                return Some(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: checked },
                    ty: native_ty(native.ret),
                    span,
                });
            }
        }
        if let Some(module) = self.namespace_imports.get(class_name).cloned() {
            if let Some(external) = self
                .external_namespace_functions
                .get(&(module.clone(), self.text(method).to_string()))
                .cloned()
            {
                if external.params.len() != args.len() {
                    self.error(
                        span,
                        format!(
                            "`{}` expects {} argument(s), found {}",
                            self.text(method),
                            external.params.len(),
                            args.len()
                        ),
                    );
                    return Some(self.poison(span));
                }
                let mut checked = Vec::with_capacity(args.len());
                for (arg, &expected) in args.iter().zip(&external.params) {
                    let mut value = self.check_expr(cx, arg);
                    coerce_numeric_literal(&mut value, expected);
                    if !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                        self.error(
                            value.span,
                            format!("argument type {:?} does not match {expected:?}", value.ty),
                        );
                    }
                    checked.push(value);
                }
                return Some(hir::Expr {
                    kind: hir::ExprKind::ExternalCall {
                        module: external.module,
                        name: external.name,
                        args: checked,
                    },
                    ty: external.return_ty,
                    span,
                });
            }
        }
        None
    }

    /// A static method called through its declaring class name
    /// (`Class.staticMethod(...)`), including the hidden-class-first form.
    fn mcall_user_static(
        &mut self,
        cx: &mut FunctionCx,
        class_name: &str,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        if let Some(&class_index) = self.classes_by_name.get(class_name) {
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
                        format!(
                            "instance method `{}` must be called through an object",
                            self.text(method)
                        ),
                    );
                    return Some(self.poison(span));
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
                    return Some(hir::Expr {
                        kind: hir::ExprKind::StaticCall {
                            func,
                            args: checked,
                        },
                        ty: return_ty,
                        span,
                    });
                }
            }
        }
        None
    }
}
