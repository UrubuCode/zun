use super::*;

/// The binary operator a compound assignment (`a OP= b`) applies, or `None`
/// for a plain `=`.
fn compound_binary_op(operator: AssignmentOperator) -> Result<Option<BinaryOp>, String> {
    Ok(match operator {
        AssignmentOperator::Assign => None,
        AssignmentOperator::Addition => Some(BinaryOp::Add),
        AssignmentOperator::Subtraction => Some(BinaryOp::Sub),
        AssignmentOperator::Multiplication => Some(BinaryOp::Mul),
        AssignmentOperator::Division => Some(BinaryOp::Div),
        AssignmentOperator::Remainder => Some(BinaryOp::Rem),
        AssignmentOperator::Exponential => Some(BinaryOp::Pow),
        AssignmentOperator::BitwiseAnd => Some(BinaryOp::BitAnd),
        AssignmentOperator::BitwiseOR => Some(BinaryOp::BitOr),
        AssignmentOperator::BitwiseXOR => Some(BinaryOp::BitXor),
        AssignmentOperator::ShiftLeft => Some(BinaryOp::Shl),
        AssignmentOperator::ShiftRight => Some(BinaryOp::Shr),
        AssignmentOperator::ShiftRightZeroFill => Some(BinaryOp::UShr),
        // Logical assignments desugar to `a = a OP b`; the resulting
        // value matches the short-circuit form for every operand.
        AssignmentOperator::LogicalAnd => Some(BinaryOp::And),
        AssignmentOperator::LogicalOr => Some(BinaryOp::Or),
        AssignmentOperator::LogicalNullish => Some(BinaryOp::Nullish),
        _ => {
            return Err("OXC adapter does not support compound assignment yet".into());
        }
    })
}

/// Compound and logical assignments to a member (target evaluated once) or a
/// logical assignment to an identifier. `None` falls through to the generic
/// `a = (a OP b)` desugaring.
fn convert_compound_fast_path(
    source: &str,
    assignment: &oxc_ast::ast::AssignmentExpression<'_>,
    op: BinaryOp,
) -> Result<Option<Expr>, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let whole = span(assignment.span);
    let logical = matches!(
        assignment.operator,
        AssignmentOperator::LogicalAnd
            | AssignmentOperator::LogicalOr
            | AssignmentOperator::LogicalNullish
    );
    let member = match &assignment.left {
        AssignmentTarget::StaticMemberExpression(m) => Some(MemberRef::Static(m)),
        AssignmentTarget::ComputedMemberExpression(m) => Some(MemberRef::Computed(m)),
        AssignmentTarget::PrivateFieldExpression(m) => Some(MemberRef::Private(m)),
        _ => None,
    };
    if let Some(member) = member {
        // The target's object and key are evaluated exactly once;
        // a logical assignment only stores when it does not
        // short-circuit.
        let (binds, read, store) = decompose_member(source, member)?;
        let rhs = convert_expression(source, &assignment.right)?;
        let body = if logical {
            Expr::Binary {
                op,
                lhs: Box::new(read),
                rhs: Box::new(store.build(rhs, whole)),
                span: whole,
            }
        } else {
            store.build(
                Expr::Binary { op, lhs: Box::new(read), rhs: Box::new(rhs), span: whole },
                whole,
            )
        };
        return Ok(Some(wrap_binds(binds, body, whole)));
    }
    if logical {
        if let AssignmentTarget::AssignmentTargetIdentifier(identifier) = &assignment.left {
            let name = Name { span: span(identifier.span) };
            let rhs = convert_expression(source, &assignment.right)?;
            return Ok(Some(Expr::Binary {
                op,
                lhs: Box::new(Expr::Ident(name)),
                rhs: Box::new(Expr::Assign { target: name, value: Box::new(rhs), span: whole }),
                span: whole,
            }));
        }
    }
    Ok(None)
}

/// The read half of `a OP= b`: the target read back as an expression.
/// Known v1 limitation: a target with side-effecting sub-expressions is
/// evaluated twice.
fn compound_read(source: &str, assignment: &oxc_ast::ast::AssignmentExpression<'_>) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    Ok(match &assignment.left {
        AssignmentTarget::AssignmentTargetIdentifier(identifier) => {
            Expr::Ident(Name { span: span(identifier.span) })
        }
        AssignmentTarget::StaticMemberExpression(member) => Expr::Field {
            object: Box::new(convert_expression(source, &member.object)?),
            field: Name { span: span(member.property.span) },
            span: span(member.span),
        },
        AssignmentTarget::ComputedMemberExpression(member) => Expr::Index {
            object: Box::new(convert_expression(source, &member.object)?),
            index: Box::new(convert_expression(source, &member.expression)?),
            span: span(member.span),
        },
        AssignmentTarget::PrivateFieldExpression(member) => Expr::PrivateField {
            object: Box::new(convert_expression(source, &member.object)?),
            field: Name { span: span(member.field.span) },
            span: span(member.span),
        },
        _ => {
            return Err(
                "OXC adapter supports identifier, field, and index assignments only"
                    .into(),
            );
        }
    })
}

