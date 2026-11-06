use super::*;

pub(in crate::oxc) fn convert_argument(source: &str, argument: &Argument<'_>) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    // `...spread` is the only argument form that is not itself an expression;
    // every other argument is an ordinary expression, so route it through the
    // full expression converter rather than re-listing a subset here.
    if let Argument::SpreadElement(spread) = argument {
        return Ok(Expr::Spread {
            expr: Box::new(convert_expression(source, &spread.argument)?),
            span: span(spread.span),
        });
    }
    let expression = argument
        .as_expression()
        .ok_or("OXC adapter cannot represent this argument")?;
    // A static `Number.f` / `Math.f` passed as a callback (`xs.filter(Number.isNaN)`)
    // becomes `(f) => Number.f(f)`; the method-name span doubles as the
    // parameter name, which is harmless because the body only reads it once.
    if let Expression::StaticMemberExpression(member) = expression {
        if let Expression::Identifier(object) = &member.object {
            let object_name = source
                .get(object.span.start as usize..object.span.end as usize)
                .unwrap_or("");
            let property = source
                .get(member.property.span.start as usize..member.property.span.end as usize)
                .unwrap_or("");
            let callable = property.chars().next().is_some_and(|c| c.is_ascii_lowercase());
            if matches!(object_name, "Number" | "Math") && callable && !member.optional {
                let property_name = Name { span: span(member.property.span) };
                let call = Expr::MethodCall {
                    receiver: Box::new(Expr::Ident(Name { span: span(object.span) })),
                    method: Name { span: span(member.property.span) },
                    args: vec![Expr::Ident(Name { span: span(member.property.span) })],
                    span: span(member.span),
                };
                return Ok(Expr::Function(FunctionExpr {
                    params: vec![crate::ast::Param {
                        name: property_name,
                        ty: None,
                        default_value: None,
                        is_rest: false,
                    }],
                    return_type: None,
                    body: Block {
                        statements: vec![Stmt::Return {
                            value: Some(call),
                            span: span(member.span),
                        }],
                        span: span(member.span),
                    },
                    is_arrow: true,
                    is_async: false,
                    is_generator: false,
                    span: span(member.span),
                }));
            }
        }
    }
    convert_expression(source, expression)
}
