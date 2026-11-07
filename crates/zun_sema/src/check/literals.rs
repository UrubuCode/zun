use crate::check::*;

impl Checker<'_> {
    /// Lower an object literal's members (data properties, method shorthand,
    /// computed-symbol methods, getters/setters, and spread) into
    /// [`hir::ObjectEntry`] entries evaluated in source order.
    pub(in crate::check) fn lower_object_members(
        &mut self,
        cx: &mut FunctionCx,
        members: &[ast::ObjectMember],
        span: Span,
    ) -> Vec<hir::ObjectEntry> {
        let mut entries = Vec::with_capacity(members.len());
        for member in members {
            match member {
                ast::ObjectMember::Spread { expr, .. } => {
                    // `{ ...o }` → objectAssign(obj, o): copy the source's own
                    // enumerable string keys (DynamicObject) or declared
                    // instance fields (typed object) into the new object.
                    let source = Self::box_to_any(self.check_expr(cx, expr));
                    let Some(builtin) = self.resolve_collection(span, "objectAssign") else {
                        continue;
                    };
                    entries.push(hir::ObjectEntry::Install {
                        builtin,
                        args: vec![source],
                    });
                }
                ast::ObjectMember::Property(property) => {
                    self.lower_object_property(cx, property, span, &mut entries);
                }
            }
        }
        entries
    }

    /// A non-arrow `function` expression that mentions `this` and is stored as a
    /// property behaves like a method: arm `closure_takes_this` so it receives
    /// the receiver as its hidden first parameter.
    pub(in crate::check) fn mark_this_closure(&mut self, value: &ast::Expr) {
        if let ast::Expr::Function(function) = value {
            if !function.is_arrow {
                let body = &self.source[function.span.start as usize..function.span.end as usize];
                self.closure_takes_this = body.contains("this");
            }
        }
    }

    pub(in crate::check) fn lower_object_property(
        &mut self,
        cx: &mut FunctionCx,
        property: &ast::ObjectProperty,
        span: Span,
        entries: &mut Vec<hir::ObjectEntry>,
    ) {
        // A statically-known string key, or `None` for a computed key.
        let named_key = match &property.key {
            ast::PropertyKey::Ident(name) => Some(self.text(name).to_string()),
            ast::PropertyKey::String { value, .. } => Some(value.clone()),
            ast::PropertyKey::Number { value, .. } => Some(value.to_string()),
            ast::PropertyKey::Computed(_) => None,
        };
        match property.kind {
            ast::PropertyKind::Init => {
                // A `function` expression that mentions `this` and is stored as
                // a data property behaves like a method: it receives the object
                // as its hidden first parameter.
                if named_key.is_some() {
                    if let ast::Expr::Function(function) = &property.value {
                        if !function.is_arrow {
                            let body = &self.source
                                [function.span.start as usize..function.span.end as usize];
                            self.closure_takes_this = body.contains("this");
                        }
                    }
                }
                let checked = self.check_expr(cx, &property.value);
                self.closure_takes_this = false;
                // A function-valued data property (`{ f: () => 1 }`) lives in the
                // object's function slot, exactly like method shorthand, so
                // `obj.f()` and the `valueOf`/`toString` coercion hooks find it.
                if let (Ty::Function(_), Some(name)) = (checked.ty, named_key.clone()) {
                    if let Some(builtin) = self.resolve_collection(span, "dynamicSetFunction") {
                        entries.push(hir::ObjectEntry::Install {
                            builtin,
                            args: vec![Self::string_literal(name, span), checked],
                        });
                        return;
                    }
                }
                // `{ [Symbol.iterator]: function () {} }`: a function value under a
                // computed key lives in the function slot, like method shorthand.
                if let (Ty::Function(_), ast::PropertyKey::Computed(key_expr)) =
                    (checked.ty, &property.key)
                {
                    let key = self.check_expr(cx, key_expr);
                    let slot_builtin = if key.ty == Ty::Symbol || key.ty == Ty::Any {
                        self.resolve_collection(span, "dynamicSetSymbolFunction")
                            .map(|builtin| (builtin, Self::box_to_any(key)))
                    } else if key.ty == Ty::String {
                        self.resolve_collection(span, "dynamicSetFunction")
                            .map(|builtin| (builtin, key))
                    } else {
                        None
                    };
                    if let Some((builtin, key)) = slot_builtin {
                        entries.push(hir::ObjectEntry::Install {
                            builtin,
                            args: vec![key, checked],
                        });
                        return;
                    }
                    self.error(span, "a computed method key must be a string or a Symbol");
                    return;
                }
                let value = Self::box_to_any(checked);
                let key = match &property.key {
                    ast::PropertyKey::Computed(key) => {
                        hir::ObjectKey::Computed(self.check_expr(cx, key))
                    }
                    _ => hir::ObjectKey::Named(named_key.expect("named key")),
                };
                // A data property that replaces an accessor installed earlier in the
                // same literal, or a string computed key (which never sets the
                // prototype, even for `["__proto__"]`), is a definition: route it
                // through `objectDefineProperty` instead of an assignment.
                let define = self.resolve_collection(span, "objectDefineProperty");
                let replaces_accessor = match (&key, define) {
                    (hir::ObjectKey::Named(name), Some(define_id)) => entries.iter().any(|entry| {
                        matches!(entry, hir::ObjectEntry::Install { builtin, args }
                            if *builtin == define_id
                                && matches!(args.first(), Some(hir::Expr { kind: hir::ExprKind::String(existing), .. }) if existing == name))
                    }),
                    _ => false,
                };
                let computed_string =
                    matches!(&key, hir::ObjectKey::Computed(k) if k.ty == Ty::String);
                if let (true, Some(define_id)) = (replaces_accessor || computed_string, define) {
                    let key_expr = match key {
                        hir::ObjectKey::Named(name) => Self::string_literal(name, span),
                        hir::ObjectKey::Computed(k) => k,
                    };
                    {
                        let descriptor = hir::Expr {
                            kind: hir::ExprKind::DynamicObjectLiteral {
                                entries: vec![
                                    hir::ObjectEntry::Data {
                                        key: hir::ObjectKey::Named("value".to_string()),
                                        value,
                                    },
                                    decorator_context_entry(
                                        "writable",
                                        hir::ExprKind::Boolean(true),
                                        Ty::Boolean,
                                        span,
                                    ),
                                    decorator_context_entry(
                                        "enumerable",
                                        hir::ExprKind::Boolean(true),
                                        Ty::Boolean,
                                        span,
                                    ),
                                    decorator_context_entry(
                                        "configurable",
                                        hir::ExprKind::Boolean(true),
                                        Ty::Boolean,
                                        span,
                                    ),
                                ],
                            },
                            ty: Ty::DynamicObject,
                            span,
                        };
                        entries.push(hir::ObjectEntry::Install {
                            builtin: define_id,
                            args: vec![key_expr, descriptor],
                        });
                        return;
                    }
                }
                entries.push(hir::ObjectEntry::Data { key, value });
            }
            ast::PropertyKind::Method => {
                // A method that mentions `this` receives the object as a hidden
                // first parameter (see `closure_takes_this`).
                if let ast::Expr::Function(function) = &property.value {
                    let body =
                        &self.source[function.span.start as usize..function.span.end as usize];
                    self.closure_takes_this = body.contains("this");
                }
                let func = self.check_expr(cx, &property.value);
                self.closure_takes_this = false;
                match &property.key {
                    ast::PropertyKey::Computed(key) => {
                        let key = self.check_expr(cx, key);
                        // A `Symbol` computed key routes to the symbol-function
                        // slot (the enabler for user `[Symbol.iterator]` /
                        // `[Symbol.hasInstance]` methods); a string key uses the
                        // ordinary string function slot.
                        if key.ty == Ty::Symbol || key.ty == Ty::Any {
                            if let Some(builtin) =
                                self.resolve_collection(span, "dynamicSetSymbolFunction")
                            {
                                entries.push(hir::ObjectEntry::Install {
                                    builtin,
                                    args: vec![Self::box_to_any(key), func],
                                });
                            }
                        } else if key.ty == Ty::String {
                            if let Some(builtin) =
                                self.resolve_collection(span, "dynamicSetFunction")
                            {
                                entries.push(hir::ObjectEntry::Install {
                                    builtin,
                                    args: vec![key, func],
                                });
                            }
                        } else {
                            self.error(span, "a computed method key must be a string or a Symbol");
                        }
                    }
                    _ => {
                        let name = named_key.expect("named key");
                        if let Some(builtin) = self.resolve_collection(span, "dynamicSetFunction") {
                            entries.push(hir::ObjectEntry::Install {
                                builtin,
                                args: vec![Self::string_literal(name, span), func],
                            });
                        }
                    }
                }
            }
            ast::PropertyKind::Get | ast::PropertyKind::Set => {
                let Some(name) = named_key else {
                    self.error(
                        span,
                        "computed getter/setter keys in object literals are not supported yet",
                    );
                    return;
                };
                if let ast::Expr::Function(function) = &property.value {
                    let body =
                        &self.source[function.span.start as usize..function.span.end as usize];
                    self.closure_takes_this = body.contains("this");
                }
                let func = self.check_expr(cx, &property.value);
                self.closure_takes_this = false;
                let accessor_slot = if property.kind == ast::PropertyKind::Get {
                    "get"
                } else {
                    "set"
                };
                // Build the accessor descriptor `{ get|set: fn }` as a dynamic
                // object (the function lives in its string function slot), then
                // install it with objectDefineProperty — the same model
                // `Object.defineProperty` uses, so the named-key accessor is
                // invoked on `obj.k` read/write.
                let Some(set_fn) = self.resolve_collection(span, "dynamicSetFunction") else {
                    return;
                };
                let descriptor = hir::Expr {
                    kind: hir::ExprKind::DynamicObjectLiteral {
                        entries: vec![
                            hir::ObjectEntry::Install {
                                builtin: set_fn,
                                args: vec![
                                    Self::string_literal(accessor_slot.to_string(), span),
                                    func,
                                ],
                            },
                            // Literal accessors are enumerable and configurable.
                            decorator_context_entry(
                                "enumerable",
                                hir::ExprKind::Boolean(true),
                                Ty::Boolean,
                                span,
                            ),
                            decorator_context_entry(
                                "configurable",
                                hir::ExprKind::Boolean(true),
                                Ty::Boolean,
                                span,
                            ),
                        ],
                    },
                    ty: Ty::DynamicObject,
                    span,
                };
                let Some(define) = self.resolve_collection(span, "objectDefineProperty") else {
                    return;
                };
                entries.push(hir::ObjectEntry::Install {
                    builtin: define,
                    args: vec![Self::string_literal(name, span), descriptor],
                });
            }
        }
    }

    /// Lower an array literal, choosing the fixed-length store path when there
    /// is no spread, and the growable builder path when a spread is present.
    pub(in crate::check) fn lower_array_literal(
        &mut self,
        cx: &mut FunctionCx,
        elements: &[ast::ArrayElement],
        span: Span,
    ) -> hir::Expr {
        let has_spread = elements
            .iter()
            .any(|element| matches!(element, ast::ArrayElement::Spread(_)));
        if !has_spread {
            // Fixed-length path: allocate N slots and store each present
            // element at its index; a hole leaves the index as `undefined`.
            let elements = elements
                .iter()
                .map(|element| match element {
                    ast::ArrayElement::Expr(expr) => Some(self.check_expr(cx, expr)),
                    ast::ArrayElement::Hole => None,
                    ast::ArrayElement::Spread(_) => unreachable!("spread handled above"),
                })
                .collect();
            return hir::Expr {
                kind: hir::ExprKind::ArrayLiteral { elements },
                ty: Ty::Array,
                span,
            };
        }
        let mut parts = Vec::with_capacity(elements.len());
        for element in elements {
            match element {
                ast::ArrayElement::Expr(expr) => {
                    parts.push(hir::ArrayPart::Element(self.check_expr(cx, expr)));
                }
                ast::ArrayElement::Hole => parts.push(hir::ArrayPart::Hole),
                ast::ArrayElement::Spread(expr) => {
                    let source = Self::box_to_any(self.check_expr(cx, expr));
                    let Some((builtin, _)) = zun_natives::resolve("array", "arraySpreadAppend")
                    else {
                        self.error(span, "missing Array intrinsic `arraySpreadAppend`");
                        continue;
                    };
                    parts.push(hir::ArrayPart::Spread { builtin, source });
                }
            }
        }
        hir::Expr {
            kind: hir::ExprKind::ArraySpreadLiteral { parts },
            ty: Ty::Array,
            span,
        }
    }

    /// Lower `x instanceof C`. A statically-known instance that is provably in
    /// `C`'s hierarchy folds to constant `true`, and a value whose static type
    /// can never be an instance folds to `false`. Otherwise — an `any`/dynamic
    /// operand, or a base-typed operand that may hold a `C` subclass at runtime
    /// — it lowers to the `instanceofClass` intrinsic, which walks the operand's
    /// actual class hierarchy against `C`'s metadata rid.
    ///
    /// Scope of the runtime walk: it matches typed class instances (including
    /// ones upcast through `any`/a base type) by their runtime `type_rid`. A
    /// plain dynamic object built with `Object.create` has no class identity, so
    /// it answers `false` (never a false positive); matching such objects
    /// against a class `prototype` stays deferred (needs a class↔prototype-object
    /// model that does not exist yet). `Symbol.hasInstance` customization IS wired
    /// when the right operand is a dynamic object carrying the method: such an
    /// operand is not a class name, so it falls through to the `instanceofDynamic`
    /// intrinsic below. Only `[Symbol.hasInstance]` on a typed class stays deferred
    /// (needs class computed-symbol-key members).
    /// Whether a tagged-template tag is the built-in `String.raw`.
    pub(in crate::check) fn tag_is_string_raw(&self, tag: &ast::Expr) -> bool {
        let (object, field) = match tag {
            ast::Expr::Field { object, field, .. } => {
                let ast::Expr::Ident(name) = object.as_ref() else {
                    return false;
                };
                (self.text(name), self.text(field))
            }
            ast::Expr::StaticField { class, field, .. } => (self.text(class), self.text(field)),
            _ => return false,
        };
        object == "String" && field == "raw" && !self.by_name.contains_key("String")
    }

    /// Lower a tagged template literal. Only `String.raw` is supported as the
    /// tag: it stringifies each substitution and joins it between the **raw**
    /// (un-escaped) literal chunks, so `` String.raw`a\n${1}b` `` is
    /// `"a\\n1b"`. User-defined tags need a strings-array-with-`.raw` plus a
    /// variadic dynamic dispatch and stay deferred.
    pub(in crate::check) fn check_tagged_template(
        &mut self,
        cx: &mut FunctionCx,
        tag: &ast::Expr,
        raw: &[String],
        exprs: &[Box<ast::Expr>],
        span: Span,
    ) -> hir::Expr {
        if !self.tag_is_string_raw(tag) {
            self.error(
                tag.span(),
                "only `String.raw` is supported as a template tag",
            );
            return self.poison(span);
        }
        let string_lit = |text: &str| hir::Expr {
            kind: hir::ExprKind::String(text.to_string()),
            ty: Ty::String,
            span,
        };
        // raw.len() == exprs.len() + 1 (the parser guarantees alignment).
        let mut result = string_lit(raw.first().map(String::as_str).unwrap_or(""));
        for (index, expression) in exprs.iter().enumerate() {
            let value = self.check_expr(cx, expression);
            let next = if value.ty == Ty::F64 {
                match zun_natives::resolve("number", "toString") {
                    Some((id, _)) => hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![value],
                        },
                        ty: Ty::String,
                        span,
                    },
                    None => {
                        self.error(span, "number template conversion is unavailable");
                        self.poison(span)
                    }
                }
            } else {
                if value.ty != Ty::String && !value.ty.is_error() {
                    self.error(
                        value.span,
                        "String.raw substitution must have type string or number",
                    );
                }
                value
            };
            result = hir::Expr {
                kind: hir::ExprKind::Binary {
                    op: ast::BinaryOp::Add,
                    lhs: Box::new(result),
                    rhs: Box::new(next),
                },
                ty: Ty::String,
                span,
            };
            let chunk = raw.get(index + 1).map(String::as_str).unwrap_or("");
            result = hir::Expr {
                kind: hir::ExprKind::Binary {
                    op: ast::BinaryOp::Add,
                    lhs: Box::new(result),
                    rhs: Box::new(string_lit(chunk)),
                },
                ty: Ty::String,
                span,
            };
        }
        result
    }
}
