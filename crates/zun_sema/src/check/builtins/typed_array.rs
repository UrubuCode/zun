use crate::check::*;

impl Checker<'_> {
    /// Lower a `Buffer.<method>(...)` static call. `Buffer` is a nominal alias
    /// for the `Uint8Array` byte-view floor, so these allocate/operate over a
    /// `U8` view; they are recognized by receiver name (like `ArrayBuffer`'s
    /// static `isView`).
    /// Lower the second-generation typed-array instance surface: the bulk
    /// `set(source, offset?)` (overloaded against the indexed
    /// `set(index, value)`), the copying `slice`, the in-place `copyWithin`,
    /// the search helpers `indexOf`/`includes`/`join`, and the functional
    /// methods. All route to the kind-agnostic `typedArray*` natives; the
    /// element kind travels with the view at run time, so the only per-kind
    /// facts needed here are the element type a `find`/`get` yields and the
    /// view type a `slice`/`map`/`filter` returns.
    pub(in crate::check) fn check_typed_array_method(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        kind: hir::BufferViewKind,
        mname: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let view_ty = Ty::BufferView(kind);
        let i64_ty = Ty::Int(IntKind::I64);
        let u64_ty = Ty::Int(IntKind::U64);
        // The element a read of this kind yields — the type `find` returns and
        // an indexed `set` value must match.
        let element_ty = crate::typed_arrays::method(kind, "get")
            .map(|(_, ret, _)| ret)
            .unwrap_or(Ty::F64);
        // `end`/length defaults: a value past any real length that `f64` holds
        // exactly, clamped down to the length by the relative-index logic.
        let length_sentinel = (1i64 << 53) as f64;
        let int_lit = |value: f64, ty: Ty, span: Span| hir::Expr {
            kind: hir::ExprKind::Number(value),
            ty,
            span,
        };
        let string_lit = |text: &str, span: Span| hir::Expr {
            kind: hir::ExprKind::String(text.to_string()),
            ty: Ty::String,
            span,
        };
        let box_any = |value: hir::Expr| {
            if value.ty == Ty::Any || value.ty.is_error() {
                value
            } else {
                hir::Expr {
                    span: value.span,
                    kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                    ty: Ty::Any,
                }
            }
        };
        let call = |checker: &mut Self, native: &str, call_args: Vec<hir::Expr>, ty: Ty| {
            let Some((id, _)) = zun_natives::resolve("buffer", native) else {
                checker.error(span, format!("missing typed-array intrinsic `{native}`"));
                return checker.poison(span);
            };
            hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: call_args }, ty, span }
        };
        // An optional integer argument with a default, coerced to `ty`.
        macro_rules! opt_int {
            ($pos:expr, $ty:expr, $default:expr) => {{
                match args.get($pos) {
                    Some(arg) => {
                        let mut value = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut value, $ty);
                        if !value.ty.is_int() && !value.ty.is_error() {
                            self.error(value.span, "typed-array index must be an integer");
                        }
                        value
                    }
                    None => int_lit($default, $ty, span),
                }
            }};
        }
        // A required callback argument.
        let check_callback = |checker: &mut Self, cx: &mut FunctionCx| -> Option<hir::Expr> {
            if args.len() != 1 {
                checker.error(span, format!("`{mname}` expects 1 argument (a callback), found {}", args.len()));
                return None;
            }
            let callback = checker.check_expr(cx, &args[0]);
            if !matches!(callback.ty, Ty::Function(_)) && !callback.ty.is_error() {
                checker.error(callback.span, "typed-array callback must be a function");
            }
            Some(callback)
        };
        match mname {
            "set" => {
                // Overload resolution: an array / typed-array first argument is
                // the bulk `set(source, offset?)`; two numeric arguments are
                // the indexed `set(index, value)`.
                if args.is_empty() || args.len() > 2 {
                    self.error(span, format!("`set` expects 1 or 2 arguments, found {}", args.len()));
                    return self.poison(span);
                }
                let first = self.check_expr(cx, &args[0]);
                let is_bulk = matches!(first.ty, Ty::Array | Ty::BufferView(_));
                if is_bulk {
                    let offset = opt_int!(1, u64_ty, 0.0);
                    call(self, "typedArraySetFrom", vec![receiver, first, offset], Ty::Boolean)
                } else {
                    // Indexed set: the generic per-kind `<native>Set`.
                    if args.len() != 2 {
                        self.error(span, "indexed `set(index, value)` expects 2 arguments".to_string());
                        return self.poison(span);
                    }
                    let Some((native, _ret, params)) = crate::typed_arrays::method(kind, "set") else {
                        self.error(span, "missing typed-array `set` intrinsic".to_string());
                        return self.poison(span);
                    };
                    let mut index = first;
                    coerce_numeric_literal(&mut index, u64_ty);
                    if !index.ty.is_int() && !index.ty.is_error() {
                        self.error(index.span, "typed-array index must be an integer");
                    }
                    let elem_param = params.get(1).copied().unwrap_or(element_ty);
                    let mut value = self.check_expr(cx, &args[1]);
                    coerce_numeric_literal(&mut value, elem_param);
                    if !value.ty.is_error() && !self.is_assignable(value.ty, elem_param) {
                        self.error(value.span, format!("typed-array element must be {elem_param:?}"));
                    }
                    call(self, native, vec![receiver, index, value], Ty::Boolean)
                }
            }
            "slice" => {
                if args.len() > 2 {
                    self.error(span, "`slice` expects at most 2 arguments".to_string());
                    return self.poison(span);
                }
                let start = opt_int!(0, i64_ty, 0.0);
                let end = opt_int!(1, i64_ty, length_sentinel);
                call(self, "typedArraySlice", vec![receiver, start, end], view_ty)
            }
            "copyWithin" => {
                if args.is_empty() || args.len() > 3 {
                    self.error(span, "`copyWithin` expects 1 to 3 arguments".to_string());
                    return self.poison(span);
                }
                let target = opt_int!(0, i64_ty, 0.0);
                let start = opt_int!(1, i64_ty, 0.0);
                let end = opt_int!(2, i64_ty, length_sentinel);
                call(self, "typedArrayCopyWithin", vec![receiver, target, start, end], view_ty)
            }
            "indexOf" | "includes" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(span, format!("`{mname}` expects 1 or 2 arguments"));
                    return self.poison(span);
                }
                let needle = box_any(self.check_expr(cx, &args[0]));
                let from = opt_int!(1, i64_ty, 0.0);
                if mname == "indexOf" {
                    call(self, "typedArrayIndexOf", vec![receiver, needle, from], i64_ty)
                } else {
                    call(self, "typedArrayIncludes", vec![receiver, needle, from], Ty::Boolean)
                }
            }
            "join" => {
                if args.len() > 1 {
                    self.error(span, "`join` expects at most 1 argument".to_string());
                    return self.poison(span);
                }
                let separator = match args.first() {
                    Some(arg) => {
                        let value = self.check_expr(cx, arg);
                        if value.ty != Ty::String && !value.ty.is_error() {
                            self.error(value.span, "`join` separator must be a string");
                        }
                        value
                    }
                    None => string_lit(",", span),
                };
                call(self, "typedArrayJoin", vec![receiver, separator], Ty::String)
            }
            "forEach" => match check_callback(self, cx) {
                Some(callback) => call(self, "typedArrayForEach", vec![receiver, callback], Ty::Void),
                None => self.poison(span),
            },
            "map" => match check_callback(self, cx) {
                Some(callback) => call(self, "typedArrayMap", vec![receiver, callback], view_ty),
                None => self.poison(span),
            },
            "filter" => match check_callback(self, cx) {
                Some(callback) => call(self, "typedArrayFilter", vec![receiver, callback], view_ty),
                None => self.poison(span),
            },
            "find" => match check_callback(self, cx) {
                Some(callback) => call(self, "typedArrayFind", vec![receiver, callback], element_ty),
                None => self.poison(span),
            },
            "findIndex" => match check_callback(self, cx) {
                Some(callback) => call(self, "typedArrayFindIndex", vec![receiver, callback], i64_ty),
                None => self.poison(span),
            },
            "some" => match check_callback(self, cx) {
                Some(callback) => call(self, "typedArraySome", vec![receiver, callback], Ty::Boolean),
                None => self.poison(span),
            },
            "every" => match check_callback(self, cx) {
                Some(callback) => call(self, "typedArrayEvery", vec![receiver, callback], Ty::Boolean),
                None => self.poison(span),
            },
            "reduce" | "reduceRight" => {
                // Both the initialized form `(cb, init)` and the no-initial
                // form `(cb)` (which seeds from the first/last element and
                // throws on an empty typed array, NUCLEUS #19).
                if args.is_empty() || args.len() > 2 {
                    self.error(span, format!("`{mname}` expects a callback and an optional initial value"));
                    return self.poison(span);
                }
                let callback = self.check_expr(cx, &args[0]);
                if !matches!(callback.ty, Ty::Function(_)) && !callback.ty.is_error() {
                    self.error(callback.span, format!("typed-array `{mname}` callback must be a function"));
                }
                let (native, call_args) = match (mname, args.len()) {
                    ("reduce", 2) => (
                        "typedArrayReduce",
                        vec![receiver, callback, box_any(self.check_expr(cx, &args[1]))],
                    ),
                    ("reduce", _) => ("typedArrayReduceNoInit", vec![receiver, callback]),
                    ("reduceRight", 2) => (
                        "typedArrayReduceRight",
                        vec![receiver, callback, box_any(self.check_expr(cx, &args[1]))],
                    ),
                    (_, _) => ("typedArrayReduceRightNoInit", vec![receiver, callback]),
                };
                call(self, native, call_args, Ty::Any)
            }
            _ => {
                self.error(span, format!("`{mname}` is not a typed-array method"));
                self.poison(span)
            }
        }
    }

    /// Lower `<Kind>Array.from(source, mapFn?)` and `<Kind>Array.of(...items)`.
    /// Both produce a view of `kind`; the kind crosses to the native as a
    /// table-order discriminant so one native serves all eleven kinds.
    /// A `DataView` element accessor (`getInt32`/`setFloat64`/…). The
    /// little-endian flag is optional and defaults to `false` (big-endian),
    /// per the spec; single-byte widths take no flag. Returns `None` for a
    /// method that is not one of these (so `byteLength`/`byteOffset`/`buffer`
    /// stay on the generic path).
    pub(in crate::check) fn check_data_view_method(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        // (native, element type, takes an endianness flag).
        let (native, elem, endian): (&str, Ty, bool) = match mname {
            "getInt8" => ("dataViewGetInt8", Ty::Int(IntKind::I8), false),
            "getUint8" => ("dataViewGetUint8", Ty::Int(IntKind::U8), false),
            "getInt16" => ("dataViewGetInt16", Ty::Int(IntKind::I16), true),
            "getUint16" => ("dataViewGetU16", Ty::Int(IntKind::U16), true),
            "getInt32" => ("dataViewGetInt32", Ty::Int(IntKind::I32), true),
            "getUint32" => ("dataViewGetU32", Ty::Int(IntKind::U32), true),
            "getInt64" => ("dataViewGetInt64", Ty::Int(IntKind::I64), true),
            "getUint64" => ("dataViewGetUint64", Ty::Int(IntKind::U64), true),
            "getFloat32" => ("dataViewGetFloat32", Ty::F32, true),
            "getFloat64" => ("dataViewGetF64", Ty::F64, true),
            "setInt8" => ("dataViewSetInt8", Ty::Int(IntKind::I8), false),
            "setUint8" => ("dataViewSetUint8", Ty::Int(IntKind::U8), false),
            "setInt16" => ("dataViewSetInt16", Ty::Int(IntKind::I16), true),
            "setUint16" => ("dataViewSetU16", Ty::Int(IntKind::U16), true),
            "setInt32" => ("dataViewSetInt32", Ty::Int(IntKind::I32), true),
            "setUint32" => ("dataViewSetU32", Ty::Int(IntKind::U32), true),
            "setInt64" => ("dataViewSetInt64", Ty::Int(IntKind::I64), true),
            "setUint64" => ("dataViewSetUint64", Ty::Int(IntKind::U64), true),
            "setFloat32" => ("dataViewSetFloat32", Ty::F32, true),
            "setFloat64" => ("dataViewSetF64", Ty::F64, true),
            // Only called behind `data_view_accessor`, which lists exactly
            // these names.
            other => unreachable!("not a DataView accessor: {other}"),
        };
        let is_set = mname.starts_with("set");
        let u64_ty = Ty::Int(IntKind::U64);
        let bool_lit = |value: bool, span: Span| hir::Expr {
            kind: hir::ExprKind::Boolean(value),
            ty: Ty::Boolean,
            span,
        };
        // Expected positional arity, excluding the optional endianness flag:
        // byteOffset (+ value for a setter).
        let fixed = if is_set { 2 } else { 1 };
        let max = fixed + usize::from(endian);
        if args.len() < fixed || args.len() > max {
            self.error(
                span,
                format!("`{mname}` expects {fixed} or {max} argument(s), found {}", args.len()),
            );
            return self.poison(span);
        }
        let mut call_args = vec![receiver];
        let mut offset = self.check_expr(cx, &args[0]);
        coerce_numeric_literal(&mut offset, u64_ty);
        // JS ToIndex: a `number` offset is truncated to an unsigned index.
        if offset.ty.is_float() {
            let span = offset.span;
            offset = hir::Expr { kind: hir::ExprKind::Convert { value: Box::new(offset) }, ty: u64_ty, span };
        }
        if !offset.ty.is_int() && !offset.ty.is_error() {
            self.error(offset.span, "DataView byte offset must be an integer");
        }
        call_args.push(offset);
        if is_set {
            let mut value = self.check_expr(cx, &args[1]);
            coerce_numeric_literal(&mut value, elem);
            if !value.ty.is_error() && !self.is_assignable(value.ty, elem) {
                self.error(value.span, format!("DataView value must be {elem:?}"));
            }
            call_args.push(value);
        }
        if endian {
            let flag = match args.get(fixed) {
                Some(arg) => {
                    let value = self.check_expr(cx, arg);
                    if value.ty != Ty::Boolean && !value.ty.is_error() {
                        self.error(value.span, "DataView endianness flag must be a boolean");
                    }
                    value
                }
                None => bool_lit(false, span),
            };
            call_args.push(flag);
        }
        let ret = if is_set { Ty::Boolean } else { elem };
        let Some((id, _)) = zun_natives::resolve("buffer", native) else {
            self.error(span, format!("missing DataView intrinsic `{native}`"));
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: call_args },
            ty: ret,
            span,
        }
    }

    pub(in crate::check) fn check_typed_array_static(
        &mut self,
        cx: &mut FunctionCx,
        kind: hir::BufferViewKind,
        method: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let view_ty = Ty::BufferView(kind);
        let Some(discriminant) = crate::typed_arrays::element_index(kind) else {
            self.error(span, "typed-array statics require an element kind".to_string());
            return self.poison(span);
        };
        let kind_lit = hir::Expr {
            kind: hir::ExprKind::Number(discriminant as f64),
            ty: Ty::Int(IntKind::U32),
            span,
        };
        let call = |checker: &mut Self, native: &str, call_args: Vec<hir::Expr>| {
            let Some((id, _)) = zun_natives::resolve("buffer", native) else {
                checker.error(span, format!("missing typed-array intrinsic `{native}`"));
                return checker.poison(span);
            };
            hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: call_args }, ty: view_ty, span }
        };
        match method {
            "of" => {
                // `<Kind>Array.of(...items)` is an array literal of the args,
                // then a copy into a view of the kind.
                let elements = args
                    .iter()
                    .map(|element| Some(self.check_expr(cx, element)))
                    .collect();
                let source = hir::Expr {
                    kind: hir::ExprKind::ArrayLiteral { elements },
                    ty: Ty::Array,
                    span,
                };
                call(self, "typedArrayOf", vec![source, kind_lit])
            }
            "from" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(span, format!("`from` expects 1 or 2 arguments, found {}", args.len()));
                    return self.poison(span);
                }
                let source = self.check_expr(cx, &args[0]);
                if !matches!(source.ty, Ty::Array | Ty::BufferView(_)) && !source.ty.is_error() {
                    self.error(source.span, "`from` source must be an array or typed array");
                }
                match args.get(1) {
                    Some(map_arg) => {
                        let map_fn = self.check_expr(cx, map_arg);
                        if !matches!(map_fn.ty, Ty::Function(_)) && !map_fn.ty.is_error() {
                            self.error(map_fn.span, "`from` map function must be a function");
                        }
                        call(self, "typedArrayFromMapped", vec![source, map_fn, kind_lit])
                    }
                    None => call(self, "typedArrayFrom", vec![source, kind_lit]),
                }
            }
            other => {
                self.error(span, format!("`{other}` is not a typed-array static"));
                self.poison(span)
            }
        }
    }
}

/// Whether `method` is a `DataView` element accessor handled by
/// [`Checker::check_data_view_method`] (the only methods whose endianness flag
/// is optional). Keeps the interceptor's gate and its match in lockstep.
pub(in crate::check) fn data_view_accessor(method: &str) -> bool {
    matches!(
        method,
        "getInt8"
            | "getUint8"
            | "getInt16"
            | "getUint16"
            | "getInt32"
            | "getUint32"
            | "getInt64"
            | "getUint64"
            | "getFloat32"
            | "getFloat64"
            | "setInt8"
            | "setUint8"
            | "setInt16"
            | "setUint16"
            | "setInt32"
            | "setUint32"
            | "setInt64"
            | "setUint64"
            | "setFloat32"
            | "setFloat64"
    )
}
