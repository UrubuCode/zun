use crate::check::*;

impl Checker<'_> {
    /// `new Array(...)` / `Array(...)` whose argument list is not a single
    /// statically-integer length (several arguments, a spread, or a non-integer
    /// single value): lowered to the prelude `__zunArrayCtor(argsArray)`.
    pub(in crate::check) fn array_ctor_dynamic(&mut self, cx: &mut FunctionCx, args: &[ast::Expr], span: Span) -> Option<hir::Expr> {
        let func = *self.by_name.get("__zunArrayCtor")?;
        let single_plain = args.len() == 1 && !matches!(args[0], ast::Expr::Spread { .. });
        if single_plain {
            let mut probe = self.check_expr(cx, &args[0]);
            coerce_numeric_literal(&mut probe, Ty::INT);
            if probe.ty.is_int() || probe.ty.is_error() {
                return None;
            }
        }
        let elements = args
            .iter()
            .map(|arg| match arg {
                ast::Expr::Spread { expr, .. } => ast::ArrayElement::Spread((**expr).clone()),
                other => ast::ArrayElement::Expr(other.clone()),
            })
            .collect();
        let list = self.check_expr(cx, &ast::Expr::ArrayLiteral { elements, span });
        Some(hir::Expr { kind: hir::ExprKind::Call { func, args: vec![list] }, ty: Ty::Array, span })
    }

    /// Whether an argument is a `function (this: T, ...)` expression that
    /// declares the hidden receiver parameter.
    pub(in crate::check) fn ast_function_takes_this(&self, expr: &ast::Expr) -> bool {
        let ast::Expr::Function(function) = expr else { return false };
        if function.is_arrow {
            return false;
        }
        let text = &self.source[function.span.start as usize..function.span.end as usize];
        text.find('(').is_some_and(|open| text[open + 1..].trim_start().starts_with("this"))
    }

    /// `str.match/replace/search/split(arg)` where `arg` is a dynamic value
    /// (`any` / dynamic object): delegate at RUNTIME to the argument's
    /// `[Symbol.match/replace/search/split]` method (kind 0/1/2/3) via the
    /// `stringSymbolDispatch` intrinsic. The result is `any` because the
    /// user method's return type is unknown. `extra` is the replacement
    /// (replace) or limit (split); other kinds pass a placeholder.
    pub(in crate::check) fn string_symbol_dispatch(
        &mut self,
        span: Span,
        kind: f64,
        receiver: hir::Expr,
        first: hir::Expr,
        extra: Option<hir::Expr>,
    ) -> hir::Expr {
        let Some((id, _)) = zun_natives::resolve("collections", "stringSymbolDispatch") else {
            self.error(span, "missing collection intrinsic `stringSymbolDispatch`");
            return self.poison(span);
        };
        let boxed = |e: hir::Expr| {
            if e.ty == Ty::Any {
                e
            } else {
                hir::Expr { span: e.span, kind: hir::ExprKind::BoxAny { value: Box::new(e) }, ty: Ty::Any }
            }
        };
        let extra = match extra {
            Some(e) => boxed(e),
            None => boxed(hir::Expr { kind: hir::ExprKind::Number(-1.0), ty: Ty::F64, span }),
        };
        let kind = hir::Expr { kind: hir::ExprKind::Number(kind), ty: Ty::Int(IntKind::I64), span };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, first, extra, kind] },
            ty: Ty::Any,
            span,
        }
    }

    /// Lower a variadic `String.fromCharCode(...)` / `String.fromCodePoint(...)`
    /// static. Each code is turned into its one-code string by a single-code
    /// native and the pieces are folded together with the string `concat`
    /// native, so the compiler needs no variadic native ABI. Zero arguments
    /// yield `""`.
    pub(in crate::check) fn check_string_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let single_native = match method {
            "fromCharCode" => "fromCharCode1",
            "fromCodePoint" => "fromCodePoint1",
            other => {
                self.error(method_span, format!("`String.{other}` is not supported in this runtime"));
                return self.poison(span);
            }
        };
        let Some((single_id, _)) = zun_natives::resolve("string", single_native) else {
            self.error(span, format!("missing String intrinsic `{single_native}`"));
            return self.poison(span);
        };
        let Some((concat_id, _)) = zun_natives::resolve("string", "concat") else {
            self.error(span, "missing String intrinsic `concat`");
            return self.poison(span);
        };
        let mut acc = hir::Expr {
            kind: hir::ExprKind::String(String::new()),
            ty: Ty::String,
            span,
        };
        for arg in args {
            let mut code = self.check_expr(cx, arg);
            coerce_numeric_literal(&mut code, Ty::Int(IntKind::U32));
            if code.ty == Ty::Any || code.ty == Ty::String || code.ty == Ty::DynamicObject {
                // A dynamic code goes through ToNumber first.
                code = self.to_number_expr(code);
            }
            if code.ty.is_float() || (code.ty.is_int() && !self.is_assignable(code.ty, Ty::Int(IntKind::U32))) {
                let cspan = code.span;
                code = hir::Expr {
                    kind: hir::ExprKind::Convert { value: Box::new(code) },
                    ty: Ty::Int(IntKind::U32),
                    span: cspan,
                };
            }
            if !code.ty.is_error() && !self.is_assignable(code.ty, Ty::Int(IntKind::U32)) {
                self.error(
                    code.span,
                    format!("`String.{method}` expects numeric code arguments"),
                );
            }
            let piece = hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id: single_id, args: vec![code] },
                ty: Ty::String,
                span,
            };
            acc = hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id: concat_id, args: vec![acc, piece] },
                ty: Ty::String,
                span,
            };
        }
        acc
    }

    /// Lower an `Array.<method>(...)` static call. `isArray(x)` tests the value;
    /// `of(...)` builds an array from its positional arguments (lowered to an
    /// array literal); `from(src, mapFn?)` materializes an array from a managed
    /// array or string (other iterables deferred), optionally mapping each item.
    pub(in crate::check) fn check_array_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let box_any = |value: hir::Expr| {
            if value.ty == Ty::Any || value.ty.is_error() {
                value
            } else {
                let span = value.span;
                hir::Expr {
                    kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                    ty: Ty::Any,
                    span,
                }
            }
        };
        match method {
            "isArray" => {
                if args.len() != 1 {
                    self.error(
                        span,
                        format!("`Array.isArray` expects 1 argument, found {}", args.len()),
                    );
                    return self.poison(span);
                }
                let arg = box_any(self.check_expr(cx, &args[0]));
                let Some((id, _)) = zun_natives::resolve("array", "arrayIsArray") else {
                    self.error(span, "missing Array intrinsic `arrayIsArray`");
                    return self.poison(span);
                };
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                    ty: Ty::Boolean,
                    span,
                }
            }
            "of" => {
                // `Array.of(...items)` is exactly an array literal of the args.
                let elements =
                    args.iter().map(|element| Some(self.check_expr(cx, element))).collect();
                hir::Expr {
                    kind: hir::ExprKind::ArrayLiteral { elements },
                    ty: Ty::Array,
                    span,
                }
            }
            "from" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(
                        span,
                        format!("`Array.from` expects 1 or 2 argument(s), found {}", args.len()),
                    );
                    return self.poison(span);
                }
                let source = box_any(self.check_expr(cx, &args[0]));
                if args.len() == 2 {
                    let map_fn = self.check_expr(cx, &args[1]);
                    if !map_fn.ty.is_error()
                        && !self.is_assignable(map_fn.ty, Ty::Function(hir::DYNAMIC_FUNCTION))
                    {
                        self.error(map_fn.span, "`Array.from` map argument must be a function");
                    }
                    let Some((id, _)) = zun_natives::resolve("array", "arrayFromMapped") else {
                        self.error(span, "missing Array intrinsic `arrayFromMapped`");
                        return self.poison(span);
                    };
                    hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![source, map_fn] },
                        ty: Ty::Array,
                        span,
                    }
                } else {
                    let Some((id, _)) = zun_natives::resolve("array", "arrayFrom") else {
                        self.error(span, "missing Array intrinsic `arrayFrom`");
                        return self.poison(span);
                    };
                    hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![source] },
                        ty: Ty::Array,
                        span,
                    }
                }
            }
            other => {
                self.error(
                    method_span,
                    format!("`Array.{other}` is not supported in this runtime"),
                );
                self.poison(span)
            }
        }
    }

    /// Lower a `Promise.<method>(...)` static call. `resolve`/`reject` take one
    /// value; the combinators (`all`/`allSettled`/`race`/`any`) take one
    /// iterable, which this floor requires to be a managed array.
    pub(in crate::check) fn check_promise_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        // `Promise.withResolvers()` takes no arguments and returns a dynamic
        // object `{ promise, resolve, reject }`. `resolve`/`reject` live in the
        // object's function slot (function values do not box into `Any`), so
        // they are called as dynamic methods (`r.resolve(v)`); the result is
        // typed `any` so those calls route through the dynamic-call path.
        if method == "withResolvers" {
            if !args.is_empty() {
                self.error(span, format!("`Promise.withResolvers` expects no arguments, found {}", args.len()));
                return self.poison(span);
            }
            let Some((id, _)) = zun_natives::resolve("promise", "promiseWithResolvers") else {
                self.error(span, "missing Promise intrinsic `promiseWithResolvers`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![] },
                ty: Ty::Any,
                span,
            };
        }
        let (native, arg_ty): (&str, Ty) = match method {
            "resolve" => ("promiseResolveStatic", Ty::Any),
            "reject" => ("promiseRejectStatic", Ty::Any),
            "all" => ("promiseAll", Ty::Array),
            "allSettled" => ("promiseAllSettled", Ty::Array),
            "race" => ("promiseRace", Ty::Array),
            "any" => ("promiseAny", Ty::Array),
            other => {
                self.error(
                    method_span,
                    format!("`Promise.{other}` is not supported in this runtime"),
                );
                return self.poison(span);
            }
        };
        // `Promise.resolve()` / `Promise.reject()` settle with `undefined`.
        let undefined_arg = ast::Expr::Unary {
            op: ast::UnaryOp::Void,
            operand: Box::new(ast::Expr::Number { value: 0.0, span }),
            span,
        };
        let arg_expr = match args.len() {
            0 if arg_ty == Ty::Any => &undefined_arg,
            1 => &args[0],
            _ => {
                self.error(span, format!("`Promise.{method}` expects 1 argument, found {}", args.len()));
                return self.poison(span);
            }
        };
        let mut arg = self.check_expr(cx, arg_expr);
        if arg_ty == Ty::Array {
            if !arg.ty.is_error() && !self.is_assignable(arg.ty, Ty::Array) {
                self.error(arg.span, format!("`Promise.{method}` expects an array of promises"));
            }
        } else {
            // `resolve`/`reject` accept any value; box a non-`any` operand.
            if arg.ty != Ty::Any {
                arg = hir::Expr {
                    span: arg.span,
                    kind: hir::ExprKind::BoxAny { value: Box::new(arg) },
                    ty: Ty::Any,
                };
            }
        }
        let Some((id, _)) = zun_natives::resolve("promise", native) else {
            self.error(span, format!("missing Promise intrinsic `{native}`"));
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
            ty: Ty::Promise,
            span,
        }
    }
}
