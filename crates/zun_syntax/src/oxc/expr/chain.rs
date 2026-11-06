use super::*;

/// Whether the member/call spine of `expression` contains an optional link
/// (`?.`), i.e. whether it is part of an optional chain.
pub(super) fn spine_has_optional(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::StaticMemberExpression(member) => {
            member.optional || spine_has_optional(&member.object)
        }
        Expression::ComputedMemberExpression(member) => {
            member.optional || spine_has_optional(&member.object)
        }
        Expression::CallExpression(call) => call.optional || spine_has_optional(&call.callee),
        Expression::TSNonNullExpression(inner) => spine_has_optional(&inner.expression),
        _ => false,
    }
}

/// Convert one link of an optional chain. Every `?.` link pushes the value
/// computed so far onto `guards` and continues on a [`Expr::ChainRoot`]
/// placeholder, so a nullish guard short-circuits the *rest* of the chain.
pub(super) fn chain_link(
    source: &str,
    expression: &Expression<'_>,
    guards: &mut Vec<Expr>,
) -> Result<Expr, String> {
    if !spine_has_optional(expression) {
        return convert_expression(source, expression);
    }
    match expression {
        Expression::StaticMemberExpression(member) => chain_static(source, member, guards),
        Expression::ComputedMemberExpression(member) => chain_computed(source, member, guards),
        Expression::CallExpression(call) => chain_call(source, call, guards),
        Expression::TSNonNullExpression(inner) => chain_link(source, &inner.expression, guards),
        other => convert_expression(source, other),
    }
}

pub(super) fn chain_guard(object: Expr, optional: bool, guards: &mut Vec<Expr>) -> Expr {
    if optional {
        let span = object.span();
        guards.push(object);
        Expr::ChainRoot { depth: 0, span }
    } else {
        object
    }
}

pub(super) fn chain_static(
    source: &str,
    member: &oxc_ast::ast::StaticMemberExpression<'_>,
    guards: &mut Vec<Expr>,
) -> Result<Expr, String> {
    let object = chain_link(source, &member.object, guards)?;
    let object = chain_guard(object, member.optional, guards);
    Ok(Expr::Field {
        object: Box::new(object),
        field: Name { span: Span::new(member.property.span.start, member.property.span.end) },
        span: Span::new(member.span.start, member.span.end),
    })
}

pub(super) fn chain_computed(
    source: &str,
    member: &oxc_ast::ast::ComputedMemberExpression<'_>,
    guards: &mut Vec<Expr>,
) -> Result<Expr, String> {
    let object = chain_link(source, &member.object, guards)?;
    let object = chain_guard(object, member.optional, guards);
    Ok(Expr::Index {
        object: Box::new(object),
        index: Box::new(convert_expression(source, &member.expression)?),
        span: Span::new(member.span.start, member.span.end),
    })
}

pub(super) fn chain_call(
    source: &str,
    call: &oxc_ast::ast::CallExpression<'_>,
    guards: &mut Vec<Expr>,
) -> Result<Expr, String> {
    let span = Span::new(call.span.start, call.span.end);
    let mut args = Vec::with_capacity(call.arguments.len());
    for argument in &call.arguments {
        args.push(convert_argument(source, argument)?);
    }
    match &call.callee {
        Expression::StaticMemberExpression(member) => {
            let object = chain_link(source, &member.object, guards)?;
            let object = chain_guard(object, member.optional, guards);
            let method = Name { span: Span::new(member.property.span.start, member.property.span.end) };
            if call.optional {
                Ok(Expr::OptionalCall {
                    callee: Box::new(Expr::Field {
                        object: Box::new(object),
                        field: method,
                        span: Span::new(member.span.start, member.span.end),
                    }),
                    args,
                    span,
                })
            } else {
                Ok(Expr::MethodCall { receiver: Box::new(object), method, args, span })
            }
        }
        Expression::ComputedMemberExpression(member) => {
            let object = chain_link(source, &member.object, guards)?;
            let object = chain_guard(object, member.optional, guards);
            let callee = Expr::Index {
                object: Box::new(object),
                index: Box::new(convert_expression(source, &member.expression)?),
                span: Span::new(member.span.start, member.span.end),
            };
            if call.optional {
                Ok(Expr::OptionalCall { callee: Box::new(callee), args, span })
            } else {
                Ok(Expr::ComputedCall { callee: Box::new(callee), args, span })
            }
        }
        other => {
            let callee = chain_link(source, other, guards)?;
            if call.optional {
                Ok(Expr::OptionalCall { callee: Box::new(callee), args, span })
            } else {
                Ok(Expr::ComputedCall { callee: Box::new(callee), args, span })
            }
        }
    }
}

pub(super) fn convert_chain(
    source: &str,
    chain: &oxc_ast::ast::ChainExpression<'_>,
) -> Result<Expr, String> {
    let mut guards: Vec<Expr> = Vec::new();
    let chain_span = Span::new(chain.span.start, chain.span.end);
    let body = match &chain.expression {
        ChainElement::StaticMemberExpression(member) => chain_static(source, member, &mut guards)?,
        ChainElement::ComputedMemberExpression(member) => {
            chain_computed(source, member, &mut guards)?
        }
        ChainElement::CallExpression(call) => chain_call(source, call, &mut guards)?,
        ChainElement::TSNonNullExpression(inner) => {
            chain_link(source, &inner.expression, &mut guards)?
        }
        _ => return Err("OXC adapter does not support optional private members yet".into()),
    };
    if guards.is_empty() {
        return Ok(body);
    }
    if guards.len() == 1 {
        let is_root = |expr: &Expr| matches!(expr, Expr::ChainRoot { .. });
        match body {
            Expr::Field { object, field, .. } if is_root(&object) => {
                let guard = guards.pop().unwrap();
                return Ok(Expr::OptionalField { object: Box::new(guard), field, span: chain_span });
            }
            Expr::Index { object, index, .. } if is_root(&object) => {
                let guard = guards.pop().unwrap();
                return Ok(Expr::OptionalIndex { object: Box::new(guard), index, span: chain_span });
            }
            Expr::MethodCall { receiver, method, args, .. } if is_root(&receiver) => {
                let guard = guards.pop().unwrap();
                return Ok(Expr::OptionalMethodCall {
                    object: Box::new(guard),
                    method,
                    args,
                    span: chain_span,
                });
            }
            other => {
                let guard = guards.pop().unwrap();
                return Ok(Expr::OptionalChain {
                    object: Box::new(guard),
                    body: Box::new(other),
                    span: chain_span,
                });
            }
        }
    }
    let mut result = body;
    for guard in guards.into_iter().rev() {
        result = Expr::OptionalChain {
            object: Box::new(guard),
            body: Box::new(result),
            span: chain_span,
        };
    }
    Ok(result)
}
