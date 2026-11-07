//! Indirect calls through a local, captured, or global function value for
//! `check_call` (see `call.rs`).
//!
//! When the callee name resolves to a value in scope (a local slot, a boxed
//! cell, a closure capture, or a global-read function), the call is dispatched
//! indirectly. `try_call_local_indirect` handles the `any` and proxy forms
//! inline and delegates a statically typed function value to
//! `call_function_value`. Order and semantics are preserved exactly.

use crate::check::*;

impl Checker<'_> {
    /// Reconstruct a callee-load `ExprKind` from a resolved prototype (a
    /// `Local`, `CellLoad`, or `CaptureLoad`), so it can be used in several
    /// positions of one lowering.
    fn clone_callee_kind(proto: &hir::ExprKind) -> hir::ExprKind {
        match proto {
            hir::ExprKind::CaptureLoad {
                environment_slot,
                ty,
                mode,
            } => hir::ExprKind::CaptureLoad {
                environment_slot: *environment_slot,
                ty: *ty,
                mode: *mode,
            },
            hir::ExprKind::Local(slot) => hir::ExprKind::Local(*slot),
            hir::ExprKind::CellLoad { slot } => hir::ExprKind::CellLoad { slot: *slot },
            _ => unreachable!(),
        }
    }

    /// Resolve the callee name to an in-scope value and dispatch indirectly:
    /// a global-read closure, an `any`-typed callee, a proxy `apply`, or a
    /// statically typed function value. `None` falls through.
    pub(in crate::check) fn try_call_local_indirect(
        &mut self,
        cx: &mut FunctionCx,
        name: &str,
        args: &mut Vec<hir::Expr>,
        span: Span,
    ) -> Option<hir::Expr> {
        // A local shadows a capture of the same name. Inside a closure a captured
        // callable is read from the environment instead of a local slot.
        if let Some(slot) = cx.lookup(name) {
            if cx.locals[slot as usize] == Ty::Function(hir::DYNAMIC_FUNCTION)
                && self.reads_through_global(cx, name, slot)
            {
                let callee = self.global_get_expr(name, span);
                return Some(hir::Expr {
                    kind: hir::ExprKind::IndirectCall {
                        callee: Box::new(callee),
                        args: std::mem::take(args),
                        func: hir::DYNAMIC_FUNCTION,
                    },
                    ty: Ty::Any,
                    span,
                });
            }
        }
        let callee_source: Option<(hir::ExprKind, Ty)> = match cx.lookup(name) {
            Some(slot) if cx.boxed.contains(&slot) => {
                Some((hir::ExprKind::CellLoad { slot }, cx.locals[slot as usize]))
            }
            Some(slot) => Some((hir::ExprKind::Local(slot), cx.locals[slot as usize])),
            None => cx.captures.get(name).map(|capture| {
                (
                    hir::ExprKind::CaptureLoad {
                        environment_slot: capture.environment_slot,
                        ty: capture.ty,
                        mode: capture.mode,
                    },
                    capture.ty,
                )
            }),
        };
        if let Some((callee_proto, function_ty)) = callee_source {
            // Calling an `any`-typed local performs a dynamic dispatch:
            // the value must be callable at run time (e.g. a function
            // read from an object's function slot via destructuring, as
            // `const { resolve } = Promise.withResolvers()` produces).
            // Lowered like a generic `Function` callback (single arg).
            if function_ty == Ty::Any {
                // A dynamic callee may be invoked with any argument count
                // (JS); the counted indirect-call form carries the arity.
                let callee_expr = hir::Expr {
                    kind: Self::clone_callee_kind(&callee_proto),
                    ty: function_ty,
                    span,
                };
                return Some(hir::Expr {
                    kind: hir::ExprKind::IndirectCall {
                        callee: Box::new(callee_expr),
                        args: std::mem::take(args),
                        func: hir::DYNAMIC_FUNCTION,
                    },
                    ty: Ty::Any,
                    span,
                });
            }
            // Calling a proxy value: `proxy(...args)` routes through the
            // handler's `apply` trap (NUCLEUS #10). The arguments are
            // packed into an array (each boxed to `Any` by `StoreArray`)
            // and dispatched to `proxyApply` with an `undefined` `this`.
            if function_ty == Ty::Proxy {
                let callee_expr = hir::Expr {
                    kind: Self::clone_callee_kind(&callee_proto),
                    ty: function_ty,
                    span,
                };
                let args_array = hir::Expr {
                    kind: hir::ExprKind::ArrayLiteral {
                        elements: std::mem::take(args).into_iter().map(Some).collect(),
                    },
                    ty: Ty::Array,
                    span,
                };
                let this_arg = hir::Expr {
                    kind: hir::ExprKind::Null,
                    ty: Ty::Any,
                    span,
                };
                let Some((id, _)) = zun_natives::resolve("collections", "proxyApply") else {
                    self.error(span, "missing `proxyApply` intrinsic");
                    return Some(self.poison(span));
                };
                return Some(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![callee_expr, this_arg, args_array],
                    },
                    ty: Ty::Any,
                    span,
                });
            }
            if let Ty::Function(func) = function_ty {
                return Some(self.call_function_value(
                    callee_proto,
                    function_ty,
                    func,
                    std::mem::take(args),
                    span,
                ));
            }
        }
        None
    }

    /// Lower a call to a statically typed function value: handle the generic
    /// `Function`/`Predicate`/`Reducer` arities, then a concrete signature's
    /// rest-parameter or fixed-parameter argument checking.
    #[allow(clippy::needless_range_loop)] // index couples argument and parameter lists
    fn call_function_value(
        &mut self,
        callee_proto: hir::ExprKind,
        function_ty: Ty,
        func: hir::FuncIndex,
        mut args: Vec<hir::Expr>,
        span: Span,
    ) -> hir::Expr {
        let return_ty = if func == hir::DYNAMIC_FUNCTION {
            // Any arity: a bound function (`f.bind(..)`) forwards N call
            // arguments (lowered to `call.indirect.n` when N != 1).
            Ty::Any
        } else if func == hir::DYNAMIC_PREDICATE {
            if args.len() != 1 {
                self.error(
                    span,
                    format!(
                        "generic Predicate callback expects exactly one argument, found {}",
                        args.len()
                    ),
                );
            }
            Ty::Boolean
        } else if func == hir::DYNAMIC_REDUCER {
            if args.len() != 2 {
                self.error(
                    span,
                    format!(
                        "generic Reducer callback expects exactly two arguments, found {}",
                        args.len()
                    ),
                );
            }
            Ty::Any
        } else {
            let (param_tys, return_ty, optional_count, signature_name) = {
                let sig = &self.signatures[func as usize];
                (
                    sig.params.clone(),
                    sig.return_ty,
                    sig.optional_count,
                    sig.name.clone(),
                )
            };
            let sig = Signature {
                name: signature_name,
                params: param_tys.clone(),
                return_ty,
                optional_count,
                rest_param: self.signatures[func as usize].rest_param,
                is_generator: self.signatures[func as usize].is_generator,
                is_static: false,
            };
            if let Some(rest_index) = sig.rest_param {
                if args.len() < rest_index {
                    self.error(
                        span,
                        format!(
                            "function value expects at least {} argument(s), found {}",
                            rest_index,
                            args.len()
                        ),
                    );
                }
                for (arg, &expected) in args.iter_mut().zip(&param_tys).take(rest_index) {
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
                // Function values packed into the rest array ride in functionBox records.
                let mut args = args;
                for index in rest_index..args.len() {
                    if matches!(args[index].ty, Ty::Function(_)) {
                        let element = std::mem::replace(&mut args[index], self.poison(span));
                        args[index] = self.box_function_argument(element);
                    }
                }
                return hir::Expr {
                    kind: hir::ExprKind::IndirectCallRest {
                        callee: Box::new(hir::Expr {
                            kind: Self::clone_callee_kind(&callee_proto),
                            ty: function_ty,
                            span,
                        }),
                        args,
                        func,
                    },
                    ty: return_ty,
                    span,
                };
            }
            // JS allows calling with fewer arguments: missing ones read
            // as `undefined`.
            let sig = Signature {
                optional_count: param_tys.len(),
                ..sig
            };
            if args.len() > param_tys.len() {
                self.error(
                    span,
                    format!(
                        "function value expects {} to {} argument(s), found {}",
                        param_tys.len().saturating_sub(optional_count),
                        param_tys.len(),
                        args.len()
                    ),
                );
            } else {
                self.pad_default_args(&mut args, &sig, span);
                for (arg, &expected) in args.iter_mut().zip(&param_tys) {
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
            }
            return_ty
        };
        let callee_expr = hir::Expr {
            kind: Self::clone_callee_kind(&callee_proto),
            ty: function_ty,
            span,
        };
        hir::Expr {
            kind: hir::ExprKind::IndirectCall {
                callee: Box::new(callee_expr),
                args,
                func,
            },
            ty: return_ty,
            span,
        }
    }
}
