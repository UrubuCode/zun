//! String-receiver method-call interceptors for `check_method_call` (see
//! `method_call.rs`).
//!
//! The value-phase interceptors for a `string` receiver: `concat`, the
//! dynamically-coerced string methods, `slice`/`split`, `replaceAll`/
//! `replace`, and the RegExp-taking `match`/`search`. Each takes the checked
//! receiver by value and returns `Ok` when it handles the call or
//! `Err(receiver)` to fall through; order is preserved exactly.

use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn mcall_string_concat(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        _method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
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
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Call { func, args: vec![receiver, list] },
                    ty: Ty::String,
                    span,
                });
            }
        }
        if receiver.ty == Ty::String && mname == "concat" {
            let Some((id, _)) = zun_natives::resolve("string", "concat") else {
                self.error(span, "missing String intrinsic `concat`");
                return Ok(self.poison(span));
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
            return Ok(acc);
        }
        // String methods called with JS-style arguments (optional,
        // `undefined`, fractional/negative, untyped): the statically
        // typed natives only accept exact integer/string arguments, so
        // anything else goes through the dynamic `stringMethod` native,
        // which applies full ToIntegerOrInfinity / ToString coercion.
        Err(receiver)
    }

    pub(in crate::check) fn mcall_string_dynamic(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        if receiver.ty == Ty::String
            && matches!(
                mname,
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
                && match mname {
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
                    return Ok(self.poison(span));
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
                        kind: hir::ExprKind::String(mname.to_string()),
                        ty: Ty::String,
                        span: method.span,
                    },
                ];
                let mut it = checked.into_iter();
                for _ in 0..2 {
                    call_args.push(it.next().unwrap_or_else(|| undefined(span)));
                }
                let result_ty = match mname {
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
                    return Ok(call);
                }
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Convert { value: Box::new(call) },
                    ty: result_ty,
                    span,
                });
            }
        }
        // `String.prototype.slice(start?, end?)` — negative indices
        // count from the end (distinct from `substring`). An absent
        // argument defaults to the whole range; `end` is passed as a
        // value past any length so it clamps to the length natively.
        Err(receiver)
    }

    pub(in crate::check) fn mcall_string_slice_split(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        _method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        if receiver.ty == Ty::String && mname == "slice" {
            if args.len() > 2 {
                self.error(span, "`slice` expects at most 2 arguments");
                return Ok(self.poison(span));
            }
            let Some((id, _)) = zun_natives::resolve("string", "slice") else {
                self.error(span, "missing String intrinsic `slice`");
                return Ok(self.poison(span));
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
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, start, end] },
                ty: Ty::String,
                span,
            });
        }
        // `String.prototype.split(separator, limit?)` — dispatch by the
        // separator's type: a RegExp stays on the regex engine, a string
        // uses the string-separator native (with an optional limit).
        if receiver.ty == Ty::String && mname == "split" {
            if args.len() > 2 {
                self.error(span, "`split` expects 1 or 2 arguments");
                return Ok(self.poison(span));
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
                return Ok(self.string_symbol_dispatch(span, 3.0, receiver, first, limit));
            }
            if first.ty == Ty::RegExp {
                if args.len() == 2 {
                    let mut limit = self.check_expr(cx, &args[1]);
                    coerce_numeric_literal(&mut limit, Ty::Int(IntKind::I64));
                    limit = self.split_limit_expr(limit);
                    let Some((id, _)) = zun_natives::resolve("collections", "regexpSplitLimit") else {
                        self.error(span, "missing collection intrinsic `regexpSplitLimit`");
                        return Ok(self.poison(span));
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![first, receiver, limit] },
                        ty: Ty::Array,
                        span,
                    });
                }
                let Some((id, _)) = zun_natives::resolve("collections", "regexpSplit") else {
                    self.error(span, "missing collection intrinsic `regexpSplit`");
                    return Ok(self.poison(span));
                };
                return Ok(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![first, receiver] },
                    ty: Ty::Array,
                    span,
                });
            }
            if !first.ty.is_error() && !self.is_assignable(first.ty, Ty::String) {
                self.error(first.span, "`split` expects a string or RegExp separator");
                return Ok(self.poison(span));
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
                return Ok(self.poison(span));
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, first, limit] },
                ty: Ty::Array,
                span,
            });
        }
        // `String.prototype.replace(search, replacement)` — dispatch by
        // the search's type: a RegExp uses the regex engine, a string
        // replaces the first occurrence via the string native (so it no
        // longer needs the `String` class in scope).
        // `replaceAll(/re/g, replacement)`: a regex search replaces every
        // match through the same natives as `replace`.
        Err(receiver)
    }

    pub(in crate::check) fn mcall_string_replace_all(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        _method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        if receiver.ty == Ty::String
            && mname == "replaceAll"
            && args.len() == 2
        {
            let first = self.check_expr(cx, &args[0]);
            let replacement = self.check_expr(cx, &args[1]);
            if matches!(first.ty, Ty::Any | Ty::DynamicObject) {
                return Ok(self.string_symbol_dispatch(span, 5.0, receiver, first, Some(replacement)));
            }
            // A dynamic (`any`) replacement is a function or ToString'd at run time.
            if replacement.ty == Ty::Any && matches!(first.ty, Ty::RegExp | Ty::String) {
                let first = Self::box_to_any(first);
                return Ok(self.string_symbol_dispatch(span, 5.0, receiver, first, Some(replacement)));
            }
            // A RegExp search must carry the `g` flag (checked at run time).
            let first = if first.ty == Ty::RegExp {
                let Some((guard, _)) = zun_natives::resolve("collections", "regexpReplaceAllGuard") else {
                    self.error(span, "missing collection intrinsic `regexpReplaceAllGuard`");
                    return Ok(self.poison(span));
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
                return Ok(self.replace_with_function(span, first, receiver, replacement, true));
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
                    return Ok(self.poison(span));
                };
                return Ok(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, first, replacement] },
                    ty: Ty::String,
                    span,
                });
            }
            if replacement.ty != Ty::String && !replacement.ty.is_error() {
                self.error(replacement.span, "`replaceAll` replacement must be a string");
                return Ok(self.poison(span));
            }
            let Some((id, _)) = zun_natives::resolve("collections", "regexpReplace") else {
                self.error(span, "missing collection intrinsic `regexpReplace`");
                return Ok(self.poison(span));
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![first, receiver, replacement] },
                ty: Ty::String,
                span,
            });
        }
        Err(receiver)
    }

    pub(in crate::check) fn mcall_string_replace_match(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        _method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        if receiver.ty == Ty::String && mname == "replace" && args.len() == 2 {
            let first = self.check_expr(cx, &args[0]);
            let replacement = self.check_expr(cx, &args[1]);
            if matches!(first.ty, Ty::Any | Ty::DynamicObject) {
                return Ok(self.string_symbol_dispatch(span, 1.0, receiver, first, Some(replacement)));
            }
            if matches!(replacement.ty, Ty::Function(_))
                && matches!(first.ty, Ty::RegExp | Ty::String)
            {
                return Ok(self.replace_with_function(span, first, receiver, replacement, false));
            }
            // A dynamic (`any`) replacement is a function or ToString'd at run time.
            if replacement.ty == Ty::Any && matches!(first.ty, Ty::RegExp | Ty::String) {
                let first = Self::box_to_any(first);
                return Ok(self.string_symbol_dispatch(span, 1.0, receiver, first, Some(replacement)));
            }
            if replacement.ty != Ty::String && !replacement.ty.is_error() {
                self.error(
                    replacement.span,
                    "`replace` replacement must be a string",
                );
                return Ok(self.poison(span));
            }
            if first.ty == Ty::RegExp {
                let Some((id, _)) = zun_natives::resolve("collections", "regexpReplace") else {
                    self.error(span, "missing collection intrinsic `regexpReplace`");
                    return Ok(self.poison(span));
                };
                return Ok(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![first, receiver, replacement] },
                    ty: Ty::String,
                    span,
                });
            }
            if !first.ty.is_error() && !self.is_assignable(first.ty, Ty::String) {
                self.error(first.span, "`replace` expects a string or RegExp search");
                return Ok(self.poison(span));
            }
            let Some((id, _)) = zun_natives::resolve("string", "replace") else {
                self.error(span, "missing String intrinsic `replace`");
                return Ok(self.poison(span));
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, first, replacement] },
                ty: Ty::String,
                span,
            });
        }
        // String methods taking a RegExp: match/search.
        if receiver.ty == Ty::String
            && matches!(mname, "match" | "search")
        {
            let expected_args = 1;
            if args.len() == expected_args {
                let first = self.check_expr(cx, &args[0]);
                if matches!(first.ty, Ty::Any | Ty::DynamicObject) {
                    let kind = if mname == "match" { 0.0 } else { 2.0 };
                    return Ok(self.string_symbol_dispatch(span, kind, receiver, first, None));
                }
                {
                    if first.ty != Ty::RegExp && !first.ty.is_error() {
                        self.error(
                            first.span,
                            format!("`{mname}` expects a RegExp argument"),
                        );
                        return Ok(self.poison(span));
                    }
                    let (native_name, return_ty) = match mname {
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
                        return Ok(self.poison(span));
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                        ty: return_ty,
                        span,
                    });
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
        Err(receiver)
    }

}
