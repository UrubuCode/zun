//! Value-phase coercion calls and the generator-call form for `check_call`
//! (see `call.rs`).
//!
//! These run after the arguments have been checked. Each gates on the callee
//! name and takes the checked arguments by `&mut`, moving them out with
//! `std::mem::take` only when it handles the call; a `None` return leaves the
//! arguments untouched for the next interceptor. Order is preserved.

use crate::check::*;

impl Checker<'_> {
    /// `BigInt(x)` / `Boolean(x)` / `String(x)` / `Number(x)` called as plain
    /// coercion functions (not constructors), unless the name is a user symbol.
    pub(in crate::check) fn try_call_coerce(
        &mut self,
        cx: &mut FunctionCx,
        name: &str,
        args: &mut Vec<hir::Expr>,
        span: Span,
    ) -> Option<hir::Expr> {
        if !matches!(name, "BigInt" | "Boolean" | "String" | "Number")
            || self.by_name.contains_key(name)
        {
            return self.try_call_coerce_one(name, args, span);
        }
        // `Boolean()` is `false` and `String()` is `""`; `BigInt()` has no
        // value to convert and falls through to its own diagnostic.
        if args.is_empty() {
            let kind = match name {
                "Boolean" => Some((hir::ExprKind::Boolean(false), Ty::Boolean)),
                "String" => Some((hir::ExprKind::String(String::new()), Ty::String)),
                _ => None,
            };
            if let Some((kind, ty)) = kind {
                return Some(hir::Expr { kind, ty, span });
            }
            return self.try_call_coerce_one(name, args, span);
        }
        // Arguments past the first are evaluated, left to right and after
        // the first, and then ignored.
        let extras = args.split_off(1);
        let result = self.try_call_coerce_one(name, args, span)?;
        if extras.is_empty() || result.ty.is_error() {
            return Some(result);
        }
        let ty = result.ty;
        let (slot, _) = self.synthetic_local(cx, ty);
        let mut tail = hir::Expr {
            kind: hir::ExprKind::Local(slot),
            ty,
            span,
        };
        for extra in extras.into_iter().rev() {
            tail = hir::Expr {
                kind: hir::ExprKind::Sequence {
                    effect: Box::new(extra),
                    value: Box::new(tail),
                },
                ty,
                span,
            };
        }
        Some(hir::Expr {
            kind: hir::ExprKind::Sequence {
                effect: Box::new(hir::Expr {
                    kind: hir::ExprKind::Assign {
                        slot,
                        value: Box::new(result),
                    },
                    ty: Ty::Any,
                    span,
                }),
                value: Box::new(tail),
            },
            ty,
            span,
        })
    }

    /// The exactly-one-argument coercion forms behind [`Self::try_call_coerce`].
    fn try_call_coerce_one(
        &mut self,
        name: &str,
        args: &mut Vec<hir::Expr>,
        span: Span,
    ) -> Option<hir::Expr> {
        // `BigInt(x)` coercion when called as a plain function (JS
        // BigInt is not constructed with `new`). A string argument
        // parses (decimal or `0x`/`0o`/`0b`); a number must be
        // integer-valued (non-integers are a runtime RangeError).
        if name == "BigInt" && !self.by_name.contains_key(name) {
            if args.len() != 1 {
                self.error(span, "`BigInt` expects one argument");
                return Some(self.poison(span));
            }
            let arg = std::mem::take(args)
                .into_iter()
                .next()
                .expect("one argument");
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

        // `Boolean(x)` coercion when called as a plain function:
        // returns the JS truthiness of `x` as a `boolean` primitive
        // (not a wrapper object). The `truthy` native mirrors the
        // interpreter's `is_truthy`, so coercion matches `if`/`&&`.
        if name == "Boolean" && !self.by_name.contains_key(name) {
            if args.len() != 1 {
                self.error(span, "`Boolean` expects one argument");
                return Some(self.poison(span));
            }
            let arg = std::mem::take(args)
                .into_iter()
                .next()
                .expect("one argument");
            if arg.ty.is_error() {
                return Some(self.poison(span));
            }
            // A value already typed `boolean` is its own truthiness.
            if arg.ty == Ty::Boolean {
                return Some(arg);
            }
            let Some((id, _)) = zun_natives::resolve("boolean", "booleanCoerce") else {
                self.error(span, "missing Boolean intrinsic `booleanCoerce`");
                return Some(self.poison(span));
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![arg],
                },
                ty: Ty::Boolean,
                span,
            });
        }

        // `String(x)` coercion when called as a plain function (JS
        // `String` is a coercion, not a constructor, called bare): the
        // ECMAScript ToString of `x` as a `string` primitive. A value
        // already typed `string` is returned unchanged; every other
        // operand is boxed into `Any` and run through the `stringCoerce`
        // native, which performs object→primitive coercion
        // (`[Symbol.toPrimitive]` / `[Symbol.toStringTag]`) at run time.
        if name == "String" && !self.by_name.contains_key(name) {
            if args.len() != 1 {
                self.error(span, "`String` expects one argument");
                return Some(self.poison(span));
            }
            let arg = std::mem::take(args)
                .into_iter()
                .next()
                .expect("one argument");
            if arg.ty.is_error() {
                return Some(self.poison(span));
            }
            // A value already typed `string` is its own ToString.
            if arg.ty == Ty::String {
                return Some(arg);
            }
            let Some((id, _)) = zun_natives::resolve("collections", "stringCoerce") else {
                self.error(span, "missing `stringCoerce` intrinsic");
                return Some(self.poison(span));
            };
            // The native reads its operand as `any`, so box a non-`any`
            // operand (primitive or object) into an `Any` carrier.
            let arg = if arg.ty == Ty::Any {
                arg
            } else {
                hir::Expr {
                    span: arg.span,
                    kind: hir::ExprKind::BoxAny {
                        value: Box::new(arg),
                    },
                    ty: Ty::Any,
                }
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![arg],
                },
                ty: Ty::String,
                span,
            });
        }

        // `Number(x)` coercion when called as a plain function (JS
        // `Number` as a coercion, not a constructor): the ECMAScript
        // ToNumber of `x` as a `number` primitive. A value already typed
        // numeric is returned unchanged; every other operand is boxed
        // into `Any` and run through the `numberCoerce` native, which
        // performs string parsing and object→primitive coercion
        // (`[Symbol.toPrimitive]("number")`) at run time.
        if name == "Number" && !self.by_name.contains_key(name) {
            // `Number()` with no argument is `+0`.
            if args.is_empty() {
                return Some(hir::Expr {
                    kind: hir::ExprKind::Number(0.0),
                    ty: Ty::F64,
                    span,
                });
            }
            if args.len() != 1 {
                self.error(span, "`Number` expects one argument");
                return Some(self.poison(span));
            }
            let arg = std::mem::take(args)
                .into_iter()
                .next()
                .expect("one argument");
            if arg.ty.is_error() {
                return Some(self.poison(span));
            }
            // A value already typed `number`/`float` is its own ToNumber.
            if arg.ty == Ty::F64 {
                return Some(arg);
            }
            let Some((id, _)) = zun_natives::resolve("collections", "numberCoerce") else {
                self.error(span, "missing `numberCoerce` intrinsic");
                return Some(self.poison(span));
            };
            // The native reads its operand as `any`, so box a non-`any`
            // operand (primitive or object) into an `Any` carrier.
            let arg = if arg.ty == Ty::Any {
                arg
            } else {
                hir::Expr {
                    span: arg.span,
                    kind: hir::ExprKind::BoxAny {
                        value: Box::new(arg),
                    },
                    ty: Ty::Any,
                }
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![arg],
                },
                ty: Ty::F64,
                span,
            });
        }
        None
    }

    /// A call to a named generator function: packs any rest arguments and
    /// builds the `GeneratorCall` that mints a fresh iterator.
    pub(in crate::check) fn try_call_generator(
        &mut self,
        name: &str,
        args: &mut Vec<hir::Expr>,
        span: Span,
    ) -> Option<hir::Expr> {
        if let Some(&func) = self.by_name.get(name) {
            if self.signatures[func as usize].is_generator {
                let Some((native_id, _)) =
                    zun_natives::resolve("buffer", "generatorIteratorNewArgs")
                else {
                    self.error(span, "missing generator iterator intrinsic");
                    return Some(self.poison(span));
                };
                // A rest parameter collects the trailing arguments into one array.
                let mut args = std::mem::take(args);
                if let Some(rest_index) = self.signatures[func as usize].rest_param {
                    let param_tys = self.signatures[func as usize].params.clone();
                    while args.len() < rest_index {
                        let expected = param_tys.get(args.len()).copied().unwrap_or(Ty::Any);
                        args.push(default_args::missing_arg(expected, span));
                    }
                    let rest: Vec<Option<hir::Expr>> = args.drain(rest_index..).map(Some).collect();
                    let mut packed = hir::Expr {
                        kind: hir::ExprKind::ArrayLiteral { elements: rest },
                        ty: Ty::Array,
                        span,
                    };
                    self.box_function_array_elements(&mut packed);
                    args.push(packed);
                }
                return Some(hir::Expr {
                    kind: hir::ExprKind::GeneratorCall {
                        func,
                        args,
                        native_id,
                    },
                    ty: Ty::Any,
                    span,
                });
            }
        }
        None
    }
}
