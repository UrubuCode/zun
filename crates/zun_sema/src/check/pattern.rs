use crate::check::*;

impl Checker<'_> {
    /// Bind a declaration/for-of pattern from the value produced by `value`,
    /// appending the lowered binding statements to `out`. Array and object
    /// patterns stash `value` into a hidden temp and read each sub-binding
    /// from it; nested patterns recurse.
    pub(in crate::check) fn bind_pattern(
        &mut self,
        cx: &mut FunctionCx,
        pattern: &ast::Pattern,
        value: hir::Expr,
        out: &mut Vec<hir::Stmt>,
    ) {
        let span = value.span;
        match pattern {
            ast::Pattern::Ident { name, ty } => {
                let declared = ty.as_ref().map(|t| self.resolve_annotation(t));
                let slot_ty = declared.unwrap_or(value.ty);
                let text = self.text(name).to_string();
                let slot = cx.declare(&text, slot_ty);
                out.push(hir::Stmt::Let { slot, init: value });
            }
            ast::Pattern::Array { elements, rest, .. } => {
                // A string destructures by code point.
                let value = if value.ty == Ty::String { self.spread_source_to_array(value) } else { value };
                if !matches!(value.ty, Ty::Array | Ty::Any | Ty::Error) {
                    self.error(span, "array destructuring requires an Array or any value");
                }
                // With defaults the iterator is pulled one element at a time so a
                // throwing default closes it before later elements are read.
                if value.ty == Ty::Any
                    && elements.iter().flatten().any(|element| element.default.is_some())
                {
                    if let (Some((open, _)), Some((step, _)), Some((rest_id, _)), Some((finish, _))) = (
                        zun_natives::resolve("array", "destructOpen"),
                        zun_natives::resolve("array", "destructStep"),
                        zun_natives::resolve("array", "destructRest"),
                        zun_natives::resolve("array", "destructFinish"),
                    ) {
                        let source = cx.declare(&self.fresh_temp_name(cx), Ty::Any);
                        out.push(hir::Stmt::Let {
                            slot: source,
                            init: hir::Expr {
                                kind: hir::ExprKind::CallBuiltin { id: open, args: vec![value] },
                                ty: Ty::Any,
                                span,
                            },
                        });
                        let local = || hir::Expr { kind: hir::ExprKind::Local(source), ty: Ty::Any, span };
                        let finish_call = |throwing: bool| hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id: finish,
                                args: vec![
                                    local(),
                                    hir::Expr { kind: hir::ExprKind::Boolean(throwing), ty: Ty::Boolean, span },
                                ],
                            },
                            ty: Ty::Boolean,
                            span,
                        };
                        let mut inner: Vec<hir::Stmt> = Vec::new();
                        for (index, element) in elements.iter().enumerate() {
                            let Some(element) = element else {
                                // A hole still consumes one value.
                                inner.push(hir::Stmt::Expr(hir::Expr {
                                    kind: hir::ExprKind::CallBuiltin {
                                        id: step,
                                        args: vec![
                                            local(),
                                            hir::Expr { kind: hir::ExprKind::Number(index as f64), ty: Ty::F64, span },
                                        ],
                                    },
                                    ty: Ty::Any,
                                    span,
                                }));
                                continue;
                            };
                            let read = hir::Expr {
                                kind: hir::ExprKind::CallBuiltin {
                                    id: step,
                                    args: vec![
                                        local(),
                                        hir::Expr { kind: hir::ExprKind::Number(index as f64), ty: Ty::F64, span },
                                    ],
                                },
                                ty: Ty::Any,
                                span,
                            };
                            let read = self.apply_default(cx, read, element.default.as_ref(), &mut inner);
                            self.bind_pattern(cx, &element.pattern, read, &mut inner);
                        }
                        if let Some(rest) = rest {
                            let read = hir::Expr {
                                kind: hir::ExprKind::CallBuiltin {
                                    id: rest_id,
                                    args: vec![
                                        local(),
                                        hir::Expr {
                                            kind: hir::ExprKind::Number(elements.len() as f64),
                                            ty: Ty::Int(IntKind::U64),
                                            span,
                                        },
                                    ],
                                },
                                ty: Ty::Array,
                                span,
                            };
                            self.bind_pattern(cx, rest, read, &mut inner);
                        }
                        let caught = cx.declare(&self.fresh_temp_name(cx), Ty::Any);
                        out.push(hir::Stmt::Try {
                            body: inner,
                            catches: vec![hir::CatchClause {
                                slot: caught,
                                ty: Ty::Any,
                                body: vec![
                                    hir::Stmt::Expr(finish_call(true)),
                                    hir::Stmt::Throw(hir::Expr {
                                        kind: hir::ExprKind::Local(caught),
                                        ty: Ty::Any,
                                        span,
                                    }),
                                ],
                            }],
                            finally_body: None,
                        });
                        out.push(hir::Stmt::Expr(finish_call(false)));
                        return;
                    }
                }
                // A dynamic source may hold a string or any iterable, not just
                // an Array: materialize it through the iteration protocol first.
                let value = if value.ty == Ty::Any {
                    // Without a rest element only `elements.len()` values are taken
                    // and an unfinished iterator is closed afterwards.
                    let limited = rest.is_none().then(|| zun_natives::resolve("array", "arrayDestructure")).flatten();
                    if let Some((id, _)) = limited {
                        hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id,
                                args: vec![
                                    value,
                                    hir::Expr {
                                        kind: hir::ExprKind::Number(elements.len() as f64),
                                        ty: Ty::Int(IntKind::U32),
                                        span,
                                    },
                                ],
                            },
                            ty: Ty::Array,
                            span,
                        }
                    } else {
                    match zun_natives::resolve("array", "arrayFrom") {
                        Some((id, _)) => hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: vec![value] },
                            ty: Ty::Array,
                            span,
                        },
                        None => value,
                    }
                    }
                } else {
                    value
                };
                let temp = cx.declare(&self.fresh_temp_name(cx), value.ty);
                let temp_ty = value.ty;
                out.push(hir::Stmt::Let { slot: temp, init: value });
                for (index, element) in elements.iter().enumerate() {
                    let Some(element) = element else { continue };
                    let read = self.array_element_read(temp, temp_ty, index, span);
                    let read = self.apply_default(cx, read, element.default.as_ref(), out);
                    self.bind_pattern(cx, &element.pattern, read, out);
                }
                if let Some(rest) = rest {
                    let read = self.array_rest_read(temp, temp_ty, elements.len(), span);
                    self.bind_pattern(cx, rest, read, out);
                }
            }
            ast::Pattern::Object { properties, rest, .. } => {
                if !matches!(value.ty, Ty::Any | Ty::DynamicObject | Ty::Instance(_) | Ty::Error) {
                    self.error(span, "object destructuring requires an object value");
                }
                let temp = cx.declare(&self.fresh_temp_name(cx), value.ty);
                let temp_ty = value.ty;
                out.push(hir::Stmt::Let { slot: temp, init: value });
                let mut excluded: Vec<String> = Vec::new();
                // Computed keys are evaluated exactly once (into a temp) and
                // then also excluded from the `...rest` object.
                let mut computed: Vec<(hir::LocalSlot, Ty)> = Vec::new();
                for property in properties {
                    let (read, static_key) = if let ast::PropertyKey::Computed(key_expr) = &property.key {
                        let key_value = self.check_expr(cx, key_expr);
                        let key_ty = key_value.ty;
                        let key_temp = cx.declare(&self.fresh_temp_name(cx), key_ty);
                        out.push(hir::Stmt::Let { slot: key_temp, init: key_value });
                        computed.push((key_temp, key_ty));
                        let object = hir::Expr { kind: hir::ExprKind::Local(temp), ty: temp_ty, span };
                        (
                            hir::Expr {
                                kind: hir::ExprKind::DynamicGetValue {
                                    object: Box::new(object),
                                    key: Box::new(hir::Expr {
                                        kind: hir::ExprKind::Local(key_temp),
                                        ty: key_ty,
                                        span,
                                    }),
                                },
                                ty: Ty::Any,
                                span,
                            },
                            None,
                        )
                    } else {
                        self.object_member_read(cx, temp, temp_ty, &property.key, span)
                    };
                    if let Some(key) = static_key {
                        excluded.push(key);
                    }
                    let read = self.apply_default(cx, read, property.default.as_ref(), out);
                    self.bind_pattern(cx, &property.value, read, out);
                }
                if let Some(rest) = rest {
                    let extra: Vec<hir::Expr> = computed
                        .iter()
                        .map(|&(slot, ty)| hir::Expr { kind: hir::ExprKind::Local(slot), ty, span })
                        .collect();
                    let read = self.object_rest_read(temp, temp_ty, &excluded, extra, span);
                    self.bind_pattern(cx, rest, read, out);
                }
            }
        }
    }

    /// Read array element `index` from the temp holding the source array.
    /// Uses `arrayAt`, which yields `undefined` for an out-of-range index
    /// (so trailing holes and defaults read `undefined` rather than trapping).
    pub(in crate::check) fn array_element_read(&mut self, temp: hir::LocalSlot, temp_ty: Ty, index: usize, span: Span) -> hir::Expr {
        let Some((id, _)) = zun_natives::resolve("array", "arrayAt") else {
            self.error(span, "missing `arrayAt` intrinsic");
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![
                    hir::Expr { kind: hir::ExprKind::Local(temp), ty: temp_ty, span },
                    hir::Expr { kind: hir::ExprKind::Number(index as f64), ty: Ty::F64, span },
                ],
            },
            ty: Ty::Any,
            span,
        }
    }

    /// Collect array elements from `start` to the end into a fresh Array.
    pub(in crate::check) fn array_rest_read(&mut self, temp: hir::LocalSlot, temp_ty: Ty, start: usize, span: Span) -> hir::Expr {
        let Some((id, _)) = zun_natives::resolve("array", "arraySlice") else {
            self.error(span, "missing `arraySlice` intrinsic");
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![
                    hir::Expr { kind: hir::ExprKind::Local(temp), ty: temp_ty, span },
                    hir::Expr {
                        kind: hir::ExprKind::Number(start as f64),
                        ty: Ty::Int(IntKind::U64),
                        span,
                    },
                ],
            },
            ty: Ty::Array,
            span,
        }
    }

    /// Read one property from the temp. Returns the read expression and, for a
    /// statically-known string key, that key (for object-rest exclusion).
    pub(in crate::check) fn object_member_read(
        &mut self,
        cx: &mut FunctionCx,
        temp: hir::LocalSlot,
        temp_ty: Ty,
        key: &ast::PropertyKey,
        span: Span,
    ) -> (hir::Expr, Option<String>) {
        let object = hir::Expr { kind: hir::ExprKind::Local(temp), ty: temp_ty, span };
        let name = match key {
            ast::PropertyKey::Ident(name) => Some(self.text(name).to_string()),
            ast::PropertyKey::String { value, .. } => Some(value.clone()),
            ast::PropertyKey::Number { value, .. } => Some(value.to_string()),
            ast::PropertyKey::Computed(_) => None,
        };
        if let Some(name) = name {
            // A typed instance reads its declared field directly.
            if let Ty::Instance(class) = temp_ty {
                if let Some((field_slot, ty)) = self.field_of(class, &name) {
                    return (
                        hir::Expr {
                            kind: hir::ExprKind::FieldGet { object: Box::new(object), class, field_slot },
                            ty,
                            span,
                        },
                        Some(name),
                    );
                }
            }
            return (
                hir::Expr {
                    kind: hir::ExprKind::DynamicGet { object: Box::new(object), name: name.clone() },
                    ty: Ty::Any,
                    span,
                },
                Some(name),
            );
        }
        // Computed key: evaluate the key expression and read dynamically.
        let ast::PropertyKey::Computed(expr) = key else { unreachable!() };
        let key_expr = self.check_expr(cx, expr);
        (
            hir::Expr {
                kind: hir::ExprKind::DynamicGetValue { object: Box::new(object), key: Box::new(key_expr) },
                ty: Ty::Any,
                span,
            },
            None,
        )
    }

    /// Collect the source object's remaining own keys into a fresh object.
    pub(in crate::check) fn object_rest_read(
        &mut self,
        temp: hir::LocalSlot,
        temp_ty: Ty,
        excluded: &[String],
        extra: Vec<hir::Expr>,
        span: Span,
    ) -> hir::Expr {
        let Some((id, _)) = zun_natives::resolve("array", "objectRest") else {
            self.error(span, "missing `objectRest` intrinsic");
            return self.poison(span);
        };
        let excluded_elements = excluded
            .iter()
            .map(|key| {
                Some(hir::Expr {
                    kind: hir::ExprKind::String(key.clone()),
                    ty: Ty::String,
                    span,
                })
            })
            .chain(extra.into_iter().map(Some))
            .collect();
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![
                    hir::Expr { kind: hir::ExprKind::Local(temp), ty: temp_ty, span },
                    hir::Expr {
                        kind: hir::ExprKind::ArrayLiteral { elements: excluded_elements },
                        ty: Ty::Array,
                        span,
                    },
                ],
            },
            ty: Ty::DynamicObject,
            span,
        }
    }
}
