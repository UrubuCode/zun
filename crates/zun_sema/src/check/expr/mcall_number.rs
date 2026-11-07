//! Numeric, boolean, and primitive-group method-call interceptors for
//! `check_method_call` (see `method_call.rs`).
//!
//! The value-phase interceptors for a `number`/`boolean` receiver, the
//! root-locale `toLocale*` family, `valueOf`, string `matchAll`, and the
//! final primitive-group native lookup. Each takes the checked receiver by
//! value and returns `Ok` when it handles the call or `Err(receiver)` to hand
//! the receiver back for the next interceptor. Order is preserved exactly.

use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn mcall_number_a(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        if let Ty::Function(func) = receiver.ty {
            return Ok(self.lower_function_reflective_call(
                cx,
                receiver,
                mname,
                func,
                args,
                span,
                method.span,
            ));
        }
        // JS-conformant `toFixed` / `toExponential` / `toPrecision` /
        // `toString(radix)`: the raw argument is passed as `any` and
        // coerced, range-checked (managed `RangeError`) and formatted
        // exactly on the interpreter floor.
        if receiver.ty == Ty::F64
            && matches!(
                mname,
                "toFixed" | "toExponential" | "toPrecision" | "toString"
            )
            && args.len() <= 1
        {
            let native_name = match mname {
                "toFixed" => "numberToFixedJs",
                "toExponential" => "numberToExponentialJs",
                "toPrecision" => "numberToPrecisionJs",
                _ => "numberToStringRadixJs",
            };
            let Some((id, _)) = zun_natives::resolve("number", native_name) else {
                self.error(span, format!("missing Number intrinsic `{native_name}`"));
                return Ok(self.poison(span));
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
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![receiver, arg],
                },
                ty: Ty::String,
                span,
            });
        }
        // `number.toString(radix?)` — base-`radix` (2..=36) rendering,
        // default 10. Intercepted before class dispatch so it supersedes
        // the no-arg extern `toString` for the radix form and the
        // no-arg form alike.
        if receiver.ty == Ty::F64 && mname == "toString" && args.len() <= 1 {
            let Some((id, _)) = zun_natives::resolve("number", "toStringRadix") else {
                self.error(span, "missing Number intrinsic `toStringRadix`");
                return Ok(self.poison(span));
            };
            let radix = match args.first() {
                Some(arg) => {
                    let mut radix = self.check_expr(cx, arg);
                    coerce_numeric_literal(&mut radix, Ty::Int(IntKind::I32));
                    if !radix.ty.is_error() && !self.is_assignable(radix.ty, Ty::Int(IntKind::I32))
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
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![receiver, radix],
                },
                ty: Ty::String,
                span,
            });
        }
        Err(receiver)
    }

    pub(in crate::check) fn mcall_number_b(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        _method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        // `number.toExponential(fractionDigits?)` /
        // `number.toPrecision(precision?)` — instance formatting,
        // intercepted before class dispatch like `toString(radix?)`.
        // The optional count is lowered to an out-of-range sentinel for
        // the native: `-1` ("absent") for `toExponential`, `0` for
        // `toPrecision`.
        if receiver.ty == Ty::F64
            && matches!(mname, "toExponential" | "toPrecision")
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
                return Ok(self.poison(span));
            };
            let count = match args.first() {
                Some(arg) => {
                    let mut count = self.check_expr(cx, arg);
                    coerce_numeric_literal(&mut count, Ty::Int(IntKind::I32));
                    if !count.ty.is_error() && !self.is_assignable(count.ty, Ty::Int(IntKind::I32))
                    {
                        self.error(count.span, format!("`{mname}` argument must be an integer"));
                    }
                    count
                }
                None => hir::Expr {
                    kind: hir::ExprKind::Number(sentinel),
                    ty: Ty::Int(IntKind::I32),
                    span,
                },
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![receiver, count],
                },
                ty: Ty::String,
                span,
            });
        }
        // `number.toFixed(digits?)` — fixed-point formatting, digits
        // default 0 (range 0..=100 enforced by the native). Hand-lowered
        // before class dispatch like `toExponential`/`toPrecision`.
        if receiver.ty == Ty::F64 && mname == "toFixed" && args.len() <= 1 {
            let Some((id, _)) = zun_natives::resolve("number", "toFixed") else {
                self.error(span, "missing Number intrinsic `toFixed`");
                return Ok(self.poison(span));
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
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![receiver, digits],
                },
                ty: Ty::String,
                span,
            });
        }
        Err(receiver)
    }

    pub(in crate::check) fn mcall_number_c(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        _method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        // Optional trailing BCP-47 `locale` (ECMA-402) for the root-locale
        // `toLocale*` family. Hand-lowered because `managed_method` is
        // fixed-arity: an omitted locale is the sentinel `""` (root).
        if (receiver.ty == Ty::String
            && matches!(
                mname,
                "localeCompare" | "toLocaleLowerCase" | "toLocaleUpperCase"
            ))
            || (receiver.ty == Ty::F64 && mname == "toLocaleString")
        {
            let (module, native, leading) = match (receiver.ty, mname) {
                (Ty::String, "localeCompare") => ("string", "localeCompare", 1usize),
                (Ty::String, "toLocaleLowerCase") => ("string", "toLocaleLowerCase", 0),
                (Ty::String, _) => ("string", "toLocaleUpperCase", 0),
                _ => ("number", "intlNumberFormat", 0),
            };
            if args.len() < leading || args.len() > leading + 1 {
                self.error(
                    span,
                    format!(
                        "`{mname}` expects {leading} or {} argument(s), found {}",
                        leading + 1,
                        args.len()
                    ),
                );
                return Ok(self.poison(span));
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
                return Ok(self.poison(span));
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
            let ty = if native == "localeCompare" {
                Ty::Int(IntKind::I32)
            } else {
                Ty::String
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: all },
                ty,
                span,
            });
        }
        // `number.valueOf()` / `string.valueOf()` — identity, returning
        // the primitive itself. No native needed: the checked receiver
        // expression already is the value.
        if matches!(receiver.ty, Ty::F64 | Ty::String | Ty::BigInt)
            && mname == "valueOf"
            && args.is_empty()
        {
            return Ok(receiver);
        }
        // `String.prototype.matchAll(regexp)` — all matches of a global
        // regexp as an array of match arrays (each like `exec`). The `g`
        // flag is required (checked natively); the array-of-arrays shape
        // is the same pragmatic choice as `keys()/values()/entries()`
        // (no distinct iterator object yet).
        if receiver.ty == Ty::String && mname == "matchAll" && args.len() == 1 {
            let first = self.check_expr(cx, &args[0]);
            if first.ty.is_error() {
                return Ok(self.poison(span));
            }
            // Every argument shape (RegExp, string, any, object carrying
            // `Symbol.matchAll`) goes through the runtime dispatch, which
            // returns the lazy match iterator.
            let first = Self::box_to_any(first);
            return Ok(self.string_symbol_dispatch(span, 4.0, receiver, first, None));
        }
        // `boolean.toString()` → `"true"` / `"false"`. Intercepted
        // before class dispatch, like `number.toString(radix?)`, since
        // the primitive `boolean` has no backing class.
        if receiver.ty == Ty::Boolean && mname == "toString" && args.is_empty() {
            let Some((id, _)) = zun_natives::resolve("boolean", "booleanToString") else {
                self.error(span, "missing Boolean intrinsic `booleanToString`");
                return Ok(self.poison(span));
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![receiver],
                },
                ty: Ty::String,
                span,
            });
        }
        Err(receiver)
    }

    #[allow(clippy::if_same_then_else, clippy::needless_range_loop)] // branch bodies and index loops mirror the spec cases
    pub(in crate::check) fn mcall_primitive_tail(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
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
                mname,
                "slice" | "indexOf" | "lastIndexOf" | "includes" | "reverse"
            )
            && !self.classes_by_name.contains_key("Array")
        {
            if mname == "reverse" {
                let Some((id, _)) = zun_natives::resolve("array", "arrayReverse") else {
                    self.error(span, "missing array intrinsic `arrayReverse`");
                    return Ok(self.poison(span));
                };
                return Ok(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![receiver],
                    },
                    ty: Ty::Array,
                    span,
                });
            }
            let native_name = match mname {
                "slice" => "arraySliceRange",
                "indexOf" => "arrayIndexOf",
                "lastIndexOf" => "arrayLastIndexOf",
                _ => "arrayIncludes",
            };
            let Some((id, native)) = zun_natives::resolve("array", native_name) else {
                self.error(span, format!("missing array intrinsic `{native_name}`"));
                return Ok(self.poison(span));
            };
            let is_slice = mname == "slice";
            let max_args = if is_slice { 2 } else { 2 };
            let min_args = if is_slice { 0 } else { 1 };
            if args.len() < min_args || args.len() > max_args {
                self.error(
                    span,
                    format!("`{mname}` called with a wrong number of arguments"),
                );
                return Ok(self.poison(span));
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
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: call_args,
                },
                ty: native_ty(native.ret),
                span,
            });
        }
        if let Some((group, class_name)) = primitive_group {
            if !self.classes_by_name.contains_key(class_name) {
                // `string`/`number` natives are bare (`toUpperCase`);
                // `array` natives carry an `array` prefix (`arraySlice`).
                let native_name = if group == "array" {
                    let mut chars = mname.chars();
                    match chars.next() {
                        Some(first) => {
                            format!("array{}{}", first.to_ascii_uppercase(), chars.as_str())
                        }
                        None => mname.to_string(),
                    }
                } else {
                    mname.to_string()
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
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id,
                                args: call_args,
                            },
                            ty: native_ty(native.ret),
                            span,
                        });
                    }
                }
                self.error(method.span, format!("no method `{mname}` on {class_name}"));
                return Ok(self.poison(span));
            }
        }
        Err(receiver)
    }
}
