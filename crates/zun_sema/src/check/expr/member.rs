use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_static_field(
        &mut self,
        _cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::StaticField { class, field, .. } = expr else {
            unreachable!()
        };
        let class_name = self.text(class).to_string();
        let Some(&class_index) = self.classes_by_name.get(&class_name) else {
            self.error(class.span, format!("unknown class `{class_name}`"));
            return self.poison(span);
        };
        let Some((field_slot, ty)) = self.static_field_of(class_index, self.text(field)) else {
            self.error(
                field.span,
                format!("unknown static field `{}`", self.text(field)),
            );
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::StaticFieldGet {
                class: self.static_field_owner(class_index, self.text(field)),
                field_slot,
            },
            ty,
            span,
        }
    }

    pub(in crate::check) fn check_private_field(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::PrivateField { object, field, .. } = expr else {
            unreachable!()
        };
        let object = &Box::new(self.peel_this_cast(cx, object));
        let ast_object = object.clone();
        // `Class.#static`: a private static field is an ordinary static slot
        // under its `#`-prefixed name.
        if let ast::Expr::Ident(class_name) = object.as_ref() {
            if cx.lookup(self.text(class_name)).is_none() {
                if let Some(&class_index) = self.classes_by_name.get(self.text(class_name)) {
                    let internal = format!("#{}#{}", self.text(class_name), self.text(field));
                    if let Some((field_slot, ty)) = self.static_field_of(class_index, &internal) {
                        return hir::Expr {
                            kind: hir::ExprKind::StaticFieldGet {
                                class: class_index,
                                field_slot,
                            },
                            ty,
                            span,
                        };
                    }
                }
            }
        }
        let object = self.check_expr(cx, object);
        let private_name = self.text(field).to_string();
        let object = self.assume_private_receiver(cx, object, &private_name);
        let name = self.text(field).to_string();
        // Private accessor (`get #x()`): not a slot, so read it through
        // the ordinary member path under its `#`-prefixed name.
        if let Ty::Instance(class) = object.ty {
            if self.private_field_of(class, &name).is_none()
                && self.has_private_member(class, &name)
            {
                let fallback = ast::Expr::Field {
                    object: ast_object,
                    field: field.clone(),
                    span,
                };
                return self.check_expr(cx, &fallback);
            }
        }
        let Some(class) = (match object.ty {
            Ty::Instance(class) => Some(class),
            _ => None,
        }) else {
            self.error(span, "private fields require an instance receiver");
            return self.poison(span);
        };
        if cx.this_class != Some(class) {
            self.error(
                field.span,
                "private field is only accessible inside its declaring class",
            );
            return self.poison(span);
        }
        let Some((field_slot, ty)) = self.private_field_of(class, &name) else {
            self.error(field.span, format!("unknown private field `#{name}`"));
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::FieldGet {
                object: Box::new(object),
                class,
                field_slot,
            },
            ty,
            span,
        }
    }

    pub(in crate::check) fn check_field(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::Field { object, field, .. } = expr else {
            unreachable!()
        };
        let fname = self.text(field).to_string();
        // Built-in metadata reads folded before the receiver is checked
        // (`Function.prototype.<m>`, `<Builtin>.<fn>.length`/`.name`, ...).
        if let Some(expr) = self.field_builtin_meta(cx, object.as_ref(), &fname, span) {
            return expr;
        }
        // Reads whose receiver is a bare identifier naming a built-in or a
        // user class/enum (`Enum.Member`, `Class.staticField`, `Math.PI`, ...).
        if let ast::Expr::Ident(class_name) = object.as_ref() {
            if let Some(expr) = self.field_static_ident(cx, class_name, &fname, field, span) {
                return expr;
            }
        }
        let object = self.check_expr(cx, object);
        // Special property reads keyed on the checked receiver's type
        // (`.constructor`, `symbol.description`, `.length`/`.size`).
        let object = match self.field_special_props(object, &fname, span) {
            Ok(expr) => return expr,
            Err(object) => object,
        };
        // `Function` value metadata (`prototype`, `name`, `length`).
        let object = match self.field_function_value(object, &fname, span) {
            Ok(expr) => return expr,
            Err(object) => object,
        };
        // The final receiver-type dispatch (instance fields/getters, dynamic
        // objects, proxies, expandos, `RegExp` property reads).
        self.field_dispatch(object, fname, field, span)
    }

    pub(in crate::check) fn check_index(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::Index { object, index, .. } = expr else {
            unreachable!()
        };
        // `Symbol[key]` with a runtime key: pick the well-known symbol by name.
        if matches!(object.as_ref(), ast::Expr::Ident(id) if self.text(id) == "Symbol" && cx.lookup("Symbol").is_none())
            && !matches!(index.as_ref(), ast::Expr::String { .. })
        {
            let key_hir = self.check_expr(cx, index);
            if !key_hir.ty.is_error() {
                let key_hir = self.to_string_for_parse(key_hir);
                let (slot, tmp) = self.synthetic_local(cx, Ty::String);
                let mut chain = ast::Expr::Unary {
                    op: ast::UnaryOp::Void,
                    operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                    span,
                };
                for well_known in [
                    "iterator",
                    "asyncIterator",
                    "hasInstance",
                    "toPrimitive",
                    "toStringTag",
                ] {
                    chain = ast::Expr::Conditional {
                        condition: Box::new(ast::Expr::Binary {
                            op: ast::BinaryOp::Eq,
                            lhs: Box::new(ast::Expr::Ident(tmp)),
                            rhs: Box::new(ast::Expr::String {
                                value: well_known.to_string(),
                                span,
                            }),
                            span,
                        }),
                        when_true: Box::new(ast::Expr::Field {
                            object: object.clone(),
                            field: ast::synthetic_ident(well_known),
                            span,
                        }),
                        when_false: Box::new(chain),
                        span,
                    };
                }
                let body = self.check_expr(cx, &chain);
                let ty = body.ty;
                return hir::Expr {
                    kind: hir::ExprKind::Sequence {
                        effect: Box::new(hir::Expr {
                            kind: hir::ExprKind::Assign {
                                slot,
                                value: Box::new(key_hir),
                            },
                            ty: Ty::String,
                            span,
                        }),
                        value: Box::new(body),
                    },
                    ty: if ty == Ty::Symbol { Ty::Any } else { ty },
                    span,
                };
            }
        }
        let ast_object = object.clone();
        let object = self.check_expr(cx, object);
        // `re[Symbol.replace]` and friends read the shared RegExp.prototype slot.
        let object = if object.ty == Ty::RegExp && self.well_known_symbol_of_key(index).is_some() {
            match zun_natives::resolve("collections", "regexpPrototype") {
                Some((id, _)) => hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: Vec::new(),
                    },
                    ty: Ty::Any,
                    span,
                },
                None => object,
            }
        } else {
            object
        };
        // `str[i]`: the one-character string at `i` (a negative index is out of
        // range, unlike `at`), or `undefined` past the end.
        if object.ty == Ty::String {
            let at = ast::Expr::MethodCall {
                receiver: ast_object,
                method: ast::synthetic_ident("at"),
                args: vec![index.as_ref().clone()],
                span,
            };
            return self.check_expr(cx, &at);
        }
        if let ast::Expr::String { value: key, .. } = index.as_ref() {
            if let Ty::Instance(class) = object.ty {
                if let Some((field_slot, ty)) = self.field_of(class, key) {
                    return hir::Expr {
                        kind: hir::ExprKind::FieldGet {
                            object: Box::new(object),
                            class,
                            field_slot,
                        },
                        ty,
                        span,
                    };
                }
            }
        }
        if matches!(object.ty, Ty::Any | Ty::DynamicObject | Ty::Instance(_)) {
            let key = self.check_expr(cx, index);
            return hir::Expr {
                kind: hir::ExprKind::DynamicGetValue {
                    object: Box::new(object),
                    key: Box::new(key),
                },
                ty: Ty::Any,
                span,
            };
        }
        if object.ty == Ty::Proxy {
            let key = self.check_expr(cx, index);
            return self.proxy_get(object, key, span);
        }
        // `view[i]` on a byte-buffer view (a Buffer / typed array) reads
        // one element, routed to the kind's `get` intrinsic.
        if let Ty::BufferView(kind) = object.ty {
            let mut index = self.check_expr(cx, index);
            coerce_numeric_literal(&mut index, Ty::Int(IntKind::U64));
            if index.ty == Ty::F64 {
                index = self.coerce_array_index(index);
            }
            if !index.ty.is_int() && !index.ty.is_error() {
                self.error(index.span, "buffer index must be an integer");
            }
            if let Some((native, ret, _)) = crate::typed_arrays::method(kind, "get") {
                if let Some((id, _)) = zun_natives::resolve("buffer", native) {
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![object, index],
                        },
                        ty: ret,
                        span,
                    };
                }
            }
            self.error(span, "missing buffer index intrinsic");
            return self.poison(span);
        }
        let mut index = self.check_expr(cx, index);
        coerce_numeric_literal(&mut index, Ty::INT);
        if object.ty == Ty::Array && matches!(index.ty, Ty::Any | Ty::DynamicObject) {
            return hir::Expr {
                kind: hir::ExprKind::DynamicGetValue {
                    object: Box::new(object),
                    key: Box::new(index),
                },
                ty: Ty::Any,
                span,
            };
        }
        if object.ty != Ty::Array && !object.ty.is_error() {
            self.error(object.span, "indexing requires an Array");
        }
        let index = self.coerce_array_index(index);
        hir::Expr {
            kind: hir::ExprKind::ArrayGet {
                object: Box::new(object),
                index: Box::new(index),
            },
            ty: Ty::Any,
            span,
        }
    }
}
