use crate::check::*;

/// Whether a function body has a `return <value>` (so an unannotated return
/// type infers to `any` rather than `void`). Recurses into control-flow blocks
/// but not into expressions, so a nested closure's returns are excluded.
pub(in crate::check) fn block_returns_value(block: &ast::Block) -> bool {
    block.statements.iter().any(stmt_returns_value)
}

pub(in crate::check) fn stmt_returns_value(stmt: &ast::Stmt) -> bool {
    match stmt {
        ast::Stmt::Return { value: Some(_), .. } => true,
        ast::Stmt::If { then_block, else_block, .. } => {
            block_returns_value(then_block)
                || else_block.as_ref().is_some_and(block_returns_value)
        }
        ast::Stmt::While { body, .. }
        | ast::Stmt::DoWhile { body, .. }
        | ast::Stmt::For { body, .. }
        | ast::Stmt::ForOf { body, .. }
        | ast::Stmt::ForIn { body, .. } => block_returns_value(body),
        ast::Stmt::Labeled { statement, .. } => stmt_returns_value(statement),
        ast::Stmt::Switch { cases, .. } => cases.iter().any(|c| block_returns_value(&c.body)),
        ast::Stmt::Try { body, catches, finally, .. } => {
            block_returns_value(body)
                || catches.iter().any(|clause| block_returns_value(&clause.body))
                || finally.as_ref().is_some_and(block_returns_value)
        }
        _ => false,
    }
}

pub(in crate::check) fn count_awaits_in_block(block: &ast::Block) -> usize {
    block.statements.iter().map(count_awaits_in_stmt).sum()
}

pub(in crate::check) fn count_yields_in_block(block: &ast::Block) -> usize {
    block.statements.iter().map(count_yields_in_stmt).sum()
}

pub(in crate::check) fn count_yields_in_stmt(stmt: &ast::Stmt) -> usize {
    use ast::Stmt::*;
    match stmt {
        Let { init, .. } | Destructure { init, .. } => count_yields_in_expr(init),
        Expr(expr) => count_yields_in_expr(expr),
        If { cond, then_block, else_block, .. } => count_yields_in_expr(cond) + count_yields_in_block(then_block) + else_block.as_ref().map_or(0, count_yields_in_block),
        While { cond, body, .. } => count_yields_in_expr(cond) + count_yields_in_block(body),
        DoWhile { body, cond, .. } => count_yields_in_block(body) + count_yields_in_expr(cond),
        For { init, cond, update, body, .. } => init.as_deref().map_or(0, count_yields_in_stmt) + cond.as_ref().map_or(0, count_yields_in_expr) + update.as_ref().map_or(0, count_yields_in_expr) + count_yields_in_block(body),
        ForOf { iterable, body, .. } => count_yields_in_expr(iterable) + count_yields_in_block(body),
        ForIn { object, body, .. } => count_yields_in_expr(object) + count_yields_in_block(body),
        Switch { discriminant, cases, .. } => count_yields_in_expr(discriminant) + cases.iter().map(|c| c.test.as_ref().map_or(0, count_yields_in_expr) + count_yields_in_block(&c.body)).sum::<usize>(),
        Labeled { statement, .. } => count_yields_in_stmt(statement),
        Break { .. } | Continue { .. } => 0,
        Return { value, .. } => value.as_ref().map_or(0, count_yields_in_expr),
        Throw { value, .. } => count_yields_in_expr(value),
        Try { body, catches, finally, .. } => count_yields_in_block(body) + catches.iter().map(|c| count_yields_in_block(&c.body)).sum::<usize>() + finally.as_ref().map_or(0, count_yields_in_block),
    }
}

