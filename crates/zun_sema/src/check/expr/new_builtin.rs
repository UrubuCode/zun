//! Hardcoded built-in constructor paths for `check_new` (see `new.rs`).
//!
//! These run after the realm-registry lowering (`new_realm.rs`) has fallen
//! through. Each helper gates on the constructor name and returns `Some` when
//! it handles the `new`, or `None` to let the next interceptor (and finally
//! the user-class construction path) try. The interceptor order mirrors the
//! original `check_new` exactly.

use crate::check::*;

impl Checker<'_> {
    /// The hardcoded built-in `new` paths, in their original order:
    /// buffer globals, `BigInt`/`Proxy`, managed collections/`Array`, and the
    /// `Number`/`String`/`Boolean` wrapper objects.
    pub(in crate::check) fn try_new_builtin(
        &mut self,
        cx: &mut FunctionCx,
        name: &str,
        class: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        if let Some(expr) = self.try_new_buffer_global(cx, name, args, span) {
            return Some(expr);
        }
        if let Some(expr) = self.try_new_bigint_proxy(cx, name, args, span) {
            return Some(expr);
        }
        if let Some(expr) = self.try_new_collection_array(cx, name, args, span) {
            return Some(expr);
        }
        self.try_new_wrappers(cx, name, class, args, span)
    }

    /// `ArrayBuffer`, `DataView`, and the `Uint8/16/32Array` / `Float64Array`
    /// buffer-view globals (the non-registry forms).
    fn try_new_buffer_global(
        &mut self,
        cx: &mut FunctionCx,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        let typed_view = match name {
            "Uint8Array" => Some("u8ArrayNew"),
            "Uint16Array" => Some("u16ArrayNew"),
            "Uint32Array" => Some("u32ArrayNew"),
            "Float64Array" => Some("f64ArrayNew"),
            _ => None,
        };
        if name == "ArrayBuffer" {
            return Some(self.check_array_buffer_new(cx, args, span));
        }
        if name == "DataView" {
            if args.len() != 3 {
                self.error(span, "`DataView` expects buffer, offset, and length");
                return Some(self.poison(span));
            }
            let mut call_args: Vec<hir::Expr> =
                args.iter().map(|arg| self.check_expr(cx, arg)).collect();
            for arg in call_args.iter_mut().skip(1) {
                coerce_numeric_literal(arg, Ty::INT);
            }
            let Some((id, _)) = zun_natives::resolve("buffer", "dataViewNew") else {
                self.error(span, "missing DataView intrinsic");
                return Some(self.poison(span));
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: call_args,
                },
                ty: Ty::BufferView(hir::BufferViewKind::DataView),
                span,
            });
        }
        if let Some(native_name) = typed_view {
            let view_kind = match name {
                "Uint8Array" => hir::BufferViewKind::U8,
                "Uint16Array" => hir::BufferViewKind::U16,
                "Uint32Array" => hir::BufferViewKind::U32,
                "Float64Array" => hir::BufferViewKind::F64,
                _ => unreachable!(),
            };
            // `new <Kind>Array(buffer, offset, length)` is a
            // fixed-length view. With the length omitted (`(buffer)` or
            // `(buffer, offset)`) it is a **length-tracking** view over
            // a resizable buffer (ES2024): its element count follows the
            // buffer's current length. Tracking routes to a
            // kind-agnostic native carrying the element discriminant.
            if args.is_empty() || args.len() > 3 {
                self.error(
                    span,
                    format!("`{name}` expects a buffer and an optional offset and length"),
                );
                return Some(self.poison(span));
            }
            if args.len() < 3 {
                let buffer = self.check_expr(cx, &args[0]);
                let offset = match args.get(1) {
                    Some(arg) => {
                        let mut offset = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut offset, Ty::INT);
                        offset
                    }
                    None => hir::Expr {
                        kind: hir::ExprKind::Number(0.0),
                        ty: Ty::Int(IntKind::U64),
                        span,
                    },
                };
                let Some(discriminant) = crate::typed_arrays::element_index(view_kind) else {
                    self.error(span, "typed array requires an element kind".to_string());
                    return Some(self.poison(span));
                };
                let kind_lit = hir::Expr {
                    kind: hir::ExprKind::Number(discriminant as f64),
                    ty: Ty::Int(IntKind::U32),
                    span,
                };
                let Some((id, _)) = zun_natives::resolve("buffer", "typedArrayTrackingNew") else {
                    self.error(
                        span,
                        "missing typed-array intrinsic `typedArrayTrackingNew`",
                    );
                    return Some(self.poison(span));
                };
                return Some(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![buffer, offset, kind_lit],
                    },
                    ty: Ty::BufferView(view_kind),
                    span,
                });
            }
            let mut call_args: Vec<hir::Expr> =
                args.iter().map(|arg| self.check_expr(cx, arg)).collect();
            for arg in call_args.iter_mut().skip(1) {
                coerce_numeric_literal(arg, Ty::INT);
            }
            let Some((id, _)) = zun_natives::resolve("buffer", native_name) else {
                self.error(
                    span,
                    format!("missing typed-array intrinsic `{native_name}`"),
                );
                return Some(self.poison(span));
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: call_args,
                },
                ty: Ty::BufferView(view_kind),
                span,
            });
        }
        None
    }

    /// `new BigInt(x)` coercion and `new Proxy(target, handler)`.
    fn try_new_bigint_proxy(
        &mut self,
        cx: &mut FunctionCx,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        // `BigInt(x)` coercion. A string argument parses (decimal or a
        // `0x`/`0o`/`0b` prefix); a number argument must be
        // integer-valued (non-integers are a runtime RangeError).
        if name == "BigInt" {
            if args.len() != 1 {
                self.error(span, "`BigInt` expects one argument");
                return Some(self.poison(span));
            }
            let arg = self.check_expr(cx, &args[0]);
            let native_name = if arg.ty.is_int() || arg.ty.is_float() {
                "bigIntFromNumber"
            } else if arg.ty == Ty::String {
                "bigIntNew"
            } else if arg.ty.is_error() {
                return Some(self.poison(span));
            } else {
                self.error(arg.span, "`BigInt` expects a string or number argument");
                return Some(self.poison(span));
            };
            let Some((id, _)) = zun_natives::resolve("collections", native_name) else {
                self.error(
                    span,
                    format!("missing collection intrinsic `{native_name}`"),
                );
                return Some(self.poison(span));
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![arg],
                },
                ty: Ty::BigInt,
                span,
            });
        }
        // `new Proxy(target, handler)`: a proxy over a *callable* target
        // boxes the function into a GC record (functions do not box into
        // `Any`) and is built with `proxyNewCallable`, so the `apply`
        // trap and direct `proxy(...)` calls can recover the function; a
        // proxy over data uses the plain `proxyNew`.
        if name == "Proxy" {
            if args.len() != 2 {
                self.error(span, "`Proxy` expects a target and a handler");
                return Some(self.poison(span));
            }
            let target = self.check_expr(cx, &args[0]);
            let handler = self.check_expr(cx, &args[1]);
            let (native_name, target) = self.box_proxy_target(target, span);
            let Some((id, _)) = zun_natives::resolve("collections", native_name) else {
                self.error(
                    span,
                    format!("missing collection intrinsic `{native_name}`"),
                );
                return Some(self.poison(span));
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![target, handler],
                },
                ty: Ty::Proxy,
                span,
            });
        }
        None
    }

    /// Prepare a `Proxy` target: a class descriptor is boxed into a GC record
    /// (`classBox`) and a function into a `functionBox` record (built with
    /// `proxyNewCallable`); data targets are used as they are. Returns the
    /// native that builds the proxy and the prepared target.
    pub(in crate::check) fn box_proxy_target(
        &mut self,
        target: hir::Expr,
        span: Span,
    ) -> (&'static str, hir::Expr) {
        let target = if matches!(target.kind, hir::ExprKind::CallBuiltin { id, .. }
            if zun_natives::resolve("collections", "realmDescriptor")
                .map(|(rid, _)| rid) == Some(id))
        {
            let Some((box_id, _)) = zun_natives::resolve("collections", "classBox") else {
                self.error(span, "missing `classBox` intrinsic");
                return ("proxyNew", self.poison(span));
            };
            let target_span = target.span;
            hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: box_id,
                    args: vec![target],
                },
                ty: Ty::DynamicObject,
                span: target_span,
            }
        } else {
            target
        };
        if matches!(target.ty, Ty::Function(_)) {
            let Some((box_id, _)) = zun_natives::resolve("collections", "functionBox") else {
                self.error(span, "missing `functionBox` intrinsic");
                return ("proxyNew", self.poison(span));
            };
            let target_span = target.span;
            let boxed = hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: box_id,
                    args: vec![target],
                },
                ty: Ty::Any,
                span: target_span,
            };
            ("proxyNewCallable", boxed)
        } else {
            ("proxyNew", target)
        }
    }

    /// The managed-collection globals (`Map`/`Set`/`WeakMap`/`WeakSet`/
    /// `RegExp`/`BigInt`/`FinalizationRegistry`/`Promise`) and `Array`.
    fn try_new_collection_array(
        &mut self,
        cx: &mut FunctionCx,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        let managed_collection = match name {
            "Map" => Some(("mapNew", Ty::Map)),
            "Set" => Some(("setNew", Ty::Set)),
            "WeakMap" => Some(("weakMapNew", Ty::WeakMap)),
            "WeakSet" => Some(("weakSetNew", Ty::WeakSet)),
            "RegExp" => Some(("regexpNew", Ty::RegExp)),
            "BigInt" => Some(("bigIntNew", Ty::BigInt)),
            "FinalizationRegistry" => Some(("finalizationRegistryNew", Ty::FinalizationRegistry)),
            "Promise" => Some(("promiseNew", Ty::Promise)),
            _ => None,
        };
        if let Some((native_name, ty)) = managed_collection {
            let valid_args = (name == "RegExp" && (args.len() == 1 || args.len() == 2))
                || (name == "BigInt" && args.len() == 1)
                || (name == "Proxy" && args.len() == 2);
            if !args.is_empty() && !valid_args {
                self.error(span, format!("`{name}` expects no constructor arguments"));
                return Some(self.poison(span));
            }
            let module = if name == "Promise" {
                "promise"
            } else {
                "collections"
            };
            let Some((id, _)) = zun_natives::resolve(module, native_name) else {
                self.error(
                    span,
                    format!("missing collection intrinsic `{native_name}`"),
                );
                return Some(self.poison(span));
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: if name == "RegExp" || name == "BigInt" || name == "Proxy" {
                        let mut checked: Vec<hir::Expr> =
                            args.iter().map(|arg| self.check_expr(cx, arg)).collect();
                        if name == "RegExp" {
                            // A dynamic (`any`) pattern or flags operand goes
                            // through ToString, as `new RegExp(x)` does.
                            let mut converted = Vec::with_capacity(checked.len());
                            for arg in checked.drain(..) {
                                converted.push(if arg.ty == Ty::Any {
                                    self.to_string_for_parse(arg)
                                } else {
                                    arg
                                });
                            }
                            checked = converted;
                        }
                        if name == "RegExp" && checked.len() == 1 {
                            // `regexpNew(pattern, flags)`: default to no flags.
                            checked.push(hir::Expr {
                                kind: hir::ExprKind::String(String::new()),
                                ty: Ty::String,
                                span,
                            });
                        }
                        checked
                    } else {
                        Vec::new()
                    },
                },
                ty,
                span,
            });
        }
        if name == "Array" || name == "array" {
            if let Some(call) = self.array_ctor_dynamic(cx, args, span) {
                return Some(call);
            }
            let mut length = self.check_expr(cx, &args[0]);
            coerce_numeric_literal(&mut length, Ty::INT);
            if !length.ty.is_int() {
                self.error(length.span, "array length must be an integer");
            }
            return Some(hir::Expr {
                kind: hir::ExprKind::ArrayNew {
                    length: Box::new(length),
                },
                ty: Ty::Array,
                span,
            });
        }
        None
    }

    /// `new Number(x)` / `new String(x)` / `new Boolean(x)` wrapper objects
    /// (lowered to the prelude `NumberObject`/`StringObject`/`BooleanObject`
    /// classes), unless the name is shadowed by a user class.
    fn try_new_wrappers(
        &mut self,
        cx: &mut FunctionCx,
        name: &str,
        class: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        // `new Boolean(x)`: the wrapper object. Lowered to
        // `new BooleanObject(<truthiness of x>)` (the prelude class in
        // `builtin/boolean.zun`); bare `Boolean(x)` stays a primitive.
        // `new Number(x)`: the wrapper object, lowered to
        // `new NumberObject(<ToNumber of x>)`.
        if name == "Number" && !self.classes_by_name.contains_key(name) {
            let Some(&wrapper) = self.classes_by_name.get("NumberObject") else {
                self.error(class.span, "missing `NumberObject` wrapper class");
                return Some(self.poison(span));
            };
            if args.len() > 1 {
                self.error(span, "`Number` expects at most one argument");
                return Some(self.poison(span));
            }
            let value = match args.first() {
                None => hir::Expr {
                    kind: hir::ExprKind::Number(0.0),
                    ty: Ty::F64,
                    span,
                },
                Some(arg) => {
                    let mut arg = self.check_expr(cx, arg);
                    coerce_numeric_literal(&mut arg, Ty::F64);
                    self.to_number_for_arg(arg)
                }
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::New {
                    class: wrapper,
                    args: vec![value],
                },
                ty: Ty::Instance(wrapper),
                span,
            });
        }
        if name == "String" && !self.classes_by_name.contains_key(name) {
            let Some(&wrapper) = self.classes_by_name.get("StringObject") else {
                self.error(class.span, "missing `StringObject` wrapper class");
                return Some(self.poison(span));
            };
            if args.len() > 1 {
                self.error(span, "`String` expects at most one argument");
                return Some(self.poison(span));
            }
            let text = match args.first() {
                None => hir::Expr {
                    kind: hir::ExprKind::String(String::new()),
                    ty: Ty::String,
                    span,
                },
                Some(arg) => {
                    let arg = self.check_expr(cx, arg);
                    if arg.ty.is_error() {
                        return Some(self.poison(span));
                    }
                    if arg.ty == Ty::String {
                        arg
                    } else if let Some(&func) = self.by_name.get("__coerceString") {
                        hir::Expr {
                            kind: hir::ExprKind::Call {
                                func,
                                args: vec![Self::box_to_any(arg)],
                            },
                            ty: Ty::String,
                            span,
                        }
                    } else {
                        self.error(span, "missing `__coerceString` helper");
                        return Some(self.poison(span));
                    }
                }
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::New {
                    class: wrapper,
                    args: vec![text],
                },
                ty: Ty::Instance(wrapper),
                span,
            });
        }
        if name == "Boolean" && !self.classes_by_name.contains_key(name) {
            let Some(&wrapper) = self.classes_by_name.get("BooleanObject") else {
                self.error(class.span, "missing `BooleanObject` wrapper class");
                return Some(self.poison(span));
            };
            if args.len() > 1 {
                self.error(span, "`Boolean` expects at most one argument");
                return Some(self.poison(span));
            }
            let flag = match args.first() {
                None => hir::Expr {
                    kind: hir::ExprKind::Boolean(false),
                    ty: Ty::Boolean,
                    span,
                },
                Some(arg) => {
                    let arg = self.check_expr(cx, arg);
                    if arg.ty.is_error() {
                        return Some(self.poison(span));
                    }
                    if arg.ty == Ty::Boolean {
                        arg
                    } else {
                        let Some((id, _)) = zun_natives::resolve("boolean", "booleanCoerce") else {
                            self.error(span, "missing Boolean intrinsic `booleanCoerce`");
                            return Some(self.poison(span));
                        };
                        hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id,
                                args: vec![arg],
                            },
                            ty: Ty::Boolean,
                            span,
                        }
                    }
                }
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::New {
                    class: wrapper,
                    args: vec![flag],
                },
                ty: Ty::Instance(wrapper),
                span,
            });
        }
        None
    }
}
