use super::*;

thread_local! {
    /// Per-compilation counter for destructuring temporaries. Reset at the top
    /// of `parse_subset` so generated names are deterministic across repeated
    /// compiles in the same process / thread.
    pub(super) static DESTRUCTURE_TEMP: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub(super) fn fresh_destructure_temp() -> Name {
    let id = DESTRUCTURE_TEMP.with(|counter| {
        let id = counter.get();
        counter.set(id + 1);
        id
    });
    crate::ast::synthetic_ident(&format!("__zd{id}"))
}

/// Store `value` into a plain (non-pattern) assignment target.
pub(super) fn store_into_target(
    source: &str,
    target: &AssignmentTarget<'_>,
    value: Expr,
    whole: Span,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let value = Box::new(value);
    match target {
        AssignmentTarget::AssignmentTargetIdentifier(identifier) => Ok(Expr::Assign {
            target: Name { span: span(identifier.span) },
            value,
            span: whole,
        }),
        AssignmentTarget::StaticMemberExpression(member) => Ok(Expr::FieldAssign {
            object: Box::new(convert_expression(source, &member.object)?),
            field: Name { span: span(member.property.span) },
            value,
            span: whole,
        }),
        AssignmentTarget::ComputedMemberExpression(member) => Ok(Expr::IndexAssign {
            object: Box::new(convert_expression(source, &member.object)?),
            index: Box::new(convert_expression(source, &member.expression)?),
            value,
            span: whole,
        }),
        AssignmentTarget::PrivateFieldExpression(member) => Ok(Expr::FieldAssign {
            object: Box::new(Expr::PrivateField {
                object: Box::new(convert_expression(source, &member.object)?),
                field: Name { span: span(member.field.span) },
                span: span(member.span),
            }),
            field: Name { span: span(member.field.span) },
            value,
            span: whole,
        }),
        _ => Err("OXC adapter supports identifier, field, and index assignments only".into()),
    }
}

/// Lower one destructuring-assignment target into a binding pattern over fresh
/// temporaries, pushing the `target = temp` stores (in source order) to `stores`.
pub(super) fn lower_assignment_target(
    source: &str,
    target: &AssignmentTarget<'_>,
    stores: &mut Vec<Stmt>,
    whole: Span,
) -> Result<crate::ast::Pattern, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    match target {
        AssignmentTarget::ArrayAssignmentTarget(array) => {
            let mut elements = Vec::with_capacity(array.elements.len());
            for element in &array.elements {
                match element {
                    None => elements.push(None),
                    Some(maybe) => {
                        let (pattern, default) = lower_maybe_default(source, maybe, stores, whole)?;
                        elements.push(Some(crate::ast::PatternElement { pattern, default }));
                    }
                }
            }
            let rest = match &array.rest {
                Some(rest) => Some(Box::new(lower_assignment_target(source, &rest.target, stores, whole)?)),
                None => None,
            };
            Ok(crate::ast::Pattern::Array { elements, rest, span: span(array.span) })
        }
        AssignmentTarget::ObjectAssignmentTarget(object) => {
            let mut properties = Vec::with_capacity(object.properties.len());
            for property in &object.properties {
                match property {
                    oxc_ast::ast::AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(shorthand) => {
                        let temp = fresh_destructure_temp();
                        let name = Name { span: span(shorthand.binding.span) };
                        stores.push(Stmt::Expr(Expr::Assign {
                            target: name,
                            value: Box::new(Expr::Ident(temp)),
                            span: whole,
                        }));
                        let default = match &shorthand.init {
                            Some(init) => Some(convert_expression(source, init)?),
                            None => None,
                        };
                        properties.push(crate::ast::PatternProperty {
                            key: crate::ast::PropertyKey::Ident(name),
                            value: crate::ast::Pattern::Ident { name: temp, ty: None },
                            default,
                        });
                    }
                    oxc_ast::ast::AssignmentTargetProperty::AssignmentTargetPropertyProperty(entry) => {
                        let key = if entry.computed {
                            let Some(key_expression) = entry.name.as_expression() else {
                                return Err("OXC adapter requires an expression computed key".into());
                            };
                            crate::ast::PropertyKey::Computed(Box::new(convert_expression(source, key_expression)?))
                        } else {
                            match &entry.name {
                                PropertyKey::StaticIdentifier(identifier) => {
                                    crate::ast::PropertyKey::Ident(Name { span: span(identifier.span) })
                                }
                                PropertyKey::StringLiteral(string) => {
                                    let value = js_str_to_string(string.value);
                                    crate::ast::PropertyKey::String { value: value.to_owned(), span: span(string.span) }
                                }
                                PropertyKey::NumericLiteral(number) => {
                                    crate::ast::PropertyKey::Number { value: number.value, span: span(number.span) }
                                }
                                _ => return Err("OXC adapter supports identifier, string, and numeric property keys only".into()),
                            }
                        };
                        let (value, default) = lower_maybe_default(source, &entry.binding, stores, whole)?;
                        properties.push(crate::ast::PatternProperty { key, value, default });
                    }
                }
            }
            let rest = match &object.rest {
                Some(rest) => Some(Box::new(lower_assignment_target(source, &rest.target, stores, whole)?)),
                None => None,
            };
            Ok(crate::ast::Pattern::Object { properties, rest, span: span(object.span) })
        }
        simple => {
            let temp = fresh_destructure_temp();
            stores.push(Stmt::Expr(store_into_target(source, simple, Expr::Ident(temp), whole)?));
            Ok(crate::ast::Pattern::Ident { name: temp, ty: None })
        }
    }
}

pub(super) fn lower_maybe_default(
    source: &str,
    maybe: &oxc_ast::ast::AssignmentTargetMaybeDefault<'_>,
    stores: &mut Vec<Stmt>,
    whole: Span,
) -> Result<(crate::ast::Pattern, Option<Expr>), String> {
    match maybe {
        oxc_ast::ast::AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(with_default) => {
            let pattern = lower_assignment_target(source, &with_default.binding, stores, whole)?;
            Ok((pattern, Some(convert_expression(source, &with_default.init)?)))
        }
        other => {
            let target = other.as_assignment_target().ok_or("OXC adapter: unsupported assignment target")?;
            Ok((lower_assignment_target(source, target, stores, whole)?, None))
        }
    }
}

/// `[a, o.b] = rhs` / `({ x, y: o.z } = rhs)`: evaluate `rhs` once, destructure
/// it into fresh temporaries, store each into its target, and yield `rhs`.
pub(super) fn convert_destructuring_assignment(
    source: &str,
    target: &AssignmentTarget<'_>,
    right: Expr,
    whole: Span,
) -> Result<Expr, String> {
    let mut stores = Vec::new();
    let pattern = lower_assignment_target(source, target, &mut stores, whole)?;
    let whole_temp = fresh_destructure_temp();
    let mut statements = vec![Stmt::Destructure {
        pattern,
        init: Expr::Ident(whole_temp),
        span: whole,
    }];
    statements.extend(stores);
    statements.push(Stmt::Return { value: Some(Expr::Ident(whole_temp)), span: whole });
    let iife = Expr::Function(FunctionExpr {
        params: vec![crate::ast::Param { name: whole_temp, ty: None, default_value: None, is_rest: false }],
        return_type: convert_type_annotation(source, None)?,
        body: Block { statements, span: whole },
        is_arrow: true,
        is_async: false,
        is_generator: false,
        span: whole,
    });
    Ok(Expr::ComputedCall {
        callee: Box::new(iife),
        args: vec![right],
        span: whole,
    })
}
