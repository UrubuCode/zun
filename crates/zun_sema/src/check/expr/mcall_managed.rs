//! String `normalize` and the table-driven managed-collection / typed-array
//! method dispatch for `check_method_call` (see `method_call.rs`).
//!
//! `mcall_managed` owns the single shared dispatch site for the managed
//! collections (Array, Map, Set) and typed arrays routed to natives,
//! including the callback `thisArg` binding. The large descriptor lookup
//! lives in the pure `managed_method_descriptor` table and the native-module
//! routing in `managed_native_module`; both are cohesive data tables the
//! dispatch consumes. Each helper returns `Ok` when it handles the call or
//! `Err(receiver)` to fall through; order is preserved.

use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn mcall_string_normalize(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        _method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        if receiver.ty == Ty::String && mname == "normalize" {
            if args.len() > 1 {
                self.error(
                    span,
                    format!(
                        "`normalize` expects 0 or 1 argument(s), found {}",
                        args.len()
                    ),
                );
                return Ok(self.poison(span));
            }
            let form = match args.first() {
                Some(arg) => {
                    let checked = self.check_expr(cx, arg);
                    if !checked.ty.is_error() && checked.ty != Ty::String {
                        self.error(checked.span, "normalize form must be a string");
                    }
                    checked
                }
                None => hir::Expr {
                    kind: hir::ExprKind::String("NFC".to_string()),
                    ty: Ty::String,
                    span,
                },
            };
            let Some((id, _)) = zun_natives::resolve("string", "stringNormalize") else {
                self.error(span, "missing string intrinsic `stringNormalize`");
                return Ok(self.poison(span));
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![receiver, form],
                },
                ty: Ty::String,
                span,
            });
        }
        Err(receiver)
    }

    /// Pure descriptor table mapping a managed-collection / typed-array
    /// receiver type and method name to its native name, return type, and
    /// expected argument types. Typed-array methods are consulted first.
    fn managed_method_descriptor(
        receiver_ty: Ty,
        mname: &str,
    ) -> Option<(&'static str, Ty, Vec<Ty>)> {
        let typed_array_method = match receiver_ty {
            Ty::BufferView(kind) => crate::typed_arrays::method(kind, mname),
            _ => None,
        };
        if typed_array_method.is_some() {
            return typed_array_method;
        }
        // Standard `Array.prototype` methods routed to natives: the native
        // name comes from the one authoritative `zun_natives` table shared
        // with the dynamic `any`-array path; only the sema-side return and
        // argument types stay here.
        if receiver_ty == Ty::Array {
            if let Some(descriptor) = Self::array_method_descriptor(mname) {
                return Some(descriptor);
            }
        }
        // Standard `Map`/`Set` methods routed to natives: the native name comes
        // from the one authoritative `zun_natives` collection-method table
        // shared with the dynamic `any`-receiver path; only the sema-side return
        // and argument types stay here.
        if matches!(receiver_ty, Ty::Map | Ty::Set) {
            if let Some(descriptor) = Self::collection_method_descriptor(receiver_ty, mname) {
                return Some(descriptor);
            }
        }
        // The remaining pure, fixed-arity managed-method rows (WeakMap /
        // WeakSet / WeakRef / RegExp / BigInt / the Promise accessors / the
        // fixed-arity String methods / DataView / ArrayBuffer) live in the one
        // authoritative `zun_natives` table, mirroring the Array and Map/Set
        // sections above; only the neutral-tag → `Ty` mapping stays here.
        let tag = Self::managed_receiver_tag(receiver_ty)?;
        let row = zun_natives::managed_method(tag, mname)?;
        Some((
            row.native,
            Self::managed_ty_to_ty(row.ret),
            row.args.iter().map(|arg| Self::managed_ty_to_ty(*arg)).collect(),
        ))
    }

    /// Map a receiver type to its neutral [`zun_natives::ManagedTy`] tag for the
    /// fixed-arity managed-method table lookup, or `None` for a receiver the
    /// table never names (those are handled by the earlier typed-array, Array,
    /// and Map/Set sections, or fall through to the primitive-tail interceptors).
    fn managed_receiver_tag(receiver_ty: Ty) -> Option<zun_natives::ManagedTy> {
        use zun_natives::ManagedTy as M;
        Some(match receiver_ty {
            Ty::WeakMap => M::WeakMap,
            Ty::WeakSet => M::WeakSet,
            Ty::WeakRef => M::WeakRef,
            Ty::RegExp => M::RegExp,
            Ty::BigInt => M::BigInt,
            Ty::Promise => M::Promise,
            Ty::String => M::Str,
            Ty::Buffer => M::Buffer,
            Ty::BufferView(hir::BufferViewKind::DataView) => M::DataView,
            _ => return None,
        })
    }

    /// Map a neutral [`zun_natives::ManagedTy`] tag from the managed-method table
    /// back to the checker's own [`Ty`]. One-to-one with
    /// [`Self::managed_receiver_tag`].
    fn managed_ty_to_ty(tag: zun_natives::ManagedTy) -> Ty {
        use zun_natives::ManagedTy as M;
        match tag {
            M::Any => Ty::Any,
            M::Bool => Ty::Boolean,
            M::Str => Ty::String,
            M::F64 => Ty::F64,
            M::F32 => Ty::F32,
            M::I8 => Ty::Int(IntKind::I8),
            M::I16 => Ty::Int(IntKind::I16),
            M::I32 => Ty::Int(IntKind::I32),
            M::I64 => Ty::Int(IntKind::I64),
            M::U8 => Ty::Int(IntKind::U8),
            M::U16 => Ty::Int(IntKind::U16),
            M::U32 => Ty::Int(IntKind::U32),
            M::U64 => Ty::Int(IntKind::U64),
            M::Array => Ty::Array,
            M::Buffer => Ty::Buffer,
            M::WeakMap => Ty::WeakMap,
            M::WeakSet => Ty::WeakSet,
            M::WeakRef => Ty::WeakRef,
            M::RegExp => Ty::RegExp,
            M::BigInt => Ty::BigInt,
            M::Promise => Ty::Promise,
            M::DataView => Ty::BufferView(hir::BufferViewKind::DataView),
        }
    }

    /// The sema-side return and argument types for a standard `Array.prototype`
    /// method, paired with its native name read from the single authoritative
    /// `zun_natives` array-method table. Returns `None` for array methods the
    /// typed path lowers through its own interceptors (`slice`, `concat`,
    /// `splice`, `sort`, `push`, `pop`, …) rather than this fixed-arity table.
    fn array_method_descriptor(mname: &str) -> Option<(&'static str, Ty, Vec<Ty>)> {
        let dynamic_fn = Ty::Function(hir::DYNAMIC_FUNCTION);
        // The callback methods invoke the dynamic function value with the
        // ECMAScript `(value, index, array)` signature (extra arguments
        // dropped to the callback's declared arity).
        let (return_ty, expected): (Ty, Vec<Ty>) = match mname {
            "forEach" => (Ty::Void, vec![dynamic_fn]),
            "map" => (Ty::Array, vec![dynamic_fn]),
            "filter" => (Ty::Array, vec![dynamic_fn]),
            "find" => (Ty::Any, vec![dynamic_fn]),
            "findIndex" => (Ty::Int(IntKind::I32), vec![dynamic_fn]),
            "some" => (Ty::Boolean, vec![dynamic_fn]),
            "every" => (Ty::Boolean, vec![dynamic_fn]),
            "join" => (Ty::String, vec![Ty::String]),
            "shift" => (Ty::Any, vec![]),
            "unshift" => (Ty::Int(IntKind::U64), vec![Ty::Any]),
            // `at(index)` with relative/undefined semantics (native).
            "at" => (Ty::Any, vec![Ty::F64]),
            "findLast" => (Ty::Any, vec![dynamic_fn]),
            "findLastIndex" => (Ty::Int(IntKind::I32), vec![dynamic_fn]),
            "flatMap" => (Ty::Array, vec![dynamic_fn]),
            "group" => (Ty::DynamicObject, vec![dynamic_fn]),
            "groupToMap" => (Ty::Map, vec![dynamic_fn]),
            "keys" => (Ty::Any, vec![]),
            "values" => (Ty::Any, vec![]),
            "entries" => (Ty::Any, vec![]),
            _ => return None,
        };
        let native = zun_natives::array_method(mname)?.native;
        Some((native, return_ty, expected))
    }

    /// The sema-side return and argument types for a standard `Map`/`Set`
    /// method, paired with its native name read from the single authoritative
    /// `zun_natives` collection-method table shared with the dynamic
    /// `any`-receiver path. `size` is a property-style accessor (no shared
    /// method-call native), so it stays here directly. The ES2025 Set
    /// operations are lowered in `mcall_buffer_set` to the prelude helpers, not
    /// routed through this native table. Returns `None` for any other member.
    fn collection_method_descriptor(
        receiver_ty: Ty,
        mname: &str,
    ) -> Option<(&'static str, Ty, Vec<Ty>)> {
        let dynamic_fn = Ty::Function(hir::DYNAMIC_FUNCTION);
        match receiver_ty {
            Ty::Map => {
                if mname == "size" {
                    return Some(("mapSize", Ty::Int(IntKind::U64), vec![]));
                }
                let (return_ty, expected): (Ty, Vec<Ty>) = match mname {
                    "set" => (Ty::Map, vec![Ty::Any, Ty::Any]),
                    "get" => (Ty::Any, vec![Ty::Any]),
                    "has" => (Ty::Boolean, vec![Ty::Any]),
                    "delete" => (Ty::Boolean, vec![Ty::Any]),
                    "clear" => (Ty::Void, vec![]),
                    "values" => (Ty::Any, vec![]),
                    "keys" => (Ty::Any, vec![]),
                    "entries" => (Ty::Any, vec![]),
                    "forEach" => (Ty::Void, vec![dynamic_fn]),
                    _ => return None,
                };
                Some((zun_natives::map_method(mname)?.native, return_ty, expected))
            }
            Ty::Set => {
                if mname == "size" {
                    return Some(("setSize", Ty::Int(IntKind::U64), vec![]));
                }
                let (return_ty, expected): (Ty, Vec<Ty>) = match mname {
                    "add" => (Ty::Set, vec![Ty::Any]),
                    "has" => (Ty::Boolean, vec![Ty::Any]),
                    "delete" => (Ty::Boolean, vec![Ty::Any]),
                    "clear" => (Ty::Void, vec![]),
                    "values" => (Ty::Any, vec![]),
                    "keys" => (Ty::Any, vec![]),
                    "entries" => (Ty::Any, vec![]),
                    "forEach" => (Ty::Void, vec![dynamic_fn]),
                    _ => return None,
                };
                Some((zun_natives::set_method(mname)?.native, return_ty, expected))
            }
            _ => None,
        }
    }

    /// The native module a managed/typed-array intrinsic resolves against,
    /// keyed by the native name (and whether the receiver is a string).
    fn managed_native_module(native_name: &str, receiver_is_string: bool) -> &'static str {
        if receiver_is_string {
            "string"
        } else if native_name.starts_with("dataView")
            || native_name.starts_with("buffer")
            // The typed-array natives, by the prefixes the master
            // table gives them. Matching the method suffix
            // instead (`ArrayGet`, `ArraySet`) missed
            // `u32ArrayLength` and friends, which then resolved
            // against `zun:collections` and were reported
            // missing; a hand-written list of prefixes missed
            // every kind added after it was written.
            || crate::typed_arrays::NATIVE_PREFIXES
                .iter()
                .any(|prefix| native_name.starts_with(prefix))
        {
            "buffer"
        } else if native_name.starts_with("promise") {
            "promise"
        } else if matches!(
            native_name,
            "arrayAt"
                | "arrayFindLast"
                | "arrayFindLastIndex"
                | "arrayFlat"
                | "arrayFlatMap"
                | "arrayGroup"
                | "arrayGroupToMap"
                | "arrayKeys"
                | "arrayValues"
                | "arrayEntries"
                | "arraySpliceInsert"
                | "arrayReduceNoInit"
                | "arrayReduceRightNoInit"
                | "arrayIsArray"
                | "arrayFrom"
                | "arrayFromMapped"
        ) {
            "array"
        } else {
            "collections"
        }
    }

    pub(in crate::check) fn mcall_managed(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        _method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        let Some((native_name, return_ty, expected)) =
            Self::managed_method_descriptor(receiver.ty, mname)
        else {
            return Err(receiver);
        };
        // A callback method's optional `thisArg` is bound onto the
        // callback (`cb.bind(thisArg)`) before the native runs.
        let undefined_ast = ast::Expr::Unary {
            op: ast::UnaryOp::Void,
            operand: Box::new(ast::Expr::Number { value: 0.0, span }),
            span,
        };
        let this_arg_ast = if matches!(receiver.ty, Ty::Array | Ty::Map | Ty::Set)
            && expected.len() == 1
            && args.len() == 2
            && expected[0] == Ty::Function(hir::DYNAMIC_FUNCTION)
        {
            args.get(1)
        } else if matches!(receiver.ty, Ty::Array | Ty::Map | Ty::Set)
            && expected.len() == 1
            && args.len() == 1
            && expected[0] == Ty::Function(hir::DYNAMIC_FUNCTION)
            && self.ast_function_takes_this(&args[0])
        {
            // A callback with an explicit `this` parameter and no thisArg
            // is called with `this` undefined.
            Some(&undefined_ast)
        } else {
            None
        };
        let args = if matches!(receiver.ty, Ty::Array | Ty::Map | Ty::Set)
            && expected.len() == 1
            && args.len() == 2
            && expected[0] == Ty::Function(hir::DYNAMIC_FUNCTION)
        {
            &args[..1]
        } else {
            args
        };
        if args.len() != expected.len() {
            self.error(
                span,
                format!(
                    "`{mname}` expects {} argument(s), found {}",
                    expected.len(),
                    args.len()
                ),
            );
            return Ok(self.poison(span));
        }
        let receiver_is_string = receiver.ty == Ty::String;
        let receiver_is_regexp = receiver.ty == Ty::RegExp;
        let mut call_args = vec![receiver];
        let callback_receiver = call_args[0].ty == Ty::Array;
        for (arg, expected_ty) in args.iter().zip(expected) {
            // A JS callback `(value, index, collection)` has dynamic
            // parameters, so hint unannotated callback params to
            // `any` (whether the receiver is an Array, Map, or Set).
            let _ = callback_receiver;
            if expected_ty == Ty::Function(hir::DYNAMIC_FUNCTION) {
                self.closure_param_hint = Some(vec![Ty::Any, Ty::Any, Ty::Any]);
            }
            let mut value = self.check_expr(cx, arg);
            self.closure_param_hint = None;
            if expected_ty == Ty::Function(hir::DYNAMIC_FUNCTION)
                && matches!(value.ty, Ty::Function(_))
            {
                if let (Some(this_ast), Some((bind_id, _))) = (
                    this_arg_ast,
                    zun_natives::resolve("collections", "functionBind"),
                ) {
                    let checked = self.check_expr(cx, this_ast);
                    let bound_this = self.box_function_argument(checked);
                    value = hir::Expr {
                        span: value.span,
                        kind: hir::ExprKind::FunctionBind {
                            callee: Box::new(value),
                            args: vec![bound_this],
                            native_id: bind_id,
                        },
                        ty: Ty::Function(hir::DYNAMIC_FUNCTION),
                    };
                }
            }
            // `re.exec(x)` / `re.test(x)` run ToString on a non-string subject.
            if receiver_is_regexp
                && expected_ty == Ty::String
                && value.ty != Ty::String
                && !value.ty.is_error()
            {
                value = self.to_string_for_parse(value);
            }
            coerce_numeric_literal(&mut value, expected_ty);
            if !value.ty.is_error() && !self.is_assignable(value.ty, expected_ty) {
                self.error(
                    value.span,
                    format!(
                        "argument type {:?} does not match {expected_ty:?}",
                        value.ty
                    ),
                );
            }
            // A function value flowing into an `any` parameter
            // (`set.add(fn)`) must be boxed like any other value.
            if expected_ty == Ty::Any && matches!(value.ty, Ty::Function(_)) {
                value = Self::box_to_any(value);
            }
            // `any` flowing into a numeric array-method parameter runs ToNumber.
            if callback_receiver && expected_ty == Ty::F64 && value.ty == Ty::Any {
                value = self.to_number_expr(value);
            }
            call_args.push(value);
        }
        let module = Self::managed_native_module(native_name, receiver_is_string);
        let Some((id, _)) = zun_natives::resolve(module, native_name) else {
            self.error(
                span,
                format!("missing collection intrinsic `{native_name}`"),
            );
            return Ok(self.poison(span));
        };
        Ok(hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: call_args,
            },
            ty: return_ty,
            span,
        })
    }
}
