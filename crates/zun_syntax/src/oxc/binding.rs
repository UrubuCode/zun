use super::*;

/// Convert an OXC property key used in an object binding pattern.
pub(super) fn convert_binding_key(
    source: &str,
    property: &oxc_ast::ast::BindingProperty<'_>,
) -> Result<crate::ast::PropertyKey, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    if property.computed {
        let Some(key_expression) = property.key.as_expression() else {
            return Err("OXC adapter requires an expression computed key".into());
        };
        return Ok(crate::ast::PropertyKey::Computed(Box::new(
            convert_expression(source, key_expression)?,
        )));
    }
    match &property.key {
        PropertyKey::StaticIdentifier(identifier) => Ok(crate::ast::PropertyKey::Ident(Name {
            span: span(identifier.span),
        })),
        PropertyKey::StringLiteral(string) => {
            let value = js_str_to_string(string.value);
            Ok(crate::ast::PropertyKey::String {
                value: value.to_owned(),
                span: span(string.span),
            })
        }
        PropertyKey::NumericLiteral(number) => Ok(crate::ast::PropertyKey::Number {
            value: number.value,
            span: span(number.span),
        }),
        _ => Err("OXC adapter supports identifier, string, and numeric property keys only".into()),
    }
}

/// Convert an OXC binding pattern (identifier, array, object, assignment) into
/// the Zun [`crate::ast::Pattern`]. Nested patterns recurse naturally.
pub(super) fn convert_binding_pattern(
    source: &str,
    pattern: &BindingPattern<'_>,
) -> Result<crate::ast::Pattern, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => Ok(crate::ast::Pattern::Ident {
            name: Name {
                span: span(identifier.span),
            },
            ty: None,
        }),
        BindingPattern::ArrayPattern(array) => {
            let mut elements = Vec::with_capacity(array.elements.len());
            for element in &array.elements {
                match element {
                    None => elements.push(None),
                    Some(binding) => {
                        let (pattern, default) = split_assignment(source, binding)?;
                        elements.push(Some(crate::ast::PatternElement { pattern, default }));
                    }
                }
            }
            let rest = match &array.rest {
                Some(rest) => Some(Box::new(convert_binding_pattern(source, &rest.argument)?)),
                None => None,
            };
            Ok(crate::ast::Pattern::Array {
                elements,
                rest,
                span: span(array.span),
            })
        }
        BindingPattern::ObjectPattern(object) => {
            let mut properties = Vec::with_capacity(object.properties.len());
            for property in &object.properties {
                let key = convert_binding_key(source, property)?;
                let (value, default) = split_assignment(source, &property.value)?;
                properties.push(crate::ast::PatternProperty {
                    key,
                    value,
                    default,
                });
            }
            let rest = match &object.rest {
                Some(rest) => Some(Box::new(convert_binding_pattern(source, &rest.argument)?)),
                None => None,
            };
            Ok(crate::ast::Pattern::Object {
                properties,
                rest,
                span: span(object.span),
            })
        }
        BindingPattern::AssignmentPattern(_) => {
            // A top-level `=` default is only meaningful attached to its
            // enclosing element/property, handled by `split_assignment`.
            Err("OXC adapter: unexpected default outside an element or property".into())
        }
    }
}

/// Split a binding pattern that may carry a top-level default (`x = 1`,
/// `{a} = obj`) into its inner pattern and optional default expression.
pub(super) fn split_assignment(
    source: &str,
    pattern: &BindingPattern<'_>,
) -> Result<(crate::ast::Pattern, Option<Expr>), String> {
    match pattern {
        BindingPattern::AssignmentPattern(assignment) => Ok((
            convert_binding_pattern(source, &assignment.left)?,
            Some(convert_expression(source, &assignment.right)?),
        )),
        other => Ok((convert_binding_pattern(source, other)?, None)),
    }
}

/// `const o: any = { m() { ... o ... } }`: an object literal whose methods
/// refer to the binding being declared. The binding is declared first and
/// assigned after, so the methods capture its (shared) cell.
pub(super) fn convert_declarator_stmts(
    source: &str,
    declarator: &oxc_ast::ast::VariableDeclarator<'_>,
) -> Result<Vec<Stmt>, String> {
    if let (
        BindingPattern::BindingIdentifier(id),
        Some(Expression::ObjectExpression(object)),
        Some(annotation),
    ) = (
        &declarator.id,
        declarator.init.as_ref(),
        declarator.type_annotation.as_deref(),
    ) {
        let annotation_text = source
            .get(annotation.span.start as usize..annotation.span.end as usize)
            .unwrap_or("");
        let object_text = source
            .get(object.span.start as usize..object.span.end as usize)
            .unwrap_or("");
        if annotation_text.trim_start_matches(':').trim() == "any"
            && contains_word(object_text, id.name.as_str())
        {
            let span = |span: oxc_span::Span| Span::new(span.start, span.end);
            let whole = span(declarator.span);
            let name = Name {
                span: span(id.span),
            };
            let value = convert_expression(source, declarator.init.as_ref().expect("checked"))?;
            return Ok(vec![
                Stmt::Let {
                    name,
                    ty: convert_type_annotation(source, declarator.type_annotation.as_deref())?,
                    init: Expr::Unary {
                        op: crate::ast::UnaryOp::Void,
                        operand: Box::new(Expr::Number {
                            value: 0.0,
                            span: whole,
                        }),
                        span: whole,
                    },
                    span: whole,
                },
                Stmt::Expr(Expr::Assign {
                    target: name,
                    value: Box::new(value),
                    span: whole,
                }),
            ]);
        }
    }
    Ok(vec![convert_declarator(source, declarator)?])
}

/// Convert one variable declarator into either a `Stmt::Let` (identifier head)
/// or a `Stmt::Destructure` (array/object pattern head).
pub(super) fn convert_declarator(
    source: &str,
    declarator: &oxc_ast::ast::VariableDeclarator<'_>,
) -> Result<Stmt, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    // An uninitialized binding (`let x;`) defaults to `undefined` (`void 0`).
    let init = match declarator.init.as_ref() {
        Some(initializer) => {
            if let (Expression::ClassExpression(_), BindingPattern::BindingIdentifier(binding)) =
                (initializer, &declarator.id)
            {
                CLASS_NAME_HINT.with(|cell| *cell.borrow_mut() = Some(span(binding.span)));
            }
            let converted = convert_expression(source, initializer);
            CLASS_NAME_HINT.with(|cell| *cell.borrow_mut() = None);
            converted?
        }
        None => Expr::Unary {
            op: crate::ast::UnaryOp::Void,
            operand: Box::new(Expr::Number {
                value: 0.0,
                span: span(declarator.span),
            }),
            span: span(declarator.span),
        },
    };
    match &declarator.id {
        BindingPattern::BindingIdentifier(identifier) => Ok(Stmt::Let {
            name: Name {
                span: span(identifier.span),
            },
            ty: convert_type_annotation(source, declarator.type_annotation.as_deref())?,
            init,
            span: span(declarator.span),
        }),
        other => Ok(Stmt::Destructure {
            pattern: convert_binding_pattern(source, other)?,
            init,
            span: span(declarator.span),
        }),
    }
}
