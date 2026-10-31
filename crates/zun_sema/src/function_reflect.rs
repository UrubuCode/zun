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
    fn box_function_argument(&mut self, expr: hir::Expr) -> hir::Expr {
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
        match mname {
            "call" => {
                if args.is_empty() {
                    self.error(span, "`call` expects at least a `this` argument");
                    return self.poison(span);
                }
                let mut combined: Vec<hir::Expr> =
                    args.iter().map(|arg| self.check_expr(cx, arg)).collect();
                let return_ty = if is_dynamic {
                    if combined.len() != 1 {
                        self.error(
                            span,
                            "`call` on a dynamic function value expects exactly one `this` \
                             argument (dynamic function values take a single parameter)",
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
                                format!(
                                    "argument type {:?} does not match {expected:?}",
                                    arg.ty
                                ),
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
                if args.len() != 2 {
                    self.error(span, "`apply` expects a `this` argument and an arguments array");
                    return self.poison(span);
                }
                let this_arg = self.check_expr(cx, &args[0]);
                let mut array = self.check_expr(cx, &args[1]);
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
                let args: Vec<hir::Expr> =
                    args.iter().map(|argument| self.check_expr(cx, argument)).collect();
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
