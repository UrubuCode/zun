use super::*;

/// Desugar `x++`/`++x`/`x--`/`--x` to `x = x ± 1` (the language has no update
/// operator). Correct for prefix form and for statement/for-update position
/// (the value is unused); a postfix result used as a value would see the new
/// value instead of the old — a known v1 limitation.
/// Convert an expression evaluated only for its effect (an expression
/// statement or a `for` update clause): a postfix update there needs no old
/// value.
pub(in crate::oxc) fn convert_effect_expression(
    source: &str,
    expression: &Expression<'_>,
) -> Result<Expr, String> {
    match expression {
        Expression::UpdateExpression(update) => convert_update(source, update, false),
        other => convert_expression(source, other),
    }
}

/// Convert `x++`/`x--`. In value position a postfix form yields the old value,
/// computed as `(x = x ± 1) ∓ 1`.
pub(super) fn convert_update(
    source: &str,
    update: &oxc_ast::ast::UpdateExpression<'_>,
    value_used: bool,
) -> Result<Expr, String> {
    if value_used && !update.prefix {
        if let oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier) = &update.argument {
            // `tmp = ToNumeric(x); x = tmp ± 1; tmp` keeps the exact old value
            // (including -0), which `(x = x ± 1) ∓ 1` cannot.
            let whole = Span::new(update.span.start, update.span.end);
            let id_span = Span::new(identifier.span.start, identifier.span.end);
            let op = match update.operator {
                oxc_ast::ast::UpdateOperator::Increment => BinaryOp::Add,
                oxc_ast::ast::UpdateOperator::Decrement => BinaryOp::Sub,
            };
            let boxed = Expr::Cast {
                expr: Box::new(Expr::Ident(Name { span: id_span })),
                ty: crate::ast::TypeAnnotation {
                    name: Name { span: Span::new(update.span.start, update.span.start) },
                    is_array: false,
                },
                span: whole,
            };
            let numeric = Expr::Binary {
                op: BinaryOp::Sub,
                lhs: Box::new(boxed),
                rhs: Box::new(Expr::Number { value: 0.0, span: whole }),
                span: whole,
            };
            let root = || Expr::ChainRoot { depth: 0, span: whole };
            let store = Expr::Assign {
                target: Name { span: id_span },
                value: Box::new(Expr::Binary {
                    op,
                    lhs: Box::new(root()),
                    rhs: Box::new(Expr::Number { value: 1.0, span: whole }),
                    span: whole,
                }),
                span: whole,
            };
            let body = Expr::Conditional {
                condition: Box::new(store),
                when_true: Box::new(root()),
                when_false: Box::new(root()),
                span: whole,
            };
            return Ok(Expr::TempBind { value: Box::new(numeric), body: Box::new(body), span: whole });
        }
    }
    let updated = convert_update_assign(source, update)?;
    if value_used && !update.prefix {
        let whole = Span::new(update.span.start, update.span.end);
        let undo = match update.operator {
            oxc_ast::ast::UpdateOperator::Increment => BinaryOp::Sub,
            oxc_ast::ast::UpdateOperator::Decrement => BinaryOp::Add,
        };
        return Ok(Expr::Binary {
            op: undo,
            lhs: Box::new(updated),
            rhs: Box::new(Expr::Number { value: 1.0, span: whole }),
            span: whole,
        });
    }
    Ok(updated)
}

pub(super) fn convert_update_assign(
    source: &str,
    update: &oxc_ast::ast::UpdateExpression<'_>,
) -> Result<Expr, String> {
    use oxc_ast::ast::SimpleAssignmentTarget;
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let op = match update.operator {
        oxc_ast::ast::UpdateOperator::Increment => BinaryOp::Add,
        oxc_ast::ast::UpdateOperator::Decrement => BinaryOp::Sub,
    };
    let whole = span(update.span);
    let one = || Expr::Number { value: 1.0, span: whole };
    let step = |read: Expr| Expr::Binary { op, lhs: Box::new(read), rhs: Box::new(one()), span: whole };
    let member = match &update.argument {
        SimpleAssignmentTarget::StaticMemberExpression(m) => Some(MemberRef::Static(m)),
        SimpleAssignmentTarget::ComputedMemberExpression(m) => Some(MemberRef::Computed(m)),
        SimpleAssignmentTarget::PrivateFieldExpression(m) => Some(MemberRef::Private(m)),
        _ => None,
    };
    if let Some(member) = member {
        let (binds, read, store) = decompose_member(source, member)?;
        return Ok(wrap_binds(binds, store.build(step(read), whole), whole));
    }
    match &update.argument {
        SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier) => {
            let target = Name { span: span(identifier.span) };
            Ok(Expr::Assign {
                target,
                value: Box::new(step(Expr::Ident(Name { span: span(identifier.span) }))),
                span: whole,
            })
        }
        SimpleAssignmentTarget::StaticMemberExpression(member) => {
            let object = convert_expression(source, &member.object)?;
            let read = Expr::Field {
                object: Box::new(convert_expression(source, &member.object)?),
                field: Name { span: span(member.property.span) },
                span: span(member.span),
            };
            Ok(Expr::FieldAssign {
                object: Box::new(object),
                field: Name { span: span(member.property.span) },
                value: Box::new(step(read)),
                span: whole,
            })
        }
        SimpleAssignmentTarget::ComputedMemberExpression(member) => {
            let read = Expr::Index {
                object: Box::new(convert_expression(source, &member.object)?),
                index: Box::new(convert_expression(source, &member.expression)?),
                span: span(member.span),
            };
            Ok(Expr::IndexAssign {
                object: Box::new(convert_expression(source, &member.object)?),
                index: Box::new(convert_expression(source, &member.expression)?),
                value: Box::new(step(read)),
                span: whole,
            })
        }
        SimpleAssignmentTarget::PrivateFieldExpression(member) => {
            let private = |source: &str| -> Result<Expr, String> {
                Ok(Expr::PrivateField {
                    object: Box::new(convert_expression(source, &member.object)?),
                    field: Name { span: span(member.field.span) },
                    span: span(member.span),
                })
            };
            let read = private(source)?;
            Ok(Expr::FieldAssign {
                object: Box::new(private(source)?),
                field: Name { span: span(member.field.span) },
                value: Box::new(step(read)),
                span: whole,
            })
        }
        _ => Err("OXC adapter supports identifier, field, and index update targets only".into()),
    }
}
