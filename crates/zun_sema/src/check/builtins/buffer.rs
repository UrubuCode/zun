use crate::check::*;

impl Checker<'_> {
    /// A Buffer argument passed through as a boxed `any` (`undefined` when the
    /// call omitted it), so a native can validate the raw JavaScript value.
    pub(in crate::check) fn buffer_raw_arg(
        &mut self,
        cx: &mut FunctionCx,
        arg: Option<&ast::Expr>,
        span: Span,
    ) -> hir::Expr {
        let expr = match arg {
            Some(arg) => self.check_expr(cx, arg),
            None => {
                let void_zero = ast::Expr::Unary {
                    op: ast::UnaryOp::Void,
                    operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                    span,
                };
                self.check_expr(cx, &void_zero)
            }
        };
        Self::box_to_any(expr)
    }

    /// `new Buffer(arg, encodingOrOffset, length)` / `Buffer(...)`.
    pub(in crate::check) fn check_buffer_ctor(
        &mut self,
        cx: &mut FunctionCx,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        if args.is_empty() || args.len() > 3 {
            self.error(span, "`Buffer` expects 1 to 3 arguments".to_string());
            return self.poison(span);
        }
        let Some((id, _)) = zun_natives::resolve("nodeBuffer", "nodeBufferCtor") else {
            self.error(span, "missing Buffer intrinsic `nodeBufferCtor`");
            return self.poison(span);
        };
        let call_args = (0..3)
            .map(|pos| self.buffer_raw_arg(cx, args.get(pos), span))
            .collect();
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: call_args },
            ty: Ty::BufferView(hir::BufferViewKind::U8),
            span,
        }
    }

    pub(in crate::check) fn check_buffer_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        // `Buffer.byteLength(...args)`: spread the list into a local once, then
        // call with its first elements (a missing one is `undefined`).
        if args.iter().any(|a| matches!(a, ast::Expr::Spread { .. })) {
            let elements = args
                .iter()
                .map(|arg| match arg {
                    ast::Expr::Spread { expr, .. } => ast::ArrayElement::Spread((**expr).clone()),
                    other => ast::ArrayElement::Expr(other.clone()),
                })
                .collect();
            let literal = ast::Expr::ArrayLiteral { elements, span };
            let list = self.check_expr(cx, &literal);
            let (slot, tmp) = self.synthetic_local(cx, Ty::Array);
            let assign = hir::Expr {
                kind: hir::ExprKind::Assign {
                    slot,
                    value: Box::new(list),
                },
                ty: Ty::Array,
                span,
            };
            let arity = match method {
                "from" | "alloc" | "copyBytesFrom" => 3,
                "concat" | "compare" | "byteLength" => 2,
                _ => 1,
            };
            let indexed: Vec<ast::Expr> = (0..arity)
                .map(|i| ast::Expr::Index {
                    object: Box::new(ast::Expr::Ident(tmp)),
                    index: Box::new(ast::Expr::Number {
                        value: i as f64,
                        span,
                    }),
                    span,
                })
                .collect();
            let call = self.check_buffer_static(cx, method, method_span, &indexed, span);
            let ty = call.ty;
            return hir::Expr {
                kind: hir::ExprKind::Sequence {
                    effect: Box::new(assign),
                    value: Box::new(call),
                },
                ty,
                span,
            };
        }
        let u8_view = Ty::BufferView(hir::BufferViewKind::U8);
        let call = |checker: &mut Self, native: &str, call_args: Vec<hir::Expr>, ty: Ty| {
            let Some((id, _)) = zun_natives::resolve("nodeBuffer", native) else {
                checker.error(span, format!("missing Buffer intrinsic `{native}`"));
                return checker.poison(span);
            };
            hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: call_args,
                },
                ty,
                span,
            }
        };
        match method {
            // Every numeric / value argument of the statics travels boxed as
            // `any`, so the native applies Node's validation (and error codes)
            // to the raw JavaScript value.
            "from" => {
                if args.len() > 3 {
                    self.error(span, "`Buffer.from` expects at most 3 arguments".to_string());
                    return self.poison(span);
                }
                let call_args = (0..3)
                    .map(|pos| self.buffer_raw_arg(cx, args.get(pos), span))
                    .collect();
                call(self, "nodeBufferFromAny", call_args, u8_view)
            }
            "of" => {
                // `Buffer.of(a, b, ..)` is `Buffer.from([a, b, ..])`.
                let elements = args
                    .iter()
                    .map(|arg| match arg {
                        ast::Expr::Spread { expr, .. } => ast::ArrayElement::Spread((**expr).clone()),
                        other => ast::ArrayElement::Expr(other.clone()),
                    })
                    .collect();
                let list = ast::Expr::ArrayLiteral { elements, span };
                let mut items = self.check_expr(cx, &list);
                items = Self::box_to_any(items);
                let undefined = self.buffer_raw_arg(cx, None, span);
                call(
                    self,
                    "nodeBufferFromAny",
                    vec![items, undefined.clone(), undefined],
                    u8_view,
                )
            }
            "alloc" => {
                if args.len() > 3 {
                    self.error(span, "`Buffer.alloc` expects at most 3 arguments".to_string());
                    return self.poison(span);
                }
                let call_args = (0..3)
                    .map(|pos| self.buffer_raw_arg(cx, args.get(pos), span))
                    .collect();
                call(self, "nodeBufferAlloc", call_args, u8_view)
            }
            "allocUnsafe" | "allocUnsafeSlow" => {
                if args.len() > 1 {
                    self.error(span, format!("`Buffer.{method}` expects at most 1 argument"));
                    return self.poison(span);
                }
                let size = self.buffer_raw_arg(cx, args.first(), span);
                call(self, "nodeBufferAllocUnsafe", vec![size], u8_view)
            }
            "concat" => {
                if args.len() > 2 {
                    self.error(
                        span,
                        "`Buffer.concat` expects a list and an optional length".to_string(),
                    );
                    return self.poison(span);
                }
                let call_args = (0..2)
                    .map(|pos| self.buffer_raw_arg(cx, args.get(pos), span))
                    .collect();
                call(self, "nodeBufferConcat", call_args, u8_view)
            }
            "copyBytesFrom" => {
                if args.len() > 3 {
                    self.error(span, "`Buffer.copyBytesFrom` expects at most 3 arguments".to_string());
                    return self.poison(span);
                }
                let call_args = (0..3)
                    .map(|pos| self.buffer_raw_arg(cx, args.get(pos), span))
                    .collect();
                call(self, "nodeBufferCopyBytesFrom", call_args, u8_view)
            }
            "isEncoding" => {
                if args.len() > 1 {
                    self.error(span, "`Buffer.isEncoding` expects at most 1 argument".to_string());
                    return self.poison(span);
                }
                let value = self.buffer_raw_arg(cx, args.first(), span);
                call(self, "nodeBufferIsEncoding", vec![value], Ty::Boolean)
            }
            "compare" => {
                if args.len() > 2 {
                    self.error(span, "`Buffer.compare` expects at most 2 arguments".to_string());
                    return self.poison(span);
                }
                let call_args = (0..2)
                    .map(|pos| self.buffer_raw_arg(cx, args.get(pos), span))
                    .collect();
                call(self, "nodeBufferCompare", call_args, Ty::F64)
            }
            "isBuffer" => {
                if args.len() > 1 {
                    self.error(span, "`Buffer.isBuffer` expects at most 1 argument".to_string());
                    return self.poison(span);
                }
                let arg = self.buffer_raw_arg(cx, args.first(), span);
                call(self, "nodeBufferIsBuffer", vec![arg], Ty::Boolean)
            }
            "byteLength" => {
                if args.len() > 2 {
                    self.error(
                        span,
                        "`Buffer.byteLength` expects at most 2 arguments".to_string(),
                    );
                    return self.poison(span);
                }
                let call_args = (0..2)
                    .map(|pos| self.buffer_raw_arg(cx, args.get(pos), span))
                    .collect();
                call(self, "nodeBufferByteLength", call_args, Ty::Int(IntKind::U64))
            }
            other => {
                self.error(
                    method_span,
                    format!("`Buffer.{other}` is not supported in this runtime"),
                );
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
        let call = |checker: &mut Self, native: &str, call_args: Vec<hir::Expr>, ty: Ty| {
            let Some((id, _)) = zun_natives::resolve("nodeBuffer", native) else {
                checker.error(span, format!("missing Buffer intrinsic `{native}`"));
                return checker.poison(span);
            };
            hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: call_args,
                },
                ty,
                span,
            }
        };
        // Every argument is passed boxed as `any` (`undefined` when omitted):
        // the natives run Node's own argument handling on the raw values.
        let raws = |checker: &mut Self, cx: &mut FunctionCx, count: usize| -> Vec<hir::Expr> {
            (0..count)
                .map(|pos| checker.buffer_raw_arg(cx, args.get(pos), span))
                .collect()
        };
        match mname {
            "toString" | "write" | "slice" | "copy" | "equals" | "compare" | "fill" => {
                let (native, max, ret) = match mname {
                    "toString" => ("nodeBufferToString", 3, Ty::String),
                    "write" => ("nodeBufferWrite", 4, u64_ty),
                    "slice" => ("nodeBufferSlice", 2, u8_view),
                    "copy" => ("nodeBufferCopy", 4, u64_ty),
                    "equals" => ("nodeBufferEquals", 1, Ty::Boolean),
                    "compare" => ("nodeBufferCompareRange", 5, Ty::F64),
                    _ => ("nodeBufferFill", 4, u8_view),
                };
                if args.len() > max || (mname != "toString" && mname != "slice" && args.is_empty()) {
                    self.error(span, format!("`{mname}` has the wrong number of arguments"));
                    return self.poison(span);
                }
                let mut call_args = vec![receiver];
                call_args.extend(raws(self, cx, max));
                call(self, native, call_args, ret)
            }
            // Node's per-encoding aliases are thin wrappers over `write` and
            // `toString` with the encoding fixed: `b.utf8Write(s, off, len)`
            // is `b.write(s, off, len, "utf8")` and `b.hexSlice(a, b)` is
            // `b.toString("hex", a, b)`. They are real own methods of
            // `Buffer.prototype`, not sugar, so code that reaches for them
            // (and this suite's buffer fixtures) needs them by name.
            other if zun_natives::buffer_encoding_alias(other).is_some() => {
                let (encoding, is_write) =
                    zun_natives::buffer_encoding_alias(other).expect("checked by the guard");
                let literal = Self::string_literal(encoding.to_owned(), span);
                if is_write {
                    if args.is_empty() || args.len() > 3 {
                        self.error(span, format!("`{other}` expects 1 to 3 arguments"));
                        return self.poison(span);
                    }
                    let mut call_args = vec![receiver];
                    call_args.extend(raws(self, cx, 3));
                    call_args.push(Self::box_to_any(literal));
                    call(self, "nodeBufferWrite", call_args, u64_ty)
                } else {
                    if args.len() > 2 {
                        self.error(span, format!("`{other}` expects at most 2 arguments"));
                        return self.poison(span);
                    }
                    let mut call_args = vec![receiver, Self::box_to_any(literal)];
                    call_args.extend(raws(self, cx, 2));
                    call(self, "nodeBufferToString", call_args, Ty::String)
                }
            }
            "toJSON" => {
                let json = call(self, "nodeBufferToJSON", vec![receiver], Ty::Any);
                json
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
                let (native, ret) = match mname {
                    "indexOf" => ("nodeBufferIndexOf", Ty::F64),
                    "lastIndexOf" => ("nodeBufferLastIndexOf", Ty::F64),
                    _ => ("nodeBufferIncludes", Ty::Boolean),
                };
                let mut call_args = vec![receiver];
                call_args.extend(raws(self, cx, 3));
                call(self, native, call_args, ret)
            }
            // Variable-width integer readers `readUIntLE(offset, byteLength)` and
            // writers `writeUIntLE(value, offset, byteLength)`, plus the fixed
            // width ones. Every argument travels **boxed as `any`** so the
            // native applies Node's validation (and its `ERR_*` errors) to the
            // raw value; an omitted one is `undefined`.
            m if m.starts_with("read") || m.starts_with("write") => {
                let native = buffer_accessor_native(m);
                let Some((id, _)) = zun_natives::resolve("nodeBuffer", &native) else {
                    self.error(span, format!("`{m}` is not a supported Buffer method"));
                    return self.poison(span);
                };
                let is_read = m.starts_with("read");
                let variable = m.ends_with("IntLE") || m.ends_with("IntBE");
                let max = match (is_read, variable) {
                    (true, true) => 2,
                    (true, false) => 1,
                    (false, true) => 3,
                    (false, false) => 2,
                };
                if args.len() > max || (!is_read && args.is_empty()) {
                    self.error(span, format!("`{m}` has the wrong number of arguments"));
                    return self.poison(span);
                }
                let mut call_args = vec![receiver];
                if is_read {
                    for pos in 0..max {
                        call_args.push(self.buffer_raw_arg(cx, args.get(pos), span));
                    }
                } else {
                    // Node order `(value, offset, byteLength?)`; the native takes
                    // `(view, offset, value, byteLength?)`.
                    let value = self.buffer_raw_arg(cx, args.first(), span);
                    call_args.push(self.buffer_raw_arg(cx, args.get(1), span));
                    call_args.push(value);
                    if variable {
                        call_args.push(self.buffer_raw_arg(cx, args.get(2), span));
                    }
                }
                let ty = {
                    let info = zun_natives::resolve("nodeBuffer", &native).map(|(_, info)| info);
                    info.map(|info| native_ty(info.ret)).unwrap_or(Ty::Any)
                };
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: call_args,
                    },
                    ty,
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
    zun_natives::node_buffer_accessor_native(method)
}

