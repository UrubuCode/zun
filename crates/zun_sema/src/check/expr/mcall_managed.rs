//! String `normalize` and the table-driven managed-collection / typed-array
//! method dispatch for `check_method_call` (see `method_call.rs`).
//!
//! `mcall_managed` owns the large `managed_method` lookup table (Array, Map,
//! Set, and typed-array methods routed to natives) together with its single
//! shared dispatch site, including the callback `thisArg` binding. It is one
//! logical check and is kept whole rather than fragmenting the table from the
//! dispatch that consumes it. Each helper returns `Ok` when it handles the
//! call or `Err(receiver)` to fall through; order is preserved.

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
                    format!("`normalize` expects 0 or 1 argument(s), found {}", args.len()),
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
                kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, form] },
                ty: Ty::String,
                span,
            });
        }
        Err(receiver)
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
        let typed_array_method = match receiver.ty {
            Ty::BufferView(kind) => crate::typed_arrays::method(kind, mname),
            _ => None,
        };
        let managed_method = typed_array_method.or_else(|| match (receiver.ty, mname) {
            // Standard `Array.prototype` methods routed to natives. The
            // callback methods invoke the dynamic function value with the
            // ECMAScript `(value, index, array)` signature (extra
            // arguments dropped to the callback's declared arity).
            (Ty::Array, "forEach") => {
                Some(("arrayForEach", Ty::Void, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
            }
            (Ty::Array, "map") => {
                Some(("arrayMap", Ty::Array, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
            }
            (Ty::Array, "filter") => {
                Some(("arrayFilter", Ty::Array, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
            }
            (Ty::Array, "find") => {
                Some(("arrayFind", Ty::Any, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
            }
            (Ty::Array, "findIndex") => Some((
                "arrayFindIndex",
                Ty::Int(IntKind::I32),
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::Array, "some") => {
                Some(("arraySome", Ty::Boolean, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
            }
            (Ty::Array, "every") => {
                Some(("arrayEvery", Ty::Boolean, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
            }
            (Ty::Array, "join") => {
                Some(("arrayJoin", Ty::String, vec![Ty::String]))
            }
            (Ty::Array, "shift") => Some(("arrayShift", Ty::Any, vec![])),
            (Ty::Array, "unshift") => {
                Some(("arrayUnshift", Ty::Int(IntKind::U64), vec![Ty::Any]))
            }
            // `at(index)` with relative/undefined semantics (native).
            (Ty::Array, "at") => Some(("arrayAt", Ty::Any, vec![Ty::F64])),
            (Ty::Array, "findLast") => Some((
                "arrayFindLast",
                Ty::Any,
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::Array, "findLastIndex") => Some((
                "arrayFindLastIndex",
                Ty::Int(IntKind::I32),
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::Array, "flatMap") => Some((
                "arrayFlatMap",
                Ty::Array,
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::Array, "group") => Some((
                "arrayGroup",
                Ty::DynamicObject,
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::Array, "groupToMap") => Some((
                "arrayGroupToMap",
                Ty::Map,
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::Array, "keys") => Some(("arrayKeys", Ty::Any, vec![])),
            (Ty::Array, "values") => Some(("arrayValues", Ty::Any, vec![])),
            (Ty::Array, "entries") => Some(("arrayEntries", Ty::Any, vec![])),
            (Ty::Map, "set") => Some(("mapSetChain", Ty::Map, vec![Ty::Any, Ty::Any])),
            (Ty::Map, "get") => Some(("mapGet", Ty::Any, vec![Ty::Any])),
            (Ty::Map, "has") => Some(("mapHas", Ty::Boolean, vec![Ty::Any])),
            (Ty::Map, "size") => Some(("mapSize", Ty::Int(IntKind::U64), vec![])),
            (Ty::Map, "delete") => Some(("mapDelete", Ty::Boolean, vec![Ty::Any])),
            (Ty::Map, "clear") => Some(("mapClear", Ty::Void, vec![])),
            (Ty::Map, "values") => Some(("mapValues", Ty::Any, vec![])),
            (Ty::Map, "keys") => Some(("mapKeys", Ty::Any, vec![])),
            (Ty::Map, "entries") => Some(("mapEntries", Ty::Any, vec![])),
            (Ty::Map, "forEach") => Some((
                "mapForEach",
                Ty::Void,
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::Set, "add") => Some(("setAddChain", Ty::Set, vec![Ty::Any])),
            (Ty::Set, "has") => Some(("setHas", Ty::Boolean, vec![Ty::Any])),
            (Ty::Set, "size") => Some(("setSize", Ty::Int(IntKind::U64), vec![])),
            (Ty::Set, "delete") => Some(("setDelete", Ty::Boolean, vec![Ty::Any])),
            (Ty::Set, "clear") => Some(("setClear", Ty::Void, vec![])),
            (Ty::Set, "values") => Some(("setValues", Ty::Any, vec![])),
            (Ty::Set, "keys") => Some(("setValues", Ty::Any, vec![])),
            (Ty::Set, "entries") => Some(("setEntries", Ty::Any, vec![])),
            (Ty::Set, "forEach") => Some((
                "setForEach",
                Ty::Void,
                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
            )),
            (Ty::WeakMap, "set") => {
                Some(("weakMapSet", Ty::WeakMap, vec![Ty::Any, Ty::Any]))
            }
            (Ty::WeakMap, "get") => Some(("weakMapGet", Ty::Any, vec![Ty::Any])),
            (Ty::WeakMap, "has") => Some(("weakMapHas", Ty::Boolean, vec![Ty::Any])),
            (Ty::WeakMap, "delete") => Some(("weakMapDelete", Ty::Boolean, vec![Ty::Any])),
            (Ty::WeakSet, "add") => Some(("weakSetAdd", Ty::WeakSet, vec![Ty::Any])),
            (Ty::WeakSet, "has") => Some(("weakSetHas", Ty::Boolean, vec![Ty::Any])),
            (Ty::WeakSet, "delete") => Some(("weakSetDelete", Ty::Boolean, vec![Ty::Any])),
            (Ty::WeakRef, "deref") => Some(("weakRefDeref", Ty::Any, vec![])),
            (Ty::RegExp, "isMatch") => Some(("regexpIsMatch", Ty::Boolean, vec![Ty::String])),
            (Ty::RegExp, "test") => Some(("regexpTest", Ty::Boolean, vec![Ty::String])),
            (Ty::RegExp, "exec") => Some(("regexpExec", Ty::Array, vec![Ty::String])),
            (Ty::RegExp, "source") => Some(("regexpSource", Ty::String, vec![])),
            (Ty::BigInt, "add") => Some(("bigIntAdd", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "sub") => Some(("bigIntSub", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "mul") => Some(("bigIntMul", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "div") => Some(("bigIntDiv", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "rem") => Some(("bigIntRem", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "pow") => Some(("bigIntPow", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "neg") => Some(("bigIntNeg", Ty::BigInt, vec![])),
            (Ty::BigInt, "and") => Some(("bigIntAnd", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "or") => Some(("bigIntOr", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "xor") => Some(("bigIntXor", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "not") => Some(("bigIntNot", Ty::BigInt, vec![])),
            (Ty::BigInt, "shl") => Some(("bigIntShl", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "shr") => Some(("bigIntShr", Ty::BigInt, vec![Ty::BigInt])),
            (Ty::BigInt, "compare") => Some(("bigIntCompare", Ty::Int(IntKind::I32), vec![Ty::BigInt])),
            (Ty::BigInt, "toString") => Some(("bigIntToString", Ty::String, vec![])),
            (Ty::Promise, "state") => Some(("promiseState", Ty::Int(IntKind::U8), vec![])),
            (Ty::Promise, "result") => Some(("promiseResult", Ty::Any, vec![])),
            (Ty::Promise, "isReady") => Some(("promiseIsReady", Ty::Boolean, vec![])),
            (Ty::Promise, "resolve") => Some(("promiseResolve", Ty::Boolean, vec![Ty::Any])),
            (Ty::Promise, "reject") => Some(("promiseReject", Ty::Boolean, vec![Ty::String])),
            (Ty::String, "toInt") => Some(("toInt", Ty::Int(IntKind::I32), vec![])),
            (Ty::String, "toFloat") => Some(("toFloat", Ty::F64, vec![])),
            (Ty::String, "toBoolean") => Some(("toBoolean", Ty::Boolean, vec![])),
            (Ty::String, "reverse") => Some(("reverse", Ty::String, vec![])),
            (Ty::String, "length") => Some(("length", Ty::Int(IntKind::U64), vec![])),
            (Ty::String, "toUpperCase") => Some(("toUpperCase", Ty::String, vec![])),
            (Ty::String, "toLowerCase") => Some(("toLowerCase", Ty::String, vec![])),
            (Ty::String, "trim") => Some(("trim", Ty::String, vec![])),
            (Ty::String, "trimStart") => Some(("trimStart", Ty::String, vec![])),
            (Ty::String, "trimEnd") => Some(("trimEnd", Ty::String, vec![])),
            (Ty::String, "charCodeAt") => {
                Some(("charCodeAt", Ty::Int(IntKind::U32), vec![Ty::Int(IntKind::U64)]))
            }
            (Ty::String, "indexOf") => {
                Some(("indexOf", Ty::Int(IntKind::I32), vec![Ty::String]))
            }
            (Ty::String, "lastIndexOf") => {
                Some(("lastIndexOf", Ty::Int(IntKind::I32), vec![Ty::String]))
            }
            (Ty::String, "substring") => Some((
                "substring",
                Ty::String,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U64)],
            )),
            (Ty::String, "repeat") => {
                Some(("repeat", Ty::String, vec![Ty::Int(IntKind::U64)]))
            }
            (Ty::String, "startsWith") => {
                Some(("startsWith", Ty::Boolean, vec![Ty::String]))
            }
            (Ty::String, "endsWith") => Some(("endsWith", Ty::Boolean, vec![Ty::String])),
            // ECMA-402 locale surface (NUCLEUS #1), ICU4X-backed.
            // `localeCompare` returns the ECMAScript -1/0/1; v1 uses
            // root collation and the root (`und`) locale for casing.
            (Ty::String, "includes") => Some(("includes", Ty::Boolean, vec![Ty::String])),
            // Index is a `number` so a negative value (from the end) can
            // cross the boundary; out of range yields `undefined`.
            (Ty::String, "at") => Some(("at", Ty::String, vec![Ty::F64])),
            (Ty::String, "codePointAt") => {
                Some(("codePointAt", Ty::Int(IntKind::U32), vec![Ty::Int(IntKind::U64)]))
            }
            (Ty::String, "replaceAll") => {
                Some(("replaceAll", Ty::String, vec![Ty::String, Ty::String]))
            }
            (Ty::BufferView(hir::BufferViewKind::DataView), "getUint32") => Some((
                "dataViewGetU32",
                Ty::Int(IntKind::U32),
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setUint32") => Some((
                "dataViewSetU32",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U32), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getUint16") => Some((
                "dataViewGetU16",
                Ty::Int(IntKind::U16),
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setUint16") => Some((
                "dataViewSetU16",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U16), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getFloat64") => Some((
                "dataViewGetF64",
                Ty::F64,
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setFloat64") => Some((
                "dataViewSetF64",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::F64, Ty::Boolean],
            )),
            // The remaining DataView element widths. JS names
            // 64-bit access `getBigInt64`/`getBigUint64` and returns a
            // BigInt; Zun has native 64-bit integers, so these return
            // `i64`/`u64` and are named without the Big.
            (Ty::BufferView(hir::BufferViewKind::DataView), "getInt8") => Some((
                "dataViewGetInt8",
                Ty::Int(IntKind::I8),
                vec![Ty::Int(IntKind::U64)],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setInt8") => Some((
                "dataViewSetInt8",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::I8)],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getUint8") => Some((
                "dataViewGetUint8",
                Ty::Int(IntKind::U8),
                vec![Ty::Int(IntKind::U64)],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setUint8") => Some((
                "dataViewSetUint8",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U8)],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getInt16") => Some((
                "dataViewGetInt16",
                Ty::Int(IntKind::I16),
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setInt16") => Some((
                "dataViewSetInt16",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::I16), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getInt32") => Some((
                "dataViewGetInt32",
                Ty::Int(IntKind::I32),
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setInt32") => Some((
                "dataViewSetInt32",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::I32), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getFloat32") => Some((
                "dataViewGetFloat32",
                Ty::F32,
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setFloat32") => Some((
                "dataViewSetFloat32",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::F32, Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getInt64") => Some((
                "dataViewGetInt64",
                Ty::Int(IntKind::I64),
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setInt64") => Some((
                "dataViewSetInt64",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::I64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "getUint64") => Some((
                "dataViewGetUint64",
                Ty::Int(IntKind::U64),
                vec![Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "setUint64") => Some((
                "dataViewSetUint64",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U64), Ty::Boolean],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "byteLength") => Some((
                "dataViewByteLength",
                Ty::Int(IntKind::U64),
                vec![],
            )),
            // `byteOffset`/`buffer`: the view's position in its
            // backing buffer and a reference to that `ArrayBuffer`.
            (Ty::BufferView(hir::BufferViewKind::DataView), "byteOffset") => Some((
                "dataViewByteOffset",
                Ty::Int(IntKind::U64),
                vec![],
            )),
            (Ty::BufferView(hir::BufferViewKind::DataView), "buffer") => Some((
                "dataViewBuffer",
                Ty::Buffer,
                vec![],
            )),
            // `ArrayBuffer`, which is what `Ty::Buffer` denotes. These
            // are methods rather than the properties JS has, matching
            // `String.length()` and `Map.size()` here; the divergence
            // is recorded in NUCLEUS rather than special-cased.
            (Ty::Buffer, "byteLength") => {
                Some(("bufferLength", Ty::Int(IntKind::U64), vec![]))
            }
            (Ty::Buffer, "slice") => Some((
                "bufferSlice",
                Ty::Buffer,
                vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U64)],
            )),
            // ES2024 resizable ArrayBuffer. As with `byteLength`, these
            // JS properties are methods here (the divergence is in
            // NUCLEUS). `resize` returns a boolean rather than JS's
            // `undefined`; a rejected resize is a runtime error.
            (Ty::Buffer, "maxByteLength") => {
                Some(("bufferMaxByteLength", Ty::Int(IntKind::U64), vec![]))
            }
            (Ty::Buffer, "resizable") => {
                Some(("bufferResizable", Ty::Boolean, vec![]))
            }
            (Ty::Buffer, "detached") => {
                Some(("bufferDetached", Ty::Boolean, vec![]))
            }
            (Ty::Buffer, "resize") => Some((
                "bufferResize",
                Ty::Boolean,
                vec![Ty::Int(IntKind::U64)],
            )),
            _ => None,
        });
        if let Some((native_name, return_ty, expected)) = managed_method {
            // A callback method's optional `thisArg` is bound onto the
            // callback (`cb.bind(thisArg)`) before the native runs.
            let undefined_ast = ast::Expr::Unary { op: ast::UnaryOp::Void, operand: Box::new(ast::Expr::Number { value: 0.0, span }), span };
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
            for (arg, expected_ty) in args.into_iter().zip(expected) {
                // A JS callback `(value, index, collection)` has dynamic
                // parameters, so hint unannotated callback params to
                // `any` (whether the receiver is an Array, Map, or Set).
                let _ = callback_receiver;
                if expected_ty == Ty::Function(hir::DYNAMIC_FUNCTION) {
                    self.closure_param_hint = Some(vec![Ty::Any, Ty::Any, Ty::Any]);
                }
                let mut value = self.check_expr(cx, arg);
                self.closure_param_hint = None;
                if expected_ty == Ty::Function(hir::DYNAMIC_FUNCTION) && matches!(value.ty, Ty::Function(_)) {
                    if let (Some(this_ast), Some((bind_id, _))) =
                        (this_arg_ast, zun_natives::resolve("collections", "functionBind"))
                    {
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
            let module = if receiver_is_string {
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
            };
            let Some((id, _)) = zun_natives::resolve(module, native_name) else {
                self.error(
                    span,
                    format!("missing collection intrinsic `{native_name}`"),
                );
                return Ok(self.poison(span));
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: call_args,
                },
                ty: return_ty,
                span,
            });
        }
        Err(receiver)
    }
}
