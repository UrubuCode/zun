use crate::check::*;

impl Checker<'_> {
    /// Lower an `Object.<method>(...)` reflective static to its managed
    /// builtin. Operates on both typed objects (declared fields) and dynamic
    /// objects (insertion-ordered dynamic keys); the heavy lifting lives in the
    /// interpreter floor. Descriptor-based statics are intentionally absent —
    /// Zun has no property-descriptor model yet (see NUCLEUS.md).
    /// Variadic `console.log/info/debug/error/warn`: format every argument with
    /// the prelude `ConsoleFormat.top`, join with single spaces (Node), and
    /// print once to stdout (or stderr for `error`/`warn`). Returns `None` for
    /// any other `console` member so it falls through to normal dispatch.
    pub(in crate::check) fn check_console_variadic(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        let target = match method {
            "log" | "info" | "debug" => "log",
            "error" | "warn" => "err",
            "group" | "groupCollapsed" => "group",
            "groupEnd" => "groupEnd",
            "count" => "count",
            "countReset" => "countReset",
            "assert" => "assert",
            _ => return None,
        };
        let fmt_class = *self.classes_by_name.get("ConsoleFormat2")?;
        let func = self.method_of(fmt_class, target)?;
        let elements: Vec<Option<hir::Expr>> = args
            .iter()
            .map(|arg| Some(self.check_expr(cx, arg)))
            .collect();
        let array = hir::Expr {
            kind: hir::ExprKind::ArrayLiteral { elements },
            ty: Ty::Array,
            span,
        };
        Some(hir::Expr {
            kind: hir::ExprKind::StaticCall {
                func,
                args: vec![array],
            },
            ty: Ty::Void,
            span,
        })
    }

    pub(in crate::check) fn check_object_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let intrinsic = |name: &str| zun_natives::resolve("collections", name).map(|(id, _)| id);
        let builtin = |checker: &mut Self, name: &str, checked: Vec<hir::Expr>, ty: Ty| {
            let Some(id) = intrinsic(name) else {
                checker.error(span, format!("missing Object intrinsic `{name}`"));
                return checker.poison(span);
            };
            hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: checked },
                ty,
                span,
            }
        };
        // `Object.getOwnPropertyDescriptor(Map, "groupBy")`: a well-known
        // built-in static method is a writable, non-enumerable, configurable
        // data property.
        if method == "getOwnPropertyDescriptor" && args.len() == 2 {
            if let (ast::Expr::Ident(global), ast::Expr::String { .. }) = (&args[0], &args[1]) {
                let global_name = self.text(global).to_string();
                let key = match &args[1] {
                    ast::Expr::String { value, .. } => value.clone(),
                    _ => String::new(),
                };
                if cx.lookup(&global_name).is_none()
                    && !self.classes_by_name.contains_key(global_name.as_str())
                    && builtin_function_length(&[global_name.as_str(), key.as_str()]).is_some()
                {
                    let flag = |name: &str, value: bool| {
                        decorator_context_entry(
                            name,
                            hir::ExprKind::Boolean(value),
                            Ty::Boolean,
                            span,
                        )
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::DynamicObjectLiteral {
                            entries: vec![
                                flag("writable", true),
                                flag("enumerable", false),
                                flag("configurable", true),
                            ],
                        },
                        ty: Ty::Any,
                        span,
                    };
                }
            }
        }
        // Expected argument count per static (`assign` is variadic: 1+).
        let fixed = match method {
            "create" if args.len() == 2 => Some(2usize),
            "keys"
            | "values"
            | "entries"
            | "freeze"
            | "isFrozen"
            | "create"
            | "getPrototypeOf"
            | "fromEntries"
            | "getOwnPropertyDescriptors"
            | "getOwnPropertyNames"
            | "getOwnPropertySymbols"
            | "seal"
            | "isSealed"
            | "preventExtensions"
            | "isExtensible" => Some(1usize),
            "setPrototypeOf"
            | "is"
            | "getOwnPropertyDescriptor"
            | "defineProperties"
            | "hasOwn"
            | "groupBy" => Some(2usize),
            "defineProperty" => Some(3usize),
            "assign" => None,
            _ => {
                self.error(
                    method_span,
                    format!("`Object.{method}` is not supported in this runtime"),
                );
                return self.poison(span);
            }
        };
        let mut checked: Vec<hir::Expr> = args
            .iter()
            .map(|argument| self.check_expr(cx, argument))
            .collect();
        if let Some(expected) = fixed {
            if checked.len() != expected {
                self.error(
                    span,
                    format!(
                        "`Object.{method}` expects {expected} argument(s), found {}",
                        checked.len()
                    ),
                );
                return self.poison(span);
            }
        } else if checked.is_empty() {
            self.error(span, "`Object.assign` expects at least one argument");
            return self.poison(span);
        }
        match method {
            "keys" | "values" | "entries" => {
                let name = match method {
                    "keys" => "objectKeys",
                    "values" => "objectValues",
                    _ => "objectEntries",
                };
                builtin(self, name, checked, Ty::Array)
            }
            "freeze" => {
                // Freezing a RegExp keeps its static type (the same object comes back).
                let ty = if checked[0].ty == Ty::RegExp {
                    Ty::RegExp
                } else {
                    Ty::Any
                };
                builtin(self, "objectFreeze", checked, ty)
            }
            "hasOwn" => builtin(self, "objectHasOwn", checked, Ty::Boolean),
            "isFrozen" => builtin(self, "objectIsFrozen", checked, Ty::Boolean),
            "create" if checked.len() == 2 => {
                // `Object.create(proto, props)`: create, then define the descriptors.
                let props = checked.pop().unwrap();
                let (slot, _) = self.synthetic_local(cx, Ty::DynamicObject);
                let local = |span| hir::Expr {
                    kind: hir::ExprKind::Local(slot),
                    ty: Ty::DynamicObject,
                    span,
                };
                let created = builtin(self, "objectCreate", checked, Ty::DynamicObject);
                let assign = hir::Expr {
                    kind: hir::ExprKind::Assign {
                        slot,
                        value: Box::new(created),
                    },
                    ty: Ty::DynamicObject,
                    span,
                };
                let define = builtin(
                    self,
                    "objectDefineProperties",
                    vec![local(span), props],
                    Ty::Any,
                );
                hir::Expr {
                    kind: hir::ExprKind::Sequence {
                        effect: Box::new(assign),
                        value: Box::new(hir::Expr {
                            kind: hir::ExprKind::Sequence {
                                effect: Box::new(define),
                                value: Box::new(local(span)),
                            },
                            ty: Ty::DynamicObject,
                            span,
                        }),
                    },
                    ty: Ty::DynamicObject,
                    span,
                }
            }
            "create" => builtin(self, "objectCreate", checked, Ty::DynamicObject),
            "getPrototypeOf" => builtin(self, "objectGetPrototypeOf", checked, Ty::Any),
            "setPrototypeOf" => builtin(self, "objectSetPrototypeOf", checked, Ty::Any),
            "fromEntries" => builtin(self, "objectFromEntries", checked, Ty::DynamicObject),
            "groupBy" => builtin(self, "objectGroupBy", checked, Ty::DynamicObject),
            "is" => builtin(self, "objectIs", checked, Ty::Boolean),
            "defineProperty" => builtin(self, "objectDefineProperty", checked, Ty::Any),
            "defineProperties" => builtin(self, "objectDefineProperties", checked, Ty::Any),
            "getOwnPropertyDescriptor" => {
                builtin(self, "objectGetOwnPropertyDescriptor", checked, Ty::Any)
            }
            "getOwnPropertyDescriptors" => builtin(
                self,
                "objectGetOwnPropertyDescriptors",
                checked,
                Ty::DynamicObject,
            ),
            "getOwnPropertyNames" => builtin(self, "objectGetOwnPropertyNames", checked, Ty::Array),
            "getOwnPropertySymbols" => {
                builtin(self, "objectGetOwnPropertySymbols", checked, Ty::Array)
            }
            "seal" => builtin(self, "objectSeal", checked, Ty::Any),
            "isSealed" => builtin(self, "objectIsSealed", checked, Ty::Boolean),
            "preventExtensions" => builtin(self, "objectPreventExtensions", checked, Ty::Any),
            "isExtensible" => builtin(self, "objectIsExtensible", checked, Ty::Boolean),
            "assign" => {
                let Some(id) = intrinsic("objectAssign") else {
                    self.error(span, "missing Object intrinsic `objectAssign`");
                    return self.poison(span);
                };
                let mut sources = checked.split_off(1);
                let mut acc = checked.pop().unwrap();
                for source in sources.drain(..) {
                    acc = hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![acc, source],
                        },
                        ty: Ty::DynamicObject,
                        span,
                    };
                }
                acc
            }
            _ => unreachable!(),
        }
    }

    /// Lower `new Function(p1, ..., body)` / `Function(...)` to a
    /// `functionConstruct([p1, ..., body])` intrinsic. The runtime compiles the
    /// strings through an injected `EvalHost` (absent under AOT, where it
    /// throws "code generation from strings disallowed"). The result is a
    /// dynamic function value, called through the ordinary indirect path.
    pub(in crate::check) fn lower_function_construct(
        &mut self,
        cx: &mut FunctionCx,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let mut elements = Vec::with_capacity(args.len().max(1));
        for arg in args {
            let checked = self.check_expr(cx, arg);
            if !checked.ty.is_error() && checked.ty != Ty::String {
                self.error(
                    checked.span,
                    "the `Function` constructor expects string arguments",
                );
                return self.poison(span);
            }
            elements.push(Some(checked));
        }
        if elements.is_empty() {
            // `new Function()` is a function with an empty body.
            elements.push(Some(hir::Expr {
                kind: hir::ExprKind::String(String::new()),
                ty: Ty::String,
                span,
            }));
        }
        let Some((id, _)) = zun_natives::resolve("collections", "functionConstruct") else {
            self.error(span, "missing `functionConstruct` intrinsic");
            return self.poison(span);
        };
        self.codegen_sites.push((span.start, span.end));
        let array = hir::Expr {
            kind: hir::ExprKind::ArrayLiteral { elements },
            ty: Ty::Array,
            span,
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![array],
            },
            ty: Ty::Function(hir::DYNAMIC_FUNCTION),
            span,
        }
    }

    /// Build a `proxyGet(proxy, key)` native call routing a property read
    /// through the handler's `get` trap (absent trap → target default).
    pub(in crate::check) fn proxy_get(
        &mut self,
        object: hir::Expr,
        key: hir::Expr,
        span: Span,
    ) -> hir::Expr {
        if !key.ty.is_error() && key.ty != Ty::String {
            self.error(key.span, "a proxy property key must be a string");
        }
        let Some((id, _)) = zun_natives::resolve("collections", "proxyGet") else {
            self.error(span, "missing `proxyGet` intrinsic");
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![object, key],
            },
            ty: Ty::Any,
            span,
        }
    }

    /// Build a `proxySet(proxy, key, value)` native call routing a property
    /// write through the handler's `set` trap (absent trap → target default).
    /// The value is boxed to `Any`.
    pub(in crate::check) fn proxy_set(
        &mut self,
        object: hir::Expr,
        key: hir::Expr,
        value: hir::Expr,
        span: Span,
    ) -> hir::Expr {
        if !key.ty.is_error() && key.ty != Ty::String {
            self.error(key.span, "a proxy property key must be a string");
        }
        let boxed = if value.ty == Ty::Any {
            value
        } else {
            hir::Expr {
                span: value.span,
                kind: hir::ExprKind::BoxAny {
                    value: Box::new(value),
                },
                ty: Ty::Any,
            }
        };
        let Some((id, _)) = zun_natives::resolve("collections", "proxySet") else {
            self.error(span, "missing `proxySet` intrinsic");
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![object, key, boxed],
            },
            ty: Ty::Boolean,
            span,
        }
    }

    /// Lower a `Reflect.<method>(...)` static call. `Reflect` is a surface-only
    /// reflection namespace (realm global, no `new`); these are the default
    /// property operations the Proxy traps fall back to.
    ///
    /// Supported: `get(target, key)`, `set(target, key, value)`,
    /// `has(target, key)`, `deleteProperty(target, key)`, `ownKeys(target)`,
    /// the descriptor-dependent statics `getOwnPropertyDescriptor`,
    /// `defineProperty`, `getPrototypeOf`, `setPrototypeOf`, `isExtensible`,
    /// `preventExtensions` (NUCLEUS #6's descriptor model, proxy-aware), and
    /// `apply(target, thisArg, argsArray)`, and `construct` for CLASS targets
    /// (`Reflect.construct` / `new proxy(...)` via class descriptors; arbitrary
    /// function constructors are out of scope).
    pub(in crate::check) fn check_reflect_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        // `Reflect.apply(target, thisArg, argsArray)`: invoke a callable (an
        // ordinary function, or a callable proxy routed through its `apply`
        // trap) with a spread arguments array. Handled before the object-target
        // path because the target is a function or proxy, not a plain object.
        if method == "apply" {
            let mut checked: Vec<hir::Expr> = args
                .iter()
                .map(|argument| self.check_expr(cx, argument))
                .collect();
            if checked.len() != 3 {
                self.error(
                    span,
                    format!(
                        "`Reflect.apply` expects 3 argument(s), found {}",
                        checked.len()
                    ),
                );
                return self.poison(span);
            }
            let array = checked.pop().unwrap();
            let mut array = self.array_like_to_array(array);
            let this_arg = checked.pop().unwrap();
            let target = checked.pop().unwrap();
            self.box_function_array_elements(&mut array);
            if array.ty != Ty::Array && !array.ty.is_error() {
                self.error(array.span, "`Reflect.apply` expects an Array of arguments");
                return self.poison(span);
            }
            if !matches!(target.ty, Ty::Function(_) | Ty::Proxy | Ty::Any) && !target.ty.is_error()
            {
                self.error(target.span, "`Reflect.apply` target must be callable");
            }
            let native_name = if target.ty == Ty::Proxy {
                "proxyApply"
            } else {
                "functionApply"
            };
            let Some((id, _)) = zun_natives::resolve("collections", native_name) else {
                self.error(span, format!("missing Reflect intrinsic `{native_name}`"));
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![target, this_arg, array],
                },
                ty: Ty::Any,
                span,
            };
        }
        // `Reflect.construct(target, argsArray, newTarget?)`: CLASS targets only
        // (a class descriptor value). An arbitrary-function constructor is out
        // of scope: Zun has no function.prototype / `this` construction model
        // (classes are frozen typed layouts). `newTarget` is approximated by the
        // target; the array supplies the positional field values.
        if method == "construct" {
            let mut checked: Vec<hir::Expr> = args
                .iter()
                .map(|argument| self.check_expr(cx, argument))
                .collect();
            if checked.len() < 2 || checked.len() > 3 {
                self.error(
                    span,
                    format!(
                        "`Reflect.construct` expects 2 or 3 argument(s), found {}",
                        checked.len()
                    ),
                );
                return self.poison(span);
            }
            let new_target = if checked.len() == 3 { checked.pop() } else { None };
            let array = checked.pop().unwrap();
            let array = self.array_like_to_array(array);
            let target = checked.pop().unwrap();
            if array.ty != Ty::Array && !array.ty.is_error() {
                self.error(
                    array.span,
                    "`Reflect.construct` expects an Array of arguments",
                );
                return self.poison(span);
            }
            if let (Some(new_target), true) = (new_target, target.ty == Ty::Any) {
                if let Some((id, _)) = zun_natives::resolve("collections", "reflectConstructNew") {
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![target, array, Self::box_to_any(new_target)],
                        },
                        ty: Ty::Any,
                        span,
                    };
                }
            }
            let native_name = match target.ty {
                Ty::Proxy => "proxyConstruct",
                Ty::Any => "reflectConstruct",
                t if t.is_error() => return self.poison(span),
                _ => {
                    self.error(target.span, "`Reflect.construct` target must be a class");
                    return self.poison(span);
                }
            };
            let Some((id, _)) = zun_natives::resolve("collections", native_name) else {
                self.error(span, format!("missing Reflect intrinsic `{native_name}`"));
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![target, array],
                },
                ty: Ty::Any,
                span,
            };
        }
        let expected = match method {
            "get" | "has" | "deleteProperty" | "getOwnPropertyDescriptor" | "setPrototypeOf" => {
                2usize
            }
            "set" | "defineProperty" => 3,
            "ownKeys" | "getPrototypeOf" | "isExtensible" | "preventExtensions" => 1,
            other => {
                self.error(
                    method_span,
                    format!("`Reflect.{other}` is not supported in this runtime"),
                );
                return self.poison(span);
            }
        };
        let mut checked: Vec<hir::Expr> = args
            .iter()
            .map(|argument| self.check_expr(cx, argument))
            .collect();
        if method == "deleteProperty" && checked.len() == 2 {
            if let (Ty::Function(func), ast::Expr::String { value, .. }) = (checked[0].ty, &args[1])
            {
                if func < hir::DYNAMIC_FUNCTION && (value == "name" || value == "length") {
                    self.deleted_fn_meta.insert((func, value.clone()));
                }
            }
        }
        // The optional `receiver` of `Reflect.get` / `Reflect.set` is accepted and
        // ignored: the operation is applied to the target directly.
        if (method == "get" && checked.len() == 3) || (method == "set" && checked.len() == 4) {
            checked.pop();
        }
        if checked.len() != expected {
            self.error(
                span,
                format!(
                    "`Reflect.{method}` expects {expected} argument(s), found {}",
                    checked.len()
                ),
            );
            return self.poison(span);
        }
        // The target must be a dynamic receiver (plain object or `any`); a Proxy
        // target is itself a dynamic receiver at runtime.
        // A managed collection target is handed to the natives as a dynamic receiver.
        if matches!(
            checked[0].ty,
            Ty::Array
                | Ty::Map
                | Ty::Set
                | Ty::WeakMap
                | Ty::WeakSet
                | Ty::WeakRef
                | Ty::Instance(_)
                | Ty::RegExp
                | Ty::Function(_)
                | Ty::BufferView(_)
                | Ty::Buffer
        ) {
            let target = checked.remove(0);
            checked.insert(0, Self::box_to_any(target));
        }
        let target_ok = matches!(checked[0].ty, Ty::DynamicObject | Ty::Any | Ty::Proxy)
            || checked[0].ty.is_error();
        if !target_ok {
            self.error(checked[0].span, "`Reflect` target must be an object");
        }
        let builtin = |checker: &mut Self, name: &str, checked: Vec<hir::Expr>, ty: Ty| {
            let Some((id, _)) = zun_natives::resolve("collections", name) else {
                checker.error(span, format!("missing Reflect intrinsic `{name}`"));
                return checker.poison(span);
            };
            hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: checked },
                ty,
                span,
            }
        };
        match method {
            "get" => {
                let key = checked.pop().unwrap();
                let object = checked.pop().unwrap();
                hir::Expr {
                    kind: hir::ExprKind::DynamicGetValue {
                        object: Box::new(object),
                        key: Box::new(key),
                    },
                    ty: Ty::Any,
                    span,
                }
            }
            "set" => {
                // `Reflect.set` returns a boolean success and respects
                // writability/extensibility (unlike `target[key] = value`),
                // so it routes to the dedicated `reflectSet` native.
                let value = checked.pop().unwrap();
                let key = checked.pop().unwrap();
                let object = checked.pop().unwrap();
                let Some((id, _)) = zun_natives::resolve("collections", "reflectSet") else {
                    self.error(span, "missing `reflectSet` intrinsic");
                    return self.poison(span);
                };
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![
                            Self::box_to_any(object),
                            Self::box_to_any(key),
                            Self::box_to_any(value),
                        ],
                    },
                    ty: Ty::Boolean,
                    span,
                }
            }
            "has" => builtin(self, "reflectHas", checked, Ty::Boolean),
            "deleteProperty" => builtin(self, "reflectDelete", checked, Ty::Boolean), // `ownKeys` reports all own string keys (not just enumerable ones),
            // and consults a proxy's `ownKeys` trap via the proxy-aware native.
            "ownKeys" => builtin(self, "reflectOwnKeys", checked, Ty::Array),
            "getOwnPropertyDescriptor" => {
                builtin(self, "objectGetOwnPropertyDescriptor", checked, Ty::Any)
            }
            "defineProperty" => builtin(self, "reflectDefineProperty", checked, Ty::Boolean),
            "getPrototypeOf" => builtin(self, "objectGetPrototypeOf", checked, Ty::Any),
            "setPrototypeOf" => builtin(self, "reflectSetPrototypeOf", checked, Ty::Boolean),
            "isExtensible" => builtin(self, "objectIsExtensible", checked, Ty::Boolean),
            "preventExtensions" => builtin(self, "objectPreventExtensions", checked, Ty::Any),
            _ => unreachable!(),
        }
    }
}
