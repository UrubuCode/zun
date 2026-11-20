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
            // A plain (JavaScript) `function () { ... this ... }` has no `this:`
            // annotation, but its body reads the receiver, so it takes the hidden
            // parameter too: `emitter.on('x', function () { this.close(); })` and
            // `fn.call(obj)` bind `this` instead of leaving it undefined.
            if !self.closure_takes_this && mentions_this_word(text) {
                self.closure_takes_this = true;
            }
            // A compiler-made function reads the receiver through a reference
            // its source text does not spell out; ask the AST.
            if !self.closure_takes_this && self.closure_reads_this(cx, function) {
                self.closure_takes_this = true;
            }
        }
        self.check_closure_expr(cx, function, span)
    }
}

/// Whether `text` uses the word `this` as code: not inside a comment or a
/// string literal, and not part of a longer identifier.
pub(in crate::check) fn mentions_this_word(text: &str) -> bool {
    let bytes = text.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else if b == b'\'' || b == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
        } else if is_ident(b) {
            let start = i;
            while i < bytes.len() && is_ident(bytes[i]) {
                i += 1;
            }
            if &bytes[start..i] == b"this" {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod this_word_tests {
    use super::mentions_this_word;

    #[test]
    fn finds_code_uses_only() {
        assert!(mentions_this_word("function () { return this.x; }"));
        assert!(mentions_this_word("function (a) { a(this); }"));
        assert!(!mentions_this_word("function (e) { // on this system\n return e; }"));
        assert!(!mentions_this_word("function (e) { /* this */ return 'this'; }"));
        assert!(!mentions_this_word("function (e) { return thisArg + this_ + $this; }"));
    }
}
