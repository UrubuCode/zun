//! FinalizationRegistry, buffer-view, Set, and Buffer method-call
//! interceptors for `check_method_call` (see `method_call.rs`).
//!
//! The value-phase interceptors for a `FinalizationRegistry`, typed-array /
//! `DataView` buffer views, set-like operations, and Node `Buffer` receivers.
//! Each takes the checked receiver by value and returns `Ok` when it handles
//! the call or `Err(receiver)` to fall through; order is preserved exactly.

use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn mcall_buffer_views(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        _method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        if receiver.ty == Ty::FinalizationRegistry && (mname == "register" || mname == "unregister")
        {
            let null_sentinel = |span| hir::Expr {
                kind: hir::ExprKind::Null,
                ty: Ty::Any,
                span,
            };
            if mname == "register" {
                if args.len() != 2 && args.len() != 3 {
                    self.error(
                        span,
                        format!(
                            "`register` expects 2 or 3 argument(s), found {}",
                            args.len()
                        ),
                    );
                    return Ok(self.poison(span));
                }
                let target = self.check_expr(cx, &args[0]);
                let held = self.check_expr(cx, &args[1]);
                if held.ty.is_error() {
                    return Ok(self.poison(span));
                }
                // The held value is arbitrary: it is boxed into `Any`,
                // GC-traced while the registration is pending, and
                // delivered to the cleanup callback on collection.
                let held = if held.ty == Ty::Any {
                    held
                } else {
                    hir::Expr {
                        span: held.span,
                        kind: hir::ExprKind::BoxAny {
                            value: Box::new(held),
                        },
                        ty: Ty::Any,
                    }
                };
                let token = match args.get(2) {
                    Some(arg) => self.check_expr(cx, arg),
                    None => null_sentinel(span),
                };
                let Some((id, _)) = zun_natives::resolve("collections", "finalizationRegister")
                else {
                    self.error(span, "missing intrinsic `finalizationRegister`");
                    return Ok(self.poison(span));
                };
                return Ok(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![receiver, target, held, token],
                    },
                    ty: Ty::Void,
                    span,
                });
            }
            // `unregister(unregisterToken)`.
            if args.len() != 1 {
                self.error(
                    span,
                    format!("`unregister` expects 1 argument, found {}", args.len()),
                );
                return Ok(self.poison(span));
            }
            let token = self.check_expr(cx, &args[0]);
            let Some((id, _)) = zun_natives::resolve("collections", "finalizationUnregister")
            else {
                self.error(span, "missing intrinsic `finalizationUnregister`");
                return Ok(self.poison(span));
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![receiver, token],
                },
                ty: Ty::Boolean,
                span,
            });
        }
        // Node `Buffer` instance surface. A Buffer is a `U8` byte view,
        // so these methods live on that kind; they take optional
        // arguments (defaults applied here) and route to the
        // `nodeBuffer` natives, which operate over the shared byte
        // floor. Placed before the generated typed-array dispatch so
        // `slice`/`subarray`/`fill` get the Buffer (optional-arg)
        // semantics rather than the typed-array ones.
        // A Buffer is a `U8` view, so it shares its type with a plain
        // `Uint8Array`. Only intercept the Buffer-unique method shapes;
        // `fill`/`subarray`/`get`/`set`/`length`/`byteLength` keep their
        // typed-array semantics (generic dispatch below). `fill(value)`
        // with a single argument is the one Node form added here (the
        // 3-arg typed-array `fill` still falls through).
        if receiver.ty == Ty::BufferView(hir::BufferViewKind::U8) {
            let intercept = match mname {
                "toString" | "write" | "slice" | "equals" | "copy" => true,
                // Node Buffer search accepts number / string / Buffer
                // needles (richer than the numeric typed-array form).
                "indexOf" | "includes" | "lastIndexOf" => true,
                "swap16" | "swap32" | "swap64" => true,
                "fill" => args.len() == 1,
                other => other.starts_with("read") || other.starts_with("write"),
            };
            if intercept {
                return Ok(self.check_buffer_method(cx, receiver, mname, args, span));
            }
        }
        // The second-generation typed-array surface (bulk `set`,
        // `slice`/`copyWithin`, the functional methods). These are
        // kind-agnostic natives that need optional-argument handling
        // and `set` arity/type overloading, so they are lowered by a
        // dedicated routine rather than the generic method table. Not
        // for `DataView`, and `slice` on a `U8` view is already taken
        // by the Node Buffer intercept above (Buffer semantics).
        if let Ty::BufferView(kind) = receiver.ty {
            if kind != hir::BufferViewKind::DataView
                && matches!(
                    mname,
                    "set"
                        | "slice"
                        | "copyWithin"
                        | "indexOf"
                        | "includes"
                        | "join"
                        | "forEach"
                        | "map"
                        | "filter"
                        | "reduce"
                        | "reduceRight"
                        | "find"
                        | "findIndex"
                        | "some"
                        | "every"
                )
            {
                return Ok(self.check_typed_array_method(cx, receiver, kind, mname, args, span));
            }
        }
        Err(receiver)
    }

    pub(in crate::check) fn mcall_buffer_set(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        _method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        // `DataView` element accessors, with an **optional** little-
        // endian flag that defaults to `false` (big-endian) as the
        // spec requires. Previously the flag was mandatory, so
        // `view.getUint32(0)` failed to type-check; the single-byte
        // accessors never take one.
        if receiver.ty == Ty::BufferView(hir::BufferViewKind::DataView) && data_view_accessor(mname)
        {
            return Ok(self.check_data_view_method(cx, receiver, mname, args, span));
        }
        // ES2025 Set operations lower to the prelude `__zunSetOp*` helpers,
        // which follow the set-like protocol on the argument. The seven member
        // names and their helper selection come from the one authoritative
        // `zun_natives` Set-operation table shared with the dynamic path.
        if let Some(set_op) = (receiver.ty == Ty::Set && args.len() == 1)
            .then(|| zun_natives::set_operation(mname))
            .flatten()
        {
            let predicate = set_op.predicate;
            let helper = set_op.helper();
            if let Some(&func) = self.by_name.get(helper) {
                let argument = self.check_expr(cx, &args[0]);
                let op = hir::Expr {
                    kind: hir::ExprKind::String(mname.to_string()),
                    ty: Ty::String,
                    span,
                };
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Call {
                        func,
                        args: vec![receiver, op, Self::box_to_any(argument)],
                    },
                    ty: if predicate { Ty::Boolean } else { Ty::Set },
                    span,
                });
            }
        }
        // `ArrayBuffer.prototype.transfer(newLength?)` and
        // `transferToFixedLength(newLength?)` (NUCLEUS #17). Hand-lowered
        // rather than routed through the fixed-arity `managed_method`
        // table because `newLength` is optional: when omitted, a `-1`
        // sentinel is synthesized meaning "keep the source length".
        // Both detach the source and yield a fresh `Buffer`.
        if receiver.ty == Ty::Buffer && (mname == "transfer" || mname == "transferToFixedLength") {
            if args.len() > 1 {
                self.error(
                    span,
                    format!("`{mname}` expects 0 or 1 argument(s), found {}", args.len()),
                );
                return Ok(self.poison(span));
            }
            let new_length = match args.first() {
                Some(arg) => {
                    let mut length = self.check_expr(cx, arg);
                    coerce_numeric_literal(&mut length, Ty::Int(IntKind::I64));
                    if !length.ty.is_error() && !length.ty.is_int() {
                        self.error(length.span, "transfer length must be an integer");
                    }
                    length
                }
                None => hir::Expr {
                    kind: hir::ExprKind::Number(-1.0),
                    ty: Ty::Int(IntKind::I64),
                    span,
                },
            };
            let native = if mname == "transfer" {
                "bufferTransfer"
            } else {
                "bufferTransferFixed"
            };
            let Some((id, _)) = zun_natives::resolve("buffer", native) else {
                self.error(span, format!("missing buffer intrinsic `{native}`"));
                return Ok(self.poison(span));
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![receiver, new_length],
                },
                ty: Ty::Buffer,
                span,
            });
        }
        // `String.prototype.normalize(form?)` (NUCLEUS #1). Hand-lowered
        // because `form` is optional: the ECMAScript default `"NFC"` is
        Err(receiver)
    }
}