pub(in crate::check) fn count_yields_in_expr(expr: &ast::Expr) -> usize {
    use ast::Expr::*;
    match expr {
        Yield { value, .. } => 1 + count_yields_in_expr(value),
        ImportCall { specifier, .. } => count_yields_in_expr(specifier),
        Assign { value, .. } | Await { operand: value, .. } | Unary { operand: value, .. } | Spread { expr: value, .. } | Cast { expr: value, .. } => count_yields_in_expr(value),
        Binary { lhs, rhs, .. } => count_yields_in_expr(lhs) + count_yields_in_expr(rhs),
        Conditional { condition, when_true, when_false, .. } => count_yields_in_expr(condition) + count_yields_in_expr(when_true) + count_yields_in_expr(when_false),
        Call { args, .. } | New { args, .. } => args.iter().map(count_yields_in_expr).sum(),
        ComputedCall { callee, args, .. } | NewDynamic { callee, args, .. } => count_yields_in_expr(callee) + args.iter().map(count_yields_in_expr).sum::<usize>(),
        MethodCall { receiver, args, .. } | OptionalMethodCall { object: receiver, args, .. } => count_yields_in_expr(receiver) + args.iter().map(count_yields_in_expr).sum::<usize>(),
        OptionalField { object, .. } | Field { object, .. } | PrivateField { object, .. } => count_yields_in_expr(object),
        OptionalIndex { object, index, .. } => count_yields_in_expr(object) + count_yields_in_expr(index),
        ChainRoot { .. } => 0,
        TempBind { value, body, .. } => count_yields_in_expr(value) + count_yields_in_expr(body),
        OptionalChain { object, body, .. } => count_yields_in_expr(object) + count_yields_in_expr(body),
        OptionalCall { callee, args, .. } => count_yields_in_expr(callee) + args.iter().map(count_yields_in_expr).sum::<usize>(),
        FieldAssign { object, value, .. } | PrivateFieldAssign { object, value, .. } => count_yields_in_expr(object) + count_yields_in_expr(value),
        Index { object, index, .. } => count_yields_in_expr(object) + count_yields_in_expr(index),
        IndexAssign { object, index, value, .. } => count_yields_in_expr(object) + count_yields_in_expr(index) + count_yields_in_expr(value),
        Template { parts, .. } => parts.iter().map(|p| match p { ast::TemplatePart::Text(_) => 0, ast::TemplatePart::Expr(e) => count_yields_in_expr(e) }).sum(),
        TaggedTemplate { tag, exprs, .. } => count_yields_in_expr(tag) + exprs.iter().map(|e| count_yields_in_expr(e)).sum::<usize>(),
        Function(_) | Number { .. } | BigInt { .. } | String { .. } | RegexLiteral { .. } | Bool { .. } | Null { .. } | Ident(_) => 0,
        StaticMethodCall { args, .. } => args.iter().map(count_yields_in_expr).sum(),
        StaticField { .. } => 0,
        ObjectLiteral { properties, .. } => properties
            .iter()
            .map(|member| match member {
                ast::ObjectMember::Property(property) => {
                    let key = match &property.key {
                        ast::PropertyKey::Computed(key) => count_yields_in_expr(key),
                        _ => 0,
                    };
                    key + count_yields_in_expr(&property.value)
                }
                ast::ObjectMember::Spread { expr, .. } => count_yields_in_expr(expr),
            })
            .sum(),
        ArrayLiteral { elements, .. } => elements
            .iter()
            .map(|element| match element {
                ast::ArrayElement::Expr(expr) | ast::ArrayElement::Spread(expr) => {
                    count_yields_in_expr(expr)
                }
                ast::ArrayElement::Hole => 0,
            })
            .sum(),
    }
}

pub(in crate::check) fn count_awaits_in_stmt(stmt: &ast::Stmt) -> usize {
    use ast::Stmt::*;
    match stmt {
        Let { init, .. } | Destructure { init, .. } => count_awaits_in_expr(init),
        Expr(expr) => count_awaits_in_expr(expr),
        If { cond, then_block, else_block, .. } => {
            count_awaits_in_expr(cond)
                + count_awaits_in_block(then_block)
                + else_block.as_ref().map_or(0, count_awaits_in_block)
        }
        While { cond, body, .. } => count_awaits_in_expr(cond) + count_awaits_in_block(body),
        DoWhile { body, cond, .. } => count_awaits_in_block(body) + count_awaits_in_expr(cond),
        For { init, cond, update, body, .. } => {
            init.as_deref().map_or(0, count_awaits_in_stmt)
                + cond.as_ref().map_or(0, count_awaits_in_expr)
                + update.as_ref().map_or(0, count_awaits_in_expr)
                + count_awaits_in_block(body)
        }
        ForOf { iterable, body, .. } => count_awaits_in_expr(iterable) + count_awaits_in_block(body),
        ForIn { object, body, .. } => count_awaits_in_expr(object) + count_awaits_in_block(body),
        Switch { discriminant, cases, .. } => count_awaits_in_expr(discriminant) + cases.iter().map(|c| c.test.as_ref().map_or(0, count_awaits_in_expr) + count_awaits_in_block(&c.body)).sum::<usize>(),
        Labeled { statement, .. } => count_awaits_in_stmt(statement),
        Break { .. } | Continue { .. } => 0,
        Return { value, .. } => value.as_ref().map_or(0, count_awaits_in_expr),
        Throw { value, .. } => count_awaits_in_expr(value),
        Try { body, catches, finally, .. } => {
            count_awaits_in_block(body)
                + catches.iter().map(|catch| count_awaits_in_block(&catch.body)).sum::<usize>()
                + finally.as_ref().map_or(0, count_awaits_in_block)
        }
    }
}

