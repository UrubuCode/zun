use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_chain_root(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::ChainRoot { depth, .. } = expr else {
            unreachable!()
        };
        let len = self.chain_roots.len();
        let Some(root) = len
            .checked_sub(1 + *depth as usize)
            .and_then(|index| self.chain_roots.get(index))
            .copied()
        else {
            self.error(span, "optional chain placeholder outside a chain");
            return self.poison(span);
        };
        self.check_expr(cx, &ast::Expr::Ident(root))
    }

    pub(in crate::check) fn check_temp_bind(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::TempBind { value, body, .. } = expr else {
            unreachable!()
        };
        let bound = self.check_expr(cx, value);
        if bound.ty.is_error() {
            return self.poison(span);
        }
        let (slot, tmp) = self.synthetic_local(cx, bound.ty);
        self.chain_roots.push(tmp);
        let result = self.check_expr(cx, body);
        self.chain_roots.pop();
        let ty = result.ty;
        hir::Expr {
            kind: hir::ExprKind::Sequence {
                effect: Box::new(hir::Expr {
                    kind: hir::ExprKind::Assign {
                        slot,
                        value: Box::new(bound),
                    },
                    ty: Ty::Any,
                    span,
                }),
                value: Box::new(result),
            },
            ty,
            span,
        }
    }

    pub(in crate::check) fn check_optional_chain(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::OptionalChain { object, body, .. } = expr else {
            unreachable!()
        };
        let guard = self.check_expr(cx, object);
        if matches!(guard.ty, Ty::Any | Ty::DynamicObject) {
            let (slot, tmp) = self.synthetic_local(cx, Ty::Any);
            let tmp_expr = ast::Expr::Ident(tmp);
            self.chain_roots.push(tmp);
            let result = self.optional_guarded(
                cx,
                slot,
                Self::box_to_any(guard),
                tmp_expr,
                body.as_ref().clone(),
                span,
            );
            self.chain_roots.pop();
            return result;
        }
        // A statically non-nullable guard: no short-circuit possible.
        let (slot, tmp) = self.synthetic_local(cx, guard.ty);
        self.chain_roots.push(tmp);
        let value = self.check_expr(cx, body);
        self.chain_roots.pop();
        let ty = value.ty;
        hir::Expr {
            kind: hir::ExprKind::Sequence {
                effect: Box::new(hir::Expr {
                    kind: hir::ExprKind::Assign {
                        slot,
                        value: Box::new(guard),
                    },
                    ty: Ty::Any,
                    span,
                }),
                value: Box::new(value),
            },
            ty,
            span,
        }
    }

    pub(in crate::check) fn check_optional_field(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::OptionalField { object, field, .. } = expr else {
            unreachable!()
        };
        let object_ast: &ast::Expr = object;
        let object = self.check_expr(cx, object);
        if !matches!(object.ty, Ty::Any | Ty::DynamicObject | Ty::Error) {
            // A statically non-nullable receiver: `?.` is a plain read.
            let read = ast::Expr::Field {
                object: Box::new(object_ast.clone()),
                field: *field,
                span,
            };
            return self.check_expr(cx, &read);
        }
        if object.ty != Ty::Any && !object.ty.is_error() {
            self.error(
                object.span,
                "optional property access requires an any receiver",
            );
        }
        hir::Expr {
            kind: hir::ExprKind::OptionalDynamicGet {
                object: Box::new(object),
                name: self.text(field).to_string(),
            },
            ty: Ty::Any,
            span,
        }
    }

    pub(in crate::check) fn check_optional_index(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::OptionalIndex { object, index, .. } = expr else {
            unreachable!()
        };
        let object_ast: &ast::Expr = object;
        let object = self.check_expr(cx, object);
        if !matches!(object.ty, Ty::Any | Ty::DynamicObject | Ty::Error) {
            let read = ast::Expr::Index {
                object: Box::new(object_ast.clone()),
                index: index.clone(),
                span,
            };
            return self.check_expr(cx, &read);
        }
        let key = self.check_expr(cx, index);
        hir::Expr {
            kind: hir::ExprKind::OptionalDynamicGetValue {
                object: Box::new(object),
                key: Box::new(key),
            },
            ty: Ty::Any,
            span,
        }
    }

    pub(in crate::check) fn check_optional_call(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::OptionalCall { callee, args, .. } = expr else {
            unreachable!()
        };
        // `recv.m?.(args)` on a dynamic receiver: bind the receiver
        // once, then `tmp.m == null ? undefined : tmp.m(args)` (keeps
        // the `this` binding).
        if let ast::Expr::Field {
            object: recv,
            field,
            ..
        } = callee.as_ref()
        {
            let recv_hir = self.check_expr(cx, recv);
            if matches!(recv_hir.ty, Ty::Any | Ty::DynamicObject) {
                let (slot, tmp) = self.synthetic_local(cx, Ty::Any);
                let tmp_expr = ast::Expr::Ident(tmp);
                // The member is read once into a second temp, then invoked with
                // the receiver as `this` (`fn.call(recv, ...args)`).
                let (_, fn_tmp) = self.synthetic_local(cx, Ty::Any);
                let fn_expr = ast::Expr::Ident(fn_tmp);
                let test = ast::Expr::Assign {
                    target: fn_tmp,
                    value: Box::new(ast::Expr::Field {
                        object: Box::new(tmp_expr.clone()),
                        field: *field,
                        span,
                    }),
                    span,
                };
                // A member that reads back nullish may still be a managed
                // method (Map, class instance, Date ...): the `?`-prefixed
                // dynamic call resolves it and yields undefined if absent.
                let probe = ast::synthetic_ident(&format!("?{}", self.text(field)));
                let probe_call = ast::Expr::MethodCall {
                    receiver: Box::new(tmp_expr.clone()),
                    method: probe,
                    args: args.clone(),
                    span,
                };

                let mut call_args = vec![tmp_expr];
                call_args.extend(args.iter().cloned());
                let call = ast::Expr::MethodCall {
                    receiver: Box::new(fn_expr),
                    method: ast::synthetic_ident("call"),
                    args: call_args,
                    span,
                };
                return self.optional_guarded_with(
                    cx,
                    slot,
                    Self::box_to_any(recv_hir),
                    test,
                    call,
                    Some(probe_call),
                    span,
                );
            }
        }
        if let ast::Expr::Index {
            object: recv,
            index: key,
            ..
        } = callee.as_ref()
        {
            let recv_hir = self.check_expr(cx, recv);
            if matches!(recv_hir.ty, Ty::Any | Ty::DynamicObject) {
                let (slot, tmp) = self.synthetic_local(cx, Ty::Any);
                let tmp_expr = ast::Expr::Ident(tmp);
                let test = ast::Expr::Index {
                    object: Box::new(tmp_expr.clone()),
                    index: key.clone(),
                    span,
                };
                let call = ast::Expr::ComputedCall {
                    callee: Box::new(test.clone()),
                    args: args.clone(),
                    span,
                };
                return self.optional_guarded(
                    cx,
                    slot,
                    Self::box_to_any(recv_hir),
                    test,
                    call,
                    span,
                );
            }
        }
        if let ast::Expr::OptionalIndex {
            object: recv,
            index: key,
            ..
        } = callee.as_ref()
        {
            let recv_hir = self.check_expr(cx, recv);
            if matches!(recv_hir.ty, Ty::Any | Ty::DynamicObject) {
                let (slot, tmp) = self.synthetic_local(cx, Ty::Any);
                let tmp_expr = ast::Expr::Ident(tmp);
                let call = ast::Expr::OptionalCall {
                    callee: Box::new(ast::Expr::Index {
                        object: Box::new(tmp_expr.clone()),
                        index: key.clone(),
                        span,
                    }),
                    args: args.clone(),
                    span,
                };
                return self.optional_guarded(
                    cx,
                    slot,
                    Self::box_to_any(recv_hir),
                    tmp_expr,
                    call,
                    span,
                );
            }
        }
        if let ast::Expr::OptionalField {
            object: recv,
            field,
            ..
        } = callee.as_ref()
        {
            // `(recv?.m)?.()` keeps `this = recv`; a nullish `recv`
            // short-circuits to undefined.
            let recv_hir = self.check_expr(cx, recv);
            if matches!(recv_hir.ty, Ty::Any | Ty::DynamicObject) {
                let (slot, tmp) = self.synthetic_local(cx, Ty::Any);
                let tmp_expr = ast::Expr::Ident(tmp);
                let call = ast::Expr::OptionalCall {
                    callee: Box::new(ast::Expr::Field {
                        object: Box::new(tmp_expr.clone()),
                        field: *field,
                        span,
                    }),
                    args: args.clone(),
                    span,
                };
                return self.optional_guarded(
                    cx,
                    slot,
                    Self::box_to_any(recv_hir),
                    tmp_expr,
                    call,
                    span,
                );
            }
        }
        let callee_hir = self.check_expr(cx, callee);
        if matches!(callee_hir.ty, Ty::Any | Ty::DynamicObject) {
            let (slot, tmp) = self.synthetic_local(cx, Ty::Any);
            let tmp_expr = ast::Expr::Ident(tmp);
            let call = ast::Expr::ComputedCall {
                callee: Box::new(tmp_expr.clone()),
                args: args.clone(),
                span,
            };
            return self.optional_guarded(
                cx,
                slot,
                Self::box_to_any(callee_hir),
                tmp_expr,
                call,
                span,
            );
        }
        let callee = callee_hir;
        let func = match callee.ty {
            Ty::Function(func) => func,
            // A dynamic callee: null/undefined short-circuits, otherwise
            // the counted dynamic call applies.
            Ty::Any | Ty::DynamicObject => hir::DYNAMIC_FUNCTION,
            Ty::Error => return self.poison(span),
            _ => {
                self.error(callee.span, "optional call requires a function value");
                return self.poison(span);
            }
        };
        let checked: Vec<hir::Expr> = args.iter().map(|arg| self.check_expr(cx, arg)).collect();
        let return_ty = match func {
            hir::DYNAMIC_FUNCTION | hir::DYNAMIC_REDUCER => Ty::Any,
            hir::DYNAMIC_PREDICATE => Ty::Boolean,
            _ => self.signatures[func as usize].return_ty,
        };
        hir::Expr {
            kind: hir::ExprKind::OptionalIndirectCall {
                callee: Box::new(callee),
                args: checked,
                func,
            },
            ty: return_ty,
            span,
        }
    }

    pub(in crate::check) fn check_optional_method_call(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::OptionalMethodCall {
            object,
            method,
            args,
            ..
        } = expr
        else {
            unreachable!()
        };
        let object_ast: &ast::Expr = object;
        let object = self.check_expr(cx, object);
        let managed = matches!(
            object.ty,
            Ty::Map
                | Ty::Set
                | Ty::WeakMap
                | Ty::WeakSet
                | Ty::WeakRef
                | Ty::RegExp
                | Ty::Promise
                | Ty::Error
        );
        if !managed {
            if matches!(object.ty, Ty::Any | Ty::DynamicObject) {
                // `a?.m(args)` on a dynamic receiver: bind the receiver
                // once, then `tmp == null ? undefined : tmp.m(args)`.
                let (slot, tmp) = self.synthetic_local(cx, Ty::Any);
                let tmp_expr = ast::Expr::Ident(tmp);
                let call = ast::Expr::MethodCall {
                    receiver: Box::new(tmp_expr.clone()),
                    method: *method,
                    args: args.clone(),
                    span,
                };
                let cond = ast::Expr::Conditional {
                    condition: Box::new(ast::Expr::Binary {
                        op: ast::BinaryOp::LooseEq,
                        lhs: Box::new(tmp_expr),
                        rhs: Box::new(ast::Expr::Null { span }),
                        span,
                    }),
                    when_true: Box::new(ast::Expr::Unary {
                        op: ast::UnaryOp::Void,
                        operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                        span,
                    }),
                    when_false: Box::new(call),
                    span,
                };
                let value = self.check_expr(cx, &cond);
                let ty = value.ty;
                let bound = Self::box_to_any(object);
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
            // A statically non-nullable receiver: `?.` is a plain call.
            let call = ast::Expr::MethodCall {
                receiver: Box::new(object_ast.clone()),
                method: *method,
                args: args.clone(),
                span,
            };
            return self.check_expr(cx, &call);
        }
        let name = self.text(method).to_string();
        let (native_name, return_ty, expected) = match (object.ty, name.as_str()) {
            (Ty::Map, "set") => ("mapSetChain", Ty::Map, vec![Ty::Any, Ty::Any]),
            (Ty::Map, "get") => ("mapGet", Ty::Any, vec![Ty::Any]),
            (Ty::Map, "has") => ("mapHas", Ty::Boolean, vec![Ty::Any]),
            (Ty::Map, "size") => ("mapSize", Ty::Int(IntKind::U64), vec![]),
            (Ty::Map, "delete") => ("mapDelete", Ty::Boolean, vec![Ty::Any]),
            (Ty::Map, "clear") => ("mapClear", Ty::Void, vec![]),
            (Ty::Set, "add") => ("setAddChain", Ty::Set, vec![Ty::Any]),
            (Ty::Set, "has") => ("setHas", Ty::Boolean, vec![Ty::Any]),
            (Ty::Set, "size") => ("setSize", Ty::Int(IntKind::U64), vec![]),
            (Ty::Set, "delete") => ("setDelete", Ty::Boolean, vec![Ty::Any]),
            (Ty::Set, "clear") => ("setClear", Ty::Void, vec![]),
            (Ty::WeakMap, "set") => ("weakMapSet", Ty::WeakMap, vec![Ty::Any, Ty::Any]),
            (Ty::WeakMap, "get") => ("weakMapGet", Ty::Any, vec![Ty::Any]),
            (Ty::WeakMap, "has") => ("weakMapHas", Ty::Boolean, vec![Ty::Any]),
            (Ty::WeakMap, "delete") => ("weakMapDelete", Ty::Boolean, vec![Ty::Any]),
            (Ty::WeakSet, "add") => ("weakSetAdd", Ty::WeakSet, vec![Ty::Any]),
            (Ty::WeakSet, "has") => ("weakSetHas", Ty::Boolean, vec![Ty::Any]),
            (Ty::WeakSet, "delete") => ("weakSetDelete", Ty::Boolean, vec![Ty::Any]),
            (Ty::WeakRef, "deref") => ("weakRefDeref", Ty::Any, vec![]),
            (Ty::RegExp, "isMatch") => ("regexpIsMatch", Ty::Boolean, vec![Ty::String]),
            (Ty::RegExp, "test") => ("regexpTest", Ty::Boolean, vec![Ty::String]),
            (Ty::RegExp, "exec") => ("regexpExec", Ty::Array, vec![Ty::String]),
            (Ty::RegExp, "source") => ("regexpSource", Ty::String, vec![]),
            (Ty::Promise, "state") => ("promiseState", Ty::Int(IntKind::U8), vec![]),
            (Ty::Promise, "result") => ("promiseResult", Ty::Any, vec![]),
            (Ty::Promise, "isReady") => ("promiseIsReady", Ty::Boolean, vec![]),
            (Ty::Promise, "resolve") => ("promiseResolve", Ty::Boolean, vec![Ty::Any]),
            (Ty::Promise, "reject") => ("promiseReject", Ty::Boolean, vec![Ty::String]),
            (Ty::Error, _) => return self.poison(span),
            (other, _) => {
                self.error(
                    object.span,
                    format!("optional managed method requires Map, Set, or WeakMap receiver, found {other:?}"),
                );
                return self.poison(span);
            }
        };
        if args.len() != expected.len() {
            self.error(
                span,
                format!(
                    "`{name}` expects {} argument(s), found {}",
                    expected.len(),
                    args.len()
                ),
            );
            return self.poison(span);
        }
        let module = if native_name.starts_with("promise") {
            "promise"
        } else {
            "collections"
        };
        let Some((id, _)) = zun_natives::resolve(module, native_name) else {
            self.error(
                span,
                format!("missing collection intrinsic `{native_name}`"),
            );
            return self.poison(span);
        };
        let mut call_args = vec![object];
        for (arg, expected_ty) in args.iter().zip(expected) {
            let mut value = self.check_expr(cx, arg);
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
            call_args.push(value);
        }
        hir::Expr {
            kind: hir::ExprKind::OptionalCallBuiltin {
                id,
                args: call_args,
            },
            ty: return_ty,
            span,
        }
    }
}
