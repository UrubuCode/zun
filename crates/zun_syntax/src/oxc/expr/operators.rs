use super::*;

pub(super) fn convert_binary(
    source: &str,
    binary: &oxc_ast::ast::BinaryExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let op = match binary.operator {
        BinaryOperator::Equality => BinaryOp::LooseEq,
        BinaryOperator::StrictEquality => BinaryOp::Eq,
        BinaryOperator::Inequality => BinaryOp::LooseNe,
        BinaryOperator::StrictInequality => BinaryOp::Ne,
        BinaryOperator::LessThan => BinaryOp::Lt,
        BinaryOperator::LessEqualThan => BinaryOp::Le,
        BinaryOperator::GreaterThan => BinaryOp::Gt,
        BinaryOperator::GreaterEqualThan => BinaryOp::Ge,
        BinaryOperator::Addition => BinaryOp::Add,
        BinaryOperator::Subtraction => BinaryOp::Sub,
        BinaryOperator::Multiplication => BinaryOp::Mul,
        BinaryOperator::Division => BinaryOp::Div,
        BinaryOperator::Remainder => BinaryOp::Rem,
        BinaryOperator::Exponential => BinaryOp::Pow,
        BinaryOperator::BitwiseAnd => BinaryOp::BitAnd,
        BinaryOperator::BitwiseOR => BinaryOp::BitOr,
        BinaryOperator::BitwiseXOR => BinaryOp::BitXor,
        BinaryOperator::ShiftLeft => BinaryOp::Shl,
        BinaryOperator::ShiftRight => BinaryOp::Shr,
        BinaryOperator::ShiftRightZeroFill => BinaryOp::UShr,
        BinaryOperator::In => BinaryOp::In,
        BinaryOperator::Instanceof => BinaryOp::Instanceof,
    };
    let lhs = convert_expression(source, &binary.left)?;
    let mut rhs = convert_expression(source, &binary.right)?;
    // `x instanceof Name` with `Name` bound to a lowered class tests against
    // the value that binding holds: the checker reads a bare identifier as a
    // class name first, and a cast (`Name as any`) as a run-time value.
    if let (BinaryOperator::Instanceof, Expression::Identifier(identifier)) =
        (binary.operator, &binary.right)
    {
        if super::super::runtime_class::names_runtime_class(identifier.name.as_str()) {
            rhs = Expr::Cast {
                expr: Box::new(rhs),
                ty: crate::ast::TypeAnnotation {
                    name: crate::ast::synthetic_ident("any"),
                    is_array: false,
                    array_depth: 0,
                    nullable: false,
                },
                span: span(identifier.span),
            };
        }
    }
    Ok(Expr::Binary {
        op,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
        span: span(binary.span),
    })
}

pub(super) fn convert_unary(
    source: &str,
    unary: &oxc_ast::ast::UnaryExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    // Unary plus (`+x`) is numeric coercion; the language has no such
    // operator, so desugar to `x - 0`, which applies ToNumber to the
    // operand with the same result for every primitive.
    if matches!(unary.operator, UnaryOperator::UnaryPlus) {
        // `x as any - 0` boxes the operand so the subtraction takes the
        // dynamic arithmetic path, which applies ToNumber at run time.
        let operand = convert_expression(source, &unary.argument)?;
        let boxed = Expr::Cast {
            expr: Box::new(operand),
            ty: crate::ast::TypeAnnotation {
                name: Name {
                    span: Span::new(unary.span.start, unary.span.start),
                },
                is_array: false,
                array_depth: 0,
                nullable: false,
            },
            span: span(unary.span),
        };
        return Ok(Expr::Binary {
            op: BinaryOp::Sub,
            lhs: Box::new(boxed),
            rhs: Box::new(Expr::Number {
                value: 0.0,
                span: span(unary.span),
            }),
            span: span(unary.span),
        });
    }
    let op = match unary.operator {
        UnaryOperator::UnaryNegation => crate::ast::UnaryOp::Neg,
        UnaryOperator::LogicalNot => crate::ast::UnaryOp::Not,
        UnaryOperator::BitwiseNot => crate::ast::UnaryOp::BitNot,
        UnaryOperator::Void => crate::ast::UnaryOp::Void,
        UnaryOperator::Typeof => crate::ast::UnaryOp::Typeof,
        UnaryOperator::Delete => crate::ast::UnaryOp::Delete,
        _ => return Err("OXC adapter does not support this unary operator yet".into()),
    };
    Ok(Expr::Unary {
        op,
        operand: Box::new(convert_expression(source, &unary.argument)?),
        span: span(unary.span),
    })
}

pub(super) fn convert_logical(
    source: &str,
    logical: &oxc_ast::ast::LogicalExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let op = match logical.operator {
        LogicalOperator::And => BinaryOp::And,
        LogicalOperator::Or => BinaryOp::Or,
        LogicalOperator::Coalesce => BinaryOp::Nullish,
    };
    Ok(Expr::Binary {
        op,
        lhs: Box::new(convert_expression(source, &logical.left)?),
        rhs: Box::new(convert_expression(source, &logical.right)?),
        span: span(logical.span),
    })
}
