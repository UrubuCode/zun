use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_assign(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::Assign { target, value, .. } = expr else {
            unreachable!()
        };
        let mut value = self.check_expr(cx, value);
        let text = self.text(target).to_string();
        match cx.lookup(&text) {
            Some(slot) => {
                cx.mutated.insert(slot);
                let slot_ty = cx.locals[slot as usize];
                coerce_numeric_literal(&mut value, slot_ty);
                if !slot_ty.is_error()
                    && !value.ty.is_error()
                    && !self.is_assignable(value.ty, slot_ty)
                {
                    self.error(
                        value.span,
                        format!("cannot assign {:?} to {slot_ty:?}", value.ty),
                    );
                }
                // A dynamic value stored into a typed primitive slot is
                // unboxed so the slot holds the primitive itself.
                value = Self::unbox_to(value, slot_ty);
                let mirror_global = cx.is_module_scope
                    && self.module_globals.contains(&text)
                    && self.globals_read_in_functions.contains(&text)
                    && cx.scopes.iter().find_map(|s| s.get(&text).copied()) == Some(slot)
                    && (slot_ty == Ty::Function(hir::DYNAMIC_FUNCTION)
                        || (!matches!(slot_ty, Ty::Function(_))
                            && !matches!(value.ty, Ty::Function(_))));
                let store = hir::Expr {
                    ty: slot_ty,
                    kind: if cx.boxed.contains(&slot) {
                        hir::ExprKind::CellStore {
                            slot,
                            value: Box::new(value),
                        }
                    } else {
                        hir::ExprKind::Assign {
                            slot,
                            value: Box::new(value),
                        }
                    },
                    span,
                };
                if mirror_global {
                    // Mirror the new value to the realm global store so
                    // function bodies (which read it via `globalGet`)
                    // observe it.
                    let boxed_slot = cx.boxed.contains(&slot);
                    let make_read = || hir::Expr {
                        kind: if boxed_slot {
                            hir::ExprKind::CellLoad { slot }
                        } else {
                            hir::ExprKind::Local(slot)
                        },
                        ty: slot_ty,
                        span,
                    };
                    let read = make_read();
                    let mirror = self.global_set_expr(&text, make_read(), span);
                    return hir::Expr {
                        ty: slot_ty,
                        kind: hir::ExprKind::Sequence {
                            effect: Box::new(hir::Expr {
                                ty: slot_ty,
                                kind: hir::ExprKind::Sequence {
                                    effect: Box::new(store),
                                    value: Box::new(mirror),
                                },
                                span,
                            }),
                            value: Box::new(read),
                        },
                        span,
                    };
                }
                store
            }
            None => {
                if let Some(capture) = cx.captures.get(&text).cloned() {
                    cx.mutated.insert(capture.outer_slot);
                    if capture.mode == hir::CaptureMode::ByValue {
                        self.error(target.span, "cannot assign to a read-only closure capture");
                        return self.poison(span);
                    }
                    let store = hir::Expr {
                        kind: hir::ExprKind::CaptureStore {
                            environment_slot: capture.environment_slot,
                            ty: capture.ty,
                            mode: capture.mode,
                            value: Box::new(value),
                        },
                        ty: capture.ty,
                        span,
                    };
                    // A closure writing a captured module global also updates the
                    // realm store, which module-scope reads consult once any
                    // function body assigns the name.
                    if self.module_globals.contains(&text)
                        && matches!(capture.ty, Ty::Any | Ty::F64 | Ty::Boolean | Ty::String)
                    {
                        let load = || hir::Expr {
                            kind: hir::ExprKind::CaptureLoad {
                                environment_slot: capture.environment_slot,
                                ty: capture.ty,
                                mode: capture.mode,
                            },
                            ty: capture.ty,
                            span,
                        };
                        let set = self.global_set_expr(&text, load(), span);
                        return hir::Expr {
                            kind: hir::ExprKind::Sequence {
                                effect: Box::new(hir::Expr {
                                    kind: hir::ExprKind::Sequence {
                                        effect: Box::new(store),
                                        value: Box::new(set),
                                    },
                                    ty: capture.ty,
                                    span,
                                }),
                                value: Box::new(load()),
                            },
                            ty: capture.ty,
                            span,
                        };
                    }
                    return store;
                }
                // Assignment to a module global (top-level `let`/`const`)
                // stores through `globalSet`.
                if self.module_globals.contains(&text) {
                    return self.global_set_expr(&text, value, span);
                }
                self.error(target.span, format!("unknown variable `{text}`"));
                self.poison(span)
            }
        }
    }

    pub(in crate::check) fn check_index_assign(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::IndexAssign {
            object,
            index,
            value,
            ..
        } = expr
        else {
            unreachable!()
        };
        let object = self.check_expr(cx, object);
        // Computed property store on a dynamic receiver: a Symbol key
        // routes to the symbol-keyed slot, a string/number key to the
        // ordinary dynamic property. The key and value are boxed to `Any`.
        if matches!(object.ty, Ty::Any | Ty::DynamicObject | Ty::Instance(_)) {
            let key = self.check_expr(cx, index);
            self.mark_this_closure(value);
            let value = self.check_expr(cx, value);
            self.closure_takes_this = false;
            let boxed = if value.ty == Ty::Any {
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
                kind: hir::ExprKind::DynamicSetValue {
                    object: Box::new(object),
                    key: Box::new(key),
                    value: Box::new(boxed),
                },
                ty: Ty::Any,
                span,
            };
        }
        if object.ty == Ty::Proxy {
            let key = self.check_expr(cx, index);
            let value = self.check_expr(cx, value);
            return self.proxy_set(object, key, value, span);
        }
        // `view[i] = v` on a byte-buffer view (Buffer / typed array)
        // stores one element via the kind's `set` intrinsic.
        if let Ty::BufferView(kind) = object.ty {
            let mut index = self.check_expr(cx, index);
            coerce_numeric_literal(&mut index, Ty::Int(IntKind::U64));
            if index.ty == Ty::F64 {
                index = self.coerce_array_index(index);
            }
            if !index.ty.is_int() && !index.ty.is_error() {
                self.error(index.span, "buffer index must be an integer");
            }
            if let Some((native, _ret, params)) = crate::typed_arrays::method(kind, "set") {
                // params = [index, element]; the element type is the kind's.
                let elem_ty = params.get(1).copied().unwrap_or(Ty::Int(IntKind::U8));
                let mut value = self.check_expr(cx, value);
                let value_ty = value.ty;
                coerce_numeric_literal(&mut value, elem_ty);
                // A JS `number` stored into an integer element is
                // converted (NaN -> 0, truncation) like a typed-array set.
                if elem_ty.is_int() && value.ty == Ty::F64 {
                    let value_span = value.span;
                    value = hir::Expr {
                        kind: hir::ExprKind::Convert {
                            value: Box::new(value),
                        },
                        ty: elem_ty,
                        span: value_span,
                    };
                }
                if !value.ty.is_error() && !self.is_assignable(value.ty, elem_ty) {
                    self.error(value.span, format!("buffer element must be {elem_ty:?}"));
                }
                if let Some((id, _)) = zun_natives::resolve("buffer", native) {
                    // The store intrinsic yields a bool; the expression's
                    // value is the assigned element (JS assignment result).
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![object, index, value],
                        },
                        ty: if value_ty.is_error() {
                            elem_ty
                        } else {
                            value_ty
                        },
                        span,
                    };
                }
            }
            self.error(span, "missing buffer index intrinsic");
            return self.poison(span);
        }
        let mut index = self.check_expr(cx, index);
        let value = self.check_expr(cx, value);
        let value_ty = value.ty;
        if object.ty == Ty::Array && matches!(index.ty, Ty::Any | Ty::DynamicObject) {
            // A dynamic key follows ToPropertyKey at run time (objects run
            // toString, canonical index strings address elements).
            return hir::Expr {
                kind: hir::ExprKind::DynamicSetValue {
                    object: Box::new(object),
                    key: Box::new(index),
                    value: Box::new(Self::box_to_any(value)),
                },
                ty: Ty::Any,
                span,
            };
        }
        coerce_numeric_literal(&mut index, Ty::INT);
        if object.ty != Ty::Array && !object.ty.is_error() {
            self.error(object.span, "indexing requires an Array");
        }
        let index = self.coerce_array_index(index);
        hir::Expr {
            kind: hir::ExprKind::ArraySet {
                object: Box::new(object),
                index: Box::new(index),
                value: Box::new(value),
            },
            ty: value_ty,
            span,
        }
    }

    pub(in crate::check) fn check_field_assign(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::FieldAssign {
            object,
            field,
            value,
            ..
        } = expr
        else {
            unreachable!()
        };
        if let ast::Expr::PrivateField {
            object: private_object,
            field,
            ..
        } = object.as_ref()
        {
            if let ast::Expr::Ident(class_name) = private_object.as_ref() {
                if cx.lookup(self.text(class_name)).is_none() {
                    if let Some(&class_index) = self.classes_by_name.get(self.text(class_name)) {
                        let internal = format!("#{}#{}", self.text(class_name), self.text(field));
                        if let Some((field_slot, ty)) = self.static_field_of(class_index, &internal)
                        {
                            let mut value = self.check_expr(cx, value);
                            coerce_numeric_literal(&mut value, ty);
                            if !ty.is_error()
                                && !value.ty.is_error()
                                && !self.is_assignable(value.ty, ty)
                            {
                                self.error(
                                    value.span,
                                    format!(
                                        "cannot assign {:?} to private static field of type {ty:?}",
                                        value.ty
                                    ),
                                );
                            }
                            return hir::Expr {
                                kind: hir::ExprKind::StaticFieldSet {
                                    class: class_index,
                                    field_slot,
                                    value: Box::new(value),
                                },
                                ty,
                                span,
                            };
                        }
                    }
                }
            }
            let peeled_object = Box::new(self.peel_this_cast(cx, private_object));
            let private_object = &peeled_object;
            let checked_object = self.check_expr(cx, private_object);
            let private_name = self.text(field).to_string();
            let checked_object = self.assume_private_receiver(cx, checked_object, &private_name);
            let name = self.text(field).to_string();
            let Some(class) = (match checked_object.ty {
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
                if !self.has_private_member(class, &name) {
                    self.error(field.span, format!("unknown private field `#{name}`"));
                    return self.poison(span);
                }
                // A private method or getter-only accessor is not
                // writable: the assignment throws a TypeError.
                if self.method_of(class, &format!("set_{name}")).is_none() {
                    return self.runtime_type_error(cx, span);
                }
                // Private setter (`set #x(v)`): store through the
                // ordinary member path under the `#`-prefixed name.
                let fallback = ast::Expr::FieldAssign {
                    object: private_object.clone(),
                    field: *field,
                    value: value.clone(),
                    span,
                };
                return self.check_expr(cx, &fallback);
            };
            let mut value = self.check_expr(cx, value);
            coerce_numeric_literal(&mut value, ty);
            if !ty.is_error() && !value.ty.is_error() && !self.is_assignable(value.ty, ty) {
                self.error(
                    value.span,
                    format!(
                        "cannot assign {:?} to private field of type {ty:?}",
                        value.ty
                    ),
                );
            }
            return hir::Expr {
                kind: hir::ExprKind::FieldSet {
                    object: Box::new(checked_object),
                    class,
                    field_slot,
                    value: Box::new(value),
                },
                ty,
                span,
            };
        }
        let fname = self.text(field).to_string();
        if let ast::Expr::Ident(class_name) = object.as_ref() {
            if let Some(&class_index) = self.classes_by_name.get(self.text(class_name)) {
                if let Some((field_slot, ty)) = self.static_field_of(class_index, &fname) {
                    let mut value = self.check_expr(cx, value);
                    coerce_numeric_literal(&mut value, ty);
                    if !ty.is_error() && !value.ty.is_error() && !self.is_assignable(value.ty, ty) {
                        self.error(
                            value.span,
                            format!(
                                "cannot assign {:?} to static field of type {ty:?}",
                                value.ty
                            ),
                        );
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::StaticFieldSet {
                            class: self.static_field_owner(class_index, &fname),
                            field_slot,
                            value: Box::new(value),
                        },
                        ty,
                        span,
                    };
                }
            }
        }
        let object = self.check_expr(cx, object);
        self.mark_this_closure(value);
        let mut value = self.check_expr(cx, value);
        self.closure_takes_this = false;
        match object.ty {
            Ty::Instance(class) => match self.field_of(class, &fname) {
                Some((slot, ty)) => {
                    coerce_numeric_literal(&mut value, ty);
                    if !ty.is_error() && !value.ty.is_error() && !self.is_assignable(value.ty, ty) {
                        self.error(
                            value.span,
                            format!("cannot assign {:?} to field of type {ty:?}", value.ty),
                        );
                    }
                    hir::Expr {
                        kind: hir::ExprKind::FieldSet {
                            object: Box::new(object),
                            class,
                            field_slot: slot,
                            value: Box::new(value),
                        },
                        ty,
                        span,
                    }
                }
                None => {
                    let setter = format!("set_{fname}");
                    let Some(func) = self.method_of(class, &setter) else {
                        self.error(
                            field.span,
                            format!("no field or setter `{fname}` on this type"),
                        );
                        return self.poison(span);
                    };
                    let (param_count, expected, return_ty) = {
                        let sig = &self.signatures[func as usize];
                        (
                            sig.params.len(),
                            sig.params.get(1).copied().unwrap_or(Ty::Error),
                            sig.return_ty,
                        )
                    };
                    if param_count != 2 {
                        self.error(
                            field.span,
                            format!("setter `{setter}` must take one argument"),
                        );
                        return self.poison(span);
                    }
                    coerce_numeric_literal(&mut value, expected);
                    if !expected.is_error()
                        && !value.ty.is_error()
                        && !self.is_assignable(value.ty, expected)
                    {
                        self.error(
                            value.span,
                            format!(
                                "cannot assign {:?} to setter argument of type {expected:?}",
                                value.ty
                            ),
                        );
                    }
                    if return_ty == Ty::Void && !value.ty.is_error() && value.ty != Ty::Void {
                        // An assignment expression evaluates to the
                        // assigned value, not the setter's result.
                        let value_ty = value.ty;
                        let temp = cx.declare(&self.fresh_temp_name(cx), value_ty);
                        let store = hir::Expr {
                            kind: hir::ExprKind::Assign {
                                slot: temp,
                                value: Box::new(value),
                            },
                            ty: value_ty,
                            span,
                        };
                        let call = hir::Expr {
                            kind: hir::ExprKind::MethodCall {
                                func,
                                args: vec![object, store],
                            },
                            ty: Ty::Void,
                            span,
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::Sequence {
                                effect: Box::new(call),
                                value: Box::new(hir::Expr {
                                    kind: hir::ExprKind::Local(temp),
                                    ty: value_ty,
                                    span,
                                }),
                            },
                            ty: value_ty,
                            span,
                        };
                    }
                    hir::Expr {
                        kind: hir::ExprKind::MethodCall {
                            func,
                            args: vec![object, value],
                        },
                        ty: return_ty,
                        span,
                    }
                }
            },
            Ty::DynamicObject | Ty::Any => {
                let boxed = if value.ty == Ty::Any {
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
                hir::Expr {
                    kind: hir::ExprKind::DynamicSet {
                        object: Box::new(object),
                        name: fname,
                        value: Box::new(boxed),
                    },
                    ty: Ty::Any,
                    span,
                }
            }
            // `arr.length = n` (truncation; growth is not supported):
            // lowered to a clamped splice removing the tail.
            Ty::Array if fname == "length" => {
                // Shrinks or grows (holes) through the dynamic length setter.
                let boxed = if value.ty == Ty::Any {
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
                hir::Expr {
                    kind: hir::ExprKind::DynamicSet {
                        object: Box::new(object),
                        name: fname,
                        value: Box::new(boxed),
                    },
                    ty: Ty::Any,
                    span,
                }
            }
            // `re.lastIndex = v`: the regex's own writable data slot.
            Ty::RegExp if fname == "lastIndex" => {
                let Some((id, _)) = zun_natives::resolve("collections", "regexpSetLastIndex")
                else {
                    self.error(span, "missing RegExp intrinsic");
                    return self.poison(span);
                };
                let held = Self::box_to_any(value);
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![object, held],
                    },
                    ty: Ty::Any,
                    span,
                }
            }
            // `proxy.key = v` routes through the handler's `set` trap.
            Ty::Proxy => {
                let key = hir::Expr {
                    kind: hir::ExprKind::String(fname),
                    ty: Ty::String,
                    span,
                };
                self.proxy_set(object, key, value, span)
            }
            Ty::Error => self.poison(span),
            Ty::Function(_) => {
                let boxed = if value.ty == Ty::Any || matches!(value.ty, Ty::Function(_)) {
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
                hir::Expr {
                    kind: hir::ExprKind::DynamicSet {
                        object: Box::new(object),
                        name: fname,
                        value: Box::new(boxed),
                    },
                    ty: Ty::Any,
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
