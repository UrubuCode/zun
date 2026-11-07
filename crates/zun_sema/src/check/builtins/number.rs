use crate::check::*;

impl Checker<'_> {
    /// Coerce a computed array index to an integer. An integer passes through;
    /// a `number`/`any` index is converted (JS `ToInteger` on the index) rather
    /// than rejected, so `arr[someNumber]` works.
    pub(in crate::check) fn coerce_array_index(&mut self, index: hir::Expr) -> hir::Expr {
        if index.ty.is_int() || index.ty.is_error() {
            return index;
        }
        if index.ty == Ty::F64 || index.ty == Ty::Any {
            let span = index.span;
            return hir::Expr {
                kind: hir::ExprKind::Convert {
                    value: Box::new(index),
                },
                ty: Ty::Int(IntKind::U64),
                span,
            };
        }
        self.error(index.span, "array index must be an integer");
        self.poison(index.span)
    }

    /// JS ToNumber of an arbitrary operand as an `f64` expression. `number`
    /// operands pass through; everything else is boxed into `Any` and run
    /// through the `numberCoerce` native (string parsing, object->primitive).
    pub(in crate::check) fn to_number_expr(&mut self, value: hir::Expr) -> hir::Expr {
        self.to_number_expr_with(value, "numberCoerce")
    }

    /// ToNumber that throws on BigInt/Symbol (`Math.*` argument coercion).
    pub(in crate::check) fn to_number_expr_strict(&mut self, value: hir::Expr) -> hir::Expr {
        self.to_number_expr_with(value, "numberCoerceStrict")
    }

    pub(in crate::check) fn to_number_expr_with(
        &mut self,
        value: hir::Expr,
        native: &str,
    ) -> hir::Expr {
        if value.ty == Ty::F64 || value.ty.is_error() {
            return value;
        }
        let span = value.span;
        let Some((id, _)) = zun_natives::resolve("collections", native) else {
            self.error(span, "missing `numberCoerce` intrinsic");
            return self.poison(span);
        };
        let arg = if value.ty == Ty::Any {
            value
        } else {
            hir::Expr {
                span,
                kind: hir::ExprKind::BoxAny {
                    value: Box::new(value),
                },
                ty: Ty::Any,
            }
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![arg],
            },
            ty: Ty::F64,
            span,
        }
    }

    /// Coerce a value to an `f64` argument: integers widen, everything else
    /// goes through JS ToNumber.
    /// ToString of a `parseInt`/`parseFloat` operand: a string passes through,
    /// anything else goes through the `String(x)` coercion helper.
    /// `split` limit operand: an integer passes through; anything else goes
    /// through ToUint32 (`undefined` meaning unbounded) at run time.
    pub(in crate::check) fn split_limit_expr(&mut self, value: hir::Expr) -> hir::Expr {
        if value.ty.is_int() || value.ty.is_error() {
            return value;
        }
        let span = value.span;
        let Some((id, _)) = zun_natives::resolve("collections", "splitLimit") else {
            return value;
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![Self::box_to_any(value)],
            },
            ty: Ty::Int(IntKind::I64),
            span,
        }
    }

    pub(in crate::check) fn to_string_for_parse(&mut self, value: hir::Expr) -> hir::Expr {
        if value.ty == Ty::String || value.ty.is_error() {
            return value;
        }
        let span = value.span;
        let Some((id, _)) = zun_natives::resolve("collections", "stringCoerceStrict") else {
            return value;
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![Self::box_to_any(value)],
            },
            ty: Ty::String,
            span,
        }
    }

    pub(in crate::check) fn to_number_for_arg(&mut self, value: hir::Expr) -> hir::Expr {
        if value.ty.is_int() {
            let span = value.span;
            return hir::Expr {
                kind: hir::ExprKind::Convert {
                    value: Box::new(value),
                },
                ty: Ty::F64,
                span,
            };
        }
        self.to_number_expr(value)
    }

    /// `Math.<fn>.apply(null, array)` and `Math.<fn>(...array)`: evaluate the
    /// array at run time through the `mathApply` intrinsic.
    pub(in crate::check) fn math_apply_expr(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        array: &ast::Expr,
        span: Span,
    ) -> hir::Expr {
        let Some((id, _)) = zun_natives::resolve("collections", "mathApply") else {
            self.error(span, "missing `mathApply` intrinsic");
            return self.poison(span);
        };
        let value = self.check_expr(cx, array);
        let value = if value.ty == Ty::Any || value.ty.is_error() {
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
        let name = hir::Expr {
            kind: hir::ExprKind::String(method.to_string()),
            ty: Ty::String,
            span,
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![name, value],
            },
            ty: Ty::F64,
            span,
        }
    }

    pub(in crate::check) fn check_math_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        if let [ast::Expr::Spread { expr, .. }] = args {
            return self.math_apply_expr(cx, method, expr, span);
        }
        // `Math.min`/`max`/`hypot` are variadic: fold the 2-arg native left to
        // right (`f(a,b,c)` → `f(f(a,b),c)`). 0 args give the identity.
        if matches!(method, "min" | "max" | "hypot") {
            let native_name = match method {
                "min" => "minNumber",
                "max" => "maxNumber",
                _ => "hypotNumber",
            };
            let Some((id, _)) = zun_natives::resolve("math", native_name) else {
                self.error(span, format!("missing Math intrinsic `{native_name}`"));
                return self.poison(span);
            };
            let identity = match method {
                "min" => f64::INFINITY,
                "max" => f64::NEG_INFINITY,
                _ => 0.0,
            };
            // Start from the identity and fold every argument through the 2-arg
            // native: `f(identity, a) = a` for min/max, `hypot(0, a) = |a|`, so
            // 0/1/N arguments all get the correct value.
            let mut acc = hir::Expr {
                kind: hir::ExprKind::Number(identity),
                ty: Ty::F64,
                span,
            };
            for arg in args {
                let mut value = self.check_expr(cx, arg);
                coerce_numeric_literal(&mut value, Ty::F64);
                if !value.ty.is_int() && value.ty != Ty::F64 {
                    value = self.to_number_expr_strict(value);
                }
                acc = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![acc, value],
                    },
                    ty: Ty::F64,
                    span,
                };
            }
            return acc;
        }
        let native_name = match method {
            "floor" => "floorNumber",
            "ceil" => "ceilNumber",
            "round" => "roundNumber",
            "trunc" => "truncNumber",
            "sign" => "signNumber",
            "abs" => "abs",
            "sqrt" => "sqrtNumber",
            "cbrt" => "cbrtNumber",
            "sin" => "sinNumber",
            "cos" => "cosNumber",
            "tan" => "tanNumber",
            "exp" => "expNumber",
            "log" => "logNumber",
            "log2" => "log2Number",
            "log10" => "log10Number",
            "expm1" => "expm1Number",
            "log1p" => "log1pNumber",
            "sinh" => "sinhNumber",
            "cosh" => "coshNumber",
            "tanh" => "tanhNumber",
            "asinh" => "asinhNumber",
            "acosh" => "acoshNumber",
            "atanh" => "atanhNumber",
            "pow" => "powNumber",
            "atan2" => "atan2Number",
            "hypot" => "hypotNumber",
            "imul" => "imulNumber",
            "clz32" => "clz32Number",
            "asin" => "asinNumber",
            "acos" => "acosNumber",
            "atan" => "atanNumber",
            "fround" => "froundNumber",
            "min" => "minNumber",
            "max" => "maxNumber",
            _ => {
                self.error(method_span, format!("`Math.{method}` is not supported yet"));
                return self.poison(span);
            }
        };
        let Some((id, native)) = zun_natives::resolve("math", native_name) else {
            self.error(span, format!("missing Math intrinsic `{native_name}`"));
            return self.poison(span);
        };
        // JS semantics: missing arguments are `undefined` (-> NaN), extras ignored.
        let missing = native.params.len().saturating_sub(args.len());
        let args = &args[..args.len().min(native.params.len())];
        if false {
            self.error(
                span,
                format!(
                    "`Math.{method}` expects {} argument(s), found {}",
                    native.params.len(),
                    args.len()
                ),
            );
            return self.poison(span);
        }
        let mut checked = Vec::with_capacity(args.len());
        for (arg, expected) in args
            .iter()
            .zip(native.params.iter().map(|ty| native_ty(*ty)))
        {
            let mut value = self.check_expr(cx, arg);
            coerce_numeric_literal(&mut value, expected);
            if expected == Ty::F64 && !value.ty.is_int() && value.ty != Ty::F64 {
                value = self.to_number_expr_strict(value);
            } else if expected == Ty::F64 && value.ty.is_int() {
                value = self.to_number_for_arg(value);
            }
            if !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                self.error(
                    value.span,
                    format!(
                        "`Math.{method}` argument type {:?} does not match {expected:?}",
                        value.ty
                    ),
                );
            }
            checked.push(value);
        }
        for _ in 0..missing {
            checked.push(hir::Expr {
                kind: hir::ExprKind::Number(f64::NAN),
                ty: Ty::F64,
                span,
            });
        }
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: checked },
            ty: native_ty(native.ret),
            span,
        }
    }

    pub(in crate::check) fn check_number_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let resolve =
            |checker: &mut Self, native: &str| match zun_natives::resolve("number", native) {
                Some((id, _)) => Some(id),
                None => {
                    checker.error(span, format!("missing Number intrinsic `{native}`"));
                    None
                }
            };
        match method {
            // Strict predicates: exactly one argument, of any type, no coercion.
            "isNaN" | "isFinite" | "isInteger" | "isSafeInteger" => {
                if args.len() != 1 {
                    self.error(
                        span,
                        format!("`Number.{method}` expects 1 argument, found {}", args.len()),
                    );
                    return self.poison(span);
                }
                let native = match method {
                    "isNaN" => "isNaNStrict",
                    "isFinite" => "isFiniteStrict",
                    "isInteger" => "isIntegerStrict",
                    _ => "isSafeIntegerStrict",
                };
                let Some(id) = resolve(self, native) else {
                    return self.poison(span);
                };
                let arg = self.check_expr(cx, &args[0]);
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![arg],
                    },
                    ty: Ty::Boolean,
                    span,
                }
            }
            "parseFloat" => {
                if args.len() != 1 {
                    self.error(
                        span,
                        format!(
                            "`Number.parseFloat` expects 1 argument, found {}",
                            args.len()
                        ),
                    );
                    return self.poison(span);
                }
                let Some(id) = resolve(self, "parseFloatPrefix") else {
                    return self.poison(span);
                };
                let arg = self.check_expr(cx, &args[0]);
                let arg = self.to_string_for_parse(arg);
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![arg],
                    },
                    ty: Ty::F64,
                    span,
                }
            }
            "parseInt" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(
                        span,
                        format!(
                            "`Number.parseInt` expects 1 or 2 arguments, found {}",
                            args.len()
                        ),
                    );
                    return self.poison(span);
                }
                let Some(id) = resolve(self, "parseIntRadix") else {
                    return self.poison(span);
                };
                let value = self.check_expr(cx, &args[0]);
                let value = self.to_string_for_parse(value);
                let radix = match args.get(1) {
                    Some(arg) => {
                        let mut radix = self.check_expr(cx, arg);
                        // A dynamic radix goes through ToNumber first.
                        if radix.ty == Ty::Any {
                            if let Some((nid, _)) =
                                zun_natives::resolve("collections", "numberCoerce")
                            {
                                let rspan = radix.span;
                                radix = hir::Expr {
                                    kind: hir::ExprKind::CallBuiltin {
                                        id: nid,
                                        args: vec![radix],
                                    },
                                    ty: Ty::F64,
                                    span: rspan,
                                };
                            }
                        }
                        coerce_numeric_literal(&mut radix, Ty::Int(IntKind::I32));
                        // JS ToInt32 on the radix: a `number`/other int radix is converted.
                        if radix.ty == Ty::F64 {
                            // Reduce modulo 2^32 first so the saturating convert
                            // sees NaN for +-Infinity (ToInt32 gives 0).
                            let rspan = radix.span;
                            let modulus = hir::Expr {
                                kind: hir::ExprKind::Number(4294967296.0),
                                ty: Ty::F64,
                                span: rspan,
                            };
                            radix = hir::Expr {
                                kind: hir::ExprKind::Binary {
                                    op: ast::BinaryOp::Rem,
                                    lhs: Box::new(radix),
                                    rhs: Box::new(modulus),
                                },
                                ty: Ty::F64,
                                span: rspan,
                            };
                        }
                        if radix.ty.is_float()
                            || (radix.ty.is_int() && radix.ty != Ty::Int(IntKind::I32))
                        {
                            let rspan = radix.span;
                            radix = hir::Expr {
                                kind: hir::ExprKind::Convert {
                                    value: Box::new(radix),
                                },
                                ty: Ty::Int(IntKind::I32),
                                span: rspan,
                            };
                        }
                        if !radix.ty.is_error()
                            && !self.is_assignable(radix.ty, Ty::Int(IntKind::I32))
                        {
                            self.error(radix.span, "`Number.parseInt` radix must be an integer");
                        }
                        radix
                    }
                    // Absent radix → 0 (auto-detect) at the native boundary.
                    None => hir::Expr {
                        kind: hir::ExprKind::Number(0.0),
                        ty: Ty::Int(IntKind::I32),
                        span,
                    },
                };
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![value, radix],
                    },
                    ty: Ty::F64,
                    span,
                }
            }
            other => {
                self.error(
                    method_span,
                    format!("`Number.{other}` is not supported in this runtime"),
                );
                self.poison(span)
            }
        }
    }
}