impl Checker<'_> {
    /// Whether a method call on a typed-array / `DataView` / `ArrayBuffer`
    /// receiver has no statically typed lowering and must be resolved by name
    /// at run time instead (`subarray(1)`, `at`, `reverse`, `entries`, ...).
    fn view_call_is_dynamic(receiver: Ty, mname: &str, argc: usize) -> bool {
        match receiver {
            Ty::BufferView(hir::BufferViewKind::DataView) => true,
            Ty::BufferView(kind) => {
                let u8_buffer = kind == hir::BufferViewKind::U8;
                if u8_buffer {
                    let node = matches!(
                        mname,
                        "toString"
                            | "write"
                            | "equals"
                            | "copy"
                            | "lastIndexOf"
                            | "swap16"
                            | "swap32"
                            | "swap64"
                    ) || mname.starts_with("read")
                        || mname.starts_with("write");
                    if node {
                        return false;
                    }
                }
                match mname {
                    "length" | "byteLength" | "byteOffset" | "buffer" => argc != 0,
                    "get" => argc != 1,
                    "subarray" => argc != 2,
                    "fill" => true,
                    "set" | "slice" | "copyWithin" | "indexOf" | "includes" | "join"
                    | "forEach" | "map" | "filter" | "reduce" | "reduceRight" | "find"
                    | "findIndex" | "some" | "every" => false,
                    _ => true,
                }
            }
            Ty::Buffer => {
                !matches!(
                    mname,
                    "byteLength"
                        | "slice"
                        | "maxByteLength"
                        | "resizable"
                        | "detached"
                        | "resize"
                        | "transfer"
                        | "transferToFixedLength"
                ) || (mname == "slice" && argc != 2)
            }
            _ => false,
        }
    }

    /// Dynamic member call on a buffer-view receiver: the same by-name
    /// `dynamicCallN` dispatch an `any` receiver gets.
    pub(in crate::check) fn mcall_view_dynamic_forced(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        if args.len() > 6
            || args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))

        {
            return Err(receiver);
        }
        let native_name = [
            "dynamicCall0",
            "dynamicCall1",
            "dynamicCall2",
            "dynamicCall3",
            "dynamicCall4",
            "dynamicCall5",
            "dynamicCall6",
        ][args.len()];
        let Some((id, _)) = zun_natives::resolve("collections", native_name) else {
            self.error(span, format!("missing collection intrinsic `{native_name}`"));
            return Ok(self.poison(span));
        };
        let mut call_args = vec![
            Self::box_to_any(receiver),
            hir::Expr {
                kind: hir::ExprKind::String(mname.to_string()),
                ty: Ty::String,
                span: method.span,
            },
        ];
        for arg in args {
            let value = self.check_expr(cx, arg);
            call_args.push(value);
        }
        Ok(hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: call_args,
            },
            ty: Ty::Any,
            span,
        })
    }
}

impl Checker<'_> {
    /// [`Self::mcall_view_dynamic_forced`] when the call has no typed form.
    pub(in crate::check) fn mcall_view_dynamic(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        if !Self::view_call_is_dynamic(receiver.ty, mname, args.len()) {
            return Err(receiver);
        }
        self.mcall_view_dynamic_forced(cx, receiver, mname, method, args, span)
    }
}
