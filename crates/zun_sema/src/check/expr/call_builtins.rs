//! Intrinsic and global-surface call interceptors for `check_call` (see
//! `call.rs`).
//!
//! These run after the argument values are checked and the indirect-call
//! forms have been ruled out. Each gates on the callee name, takes the checked
//! arguments by `&mut`, and moves them out (with `std::mem::take`) only when it
//! handles the call. Interceptor order is preserved exactly.

use crate::check::*;

impl Checker<'_> {
    /// The implicit `print` intrinsic and the `timers` macrotask surface
    /// (`setTimeout`/`setInterval`/`clearTimeout`/`clearInterval`/
    /// `queueMicrotask`/`setImmediate`).
    pub(in crate::check) fn try_call_print_timers(
        &mut self,
        cx: &mut FunctionCx,
        name: &str,
        args: &mut Vec<hir::Expr>,
        span: Span,
    ) -> Option<hir::Expr> {
        // `print` is the one implicit intrinsic: one argument of any
        // primitive type, returns void.
        if name == "print" {
            if args.len() != 1 {
                self.error(span, "print takes exactly one argument");
            }
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: zun_natives::PRINT,
                    args: std::mem::take(args),
                },
                ty: Ty::Void,
                span,
            });
        }

        // The `timers` macrotask surface (ASYNC slice 4): bare global
        // functions on the VM's unified event loop. Resolved here like
        // `print`, but only when the name is not a user symbol (a local
        // or top-level function of the same name shadows it, handled
        // above). Each lowers to the interpreter's timer floor.
        if matches!(
            name,
            "setTimeout"
                | "setInterval"
                | "clearTimeout"
                | "clearInterval"
                | "queueMicrotask"
                | "setImmediate"
        ) && !self.by_name.contains_key(name)
            && cx.lookup(name).is_none()
        {
            let Some((id, _)) = zun_natives::resolve("timers", name) else {
                self.error(span, format!("missing timer intrinsic `{name}`"));
                return Some(self.poison(span));
            };
            let args = std::mem::take(args);
            let number = |span| hir::Expr {
                kind: hir::ExprKind::Number(0.0),
                ty: Ty::F64,
                span,
            };
            let is_callback = |arg: &hir::Expr| {
                matches!(arg.ty, Ty::Function(_)) || arg.ty.is_error()
            };
            let (built_args, ret) = match name {
                "setTimeout" | "setInterval" => {
                    if args.is_empty() {
                        self.error(span, format!("`{name}` expects a callback and an optional delay"));
                        return Some(self.poison(span));
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
                            return Some(self.poison(span));
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
                        return Some(hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id: args_id, args: vec![cb, ms, array] },
                            ty: Ty::F64,
                            span,
                        });
                    }
                }
                "setImmediate" | "queueMicrotask" => {
                    if args.len() != 1 {
                        self.error(span, format!("`{name}` expects a single callback"));
                        return Some(self.poison(span));
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
                        return Some(self.poison(span));
                    }
                    let mut handle = args.into_iter().next().expect("handle");
                    if !handle.ty.is_int() && !handle.ty.is_float() && !handle.ty.is_error() {
                        self.error(handle.span, format!("`{name}` expects a numeric handle"));
                    }
                    coerce_numeric_literal(&mut handle, Ty::F64);
                    (vec![handle], Ty::Void)
                }
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: built_args },
                ty: ret,
                span,
            });
        }
        None
    }

    /// Prelude plumbing for the decorator runtime and dynamic property/symbol
    /// setters (`decoratorAccess*`, `decoratorAdd*`, `decoratorRunInitializers`,
    /// `dynamicSetFunction`, `dynamicSetSymbolFunction`).
    pub(in crate::check) fn try_call_decorators(
        &mut self,
        name: &str,
        args: &mut Vec<hir::Expr>,
        span: Span,
    ) -> Option<hir::Expr> {
        if matches!(
            name,
            "decoratorAddInstanceInitializer" | "decoratorAccessGet" | "decoratorAccessSet"
        ) && !self.by_name.contains_key(name)
        {
            // Prelude plumbing for `DecoratorContext` / `DecoratorAccess`
            // (builtin/decorators.zun): checked against the native signature.
            let Some((id, native)) = zun_natives::resolve("collections", name) else {
                return Some(self.poison(span));
            };
            if args.len() != native.params.len() {
                self.error(span, format!("`{name}` has an invalid argument list"));
                return Some(self.poison(span));
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
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: std::mem::take(args) },
                ty: native_ty(native.ret),
                span,
            });
        }

        if name == "decoratorAddInitializer" || name == "decoratorRunInitializers" {
            // Prelude plumbing for the typed `DecoratorContext`
            // (builtin/decorators.zun).
            let expected = usize::from(name == "decoratorAddInitializer");
            if args.len() != expected
                || (expected == 1 && !matches!(args[0].ty, Ty::Function(_)))
            {
                self.error(span, format!("`{name}` has an invalid argument list"));
                return Some(self.poison(span));
            }
            let Some((id, native)) = zun_natives::resolve("collections", name) else {
                return Some(self.poison(span));
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: std::mem::take(args) },
                ty: native_ty(native.ret),
                span,
            });
        }

        if name == "dynamicSetFunction" {
            if args.len() != 3 {
                self.error(span, "dynamicSetFunction expects an object, name, and function");
                return Some(self.poison(span));
            }
            let Some((id, native)) = zun_natives::resolve("collections", name) else {
                return Some(self.poison(span));
            };
            if args[0].ty != Ty::DynamicObject
                || args[1].ty != Ty::String
                || !matches!(args[2].ty, Ty::Function(_))
            {
                self.error(span, "dynamicSetFunction expects (dynamic, string, function)");
                return Some(self.poison(span));
            }
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: std::mem::take(args) },
                ty: native_ty(native.ret),
                span,
            });
        }

        if name == "dynamicSetSymbolFunction" {
            if args.len() != 3 {
                self.error(span, "dynamicSetSymbolFunction expects an object, symbol, and function");
                return Some(self.poison(span));
            }
            let Some((id, native)) = zun_natives::resolve("collections", name) else {
                return Some(self.poison(span));
            };
            if args[0].ty != Ty::DynamicObject
                || args[1].ty != Ty::Symbol
                || !matches!(args[2].ty, Ty::Function(_))
            {
                self.error(span, "dynamicSetSymbolFunction expects (dynamic, symbol, function)");
                return Some(self.poison(span));
            }
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: std::mem::take(args) },
                ty: native_ty(native.ret),
                span,
            });
        }
        None
    }

    /// `dynamic_new`, `box_any`, and `Symbol(description?)`.
    pub(in crate::check) fn try_call_dynamic_misc(
        &mut self,
        name: &str,
        args: &mut Vec<hir::Expr>,
        span: Span,
    ) -> Option<hir::Expr> {
        if name == "dynamic_new" {
            if !args.is_empty() {
                self.error(span, "dynamic_new takes no arguments");
            }
            return Some(hir::Expr {
                kind: hir::ExprKind::DynamicNew,
                ty: Ty::DynamicObject,
                span,
            });
        }

        if name == "box_any" {
            if args.len() != 1 {
                self.error(span, "box_any takes exactly one argument");
                return Some(self.poison(span));
            }
            let value = args.pop().unwrap();
            return Some(hir::Expr {
                kind: hir::ExprKind::BoxAny {
                    value: Box::new(value),
                },
                ty: Ty::Any,
                span,
            });
        }

        // `Symbol(description?)` mints a fresh, unique symbol. The
        // description is optional; when present it must be a string.
        if name == "Symbol" && !self.by_name.contains_key("Symbol") {
            if args.len() > 1 {
                self.error(span, "`Symbol` takes an optional description argument");
                return Some(self.poison(span));
            }
            let Some((id, _)) = zun_natives::resolve("collections", "symbolNew") else {
                self.error(span, "missing `symbolNew` intrinsic");
                return Some(self.poison(span));
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
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![description] },
                ty: Ty::Symbol,
                span,
            });
        }
        None
    }

    /// `dynamic_get`/`reflect_get` and `dynamic_set`/`reflect_set` dynamic
    /// property access.
    pub(in crate::check) fn try_call_dynamic_reflect(
        &mut self,
        name: &str,
        args: &mut Vec<hir::Expr>,
        span: Span,
    ) -> Option<hir::Expr> {
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
                return Some(hir::Expr {
                    kind: hir::ExprKind::DynamicGetValue {
                        object: Box::new(object),
                        key: Box::new(key),
                    },
                    ty: Ty::Any,
                    span,
                });
            }
            let Some((object, property)) = dynamic_property(self, args, span) else {
                return Some(self.poison(span));
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::DynamicGet {
                    object: Box::new(object),
                    name: property,
                },
                ty: Ty::Any,
                span,
            });
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
                return Some(hir::Expr {
                    kind: hir::ExprKind::DynamicSetValue {
                        object: Box::new(object),
                        key: Box::new(key),
                        value: Box::new(value),
                    },
                    ty: Ty::Any,
                    span,
                });
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
                return Some(self.poison(span));
            }
            let property = match &args[1].kind {
                hir::ExprKind::String(name) => name.clone(),
                _ => {
                    self.error(
                        args[1].span,
                        "dynamic property name must be a string literal",
                    );
                    return Some(self.poison(span));
                }
            };
            let object = args.remove(0);
            args.remove(0);
            let value = args.remove(0);
            return Some(hir::Expr {
                kind: hir::ExprKind::DynamicSet {
                    object: Box::new(object),
                    name: property,
                    value: Box::new(value),
                },
                ty: Ty::Any,
                span,
            });
        }
        None
    }

    /// An imported intrinsic, a declared external (FFI) function, or a
    /// module-level binding called from another function.
    pub(in crate::check) fn try_call_imports_external(
        &mut self,
        callee: &ast::Name,
        name: &str,
        args: &mut Vec<hir::Expr>,
        span: Span,
    ) -> Option<hir::Expr> {
        // An imported intrinsic: check against its native signature.
        if let Some((intrinsic_id, intrinsic)) = self.imports.get(name).copied() {
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
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: intrinsic_id,
                    args: std::mem::take(args),
                },
                ty: native_ty(intrinsic.ret),
                span,
            });
        }

        if let Some(external) = self.external_functions.get(name).cloned() {
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
            return Some(hir::Expr {
                kind: hir::ExprKind::ExternalCall {
                    module: external.module,
                    name: external.name,
                    args: std::mem::take(args),
                },
                ty: external.return_ty,
                span,
            });
        }

        if self.module_globals.contains(name) {
            // A module-level binding called from another function:
            // read it from the realm store and call it dynamically.
            let callee_value = self.global_get_expr(name, callee.span);
            return Some(hir::Expr {
                kind: hir::ExprKind::IndirectCall {
                    callee: Box::new(callee_value),
                    args: std::mem::take(args),
                    func: hir::DYNAMIC_FUNCTION,
                },
                ty: Ty::Any,
                span,
            });
        }
        None
    }
}
