//! `Function.prototype.call`/`apply`/`bind` recognition on a `Ty::Function`
//! receiver, lowered to runtime helpers.
//!
//! Zun threads a method's receiver (`this`) as the leading positional
//! parameter slot, so these reflective invokers reduce to ordinary positional
//! argument manipulation:
//!
//! - `f.call(thisArg, ...args)` is exactly `f(thisArg, ...args)` — it reuses the
//!   indirect-call path with `thisArg` prepended as the leading argument.
//! - `f.apply(thisArg, argsArray)` prepends `thisArg` and spreads the array via
//!   the `functionApply` runtime helper.
//! - `f.bind(thisArg, ...partial)` allocates a first-class bound function value
//!   whose prepended arguments are `[thisArg, ...partial]` via `functionBind`.
//!
//! A plain top-level function has no dedicated receiver slot: its `thisArg` is
//! simply bound to its first declared parameter. A function that declares no
//! parameters ignores `thisArg` (the argument-count check rejects passing one),
//! matching the "`this` is the leading parameter" model.

use super::*;

impl<'a> Checker<'a> {
    /// Wrap a function-valued expression in the `functionBox` intrinsic so it
    /// can travel through an `Any`-boxing argument lane (functions deliberately
    /// do not box into `Any`). A non-function expression is returned unchanged.
    pub(super) fn box_function_argument(&mut self, expr: hir::Expr) -> hir::Expr {
        if !matches!(expr.ty, Ty::Function(_)) {
            return expr;
        }
        let Some((id, _)) = zun_natives::resolve("collections", "functionBox") else {
            self.error(expr.span, "missing `functionBox` intrinsic");
            return expr;
        };
        let span = expr.span;
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![expr],
            },
            // The wrapper is an opaque dynamic object; it is only ever unwrapped
            // again on the bound-call path, never observed as a value.
            ty: Ty::Any,
            span,
        }
    }

    /// Argument-marshalling site for `apply` / `Reflect.apply`: when the
    /// arguments array is a literal, wrap each function-valued element in a
    /// `functionBox` so it survives `StoreArray`'s `Any` boxing. The unwrap is
    /// scoped to the `functionApply` native (not to every array read), so a
    /// function stored in a generic array elsewhere is still not observable.
    pub(super) fn box_function_array_elements(&mut self, array: &mut hir::Expr) {
        if let hir::ExprKind::ArrayLiteral { elements } = &mut array.kind {
            for slot in elements.iter_mut() {
                if let Some(element) = slot.take() {
                    *slot = Some(self.box_function_argument(element));
                }
            }
        }
    }

    /// `f.call(...)` / `f.apply(...)` lowered as the dynamic member call on `f`
    /// boxed into an `any` temp, so the runtime's receiver handling decides what
    /// `thisArg` binds to.
    fn lower_dynamic_reflective(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        method: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let (slot, tmp) = self.synthetic_local(cx, Ty::Any);
        let boxed = self.box_function_argument(receiver);
        let dynamic_call = ast::Expr::MethodCall {
            receiver: Box::new(ast::Expr::Ident(tmp)),
            method: ast::synthetic_ident(method),
            args: args.to_vec(),
            span,
        };
        let call = self.check_expr(cx, &dynamic_call);
        hir::Expr {
            kind: hir::ExprKind::Sequence {
                effect: Box::new(hir::Expr {
                    kind: hir::ExprKind::Assign {
                        slot,
                        value: Box::new(boxed),
                    },
                    ty: Ty::Any,
                    span,
                }),
                value: Box::new(call),
            },
            ty: Ty::Any,
            span,
        }
    }

    #[allow(clippy::too_many_arguments)] // mirrors the method-call lowering context
    pub(super) fn lower_function_reflective_call(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        func: hir::FuncIndex,
        args: &[ast::Expr],
        span: Span,
        method_span: Span,
    ) -> hir::Expr {
        let is_dynamic = matches!(
            func,
            hir::DYNAMIC_FUNCTION | hir::DYNAMIC_PREDICATE | hir::DYNAMIC_REDUCER
        );
        // A free function with no `this` parameter has no receiver slot to bind:
        // `f.call(thisArg, a, b)` runs `f(a, b)` and ignores `thisArg`.
        let free_function = !is_dynamic
            && !self.receiver_funcs.contains(&func)
            && !self.signatures[func as usize].is_static
            && self.by_name.get(&self.signatures[func as usize].name) == Some(&func);
        match mname {
            "call" => {
                if args.is_empty() {
                    self.error(span, "`call` expects at least a `this` argument");
                    return self.poison(span);
                }
                // A dynamic function value (a bound function included), a free
                // function with no receiver slot, or a target with a rest
                // parameter is called through the dynamic `call` path, which owns
                // the receiver and rest handling, instead of positionally.
                if func == hir::DYNAMIC_FUNCTION
                    || free_function
                    || (!is_dynamic && self.signatures[func as usize].rest_param.is_some())
                {
                    return self.lower_dynamic_reflective(cx, receiver, "call", args, span);
                }
                // A method borrowed onto a receiver its class does not describe
                // (`w.kindOfThis.call(42)`, `acct.show.call({ label: "x" })`):
                // JS binds any value as `this`, so the call cannot be the
                // positional one, whose receiver slot is typed as the class.
                // The receiver is evaluated once into a temporary and the call
                // goes through the dynamic `call` path, which owns the binding.
                let mut this_checked = Some(self.check_expr(cx, &args[0]));
                if !is_dynamic {
                    let receiver_ty = self.signatures[func as usize].params.first().copied();
                    let this_ty = this_checked.as_ref().map(|this| this.ty).unwrap_or(Ty::Error);
                    if let Some(receiver_ty @ Ty::Instance(_)) = receiver_ty {
                        if !self.signatures[func as usize].is_static
                            && !this_ty.is_error()
                            && !self.is_assignable(this_ty, receiver_ty)
                        {
                            let this_value = this_checked.take().expect("checked above");
                            let (slot, tmp) = self.synthetic_local(cx, this_ty);
                            let mut dynamic_args = args.to_vec();
                            dynamic_args[0] = ast::Expr::Ident(tmp);
                            let call = self.lower_dynamic_reflective(
                                cx,
                                receiver,
                                "call",
                                &dynamic_args,
                                span,
                            );
                            return hir::Expr {
                                kind: hir::ExprKind::Sequence {
                                    effect: Box::new(hir::Expr {
                                        kind: hir::ExprKind::Assign {
                                            slot,
                                            value: Box::new(this_value),
                                        },
                                        ty: this_ty,
                                        span,
                                    }),
                                    value: Box::new(call),
                                },
                                ty: Ty::Any,
                                span,
                            };
                        }
                    }
                }
                let mut combined: Vec<hir::Expr> = Vec::with_capacity(args.len());
                combined.extend(this_checked);
                for arg in &args[1..] {
                    combined.push(self.check_expr(cx, arg));
                }
                let return_ty = if is_dynamic {
                    // A plain dynamic function value accepts any argument count, like
                    // a direct call to it; only the fixed-shape predicate and reducer
                    // callbacks keep their single-argument shape.
                    if func != hir::DYNAMIC_FUNCTION && combined.len() != 1 {
                        self.error(
                            span,
                            "`call` on a dynamic callback value expects exactly one `this` \
                             argument",
                        );
                    }
                    match func {
                        hir::DYNAMIC_PREDICATE => Ty::Boolean,
                        _ => Ty::Any,
                    }
                } else {
                    let (params, ret) = {
                        let signature = &self.signatures[func as usize];
                        (signature.params.clone(), signature.return_ty)
                    };
                    // Omitted trailing arguments read as `undefined`; only a parameter
                    // that can hold `undefined` (`any`) or has a default may be omitted.
                    let required = params.len().saturating_sub(self.signatures[func as usize].optional_count);
                    if combined.len() < params.len()
                        && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
                        && params[combined.len()..]
                            .iter()
                            .enumerate()
                            .all(|(offset, ty)| *ty == Ty::Any || combined.len() + offset >= required)
                    {
                        while combined.len() < params.len() {
                            let ty = params[combined.len()];
                            combined.push(default_args::missing_arg(ty, span));
                        }
                    }
                    if combined.len() != params.len() {
                        self.error(
                            span,
                            format!(
                                "`call` passes {} argument(s) including `this`, but the function \
                                 takes {}",
                                combined.len(),
                                params.len()
                            ),
                        );
                    }
                    for (arg, &expected) in combined.iter_mut().zip(&params) {
                        coerce_numeric_literal(arg, expected);
                        if !expected.is_error()
                            && !arg.ty.is_error()
                            && !self.is_assignable(arg.ty, expected)
                        {
                            self.error(
                                arg.span,
                                format!("argument type {:?} does not match {expected:?}", arg.ty),
                            );
                        }
                    }
                    ret
                };
                hir::Expr {
                    kind: hir::ExprKind::IndirectCall {
                        callee: Box::new(receiver),
                        args: combined,
                        func,
                    },
                    ty: return_ty,
                    span,
                }
            }
            "apply" => {
                // A free function has no receiver slot for the positional `apply`
                // lowering to bind, so it runs through the dynamic `apply` path.
                if free_function {
                    return self.lower_dynamic_reflective(cx, receiver, "apply", args, span);
                }
                if args.len() > 2 {
                    self.error(
                        span,
                        "`apply` expects a `this` argument and an arguments array",
                    );
                    return self.poison(span);
                }
                // `f.apply()` and `f.apply(thisArg)` are legal: an omitted `this`
                // or arguments list is `undefined`, which applies no arguments.
                let this_arg = match args.first() {
                    Some(arg) => self.check_expr(cx, arg),
                    None => default_args::missing_arg(Ty::Any, span),
                };
                let array = match args.get(1) {
                    Some(arg) => self.check_expr(cx, arg),
                    None => default_args::missing_arg(Ty::Any, span),
                };
                // A nullish argument list (`f.apply(x, null)`) applies none.
                let array = if array.ty != Ty::Array && !array.ty.is_error() {
                    match self.by_name.get("__zunApplyList") {
                        Some(&helper) => hir::Expr {
                            kind: hir::ExprKind::Call {
                                func: helper,
                                args: vec![Self::box_to_any(array)],
                            },
                            ty: Ty::Any,
                            span,
                        },
                        None => array,
                    }
                } else {
                    array
                };
                let mut array = self.array_like_to_array(array);
                self.box_function_array_elements(&mut array);
                if array.ty != Ty::Array && !array.ty.is_error() {
                    self.error(array.span, "`apply` expects an Array of arguments");
                    return self.poison(span);
                }
                let Some((id, _)) = zun_natives::resolve("collections", "functionApply") else {
                    self.error(span, "missing `functionApply` intrinsic");
                    return self.poison(span);
                };
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![receiver, this_arg, array],
                    },
                    ty: Ty::Any,
                    span,
                }
            }
            "bind" => {
                if args.is_empty() {
                    self.error(span, "`bind` expects at least a `this` argument");
                    return self.poison(span);
                }
                let prepended: Vec<hir::Expr> = args
                    .iter()
                    .map(|arg| {
                        let checked = self.check_expr(cx, arg);
                        // A function-valued partial argument cannot be boxed
                        // into the `Any`-typed `__args` lane; wrap it in a
                        // GC-traced `functionBox` object that `resolve_call_target`
                        // unwraps back to the callable before dispatch.
                        self.box_function_argument(checked)
                    })
                    .collect();
                let Some((id, _)) = zun_natives::resolve("collections", "functionBind") else {
                    self.error(span, "missing `functionBind` intrinsic");
                    return self.poison(span);
                };
                hir::Expr {
                    kind: hir::ExprKind::FunctionBind {
                        callee: Box::new(receiver),
                        args: prepended,
                        native_id: id,
                    },
                    // A bound function is a first-class dynamic callable.
                    ty: Ty::Function(hir::DYNAMIC_FUNCTION),
                    span,
                }
            }
            // Any other name reads an expando property off the function
            // object and invokes it through the dynamic-call form.
            other => {
                let callee = hir::Expr {
                    kind: hir::ExprKind::DynamicGet {
                        object: Box::new(receiver),
                        name: other.to_string(),
                    },
                    ty: Ty::Any,
                    span: method_span,
                };
                let args: Vec<hir::Expr> = args
                    .iter()
                    .map(|argument| self.check_expr(cx, argument))
                    .collect();
                hir::Expr {
                    kind: hir::ExprKind::IndirectCall {
                        callee: Box::new(callee),
                        args,
                        func: hir::DYNAMIC_FUNCTION,
                    },
                    ty: Ty::Any,
                    span,
                }
            }
        }
    }
}
