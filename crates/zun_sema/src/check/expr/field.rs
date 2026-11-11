//! Extracted interceptors for `check_field` (see `member.rs`).
//!
//! `check_field` is a long chain of independent interceptors: each inspects the
//! receiver (first as an AST node, then as a checked `hir::Expr`) and either
//! handles the read or falls through to the next. These helpers keep that chain
//! readable while preserving its exact order — the first match wins.
//!
//! AST-phase helpers take the receiver by reference and return
//! `Option<hir::Expr>` (`Some` = handled). The value-phase helpers consume the
//! checked receiver and return `Result<hir::Expr, hir::Expr>` (`Ok` = handled,
//! `Err` = the receiver handed back unchanged for the next interceptor).

use crate::check::*;

impl Checker<'_> {
    /// Built-in metadata reads folded to literals before the receiver is
    /// checked: `Function.prototype.<m>`, `[].indexOf.length`, the
    /// `<Builtin>.<fn>.length`/`.name` chains, and `Math.<fn>.length`/`.name`.
    pub(in crate::check) fn field_builtin_meta(
        &mut self,
        cx: &mut FunctionCx,
        object: &ast::Expr,
        fname: &str,
        span: Span,
    ) -> Option<hir::Expr> {
        // `Function.prototype.<m>` has no first-class value here: it reads
        // as `undefined`, so comparisons against it stay stable.
        if let ast::Expr::Field {
            object: inner,
            field: proto,
            ..
        } = object
        {
            if self.text(proto) == "prototype"
                && matches!(inner.as_ref(), ast::Expr::Ident(n) if self.text(n) == "Function")
                && !self.classes_by_name.contains_key("Function")
            {
                return Some(hir::Expr {
                    kind: hir::ExprKind::Null,
                    ty: Ty::Void,
                    span,
                });
            }
        }
        // `[].indexOf.length`: a method read off an array literal.
        if fname == "length" {
            if let ast::Expr::Field {
                object: inner,
                field: method,
                ..
            } = object
            {
                if matches!(inner.as_ref(), ast::Expr::ArrayLiteral { .. })
                    && !self.classes_by_name.contains_key("Array")
                {
                    if let Some(arity) =
                        builtin_function_length(&["Array", "prototype", self.text(method)])
                    {
                        return Some(hir::Expr {
                            kind: hir::ExprKind::Number(arity),
                            ty: Ty::F64,
                            span,
                        });
                    }
                }
            }
        }
        // `<Builtin>.<fn>.length` / `.name` for well-known built-in paths.
        if (fname == "length" || fname == "name") && !matches!(object, ast::Expr::Ident(_)) {
            let mut parts: Vec<String> = Vec::new();
            let mut cursor = object;
            let mut ok = true;
            loop {
                match cursor {
                    ast::Expr::Field {
                        object: inner,
                        field: f,
                        ..
                    } => {
                        parts.push(self.text(f).to_string());
                        cursor = inner.as_ref();
                    }
                    ast::Expr::Ident(root) => {
                        parts.push(self.text(root).to_string());
                        break;
                    }
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                parts.reverse();
                let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
                // `Date.prototype.<m>.length` / `.name`: the spec arity of the
                // prelude `Date` methods (several take trailing optional
                // arguments through a rest parameter).
                if refs.len() == 3
                    && refs[0] == "Date"
                    && refs[1] == "prototype"
                    && cx.lookup("Date").is_none()
                    && self.classes_by_name.contains_key("Date")
                {
                    if let Some(arity) = date_function_length(refs[2]) {
                        return Some(if fname == "length" {
                            hir::Expr {
                                kind: hir::ExprKind::Number(arity),
                                ty: Ty::F64,
                                span,
                            }
                        } else {
                            hir::Expr {
                                kind: hir::ExprKind::String(refs[2].to_string()),
                                ty: Ty::String,
                                span,
                            }
                        });
                    }
                }
                let shadowed = self.classes_by_name.contains_key(parts[0].as_str());
                if !shadowed && cx.lookup(refs[0]).is_none() {
                    if let Some(arity) = builtin_function_length(&refs) {
                        return Some(if fname == "length" {
                            hir::Expr {
                                kind: hir::ExprKind::Number(arity),
                                ty: Ty::F64,
                                span,
                            }
                        } else {
                            hir::Expr {
                                kind: hir::ExprKind::String(refs[refs.len() - 1].to_string()),
                                ty: Ty::String,
                                span,
                            }
                        });
                    }
                }
            }
        }
        // `Math.<fn>.length` / `Math.<fn>.name` fold to literals.
        if let ast::Expr::Field {
            object: inner_obj,
            field: inner_field,
            ..
        } = object
        {
            if let ast::Expr::Ident(m) = inner_obj.as_ref() {
                if self.text(m) == "Math"
                    && !self.classes_by_name.contains_key("Math")
                    && (fname == "length" || fname == "name")
                {
                    let inner = self.text(inner_field).to_string();
                    if let Some(arity) = math_function_length(&inner) {
                        return Some(if fname == "length" {
                            hir::Expr {
                                kind: hir::ExprKind::Number(arity),
                                ty: Ty::F64,
                                span,
                            }
                        } else {
                            hir::Expr {
                                kind: hir::ExprKind::String(inner),
                                ty: Ty::String,
                                span,
                            }
                        });
                    }
                }
            }
        }
        None
    }

    /// Reads whose receiver is a bare identifier naming a built-in or a
    /// user class/enum. Dispatches, in order, to the prototype/constant
    /// singletons, the user-class static members, and the `Number`/`Symbol`
    /// constant folds.
    pub(in crate::check) fn field_static_ident(
        &mut self,
        cx: &mut FunctionCx,
        class_name: &ast::Name,
        fname: &str,
        field: &ast::Name,
        span: Span,
    ) -> Option<hir::Expr> {
        if let Some(expr) = self.field_class_singletons(cx, class_name, fname, field, span) {
            return Some(expr);
        }
        if let Some(expr) = self.field_class_static(class_name, fname, span) {
            return Some(expr);
        }
        self.field_number_symbol_const(class_name, fname, span)
    }

    /// Built-in prototype singletons, `Math` constants, and enum member folds
    /// reached through a bare identifier receiver.
    fn field_class_singletons(
        &mut self,
        cx: &mut FunctionCx,
        class_name: &ast::Name,
        fname: &str,
        field: &ast::Name,
        span: Span,
    ) -> Option<hir::Expr> {
        // `Object.prototype` as a value: the realm's shared root
        // prototype object (what `Object.getPrototypeOf` yields for
        // a plain object).
        if self.text(class_name) == "Object"
            && fname == "prototype"
            && !self.classes_by_name.contains_key("Object")
        {
            if let Some((id, _)) = zun_natives::resolve("collections", "objectPrototype") {
                return Some(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: Vec::new(),
                    },
                    ty: Ty::Any,
                    span,
                });
            }
        }
        // `RegExp.prototype` as a value: the shared prototype singleton
        // that owns the built-in `@@match`/`@@replace`/... methods.
        if self.text(class_name) == "RegExp"
            && fname == "prototype"
            && !self.classes_by_name.contains_key("RegExp")
            && cx.lookup("RegExp").is_none()
        {
            if let Some((id, _)) = zun_natives::resolve("collections", "regexpPrototype") {
                return Some(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: Vec::new(),
                    },
                    ty: Ty::Any,
                    span,
                });
            }
        }
        // `Function.prototype` as a value: the shared prototype singleton.
        if self.text(class_name) == "Function"
            && fname == "prototype"
            && !self.classes_by_name.contains_key("Function")
            && cx.lookup("Function").is_none()
        {
            if let Some((id, _)) = zun_natives::resolve("collections", "functionPrototype") {
                return Some(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: Vec::new(),
                    },
                    ty: Ty::Any,
                    span,
                });
            }
        }
        // `String.prototype` as a value: the shared prototype singleton.
        if self.text(class_name) == "String"
            && fname == "prototype"
            && cx.lookup("String").is_none()
        {
            if let Some((id, _)) = zun_natives::resolve("collections", "stringPrototype") {
                return Some(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: Vec::new(),
                    },
                    ty: Ty::Any,
                    span,
                });
            }
        }
        if self.text(class_name) == "Math" && !self.classes_by_name.contains_key("Math") {
            let constant = match fname {
                "PI" => Some(std::f64::consts::PI),
                "E" => Some(std::f64::consts::E),
                "LN2" => Some(std::f64::consts::LN_2),
                "LN10" => Some(std::f64::consts::LN_10),
                "LOG2E" => Some(std::f64::consts::LOG2_E),
                "LOG10E" => Some(std::f64::consts::LOG10_E),
                "SQRT2" => Some(std::f64::consts::SQRT_2),
                "SQRT1_2" => Some(std::f64::consts::FRAC_1_SQRT_2),
                _ => None,
            };
            if let Some(value) = constant {
                return Some(hir::Expr {
                    kind: hir::ExprKind::Number(value),
                    ty: Ty::F64,
                    span,
                });
            }
        }
        // `Enum.Member` folds to the member's compile-time literal.
        let enum_name = self.text(class_name).to_string();
        if self.enums.contains_key(&enum_name) {
            match self.enums[&enum_name].get(fname).cloned() {
                Some(types::EnumConst::Num(n)) => {
                    return Some(hir::Expr {
                        kind: hir::ExprKind::Number(n),
                        ty: Ty::F64,
                        span,
                    });
                }
                Some(types::EnumConst::Str(s)) => {
                    return Some(hir::Expr {
                        kind: hir::ExprKind::String(s),
                        ty: Ty::String,
                        span,
                    });
                }
                None => {
                    self.error(
                        field.span,
                        format!("enum `{enum_name}` has no member `{fname}`"),
                    );
                    return Some(hir::Expr {
                        kind: hir::ExprKind::Number(0.0),
                        ty: Ty::Error,
                        span,
                    });
                }
            }
        }
        None
    }

    /// Static-field, static-accessor and static-method reads on a user class
    /// (plus the `console.<method>` value form).
    fn field_class_static(
        &mut self,
        class_name: &ast::Name,
        fname: &str,
        span: Span,
    ) -> Option<hir::Expr> {
        if let Some(&class_index) = self.classes_by_name.get(self.text(class_name)) {
            if let Some((field_slot, ty)) = self.static_field_of(class_index, fname) {
                return Some(hir::Expr {
                    kind: hir::ExprKind::StaticFieldGet {
                        class: self.static_field_owner(class_index, fname),
                        field_slot,
                    },
                    ty,
                    span,
                });
            }
            // `console.<method>` read as a value: a reference to the
            // variadic prelude entry point.
            if self.text(class_name) == "console" {
                let target = match fname {
                    "log" | "info" | "debug" | "dir" => Some("v_log"),
                    "error" | "warn" => Some("v_err"),
                    "group" | "groupCollapsed" => Some("v_group"),
                    "groupEnd" => Some("v_groupEnd"),
                    "count" => Some("v_count"),
                    "countReset" => Some("v_countReset"),
                    "assert" => Some("v_assert"),
                    "table" | "time" | "timeEnd" | "timeLog" | "trace" => Some("v_log"),
                    _ => None,
                };
                if let (Some(target), Some(&fmt_class)) =
                    (target, self.classes_by_name.get("ConsoleFormat2"))
                {
                    if let Some(func) = self.method_of(fmt_class, target) {
                        return Some(hir::Expr {
                            kind: hir::ExprKind::FunctionRef { func },
                            ty: Ty::Function(func),
                            span,
                        });
                    }
                }
            }
            // A static accessor read (`Class.prop` with `static get prop()`).
            if let Some(func) = self.method_of(class_index, &format!("get_{fname}")) {
                let hidden_class = self.static_this.contains_key(&func);
                if self.signatures[func as usize].is_static
                    && self.signatures[func as usize].params.len() == hidden_class as usize
                {
                    let ty = self.signatures[func as usize].return_ty;
                    let mut getter_args = Vec::new();
                    if hidden_class {
                        let token = zun_natives::USER_CLASS_FLAG | (class_index + 1);
                        getter_args.push(self.realm_descriptor_expr(token, span));
                    }
                    return Some(hir::Expr {
                        kind: hir::ExprKind::StaticCall {
                            func,
                            args: getter_args,
                        },
                        ty,
                        span,
                    });
                }
            }
            // A static method that takes its receiving class (`this`) is a
            // plain function reference; `.call(Class)` supplies the receiver.
            if let Some(func) = self.method_of(class_index, fname) {
                if self.static_this.contains_key(&func) {
                    return Some(hir::Expr {
                        kind: hir::ExprKind::FunctionRef { func },
                        ty: Ty::Function(func),
                        span,
                    });
                }
            }
            // A static method read as a value (`const f = A.method`).
            if self.method_of(class_index, fname).is_some() {
                return Some(self.method_value_expr(class_index, fname, span));
            }
        }
        None
    }

    /// `Number.*` numeric constants and well-known `Symbol.*` values reached
    /// through a bare identifier receiver.
    fn field_number_symbol_const(
        &mut self,
        class_name: &ast::Name,
        fname: &str,
        span: Span,
    ) -> Option<hir::Expr> {
        // `Number.*` numeric constants fold to compile-time `number`
        // literals. Keyed on the receiver name (the `Number` class
        // has no static fields, so `static_field_of` above returns
        // `None` and falls through here).
        if self.text(class_name) == "Number" {
            let constant = match fname {
                "MAX_SAFE_INTEGER" => Some(9_007_199_254_740_991.0),
                "MIN_SAFE_INTEGER" => Some(-9_007_199_254_740_991.0),
                "MAX_VALUE" => Some(f64::MAX),
                "MIN_VALUE" => Some(f64::MIN_POSITIVE.min(f64::from_bits(1))),
                "EPSILON" => Some(f64::EPSILON),
                "POSITIVE_INFINITY" => Some(f64::INFINITY),
                "NEGATIVE_INFINITY" => Some(f64::NEG_INFINITY),
                "NaN" => Some(f64::NAN),
                _ => None,
            };
            if let Some(value) = constant {
                return Some(hir::Expr {
                    kind: hir::ExprKind::Number(value),
                    ty: Ty::F64,
                    span,
                });
            }
        }
        // Well-known symbols as first-class values (`Symbol.iterator`,
        // `Symbol.asyncIterator`, `Symbol.hasInstance`,
        // `Symbol.toPrimitive`, `Symbol.toStringTag`). Their identities
        // are stable per isolate.
        if self.text(class_name) == "Symbol" && !self.classes_by_name.contains_key("Symbol") {
            // Stable well-known symbol ids. Source of truth:
            // `zun_core::WellKnownSymbol` (the runtime registry seeds
            // these at the same ids). Kept inline so the compile-time
            // crate does not depend on the runtime crate.
            let well_known_id = match fname {
                "iterator" => Some(0.0),
                "asyncIterator" => Some(1.0),
                "hasInstance" => Some(2.0),
                "toPrimitive" => Some(3.0),
                "toStringTag" => Some(4.0),
                "match" => Some(5.0),
                "replace" => Some(6.0),
                "search" => Some(7.0),
                "split" => Some(8.0),
                "species" => Some(9.0),
                "isConcatSpreadable" => Some(10.0),
                "unscopables" => Some(11.0),
                "matchAll" => Some(12.0),
                _ => None,
            };
            if let Some(well_known_id) = well_known_id {
                let Some((id, _)) = zun_natives::resolve("collections", "symbolWellKnown") else {
                    self.error(span, "missing `symbolWellKnown` intrinsic");
                    return Some(self.poison(span));
                };
                return Some(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![hir::Expr {
                            kind: hir::ExprKind::Number(well_known_id),
                            ty: Ty::F64,
                            span,
                        }],
                    },
                    ty: Ty::Symbol,
                    span,
                });
            }
        }
        None
    }

    /// Special property reads on a checked receiver whose type is known:
    /// `.constructor`, `symbol.description`, the `.length`/`.size` spec
    /// properties on arrays, strings, buffer views and maps/sets.
    pub(in crate::check) fn field_special_props(
        &mut self,
        object: hir::Expr,
        fname: &str,
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        // `value.constructor` on a statically typed receiver: box it and
        // use the dynamic read, which exposes the `{ name, __classDesc }`
        // constructor record of typed instances and built-in values.
        if fname == "constructor"
            && matches!(
                object.ty,
                Ty::Instance(_)
                    | Ty::Array
                    | Ty::Map
                    | Ty::Set
                    | Ty::WeakMap
                    | Ty::WeakSet
                    | Ty::WeakRef
                    | Ty::Promise
                    | Ty::RegExp
                    | Ty::String
                    | Ty::Boolean
                    | Ty::Float(_)
                    | Ty::Int(_)
                    | Ty::BigInt
                    | Ty::BufferView(_)
                    | Ty::FinalizationRegistry
            )
            && !matches!(object.ty, Ty::Instance(c) if self.method_of(c, "get_constructor").is_some()
                || self.field_of(c, "constructor").is_some())
        {
            return Ok(hir::Expr {
                kind: hir::ExprKind::DynamicGet {
                    object: Box::new(Self::box_to_any(object)),
                    name: fname.to_string(),
                },
                ty: Ty::Any,
                span,
            });
        }
        // `symbol.description` — the creation description or `undefined`.
        if object.ty == Ty::Symbol && fname == "description" {
            let Some((id, _)) = zun_natives::resolve("collections", "symbolDescription") else {
                self.error(span, "missing `symbolDescription` intrinsic");
                return Ok(self.poison(span));
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![object],
                },
                ty: Ty::Any,
                span,
            });
        }
        if object.ty == Ty::Array && fname == "length" {
            return Ok(hir::Expr {
                kind: hir::ExprKind::ArrayLength {
                    object: Box::new(object),
                },
                ty: Ty::Int(IntKind::U64),
                span,
            });
        }
        // `str.length` as a PROPERTY (the spec form), lowering to the
        // same `string.length` native the method form uses — intercepted
        // on the primitive receiver ahead of class dispatch, like the
        // `.length` cases for arrays and views above.
        if object.ty == Ty::String && fname == "length" {
            let Some((id, _)) = zun_natives::resolve("string", "length") else {
                self.error(span, "missing String intrinsic `length`");
                return Ok(self.poison(span));
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![object],
                },
                ty: Ty::Int(IntKind::U64),
                span,
            });
        }
        // `view.length` as a property (Node `Buffer.length`, and the
        // typed arrays' element count). Lowers to the kind's `length`
        // intrinsic, so the method form `view.length()` still works too.
        if let Ty::BufferView(kind) = object.ty {
            if fname == "length" {
                if let Some((native, ret, _)) = crate::typed_arrays::method(kind, "length") {
                    if let Some((id, _)) = zun_natives::resolve("buffer", native) {
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id,
                                args: vec![object],
                            },
                            ty: ret,
                            span,
                        });
                    }
                }
            }
        }
        // `map.size`/`set.size` as a PROPERTY (the spec form), lowering
        // to the same `mapSize`/`setSize` native the method form used.
        // Follows the `.length`-on-views/strings precedent above: a
        // field name is intercepted on a known receiver type ahead of
        // class dispatch, so the backing class need not be in scope. The
        // method form `map.size()` still resolves via the method table.
        if fname == "size" {
            let native = match object.ty {
                Ty::Map => Some("mapSize"),
                Ty::Set => Some("setSize"),
                _ => None,
            };
            if let Some(native) = native {
                let Some((id, _)) = zun_natives::resolve("collections", native) else {
                    self.error(span, format!("missing intrinsic `{native}`"));
                    return Ok(self.poison(span));
                };
                return Ok(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![object],
                    },
                    ty: Ty::Int(IntKind::U64),
                    span,
                });
            }
        }
        Err(object)
    }

    /// `Function` value metadata: `prototype`, `name`, and `length` read off a
    /// statically known function value.
    pub(in crate::check) fn field_function_value(
        &mut self,
        object: hir::Expr,
        fname: &str,
        span: Span,
    ) -> Result<hir::Expr, hir::Expr> {
        // `Function.prototype.name`/`length` resolve from the compile-time
        // signature of a function value; a dynamic function value (a
        // closure) has no statically known name or declared arity.
        if let Ty::Function(func) = object.ty {
            if fname == "prototype" && func < hir::DYNAMIC_FUNCTION {
                // A function's `prototype` is an ordinary (initially empty) object.
                return Ok(hir::Expr {
                    kind: hir::ExprKind::DynamicObjectLiteral {
                        entries: Vec::new(),
                    },
                    ty: Ty::DynamicObject,
                    span,
                });
            }
            if (fname == "name" || fname == "length")
                && self.deleted_fn_meta.contains(&(func, fname.to_string()))
            {
                return Ok(if fname == "name" {
                    hir::Expr {
                        kind: hir::ExprKind::String(String::new()),
                        ty: Ty::String,
                        span,
                    }
                } else {
                    hir::Expr {
                        kind: hir::ExprKind::Number(0.0),
                        ty: Ty::F64,
                        span,
                    }
                });
            }
            if fname == "name" {
                let name = if matches!(
                    func,
                    hir::DYNAMIC_FUNCTION | hir::DYNAMIC_PREDICATE | hir::DYNAMIC_REDUCER
                ) {
                    String::new()
                } else {
                    self.signatures[func as usize].name.clone()
                };
                return Ok(hir::Expr {
                    kind: hir::ExprKind::String(name),
                    ty: Ty::String,
                    span,
                });
            }
            if fname == "length" {
                let length = if matches!(
                    func,
                    hir::DYNAMIC_FUNCTION | hir::DYNAMIC_PREDICATE | hir::DYNAMIC_REDUCER
                ) {
                    0
                } else {
                    let signature = &self.signatures[func as usize];
                    let rest = usize::from(signature.rest_param.is_some());
                    signature
                        .params
                        .len()
                        .saturating_sub(signature.optional_count)
                        .saturating_sub(rest)
                };
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Number(length as f64),
                    ty: Ty::F64,
                    span,
                });
            }
        }
        Err(object)
    }

    /// The final receiver-type dispatch: instance fields/getters, dynamic
    /// objects, proxies, match-result expandos, function expandos, and the
    /// `RegExp` property reads.
    pub(in crate::check) fn field_dispatch(
        &mut self,
        object: hir::Expr,
        fname: String,
        field: &ast::Name,
        span: Span,
    ) -> hir::Expr {
        match object.ty {
            Ty::Instance(class) => match self.field_of(class, &fname) {
                Some((slot, ty)) => hir::Expr {
                    kind: hir::ExprKind::FieldGet {
                        object: Box::new(object),
                        class,
                        field_slot: slot,
                    },
                    ty,
                    span,
                },
                None => {
                    let getter = format!("get_{fname}");
                    let Some(func) = self.method_of(class, &getter) else {
                        // An extracted method (`const f = obj.method`) is a
                        // function value whose first parameter is `this`.
                        if let Some(method) = self.method_of(class, &fname) {
                            return hir::Expr {
                                kind: hir::ExprKind::FunctionRef { func: method },
                                ty: Ty::Function(method),
                                span,
                            };
                        }
                        self.error(
                            field.span,
                            format!("no field or getter `{fname}` on this type"),
                        );
                        return self.poison(span);
                    };
                    let sig = &self.signatures[func as usize];
                    if sig.params.len() != 1 {
                        self.error(
                            field.span,
                            format!("getter `{getter}` must take no arguments"),
                        );
                        return self.poison(span);
                    }
                    hir::Expr {
                        kind: hir::ExprKind::MethodCall {
                            func,
                            args: vec![object],
                        },
                        ty: sig.return_ty,
                        span,
                    }
                }
            },
            Ty::DynamicObject | Ty::Any => hir::Expr {
                kind: hir::ExprKind::DynamicGet {
                    object: Box::new(object),
                    name: fname,
                },
                ty: Ty::Any,
                span,
            },
            // `proxy.key` routes through the handler's `get` trap (the
            // native falls back to the target when the trap is absent).
            Ty::Proxy => self.proxy_get(
                object,
                hir::Expr {
                    kind: hir::ExprKind::String(fname),
                    ty: Ty::String,
                    span,
                },
                span,
            ),
            // Match-result arrays carry `index` / `input` / `groups` (and a
            // template strings array `raw`) as expando properties.
            Ty::Array if matches!(fname.as_str(), "index" | "input" | "groups" | "raw") => {
                hir::Expr {
                    kind: hir::ExprKind::DynamicGet {
                        object: Box::new(object),
                        name: fname,
                    },
                    ty: Ty::Any,
                    span,
                }
            }
            // Function objects carry an expando property bag
            // (`fn['x'] = ...`), resolved at run time by the interpreter.
            Ty::Function(_) => hir::Expr {
                kind: hir::ExprKind::DynamicGet {
                    object: Box::new(object),
                    name: fname,
                },
                ty: Ty::Any,
                span,
            },
            Ty::Error => self.poison(span),
            Ty::RegExp if fname == "source" || fname == "flags" => {
                let native = if fname == "source" {
                    "regexpSource"
                } else {
                    "regexpFlags"
                };
                let Some((id, _)) = zun_natives::resolve("collections", native) else {
                    self.error(span, "missing RegExp intrinsic");
                    return self.poison(span);
                };
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![object],
                    },
                    ty: Ty::String,
                    span,
                }
            }
            Ty::RegExp if fname == "lastIndex" => {
                let Some((id, _)) = zun_natives::resolve("collections", "regexpLastIndex") else {
                    self.error(span, "missing RegExp intrinsic");
                    return self.poison(span);
                };
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![object],
                    },
                    ty: Ty::Any,
                    span,
                }
            }
            Ty::RegExp
                if matches!(
                    fname.as_str(),
                    "hasIndices"
                        | "global"
                        | "ignoreCase"
                        | "multiline"
                        | "dotAll"
                        | "unicode"
                        | "unicodeSets"
                        | "sticky"
                ) =>
            {
                let Some((id, _)) = zun_natives::resolve("collections", "regexpFlag") else {
                    self.error(span, "missing RegExp intrinsic");
                    return self.poison(span);
                };
                let name = hir::Expr {
                    kind: hir::ExprKind::String(fname),
                    ty: Ty::String,
                    span,
                };
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![object, name],
                    },
                    ty: Ty::Boolean,
                    span,
                }
            }
            other => {
                self.error(object.span, format!("type {other:?} has no fields"));
                self.poison(span)
            }
        }
    }
}