pub(in crate::check) fn count_awaits_in_expr(expr: &ast::Expr) -> usize {
    use ast::Expr::*;
    match expr {
        Function(_) | Number { .. } | BigInt { .. } | String { .. } | RegexLiteral { .. } | Bool { .. } | Null { .. } | Ident(_) => 0,
        Template { parts, .. } => parts
            .iter()
            .map(|part| match part {
                ast::TemplatePart::Text(_) => 0,
                ast::TemplatePart::Expr(expr) => count_awaits_in_expr(expr),
            })
            .sum(),
        TaggedTemplate { tag, exprs, .. } => count_awaits_in_expr(tag) + exprs.iter().map(|e| count_awaits_in_expr(e)).sum::<usize>(),
        ImportCall { specifier, .. } => count_awaits_in_expr(specifier),
        Assign { value, .. } | Await { operand: value, .. } | Yield { value, .. } | Unary { operand: value, .. }
        | Spread { expr: value, .. } | Cast { expr: value, .. } => {
            usize::from(matches!(expr, Await { .. })) + count_awaits_in_expr(value)
        }
        Binary { lhs, rhs, .. } => count_awaits_in_expr(lhs) + count_awaits_in_expr(rhs),
        Conditional { condition, when_true, when_false, .. } => {
            count_awaits_in_expr(condition)
                + count_awaits_in_expr(when_true)
                + count_awaits_in_expr(when_false)
        }
        Call { args, .. } => args.iter().map(count_awaits_in_expr).sum(),
        ComputedCall { callee, args, .. } | NewDynamic { callee, args, .. } => {
            count_awaits_in_expr(callee) + args.iter().map(count_awaits_in_expr).sum::<usize>()
        }
        MethodCall { receiver, args, .. } => {
            count_awaits_in_expr(receiver) + args.iter().map(count_awaits_in_expr).sum::<usize>()
        }
        StaticMethodCall { args, .. } => args.iter().map(count_awaits_in_expr).sum(),
        StaticField { .. } => 0,
        OptionalField { object, .. } => count_awaits_in_expr(object),
        OptionalIndex { object, index, .. } => count_awaits_in_expr(object) + count_awaits_in_expr(index),
        ChainRoot { .. } => 0,
        TempBind { value, body, .. } => count_awaits_in_expr(value) + count_awaits_in_expr(body),
        OptionalChain { object, body, .. } => count_awaits_in_expr(object) + count_awaits_in_expr(body),
        OptionalCall { callee, args, .. } => count_awaits_in_expr(callee) + args.iter().map(count_awaits_in_expr).sum::<usize>(),
        OptionalMethodCall { object, args, .. } => {
            count_awaits_in_expr(object) + args.iter().map(count_awaits_in_expr).sum::<usize>()
        }
        New { args, .. } => args.iter().map(count_awaits_in_expr).sum(),
        Field { object, .. } | PrivateField { object, .. } => count_awaits_in_expr(object),
        FieldAssign { object, value, .. } | PrivateFieldAssign { object, value, .. } => count_awaits_in_expr(object) + count_awaits_in_expr(value),
        Index { object, index, .. } => count_awaits_in_expr(object) + count_awaits_in_expr(index),
        IndexAssign { object, index, value, .. } => {
            count_awaits_in_expr(object) + count_awaits_in_expr(index) + count_awaits_in_expr(value)
        }
        ObjectLiteral { properties, .. } => properties
            .iter()
            .map(|member| match member {
                ast::ObjectMember::Property(property) => {
                    let key = match &property.key {
                        ast::PropertyKey::Computed(key) => count_awaits_in_expr(key),
                        _ => 0,
                    };
                    key + count_awaits_in_expr(&property.value)
                }
                ast::ObjectMember::Spread { expr, .. } => count_awaits_in_expr(expr),
            })
            .sum(),
        ArrayLiteral { elements, .. } => elements
            .iter()
            .map(|element| match element {
                ast::ArrayElement::Expr(expr) | ast::ArrayElement::Spread(expr) => {
                    count_awaits_in_expr(expr)
                }
                ast::ArrayElement::Hole => 0,
            })
            .sum(),
    }
}
