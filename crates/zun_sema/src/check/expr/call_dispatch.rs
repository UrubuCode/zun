//! The final named-function dispatch for `check_call` (see `call.rs`).
//!
//! Reached once every earlier interceptor has fallen through: the callee name
//! is looked up in the module's function table and the arguments are checked
//! against its signature. An unknown name routes to `call_unknown_fallback`,
//! which treats it as a run-time error (dead code). Semantics are preserved.

use crate::check::*;

impl Checker<'_> {
    /// Dispatch a call to a module-level function resolved by name, checking
    /// arity and argument types against its signature (rest-parameter or
    /// fixed), and threading the real argument count to callees that read
    /// `arguments`.
    pub(in crate::check) fn call_named_dispatch(
        &mut self,
        cx: &mut FunctionCx,
        callee: &ast::Name,
        name: &str,
        mut args: Vec<hir::Expr>,
        span: Span,
    ) -> hir::Expr {
        let Some(&func) = self.by_name.get(name) else {
            return self.call_unknown_fallback(cx, callee, name, args, span);
        };
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
        let actual_argc = args.len();
        let rest_index = sig.rest_param;
        if let Some(rest_index) = rest_index {
            let rest_required = rest_index.saturating_sub(optional_count);
            if args.len() < rest_required {
                self.error(
                    span,
                    format!(
                        "`{name}` expects at least {} argument(s), found {}",
                        rest_required,
                        args.len()
                    ),
                );
            } else {
                while args.len() < rest_index {
                    args.push(default_args::missing_arg(param_tys[args.len()], span));
                }
                // Function values packed into the rest array ride in functionBox records.
                for index in rest_index..args.len() {
                    if matches!(args[index].ty, Ty::Function(_)) {
                        let element = std::mem::replace(&mut args[index], self.poison(span));
                        args[index] = self.box_function_argument(element);
                    }
                }
                for (arg, &expected) in args.iter_mut().take(rest_index).zip(&param_tys) {
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
        } else if args.len() < param_tys.len().saturating_sub(optional_count)
            || args.len() > param_tys.len()
        {
            self.error(
                span,
                format!(
                    "`{name}` expects {} to {} argument(s), found {}",
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
        // A callee that reads `arguments` learns the real argument count
        // (before default padding) through the last argument's evaluation.
        if self.arguments_functions.contains(&func) {
            if let (Some(last), Some((mark_id, _))) = (
                args.pop(),
                zun_natives::resolve("collections", "setCallArgc"),
            ) {
                let (ty, last_span) = (last.ty, last.span);
                args.push(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: mark_id,
                        args: vec![
                            last,
                            hir::Expr {
                                kind: hir::ExprKind::Number(actual_argc as f64),
                                ty: Ty::F64,
                                span: last_span,
                            },
                        ],
                    },
                    ty,
                    span: last_span,
                });
            }
        }
        hir::Expr {
            kind: if rest_index.is_some() {
                hir::ExprKind::CallRest { func, args }
            } else {
                hir::ExprKind::Call { func, args }
            },
            ty: return_ty,
            span,
        }
    }

    /// An undeclared callee name: `undefined()` is a run-time TypeError and any
    /// other name a ReferenceError raised by reading it, both lowered to a
    /// dynamic indirect call; otherwise an `unknown function` diagnostic.
    fn call_unknown_fallback(
        &mut self,
        cx: &mut FunctionCx,
        callee: &ast::Name,
        name: &str,
        args: Vec<hir::Expr>,
        span: Span,
    ) -> hir::Expr {
        // An undeclared callee is a run-time error (it may sit in dead
        // code): `undefined()` is a TypeError, any other undeclared
        // name a ReferenceError raised by reading it.
        let callee_value = if name == "undefined" {
            Some(self.check_expr(
                cx,
                &ast::Expr::Unary {
                    op: ast::UnaryOp::Void,
                    operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                    span,
                },
            ))
        } else {
            zun_natives::resolve("collections", "globalRead").map(|(id, _)| hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![Self::string_literal(name.to_string(), span)],
                },
                ty: Ty::Any,
                span,
            })
        };
        if let Some(callee_value) = callee_value {
            return hir::Expr {
                kind: hir::ExprKind::IndirectCall {
                    callee: Box::new(callee_value),
                    args,
                    func: hir::DYNAMIC_FUNCTION,
                },
                ty: Ty::Any,
                span,
            };
        }
        self.error(callee.span, format!("unknown function `{name}`"));
        self.poison(span)
    }
}