pub(super) fn convert_assignment(source: &str, assignment: &oxc_ast::ast::AssignmentExpression<'_>) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let compound = compound_binary_op(assignment.operator)?;
    if let Some(op) = compound {
        if let Some(expr) = convert_compound_fast_path(source, assignment, op)? {
            return Ok(expr);
        }
    }
    let mut value = convert_expression(source, &assignment.right)?;
    if let Some(op) = compound {
        // `a OP= b` desugars to `a = (a OP b)`.
        let read = compound_read(source, assignment)?;
        value = Expr::Binary {
            op,
            lhs: Box::new(read),
            rhs: Box::new(value),
            span: span(assignment.span),
        };
    }
    store_assignment_value(source, assignment, Box::new(value))
}

fn store_assignment_value(
    source: &str,
    assignment: &oxc_ast::ast::AssignmentExpression<'_>,
    value: Box<Expr>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    match &assignment.left {
        AssignmentTarget::AssignmentTargetIdentifier(identifier)
            if is_immutable_self_name(identifier.name.as_str()) =>
        {
            Ok(throw_const_assignment(*value, span(assignment.span)))
        }
        AssignmentTarget::AssignmentTargetIdentifier(identifier) => Ok(Expr::Assign {
            target: Name {
                span: span(identifier.span),
            },
            value,
            span: span(assignment.span),
        }),
        AssignmentTarget::StaticMemberExpression(member) => Ok(Expr::FieldAssign {
            object: Box::new(convert_expression(source, &member.object)?),
            field: Name {
                span: span(member.property.span),
            },
            value,
            span: span(assignment.span),
        }),
        // `obj['name'] = v` with a plain identifier-shaped literal is `obj.name = v`.
        AssignmentTarget::ComputedMemberExpression(member)
            if matches!(&member.expression, Expression::StringLiteral(text)
                if text.value.as_str().is_some_and(is_plain_identifier)
                    && (text.span.end - text.span.start) as usize
                        == text.value.as_str().map_or(0, str::len) + 2) =>
        {
            let Expression::StringLiteral(text) = &member.expression else { unreachable!() };
            Ok(Expr::FieldAssign {
                object: Box::new(convert_expression(source, &member.object)?),
                field: Name { span: Span::new(text.span.start + 1, text.span.end - 1) },
                value,
                span: span(assignment.span),
            })
        }
        AssignmentTarget::ComputedMemberExpression(member) => Ok(Expr::IndexAssign {
            object: Box::new(convert_expression(source, &member.object)?),
            index: Box::new(convert_expression(source, &member.expression)?),
            value,
            span: span(assignment.span),
        }),
        // Sema lowers a private store from `FieldAssign` over a
        // `PrivateField` target (the shape the hand parser emits).
        AssignmentTarget::PrivateFieldExpression(member) => Ok(Expr::FieldAssign {
            object: Box::new(Expr::PrivateField {
                object: Box::new(convert_expression(source, &member.object)?),
                field: Name {
                    span: span(member.field.span),
                },
                span: span(member.span),
            }),
            field: Name {
                span: span(member.field.span),
            },
            value,
            span: span(assignment.span),
        }),
        // `(x as any) = v` assigns the identifier; the cast only
        // silences the type checker.
        AssignmentTarget::TSAsExpression(cast)
            if matches!(peel_parens(&cast.expression), Expression::Identifier(_)) =>
        {
            let Expression::Identifier(identifier) = peel_parens(&cast.expression) else { unreachable!() };
            if is_immutable_self_name(identifier.name.as_str()) {
                return Ok(throw_const_assignment(*value, span(assignment.span)));
            }
            Ok(Expr::Assign {
                target: Name { span: span(identifier.span) },
                value,
                span: span(assignment.span),
            })
        }
        _ => {
            Err("OXC adapter supports identifier, field, and index assignments only".into())
        }
    }
}

/// Whether this is a plain `=` whose target is an array/object pattern.
pub(super) fn is_destructuring_assignment(assignment: &oxc_ast::ast::AssignmentExpression<'_>) -> bool {
    assignment.operator == AssignmentOperator::Assign
        && matches!(
            &assignment.left,
            AssignmentTarget::ArrayAssignmentTarget(_) | AssignmentTarget::ObjectAssignmentTarget(_)
        )
}
