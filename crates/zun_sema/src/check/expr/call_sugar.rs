//! AST-phase interceptors for `check_call` (see `call.rs`).
//!
//! These run before the call arguments are checked, on the raw AST. Each
//! returns `Some` when it handles the call and `None` to fall through, in the
//! original order (first match wins).

use crate::check::*;

impl Checker<'_> {
    /// Name-based call forms resolved before argument checking: `super`, the
    /// `new`-less Error family and `Array`, the internal
    /// `__generatorReturnValue`, the global `parseInt`/`parseFloat`,
    /// `Object`, `isNaN`/`isFinite`, `encode`/`decodeURIComponent`, and
    /// `Function(...)`.
    pub(in crate::check) fn try_call_name_sugar(
        &mut self,
        cx: &mut FunctionCx,
        callee: &ast::Name,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        if name == "super" {
            return Some(self.check_super_call(cx, None, args, span));
        }
        // The Error family is callable without `new` (`TypeError("x")`).
        if matches!(
            name,
            "Error" | "EvalError" | "RangeError" | "ReferenceError" | "SyntaxError"
                | "TypeError" | "URIError" | "AggregateError"
        ) && self.classes_by_name.contains_key(name)
            && !self.by_name.contains_key(name)
            && cx.lookup(name).is_none()
        {
            return Some(self.check_expr(
                cx,
                &ast::Expr::New { class: *callee, args: args.to_vec(), span },
            ));
        }
        // `Array(n)` behaves exactly like `new Array(n)`.
        if name == "Array"
            && !self.by_name.contains_key(name)
            && cx.lookup(name).is_none()
        {
            return Some(self.check_expr(
                cx,
                &ast::Expr::New { class: *callee, args: args.to_vec(), span },
            ));
        }
        // Internal: the value of the `generator.return(v)` request being
        // unwound (used by the `yield*` desugaring to forward it).
        if name == "__generatorReturnValue" && args.is_empty() {
            if let Some((id, _)) = zun_natives::resolve("collections", "generatorReturnValue") {
                return Some(hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: Vec::new() }, ty: Ty::Any, span });
            }
        }
        // Global `parseInt`/`parseFloat` are the same as their `Number.*`
        // statics; route to that implementation when no user binding
        // shadows the name.
        if matches!(name, "parseInt" | "parseFloat")
            && !self.by_name.contains_key(name)
        {
            return Some(self.check_number_static(cx, name, callee.span, args, span));
        }
        // `Object(x)`: primitives become their wrapper objects, anything
        // already an object is returned unchanged.
        if name == "Object"
            && !self.by_name.contains_key(name)
            && cx.lookup(name).is_none()
            && args.len() == 1
        {
            let arg = self.check_expr(cx, &args[0]);
            let wrapper_name = match arg.ty {
                Ty::F64 => Some("NumberObject"),
                Ty::Boolean => Some("BooleanObject"),
                _ => None,
            };
            if let Some(wrapper) =
                wrapper_name.and_then(|name| self.classes_by_name.get(name).copied())
            {
                return Some(hir::Expr {
                    kind: hir::ExprKind::New { class: wrapper, args: vec![arg] },
                    ty: Ty::Instance(wrapper),
                    span,
                });
            }
            return Some(arg);
        }
        // Global `isNaN`/`isFinite` coerce with ToNumber first (unlike
        // the strict `Number.*` predicates).
        if matches!(name, "isNaN" | "isFinite")
            && !self.by_name.contains_key(name)
            && cx.lookup(name).is_none()
        {
            if args.len() != 1 {
                self.error(span, format!("`{name}` expects 1 argument, found {}", args.len()));
                return Some(self.poison(span));
            }
            let Some((id, _)) = zun_natives::resolve("number", name) else {
                self.error(span, format!("missing Number intrinsic `{name}`"));
                return Some(self.poison(span));
            };
            let mut arg = self.check_expr(cx, &args[0]);
            coerce_numeric_literal(&mut arg, Ty::F64);
            let arg = self.to_number_for_arg(arg);
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                ty: Ty::Boolean,
                span,
            });
        }
        if matches!(name, "encodeURIComponent" | "decodeURIComponent")
            && !self.by_name.contains_key(name)
            && cx.lookup(name).is_none()
            && args.len() == 1
        {
            let Some((id, _)) = zun_natives::resolve("string", name) else {
                self.error(span, format!("missing string intrinsic `{name}`"));
                return Some(self.poison(span));
            };
            let arg = self.check_expr(cx, &args[0]);
            let arg = if arg.ty == Ty::String { arg } else { self.to_string_for_parse(arg) };
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                ty: Ty::String,
                span,
            });
        }
        // `Function(p1, ..., body)` behaves exactly like `new Function(...)`.
        if name == "Function"
            && !self.by_name.contains_key(name)
            && !self.classes_by_name.contains_key(name)
        {
            return Some(self.lower_function_construct(cx, args, span));
        }
        None
    }

    /// Spread-argument call forms on a named callee: a trailing spread that
    /// lands exactly on the rest parameter, and the general spread lowering.
    pub(in crate::check) fn try_call_spread(
        &mut self,
        cx: &mut FunctionCx,
        callee: &ast::Name,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        if let Some(ast::Expr::Spread { expr, .. }) = args.last() {
            if let Some(&func) = self.by_name.get(name) {
                let rest_param = self.signatures[func as usize].rest_param;
                if rest_param == Some(args.len() - 1) {
                    let mut fixed_args = Vec::with_capacity(args.len() - 1);
                    for argument in &args[..args.len() - 1] {
                        fixed_args.push(self.check_expr(cx, argument));
                    }
                    let array = self.check_expr(cx, expr);
                    let array = self.spread_source_to_array(array);
                    if array.ty != Ty::Array && !array.ty.is_error() {
                        self.error(array.span, "spread call requires an Array value");
                        return Some(self.poison(span));
                    }
                    let params = self.signatures[func as usize].params.clone();
                    for (argument, &expected) in fixed_args.iter_mut().zip(&params) {
                        coerce_numeric_literal(argument, expected);
                        if !expected.is_error()
                            && !argument.ty.is_error()
                            && !self.is_assignable(argument.ty, expected)
                        {
                            self.error(
                                argument.span,
                                format!("argument type {:?} does not match {expected:?}", argument.ty),
                            );
                        }
                    }
                    let return_ty = self.signatures[func as usize].return_ty;
                    return Some(hir::Expr {
                        kind: if fixed_args.is_empty() {
                            hir::ExprKind::CallRestArray {
                                func,
                                args: Box::new(array),
                            }
                        } else {
                            hir::ExprKind::CallRestArrayFixed {
                                func,
                                fixed_args,
                                args: Box::new(array),
                            }
                        },
                        ty: return_ty,
                        span,
                    });
                }
            }
        }
        if args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
            && (self.by_name.contains_key(name) || cx.lookup(name).is_some() || cx.captures.contains_key(name))
        {
            // A named function whose only parameter is the rest array
            // takes the flattened argument array directly.
            if let Some(&func) = self.by_name.get(name) {
                if cx.lookup(name).is_none()
                    && self.signatures[func as usize].rest_param == Some(0)
                {
                    let elements = args
                        .iter()
                        .map(|arg| match arg {
                            ast::Expr::Spread { expr, .. } => {
                                ast::ArrayElement::Spread((**expr).clone())
                            }
                            other => ast::ArrayElement::Expr(other.clone()),
                        })
                        .collect();
                    let literal = ast::Expr::ArrayLiteral { elements, span };
                    let array = self.check_expr(cx, &literal);
                    let return_ty = self.signatures[func as usize].return_ty;
                    return Some(hir::Expr {
                        kind: hir::ExprKind::CallRestArray { func, args: Box::new(array) },
                        ty: return_ty,
                        span,
                    });
                }
            }
            // `f(a, ...xs)` on a named function with fixed parameters
            // followed by a rest parameter: flatten every argument into
            // one array, then split it into the fixed prefix and a
            // fresh rest array.
            if let Some(&func) = self.by_name.get(name) {
                let rest_index = self.signatures[func as usize].rest_param;
                if let (true, Some(rest_index)) = (cx.lookup(name).is_none(), rest_index) {
                    if rest_index > 0 && self.signatures[func as usize].params.len() == rest_index + 1 {
                        let elements = args
                            .iter()
                            .map(|arg| match arg {
                                ast::Expr::Spread { expr, .. } => {
                                    ast::ArrayElement::Spread((**expr).clone())
                                }
                                other => ast::ArrayElement::Expr(other.clone()),
                            })
                            .collect();
                        let literal = ast::Expr::ArrayLiteral { elements, span };
                        let flat = self.check_expr(cx, &literal);
                        if flat.ty == Ty::Array {
                            let temp = cx.declare(&self.fresh_temp_name(cx), Ty::Array);
                            let params = self.signatures[func as usize].params.clone();
                            let mut fixed_args = Vec::with_capacity(rest_index);
                            for (index, &expected) in params.iter().take(rest_index).enumerate() {
                                let _ = expected;
                                let read = self.array_element_read(temp, Ty::Array, index, span);
                                fixed_args.push(read);
                            }
                            let rest = self.array_rest_read(temp, Ty::Array, rest_index, span);
                            let return_ty = self.signatures[func as usize].return_ty;
                            return Some(hir::Expr {
                                kind: hir::ExprKind::Sequence {
                                    effect: Box::new(hir::Expr {
                                        kind: hir::ExprKind::Assign { slot: temp, value: Box::new(flat) },
                                        ty: Ty::Array,
                                        span,
                                    }),
                                    value: Box::new(hir::Expr {
                                        kind: hir::ExprKind::CallRestArrayFixed {
                                            func,
                                            fixed_args,
                                            args: Box::new(rest),
                                        },
                                        ty: return_ty,
                                        span,
                                    }),
                                },
                                ty: return_ty,
                                span,
                            });
                        }
                    }
                }
            }
            let callee_expr = ast::Expr::Ident(*callee);
            if let Some(call) = self.check_spread_call(cx, &callee_expr, args, span) {
                return Some(call);
            }
        }
        None
    }
}
