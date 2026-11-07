use super::*;

pub(super) fn convert_object_literal(
    source: &str,
    object: &oxc_ast::ast::ObjectExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let mut properties = Vec::with_capacity(object.properties.len());
    for property in &object.properties {
        let property = match property {
            ObjectPropertyKind::ObjectProperty(property) => property,
            ObjectPropertyKind::SpreadProperty(spread) => {
                properties.push(crate::ast::ObjectMember::Spread {
                    expr: convert_expression(source, &spread.argument)?,
                    span: span(spread.span),
                });
                continue;
            }
        };
        // Method shorthand and accessors carry a function value; a plain
        // data property is `PropertyKind::Init` and not a method.
        let member_kind = if property.method {
            crate::ast::PropertyKind::Method
        } else {
            match property.kind {
                PropertyKind::Init => crate::ast::PropertyKind::Init,
                PropertyKind::Get => crate::ast::PropertyKind::Get,
                PropertyKind::Set => crate::ast::PropertyKind::Set,
            }
        };
        let key = if property.computed {
            let Some(key_expression) = property.key.as_expression() else {
                return Err("OXC adapter requires an expression computed key".into());
            };
            crate::ast::PropertyKey::Computed(Box::new(convert_expression(source, key_expression)?))
        } else {
            match &property.key {
                PropertyKey::StaticIdentifier(identifier) => crate::ast::PropertyKey::Ident(Name {
                    span: span(identifier.span),
                }),
                PropertyKey::StringLiteral(string) => {
                    let value = js_str_to_string(string.value);
                    crate::ast::PropertyKey::String {
                        value: value.to_owned(),
                        span: span(string.span),
                    }
                }
                PropertyKey::NumericLiteral(number) => crate::ast::PropertyKey::Number {
                    value: number.value,
                    span: span(number.span),
                },
                _ => {
                    return Err(
                        "OXC adapter supports identifier, string, and numeric property keys only"
                            .into(),
                    )
                }
            }
        };
        let value = match member_kind {
            crate::ast::PropertyKind::Init => convert_expression(source, &property.value)?,
            // Method / getter / setter: the value is a function.
            _ => {
                let Expression::FunctionExpression(function) = &property.value else {
                    return Err(
                        "OXC adapter expected a function value for an object method or accessor"
                            .into(),
                    );
                };
                convert_oxc_function(source, function)?
            }
        };
        properties.push(crate::ast::ObjectMember::Property(
            crate::ast::ObjectProperty {
                key,
                value,
                kind: member_kind,
            },
        ));
    }
    Ok(Expr::ObjectLiteral {
        properties,
        span: span(object.span),
    })
}

pub(super) fn convert_array_literal(
    source: &str,
    array: &oxc_ast::ast::ArrayExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let mut elements = Vec::with_capacity(array.elements.len());
    for element in &array.elements {
        match element {
            ArrayExpressionElement::Elision(_) => {
                elements.push(crate::ast::ArrayElement::Hole);
            }
            ArrayExpressionElement::SpreadElement(spread) => {
                elements.push(crate::ast::ArrayElement::Spread(convert_expression(
                    source,
                    &spread.argument,
                )?));
            }
            other => {
                let Some(expression) = other.as_expression() else {
                    return Err("OXC adapter expected an array element expression".into());
                };
                elements.push(crate::ast::ArrayElement::Expr(convert_expression(
                    source, expression,
                )?));
            }
        }
    }
    Ok(Expr::ArrayLiteral {
        elements,
        span: span(array.span),
    })
}
