use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_object_literal(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::ObjectLiteral { properties, .. } = expr else {
            unreachable!()
        };
        let entries = self.lower_object_members(cx, properties, span);
        hir::Expr {
            kind: hir::ExprKind::DynamicObjectLiteral { entries },
            ty: Ty::DynamicObject,
            span,
        }
    }

    pub(in crate::check) fn check_bigint_literal(
        &mut self,
        _cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::BigInt { value, .. } = expr else {
            unreachable!()
        };
        let Some((id, native)) = zun_natives::resolve("collections", "bigIntNew") else {
            self.error(span, "BigInt literal intrinsic is unavailable");
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![hir::Expr {
                    kind: hir::ExprKind::String(value.clone()),
                    ty: Ty::String,
                    span,
                }],
            },
            ty: native_ty(native.ret),
            span,
        }
    }

    pub(in crate::check) fn check_regex_literal(
        &mut self,
        _cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::RegexLiteral { pattern, flags, .. } = expr else {
            unreachable!()
        };
        // Same lowering as `new RegExp(pattern, flags)`.
        let Some((id, _)) = zun_natives::resolve("collections", "regexpNew") else {
            self.error(span, "missing collection intrinsic `regexpNew`");
            return self.poison(span);
        };
        let string = |value: &str| hir::Expr {
            kind: hir::ExprKind::String(value.to_owned()),
            ty: Ty::String,
            span,
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![string(pattern), string(flags)],
            },
            ty: Ty::RegExp,
            span,
        }
    }

    pub(in crate::check) fn check_template(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::Template { parts, .. } = expr else {
            unreachable!()
        };
        let mut result = hir::Expr {
            kind: hir::ExprKind::String(String::new()),
            ty: Ty::String,
            span,
        };
        for part in parts {
            let next = match part {
                ast::TemplatePart::Text(text) => hir::Expr {
                    kind: hir::ExprKind::String(text.clone()),
                    ty: Ty::String,
                    span,
                },
                ast::TemplatePart::Expr(expression) => {
                    let value = self.check_expr(cx, expression);
                    if value.ty == Ty::F64 {
                        if let Some((id, _)) = zun_natives::resolve("number", "toString") {
                            hir::Expr {
                                kind: hir::ExprKind::CallBuiltin {
                                    id,
                                    args: vec![value],
                                },
                                ty: Ty::String,
                                span,
                            }
                        } else {
                            self.error(span, "number template conversion is unavailable");
                            self.poison(span)
                        }
                    } else if !matches!(value.ty, Ty::String | Ty::Error | Ty::Void)
                        && zun_natives::resolve("collections", "stringCoerceStrict").is_some()
                    {
                        // Template interpolation is ToString: a Symbol throws.
                        let (id, _) = zun_natives::resolve("collections", "stringCoerceStrict")
                            .expect("checked above");
                        let arg = if value.ty == Ty::Any {
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
                            kind: hir::ExprKind::CallBuiltin {
                                id,
                                args: vec![arg],
                            },
                            ty: Ty::String,
                            span,
                        }
                    } else {
                        if value.ty != Ty::String && !value.ty.is_error() {
                            self.error(
                                value.span,
                                "template interpolation must have type string or number",
                            );
                        }
                        value
                    }
                }
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
        }
        result
    }

    pub(in crate::check) fn check_tagged_template_expr(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::TaggedTemplate {
            tag,
            cooked,
            cooked_undefined,
            raw,
            exprs,
            ..
        } = expr
        else {
            unreachable!()
        };
        if !self.tag_is_string_raw(tag) {
            // A user tag is called as `tag(strings, ...substitutions)`.
            let literal = |chunks: &[String]| ast::Expr::ArrayLiteral {
                elements: chunks
                    .iter()
                    .map(|chunk| {
                        ast::ArrayElement::Expr(ast::Expr::String {
                            value: chunk.clone(),
                            span,
                        })
                    })
                    .collect(),
                span,
            };
            let cooked_array = self.check_expr(cx, &literal(cooked));
            let raw_array = self.check_expr(cx, &literal(raw));
            let Some((template_id, _)) = zun_natives::resolve("collections", "templateObject")
            else {
                self.error(span, "missing `templateObject` intrinsic");
                return self.poison(span);
            };
            let (slot, tmp) = self.synthetic_local(cx, Ty::Array);
            let mut args = vec![ast::Expr::Ident(tmp)];
            args.extend(exprs.iter().map(|expression| (**expression).clone()));
            // JS ignores substitutions beyond the tag's declared parameters.
            if let ast::Expr::Ident(tag_name) = tag.as_ref() {
                let tag_text = self.text(tag_name).to_string();
                if cx.lookup(&tag_text).is_none() {
                    if let Some(&func) = self.by_name.get(&tag_text) {
                        let sig = &self.signatures[func as usize];
                        if sig.rest_param.is_none() && args.len() > sig.params.len() {
                            let keep = sig.params.len().max(1);
                            args.truncate(keep);
                        }
                    }
                }
            }
            let call = match tag.as_ref() {
                ast::Expr::Ident(name) => ast::Expr::Call {
                    callee: *name,
                    args,
                    span,
                },
                other => ast::Expr::ComputedCall {
                    callee: Box::new(other.clone()),
                    args,
                    span,
                },
            };
            let call = self.check_expr(cx, &call);
            let ty = call.ty;
            return hir::Expr {
                kind: hir::ExprKind::Sequence {
                    effect: Box::new(hir::Expr {
                        kind: hir::ExprKind::Assign {
                            slot,
                            value: Box::new(hir::Expr {
                                kind: hir::ExprKind::CallBuiltin {
                                    id: template_id,
                                    args: vec![
                                        cooked_array,
                                        raw_array,
                                        hir::Expr {
                                            kind: hir::ExprKind::Number(span.start as f64),
                                            ty: Ty::Int(IntKind::I32),
                                            span,
                                        },
                                        hir::Expr {
                                            kind: hir::ExprKind::String(
                                                cooked_undefined
                                                    .iter()
                                                    .map(|u| if *u { '1' } else { '0' })
                                                    .collect(),
                                            ),
                                            ty: Ty::String,
                                            span,
                                        },
                                    ],
                                },
                                ty: Ty::Array,
                                span,
                            }),
                        },
                        ty: Ty::Array,
                        span,
                    }),
                    value: Box::new(call),
                },
                ty,
                span,
            };
        }
        self.check_tagged_template(cx, tag, raw, exprs, span)
    }

    pub(in crate::check) fn check_function_expr(
        &mut self,
        cx: &mut FunctionCx,
        expr: &ast::Expr,
    ) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::Function(function) = expr else {
            unreachable!()
        };
        // A `function (this: T, ...)` expression declares an explicit
        // receiver, so it takes the hidden `this` parameter wherever it
        // appears (returned, bound to a variable, passed as an argument).
        if !function.is_arrow && !self.closure_takes_this {
            let text = &self.source[function.span.start as usize..function.span.end as usize];
            if let Some(open) = text.find('(') {
                if text[open + 1..].trim_start().starts_with("this") {
                    self.closure_takes_this = true;
                }
            }
        }
        self.check_closure_expr(cx, function, span)
    }
}
