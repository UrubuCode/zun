use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_buffer_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let u8_view = Ty::BufferView(hir::BufferViewKind::U8);
        let string_lit = |text: &str, span: Span| hir::Expr {
            kind: hir::ExprKind::String(text.to_string()),
            ty: Ty::String,
            span,
        };
        let int_lit = |value: u64, span: Span| hir::Expr {
            kind: hir::ExprKind::Number(value as f64),
            ty: Ty::Int(IntKind::U64),
            span,
        };
        let call = |checker: &mut Self, native: &str, call_args: Vec<hir::Expr>, ty: Ty| {
            let Some((id, _)) = zun_natives::resolve("nodeBuffer", native) else {
                checker.error(span, format!("missing Buffer intrinsic `{native}`"));
                return checker.poison(span);
            };
            hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: call_args }, ty, span }
        };
        match method {
            "from" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(span, "`Buffer.from` expects 1 or 2 arguments".to_string());
                    return self.poison(span);
                }
                let source = self.check_expr(cx, &args[0]);
                match source.ty {
                    Ty::String => {
                        let encoding = match args.get(1) {
                            Some(arg) => {
                                let value = self.check_expr(cx, arg);
                                if value.ty != Ty::String && !value.ty.is_error() {
                                    self.error(value.span, "Buffer encoding must be a string");
                                }
                                value
                            }
                            None => string_lit("utf8", span),
                        };
                        call(self, "nodeBufferFromString", vec![source, encoding], u8_view)
                    }
                    Ty::Array => call(self, "nodeBufferFromArray", vec![source], u8_view),
                    Ty::Buffer => {
                        call(self, "nodeBufferFromArrayBuffer", vec![source], u8_view)
                    }
                    other if !other.is_error() => {
                        self.error(
                            source.span,
                            format!("`Buffer.from` expects a string, array or ArrayBuffer, found {other:?}"),
                        );
                        self.poison(span)
                    }
                    _ => self.poison(span),
                }
            }
            "alloc" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(span, "`Buffer.alloc` expects 1 or 2 arguments".to_string());
                    return self.poison(span);
                }
                let mut size = self.check_expr(cx, &args[0]);
                coerce_numeric_literal(&mut size, Ty::Int(IntKind::U64));
                if !size.ty.is_int() && !size.ty.is_error() {
                    self.error(size.span, "Buffer size must be an integer");
                }
                let fill = match args.get(1) {
                    Some(arg) => {
                        let mut value = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut value, Ty::Int(IntKind::U8));
                        if !value.ty.is_int() && !value.ty.is_error() {
                            self.error(value.span, "Buffer fill byte must be an integer");
                        }
                        value
                    }
                    None => int_lit(0, span),
                };
                call(self, "nodeBufferAlloc", vec![size, fill], u8_view)
            }
            "allocUnsafe" => {
                if args.len() != 1 {
                    self.error(span, "`Buffer.allocUnsafe` expects 1 argument".to_string());
                    return self.poison(span);
                }
                let mut size = self.check_expr(cx, &args[0]);
                coerce_numeric_literal(&mut size, Ty::Int(IntKind::U64));
                if !size.ty.is_int() && !size.ty.is_error() {
                    self.error(size.span, "Buffer size must be an integer");
                }
                call(self, "nodeBufferAllocUnsafe", vec![size], u8_view)
            }
            "concat" => {
                if args.len() != 1 {
                    self.error(span, "`Buffer.concat` expects 1 argument (a list)".to_string());
                    return self.poison(span);
                }
                let list = self.check_expr(cx, &args[0]);
                if list.ty != Ty::Array && !list.ty.is_error() {
                    self.error(list.span, "`Buffer.concat` expects an array of buffers");
                }
                call(self, "nodeBufferConcat", vec![list], u8_view)
            }
            "isBuffer" => {
                if args.len() != 1 {
                    self.error(span, "`Buffer.isBuffer` expects 1 argument".to_string());
                    return self.poison(span);
                }
                let mut arg = self.check_expr(cx, &args[0]);
                if arg.ty != Ty::Any && !arg.ty.is_error() {
                    arg = hir::Expr {
                        span: arg.span,
                        kind: hir::ExprKind::BoxAny { value: Box::new(arg) },
                        ty: Ty::Any,
                    };
                }
                call(self, "nodeBufferIsBuffer", vec![arg], Ty::Boolean)
            }
            "byteLength" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(span, "`Buffer.byteLength` expects 1 or 2 arguments".to_string());
                    return self.poison(span);
                }
                let string = self.check_expr(cx, &args[0]);
                if string.ty != Ty::String && !string.ty.is_error() {
                    self.error(string.span, "`Buffer.byteLength` expects a string");
                }
                let encoding = match args.get(1) {
                    Some(arg) => self.check_expr(cx, arg),
                    None => string_lit("utf8", span),
                };
                call(self, "nodeBufferByteLength", vec![string, encoding], Ty::Int(IntKind::U64))
            }
            other => {
                self.error(method_span, format!("`Buffer.{other}` is not supported in this runtime"));
                self.poison(span)
            }
        }
    }

    /// Lower a `Buffer` instance method on a `U8` byte view. Handles the
    /// optional-argument surface (defaults applied here); fixed readers/writers
    /// are checked against their `nodeBuffer` native signature. A length-valued
    /// default (`toString`/`slice` end) is the sentinel `u64::MAX`, clamped to
    /// the real length by the interpreter floor.
    pub(in crate::check) fn check_buffer_method(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let u8_view = Ty::BufferView(hir::BufferViewKind::U8);
        let u64_ty = Ty::Int(IntKind::U64);
        let string_lit = |text: &str, span: Span| hir::Expr {
            kind: hir::ExprKind::String(text.to_string()),
            ty: Ty::String,
            span,
        };
        let int_lit = |value: u64, span: Span| hir::Expr {
            kind: hir::ExprKind::Number(value as f64),
            ty: u64_ty,
            span,
        };
        let call = |checker: &mut Self, native: &str, call_args: Vec<hir::Expr>, ty: Ty| {
            let Some((id, _)) = zun_natives::resolve("nodeBuffer", native) else {
                checker.error(span, format!("missing Buffer intrinsic `{native}`"));
                return checker.poison(span);
            };
            hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: call_args }, ty, span }
        };
        // Check an optional integer argument, applying `default` when absent.
        macro_rules! opt_int {
            ($pos:expr, $default:expr) => {{
                match args.get($pos) {
                    Some(arg) => {
                        let mut value = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut value, u64_ty);
                        if !value.ty.is_int() && !value.ty.is_error() {
                            self.error(value.span, "Buffer offset/index must be an integer");
                        }
                        value
                    }
                    None => int_lit($default, span),
                }
            }};
        }
        match mname {
            "toString" => {
                if args.len() > 3 {
                    self.error(span, "`toString` expects at most 3 arguments".to_string());
                    return self.poison(span);
                }
                let encoding = match args.first() {
                    Some(arg) => {
                        let value = self.check_expr(cx, arg);
                        if value.ty != Ty::String && !value.ty.is_error() {
                            self.error(value.span, "Buffer encoding must be a string");
                        }
                        value
                    }
                    None => string_lit("utf8", span),
                };
                let start = opt_int!(1, 0);
                let end = opt_int!(2, u64::MAX);
                call(self, "nodeBufferToString", vec![receiver, encoding, start, end], Ty::String)
            }
            "write" => {
                if args.is_empty() || args.len() > 3 {
                    self.error(span, "`write` expects 1 to 3 arguments".to_string());
                    return self.poison(span);
                }
                let string = self.check_expr(cx, &args[0]);
                if string.ty != Ty::String && !string.ty.is_error() {
                    self.error(string.span, "`write` expects a string");
                }
                let offset = opt_int!(1, 0);
                let encoding = match args.get(2) {
                    Some(arg) => self.check_expr(cx, arg),
                    None => string_lit("utf8", span),
                };
                call(self, "nodeBufferWrite", vec![receiver, string, offset, encoding], u64_ty)
            }
            "slice" => {
                if args.len() > 2 {
                    self.error(span, "`slice` expects at most 2 arguments".to_string());
                    return self.poison(span);
                }
                // Signed indices: a negative counts from the end. The `end`
                // default is a large positive sentinel the floor clamps to the
                // real length (a negative sentinel would be read as `len-1`).
                let i64_ty = Ty::Int(IntKind::I64);
                let opt_signed = |checker: &mut Self, cx: &mut FunctionCx, pos: usize, default: f64| {
                    match args.get(pos) {
                        Some(arg) => {
                            let mut value = checker.check_expr(cx, arg);
                            coerce_numeric_literal(&mut value, i64_ty);
                            if !value.ty.is_int() && !value.ty.is_error() {
                                checker.error(value.span, "Buffer offset/index must be an integer");
                            }
                            value
                        }
                        None => hir::Expr {
                            kind: hir::ExprKind::Number(default),
                            ty: i64_ty,
                            span,
                        },
                    }
                };
                let start = opt_signed(self, cx, 0, 0.0);
                let end = opt_signed(self, cx, 1, (1i64 << 53) as f64);
                call(self, "nodeBufferSlice", vec![receiver, start, end], u8_view)
            }
            "fill" => {
                if args.len() != 1 {
                    self.error(span, "`fill` expects 1 argument (a byte value)".to_string());
                    return self.poison(span);
                }
                let mut value = self.check_expr(cx, &args[0]);
                coerce_numeric_literal(&mut value, Ty::Int(IntKind::U8));
                if !value.ty.is_int() && !value.ty.is_error() {
                    self.error(value.span, "Buffer fill byte must be an integer");
                }
                call(self, "nodeBufferFill", vec![receiver, value], Ty::Boolean)
            }
            "equals" => {
                if args.len() != 1 {
                    self.error(span, "`equals` expects 1 argument".to_string());
                    return self.poison(span);
                }
                let other = self.check_expr(cx, &args[0]);
                if other.ty != u8_view && !other.ty.is_error() {
                    self.error(other.span, "`equals` expects another Buffer");
                }
                call(self, "nodeBufferEquals", vec![receiver, other], Ty::Boolean)
            }
            "copy" => {
                if args.is_empty() || args.len() > 4 {
                    self.error(span, "`copy` expects 1 to 4 arguments".to_string());
                    return self.poison(span);
                }
                let target = self.check_expr(cx, &args[0]);
                if target.ty != u8_view && !target.ty.is_error() {
                    self.error(target.span, "`copy` target must be a Buffer");
                }
                // Signed indices so a negative counts from the end (consistent
                // with `slice`). The `sourceEnd` default is a large positive
                // sentinel the floor clamps to the real length (NUCLEUS #21).
                let i64_ty = Ty::Int(IntKind::I64);
                let opt_signed = |checker: &mut Self, cx: &mut FunctionCx, pos: usize, default: f64| {
                    match args.get(pos) {
                        Some(arg) => {
                            let mut value = checker.check_expr(cx, arg);
                            coerce_numeric_literal(&mut value, i64_ty);
                            if !value.ty.is_int() && !value.ty.is_error() {
                                checker.error(value.span, "Buffer offset/index must be an integer");
                            }
                            value
                        }
                        None => hir::Expr {
                            kind: hir::ExprKind::Number(default),
                            ty: i64_ty,
                            span,
                        },
                    }
                };
                let target_start = opt_signed(self, cx, 1, 0.0);
                let source_start = opt_signed(self, cx, 2, 0.0);
                let source_end = opt_signed(self, cx, 3, (1i64 << 53) as f64);
                call(
                    self,
                    "nodeBufferCopy",
                    vec![receiver, target, target_start, source_start, source_end],
                    u64_ty,
                )
            }
            "swap16" | "swap32" | "swap64" => {
                if !args.is_empty() {
                    self.error(span, format!("`{mname}` expects no arguments"));
                    return self.poison(span);
                }
                let native = match mname {
                    "swap16" => "nodeBufferSwap16",
                    "swap32" => "nodeBufferSwap32",
                    _ => "nodeBufferSwap64",
                };
                call(self, native, vec![receiver], u8_view)
            }
            "indexOf" | "includes" | "lastIndexOf" => {
                if args.is_empty() || args.len() > 3 {
                    self.error(span, format!("`{mname}` expects 1 to 3 arguments"));
                    return self.poison(span);
                }
                // The needle (number / string / Buffer) is boxed to `Any`; the
                // optional byteOffset is signed; the optional encoding is a
                // string. `lastIndexOf` defaults its offset to the length (a
                // large sentinel the floor clamps down).
                let needle_raw = self.check_expr(cx, &args[0]);
                let needle = if needle_raw.ty == Ty::Any || needle_raw.ty.is_error() {
                    needle_raw
                } else {
                    hir::Expr {
                        span: needle_raw.span,
                        kind: hir::ExprKind::BoxAny { value: Box::new(needle_raw) },
                        ty: Ty::Any,
                    }
                };
                let i64_ty = Ty::Int(IntKind::I64);
                let default_offset = if mname == "lastIndexOf" { (1i64 << 53) as f64 } else { 0.0 };
                let byte_offset = match args.get(1) {
                    Some(arg) => {
                        let mut value = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut value, i64_ty);
                        if !value.ty.is_int() && !value.ty.is_error() {
                            self.error(value.span, "Buffer byteOffset must be an integer");
                        }
                        value
                    }
                    None => hir::Expr {
                        kind: hir::ExprKind::Number(default_offset),
                        ty: i64_ty,
                        span,
                    },
                };
                let encoding = match args.get(2) {
                    Some(arg) => self.check_expr(cx, arg),
                    None => string_lit("utf8", span),
                };
                let (native, ret) = match mname {
                    "indexOf" => ("nodeBufferIndexOf", Ty::Int(IntKind::I64)),
                    "lastIndexOf" => ("nodeBufferLastIndexOf", Ty::Int(IntKind::I64)),
                    _ => ("nodeBufferIncludes", Ty::Boolean),
                };
                call(self, native, vec![receiver, needle, byte_offset, encoding], ret)
            }
            // Variable-width integer readers `readUIntLE(offset, byteLength)`.
            "readUIntLE" | "readUIntBE" | "readIntLE" | "readIntBE" => {
                let native = buffer_accessor_native(mname);
                let Some((id, info)) = zun_natives::resolve("nodeBuffer", &native) else {
                    self.error(span, format!("`{mname}` is not a supported Buffer reader"));
                    return self.poison(span);
                };
                if args.len() != 2 {
                    self.error(span, format!("`{mname}` expects an offset and a byteLength"));
                    return self.poison(span);
                }
                let offset = opt_int!(0, 0);
                let byte_length = opt_int!(1, 1);
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, offset, byte_length] },
                    ty: native_ty(info.ret),
                    span,
                }
            }
            // Variable-width integer writers `writeUIntLE(value, offset, byteLength)`.
            "writeUIntLE" | "writeUIntBE" | "writeIntLE" | "writeIntBE" => {
                let native = buffer_accessor_native(mname);
                let Some((id, info)) = zun_natives::resolve("nodeBuffer", &native) else {
                    self.error(span, format!("`{mname}` is not a supported Buffer writer"));
                    return self.poison(span);
                };
                if args.len() != 3 {
                    self.error(span, format!("`{mname}` expects a value, offset and byteLength"));
                    return self.poison(span);
                }
                // Node order `(value, offset, byteLength)`; native takes
                // `(view, offset, value, byteLength)`.
                let mut value = self.check_expr(cx, &args[0]);
                let value_ty = native_ty(info.params[2]);
                coerce_numeric_literal(&mut value, value_ty);
                if !value.ty.is_error() && !self.is_assignable(value.ty, value_ty) {
                    self.error(value.span, format!("argument type {:?} does not match {value_ty:?}", value.ty));
                }
                let offset = opt_int!(1, 0);
                let byte_length = opt_int!(2, 1);
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![receiver, offset, value, byte_length],
                    },
                    ty: Ty::Int(IntKind::U64),
                    span,
                }
            }
            read if read.starts_with("read") => {
                let native = buffer_accessor_native(read);
                let Some((id, info)) = zun_natives::resolve("nodeBuffer", &native) else {
                    self.error(span, format!("`{read}` is not a supported Buffer reader"));
                    return self.poison(span);
                };
                if args.len() > 1 {
                    self.error(span, format!("`{read}` expects at most 1 argument"));
                    return self.poison(span);
                }
                let offset = opt_int!(0, 0);
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, offset] },
                    ty: native_ty(info.ret),
                    span,
                }
            }
            write if write.starts_with("write") => {
                let native = buffer_accessor_native(write);
                let Some((id, info)) = zun_natives::resolve("nodeBuffer", &native) else {
                    self.error(span, format!("`{write}` is not a supported Buffer writer"));
                    return self.poison(span);
                };
                if args.is_empty() || args.len() > 2 {
                    self.error(span, format!("`{write}` expects 1 or 2 arguments"));
                    return self.poison(span);
                }
                // Node order is `write<Kind>(value, offset?)`; the native takes
                // `(view, offset, value)`.
                let mut value = self.check_expr(cx, &args[0]);
                let value_ty = native_ty(info.params[2]);
                coerce_numeric_literal(&mut value, value_ty);
                if !value.ty.is_error() && !self.is_assignable(value.ty, value_ty) {
                    self.error(value.span, format!("argument type {:?} does not match {value_ty:?}", value.ty));
                }
                let offset = opt_int!(1, 0);
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, offset, value] },
                    ty: Ty::Int(IntKind::U64),
                    span,
                }
            }
            other => {
                self.error(span, format!("`{other}` is not a supported Buffer method"));
                self.poison(span)
            }
        }
    }
}

/// The `nodeBuffer` native name for a Buffer reader/writer method, e.g.
/// `readUInt16LE` -> `nodeBufferReadUInt16LE`.
pub(in crate::check) fn buffer_accessor_native(method: &str) -> String {
    let mut chars = method.chars();
    let first = chars.next().map(|c| c.to_ascii_uppercase()).unwrap_or_default();
    format!("nodeBuffer{first}{}", chars.as_str())
}
