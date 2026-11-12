use crate::check::*;

impl Checker<'_> {
    /// Lower `key in object` to the dynamic `has` mechanism. A Proxy receiver
    /// routes through its `has` trap (`proxyHas`); a plain object or `any`
    /// receiver uses `reflectHas`. The key must be a string (symbol keys are
    /// deferred).
    pub(in crate::check) fn check_in_operator(
        &mut self,
        cx: &mut FunctionCx,
        key: &ast::Expr,
        object: &ast::Expr,
        span: Span,
    ) -> hir::Expr {
        // `"name" in Map.prototype` for well-known built-in prototypes: a
        // prototype method is present, a static-only method is absent.
        if let (
            ast::Expr::String { value, .. },
            ast::Expr::Field {
                object: proto_owner,
                field: proto_field,
                ..
            },
        ) = (key, object)
        {
            if self.text(proto_field) == "prototype" {
                if let ast::Expr::Ident(global) = proto_owner.as_ref() {
                    let global_name = self.text(global).to_string();
                    if cx.lookup(&global_name).is_none()
                        && !self.classes_by_name.contains_key(global_name.as_str())
                    {
                        let on_proto = builtin_function_length(&[
                            global_name.as_str(),
                            "prototype",
                            value.as_str(),
                        ])
                        .is_some();
                        let on_static =
                            builtin_function_length(&[global_name.as_str(), value.as_str()])
                                .is_some();
                        if on_proto || on_static {
                            return hir::Expr {
                                kind: hir::ExprKind::Boolean(on_proto),
                                ty: Ty::Boolean,
                                span,
                            };
                        }
                    }
                }
            }
        }
        let key = self.check_expr(cx, key);
        let object = self.check_expr(cx, object);
        // A non-string key (e.g. an array index) is converted with ToString,
        // matching ECMAScript property-key coercion.
        let dynamic_receiver = matches!(object.ty, Ty::DynamicObject | Ty::Any);
        if key.ty == Ty::Symbol && dynamic_receiver {
            // A Symbol key is looked up by identity, never stringified.
            if let Some((id, _)) = zun_natives::resolve("collections", "symbolHas") {
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![Self::box_to_any(object), Self::box_to_any(key)],
                    },
                    ty: Ty::Boolean,
                    span,
                };
            }
        }
        let key = if key.ty != Ty::String && !key.ty.is_error() {
            self.to_string_for_parse(key)
        } else {
            key
        };
        if object.ty == Ty::String {
            // A string reaching `in` came from `Object(str)`: index / length keys.
            if let Some((id, _)) = zun_natives::resolve("string", "stringHasProperty") {
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![object, key],
                    },
                    ty: Ty::Boolean,
                    span,
                };
            }
        }
        let native = match object.ty {
            Ty::Array => "arrayHas",
            Ty::Proxy => "proxyHas",
            Ty::DynamicObject | Ty::Any => "reflectHas",
            Ty::Map | Ty::Set | Ty::WeakMap | Ty::WeakSet => {
                let Some((id, _)) = zun_natives::resolve("collections", "collectionHas") else {
                    self.error(span, "missing `collectionHas` intrinsic");
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![object, key],
                    },
                    ty: Ty::Boolean,
                    span,
                };
            }
            Ty::Error => return self.poison(span),
            Ty::Instance(_) | Ty::Function(_) => {
                let Some((id, _)) = zun_natives::resolve("collections", "reflectHas") else {
                    self.error(span, "missing `reflectHas` intrinsic");
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![Self::box_to_any(object), key],
                    },
                    ty: Ty::Boolean,
                    span,
                };
            }
            Ty::BufferView(_) | Ty::Buffer => {
                let Some((id, _)) = zun_natives::resolve("collections", "reflectHas") else {
                    self.error(span, "missing `reflectHas` intrinsic");
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![Self::box_to_any(object), Self::box_to_any(key)],
                    },
                    ty: Ty::Boolean,
                    span,
                };
            }
            other => {
                self.error(
                    object.span,
                    format!("`in` requires an object receiver, found {other:?}"),
                );
                return self.poison(span);
            }
        };
        let Some((id, _)) = zun_natives::resolve("collections", native) else {
            self.error(span, format!("missing `{native}` intrinsic"));
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![object, key],
            },
            ty: Ty::Boolean,
            span,
        }
    }

    /// Lower `delete obj.k` / `delete obj[k]` to the dynamic delete mechanism
    /// (`reflectDelete`, or a Proxy's `deleteProperty` trap via `proxyDelete`).
    /// A non-member operand is a semantic error. The configurable-property
    /// descriptor model is deferred, so a successful delete always yields
    /// `true`.
    pub(in crate::check) fn check_delete_operator(
        &mut self,
        cx: &mut FunctionCx,
        operand: &ast::Expr,
        span: Span,
    ) -> hir::Expr {
        // `delete (arr as any)[k]`: an `as any` cast around a plain identifier
        // does not change which object is deleted from.
        if let ast::Expr::Index {
            object,
            index,
            span: index_span,
        } = operand
        {
            if let ast::Expr::Cast {
                expr: inner, ty, ..
            } = object.as_ref()
            {
                if self.text(&ty.name) == "any" && matches!(inner.as_ref(), ast::Expr::Ident(_)) {
                    let peeled = ast::Expr::Index {
                        object: inner.clone(),
                        index: index.clone(),
                        span: *index_span,
                    };
                    return self.check_delete_operator(cx, &peeled, span);
                }
            }
        }
        // `delete a?.b` / `delete a?.[k]` / `delete a?.b.c`: a nullish guard
        // short-circuits to `true`; otherwise delete through the chain root.
        {
            let guarded: Option<(&ast::Expr, ast::Expr)> = match operand {
                ast::Expr::OptionalField { object, field, .. } => Some((
                    object.as_ref(),
                    ast::Expr::Field {
                        object: Box::new(ast::Expr::ChainRoot { depth: 0, span }),
                        field: *field,
                        span,
                    },
                )),
                ast::Expr::OptionalIndex { object, index, .. } => Some((
                    object.as_ref(),
                    ast::Expr::Index {
                        object: Box::new(ast::Expr::ChainRoot { depth: 0, span }),
                        index: index.clone(),
                        span,
                    },
                )),
                ast::Expr::OptionalChain { object, body, .. } => {
                    Some((object.as_ref(), body.as_ref().clone()))
                }
                _ => None,
            };
            if let Some((guard_ast, body)) = guarded {
                let guard = self.check_expr(cx, guard_ast);
                let (slot, tmp) = self.synthetic_local(
                    cx,
                    if matches!(guard.ty, Ty::Any | Ty::DynamicObject) {
                        Ty::Any
                    } else {
                        guard.ty
                    },
                );
                let tmp_expr = ast::Expr::Ident(tmp);
                self.chain_roots.push(tmp);
                let delete = ast::Expr::Unary {
                    op: ast::UnaryOp::Delete,
                    operand: Box::new(body),
                    span,
                };
                let cond = if matches!(guard.ty, Ty::Any | Ty::DynamicObject) {
                    ast::Expr::Conditional {
                        condition: Box::new(ast::Expr::Binary {
                            op: ast::BinaryOp::LooseEq,
                            lhs: Box::new(tmp_expr),
                            rhs: Box::new(ast::Expr::Null { span }),
                            span,
                        }),
                        when_true: Box::new(ast::Expr::Bool { value: true, span }),
                        when_false: Box::new(delete),
                        span,
                    }
                } else {
                    delete
                };
                let value = self.check_expr(cx, &cond);
                self.chain_roots.pop();
                let ty = value.ty;
                let bound = if matches!(guard.ty, Ty::Any | Ty::DynamicObject) {
                    Self::box_to_any(guard)
                } else {
                    guard
                };
                return hir::Expr {
                    kind: hir::ExprKind::Sequence {
                        effect: Box::new(hir::Expr {
                            kind: hir::ExprKind::Assign {
                                slot,
                                value: Box::new(bound),
                            },
                            ty: Ty::Any,
                            span,
                        }),
                        value: Box::new(value),
                    },
                    ty,
                    span,
                };
            }
        }
        // `delete arr[i]` on a typed array leaves a hole.
        if let ast::Expr::Index { object, index, .. } = operand {
            if matches!(object.as_ref(), ast::Expr::Ident(_)) {
                let probe = self.check_expr(cx, object);
                if probe.ty == Ty::Array {
                    let mut key = self.check_expr(cx, index);
                    if !key.ty.is_error() && matches!(key.ty, Ty::Any | Ty::F64) || key.ty.is_int()
                    {
                        key = Self::box_to_any(key);
                    }
                    if let Some((id, _)) = zun_natives::resolve("collections", "arrayDeleteIndex") {
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id,
                                args: vec![probe, key],
                            },
                            ty: Ty::Boolean,
                            span,
                        };
                    }
                }
            }
        }
        let (object_ast, key) = match operand {
            ast::Expr::Field { object, field, .. } => {
                let key = hir::Expr {
                    kind: hir::ExprKind::String(self.text(field).to_string()),
                    ty: Ty::String,
                    span: field.span,
                };
                (object.as_ref(), key)
            }
            ast::Expr::Index { object, index, .. } => {
                let key = self.check_expr(cx, index);
                let key = if matches!(key.ty, Ty::Any | Ty::F64) || key.ty.is_int() {
                    self.to_string_for_parse(key)
                } else {
                    key
                };
                // A Symbol key rides the dynamic lane (boxed) to `deleteStrict`.
                let key = if key.ty == Ty::Symbol {
                    Self::box_to_any(key)
                } else {
                    key
                };
                if !key.ty.is_error() && key.ty != Ty::String && key.ty != Ty::Any {
                    self.error(key.span, "`delete obj[k]` requires a string key");
                }
                (object.as_ref(), key)
            }
            _ => {
                // `delete <non-reference>` evaluates to `true`.
                return hir::Expr {
                    kind: hir::ExprKind::Boolean(true),
                    ty: Ty::Boolean,
                    span,
                };
            }
        };
        let object = self.check_expr(cx, object_ast);
        let native = match object.ty {
            Ty::Proxy => "proxyDelete",
            // Modules are strict: a refused delete throws a TypeError.
            Ty::DynamicObject | Ty::Any => "deleteStrict",
            Ty::Error => return self.poison(span),
            other => {
                self.error(
                    object.span,
                    format!("`delete` requires an object receiver, found {other:?}"),
                );
                return self.poison(span);
            }
        };
        let Some((id, _)) = zun_natives::resolve("collections", native) else {
            self.error(span, format!("missing `{native}` intrinsic"));
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![object, key],
            },
            ty: Ty::Boolean,
            span,
        }
    }

    /// Lower `typeof x`. A statically-typed operand folds to a constant JS type
    /// string (every non-`any` value has a known type in Zun, so the fold is
    /// exact). An `any` operand has no static type, so it is lowered to the
    /// `typeofAny` intrinsic, which inspects the runtime tag/heap value.
    pub(in crate::check) fn check_typeof_operator(
        &mut self,
        cx: &mut FunctionCx,
        operand: &ast::Expr,
        span: Span,
    ) -> hir::Expr {
        // `typeof Map.groupBy` / `typeof Math.max`: well-known built-in functions.
        {
            let mut parts: Vec<String> = Vec::new();
            let mut cursor = operand;
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
            if ok && parts.len() >= 2 {
                parts.reverse();
                let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
                let known = builtin_function_length(&refs).is_some()
                    || (refs.len() == 2
                        && refs[0] == "Math"
                        && math_function_length(refs[1]).is_some());
                if known
                    && !self.classes_by_name.contains_key(refs[0])
                    && cx.lookup(refs[0]).is_none()
                {
                    return hir::Expr {
                        kind: hir::ExprKind::String("function".to_string()),
                        ty: Ty::String,
                        span,
                    };
                }
            }
        }
        let mut operand = self.check_expr(cx, operand);
        // `typeof undeclared` is "undefined", never a ReferenceError.
        if let hir::ExprKind::CallBuiltin { id, args } = &mut operand.kind {
            if let (Some((read, _)), Some((get, _))) = (
                zun_natives::resolve("collections", "globalRead"),
                zun_natives::resolve("collections", "globalGet"),
            ) {
                if *id == read {
                    *id = get;
                    let _ = args;
                }
            }
        }
        let type_string = match operand.ty {
            Ty::Error => return self.poison(span),
            Ty::Int(_) | Ty::Float(_) => "number",
            Ty::String => "string",
            Ty::Boolean => "boolean",
            Ty::BigInt => "bigint",
            Ty::Symbol => "symbol",
            Ty::Function(_) => "function",
            Ty::Void => "undefined",
            // An `any` operand carries no static type, so inspect its runtime
            // tag with the `typeofAny` intrinsic instead of folding.
            Ty::Any => {
                let Some((id, _)) = zun_natives::resolve_name("typeofAny") else {
                    self.error(span, "missing `typeofAny` intrinsic".to_string());
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![operand],
                    },
                    ty: Ty::String,
                    span,
                };
            }
            // Arrays, maps, sets, proxies, instances, buffers, null → "object".
            _ => "object",
        };
        let folded = hir::Expr {
            kind: hir::ExprKind::String(type_string.to_string()),
            ty: Ty::String,
            span,
        };
        // The operand is still evaluated for its side effects (`typeof arr.push(1)`).
        if matches!(
            operand.kind,
            hir::ExprKind::Local(_)
                | hir::ExprKind::Number(_)
                | hir::ExprKind::String(_)
                | hir::ExprKind::Boolean(_)
                | hir::ExprKind::Null
        ) {
            return folded;
        }
        hir::Expr {
            kind: hir::ExprKind::Sequence {
                effect: Box::new(operand),
                value: Box::new(folded),
            },
            ty: Ty::String,
            span,
        }
    }

    pub(in crate::check) fn check_instanceof_operator(
        &mut self,
        cx: &mut FunctionCx,
        value: &ast::Expr,
        class: &ast::Expr,
        span: Span,
    ) -> hir::Expr {
        let value = self.check_expr(cx, value);
        let ast::Expr::Ident(class_name) = class else {
            // A non-identifier right operand (`x instanceof (y as any)`) is a
            // run-time value: dispatch dynamically.
            let rhs = self.check_expr(cx, class);
            if value.ty.is_error() || rhs.ty.is_error() {
                return self.poison(span);
            }
            let Some((id, _)) = zun_natives::resolve("typeinfo", "instanceofDynamic") else {
                self.error(span, "missing `instanceofDynamic` intrinsic".to_string());
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![Self::box_to_any(value), Self::box_to_any(rhs)],
                },
                ty: Ty::Boolean,
                span,
            };
        };
        let name = self.text(class_name).to_string();
        if !self.classes_by_name.contains_key(&name)
            && matches!(
                name.as_str(),
                "Array" | "Map" | "Set" | "Promise" | "RegExp" | "Object" | "Function"
            )
            && !cx.scopes.iter().any(|s| s.contains_key(name.as_str()))
        {
            if value.ty.is_error() {
                return self.poison(span);
            }
            let Some((id, _)) = zun_natives::resolve("typeinfo", "instanceofBuiltin") else {
                self.error(span, "missing `instanceofBuiltin` intrinsic".to_string());
                return self.poison(span);
            };
            let value = Self::box_to_any(value);
            let ctor = hir::Expr {
                kind: hir::ExprKind::String(name),
                ty: Ty::String,
                span,
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![value, ctor],
                },
                ty: Ty::Boolean,
                span,
            };
        }
        // `x instanceof String|Number|Boolean` tests against the prelude
        // wrapper class (`new String(..)` builds a `StringObject`).
        let wrapper_name = match name.as_str() {
            "String" => Some("StringObject"),
            "Number" => Some("NumberObject"),
            "Boolean" => Some("BooleanObject"),
            _ => None,
        };
        let wrapper_target = match wrapper_name {
            Some(w)
                if !self.classes_by_name.contains_key(&name)
                    && !cx.scopes.iter().any(|s| s.contains_key(name.as_str())) =>
            {
                self.classes_by_name.get(w).copied()
            }
            _ => None,
        };
        let Some(target) = wrapper_target.or_else(|| self.classes_by_name.get(&name).copied())
        else {
            // Not a class name: the right operand may be a dynamic object
            // carrying a `[Symbol.hasInstance]` method. If the identifier
            // resolves to an `any`/dynamic value, dispatch through that method
            // at runtime (`C[Symbol.hasInstance](value)`); otherwise it is a
            // genuine unknown-class reference.
            let rhs = self.check_expr(cx, class);
            if matches!(rhs.ty, Ty::Any | Ty::DynamicObject) {
                let Some((id, _)) = zun_natives::resolve("typeinfo", "instanceofDynamic") else {
                    self.error(span, "missing `instanceofDynamic` intrinsic".to_string());
                    return self.poison(span);
                };
                // The operand reaches the native as `any` (the method's
                // parameter is `any`), so box a non-dynamic left operand.
                let value = if value.ty == Ty::Any {
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
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![value, rhs],
                    },
                    ty: Ty::Boolean,
                    span,
                };
            }
            if !rhs.ty.is_error() {
                self.error(class_name.span, format!("unknown class `{name}`"));
            }
            return self.poison(span);
        };
        // Decide between a constant fold and a runtime class-hierarchy walk.
        // `None` means "check at runtime": an upcast base-typed operand that may
        // hold a `target` subclass, or an `any`/dynamic operand with no static
        // class. (The old code folded these to an unsound constant `false`.)
        let folded = if self.classes_with_has_instance.contains(&target) {
            // A custom `[Symbol.hasInstance]` always decides membership, so the
            // answer can never be constant-folded — force the runtime path,
            // which consults the per-type symbol-method side table.
            None
        } else {
            match value.ty {
                Ty::Error => return self.poison(span),
                // A statically-known instance provably in `target`'s hierarchy.
                Ty::Instance(c) if self.class_is_a(c, target) => Some(true),
                Ty::Instance(_) | Ty::Any | Ty::DynamicObject => None,
                // A primitive value is never an instance of a class.
                _ => Some(false),
            }
        };
        let Some(result) = folded else {
            // Runtime check against `target`'s metadata TypeDef rid. The rid is
            // `target + 1`: `emit_types` in `zun_ilgen` emits one TypeDef per
            // class in order, so class index `i` owns rid `i + 1` (asserted
            // there).
            let Some((id, _)) = zun_natives::resolve_name("instanceofClass") else {
                self.error(span, "missing `instanceofClass` intrinsic".to_string());
                return self.poison(span);
            };
            let class_rid = hir::Expr {
                kind: hir::ExprKind::Number((target + 1) as f64),
                ty: Ty::Int(IntKind::U32),
                span,
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![value, class_rid],
                },
                ty: Ty::Boolean,
                span,
            };
        };
        hir::Expr {
            kind: hir::ExprKind::Boolean(result),
            ty: Ty::Boolean,
            span,
        }
    }

    /// Whether the class or one of its bases declares `[Symbol.iterator]`.
    pub(in crate::check) fn class_has_iterator(&self, class_index: u32) -> bool {
        let mut current = Some(class_index);
        while let Some(c) = current {
            if self.classes_with_iterator.contains(&c) {
                return true;
            }
            current = self.classes.get(c as usize).and_then(|info| info.base);
        }
        false
    }

    /// Whether class `class_index` is `target` or a (transitive) subclass of it.
    pub(in crate::check) fn class_is_a(&self, mut class_index: u32, target: u32) -> bool {
        loop {
            if class_index == target {
                return true;
            }
            match self.classes.get(class_index as usize).and_then(|c| c.base) {
                Some(base) => class_index = base,
                None => return false,
            }
        }
    }
}
