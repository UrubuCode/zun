use super::*;

pub(super) fn collect_mutated_names(
    checker: &Checker<'_>,
    statements: &[ast::Stmt],
    names: &mut HashSet<String>,
) {
    for statement in statements {
        match statement {
            ast::Stmt::Let { init, .. }
            | ast::Stmt::ArrayDestructure { init, .. }
            | ast::Stmt::ObjectDestructure { init, .. }
            | ast::Stmt::Expr(init) => collect_mutated_expr(checker, init, names),
            ast::Stmt::If { cond, then_block, else_block, .. } => {
                collect_mutated_expr(checker, cond, names);
                collect_mutated_names(checker, &then_block.statements, names);
                if let Some(block) = else_block { collect_mutated_names(checker, &block.statements, names); }
            }
            ast::Stmt::While { cond, body, .. } => {
                collect_mutated_expr(checker, cond, names);
                collect_mutated_names(checker, &body.statements, names);
            }
            ast::Stmt::DoWhile { body, cond, .. } => {
                collect_mutated_names(checker, &body.statements, names);
                collect_mutated_expr(checker, cond, names);
            }
            ast::Stmt::For { init, cond, update, body, .. } => {
                if let Some(init) = init { collect_mutated_names(checker, std::slice::from_ref(init), names); }
                if let Some(cond) = cond { collect_mutated_expr(checker, cond, names); }
                if let Some(update) = update { collect_mutated_expr(checker, update, names); }
                collect_mutated_names(checker, &body.statements, names);
            }
            ast::Stmt::ForOf { iterable, body, .. } => {
                collect_mutated_expr(checker, iterable, names);
                collect_mutated_names(checker, &body.statements, names);
            }
            ast::Stmt::Return { value, .. } => {
                if let Some(value) = value { collect_mutated_expr(checker, value, names); }
            }
            ast::Stmt::Throw { value, .. } => collect_mutated_expr(checker, value, names),
            ast::Stmt::Try { body, catches, finally, .. } => {
                collect_mutated_names(checker, &body.statements, names);
                for catch in catches { collect_mutated_names(checker, &catch.body.statements, names); }
                if let Some(block) = finally { collect_mutated_names(checker, &block.statements, names); }
            }
            ast::Stmt::Labeled { statement, .. } => collect_mutated_names(checker, std::slice::from_ref(statement), names),
            ast::Stmt::Break { .. } | ast::Stmt::Continue { .. } => {}
        }
    }
}

pub(super) fn collect_mutated_expr(
    checker: &Checker<'_>,
    expr: &ast::Expr,
    names: &mut HashSet<String>,
) {
    match expr {
        ast::Expr::Assign { target, value, .. } => {
            names.insert(checker.text(target).to_string());
            collect_mutated_expr(checker, value, names);
        }
        ast::Expr::Binary { lhs, rhs, .. } => {
            collect_mutated_expr(checker, lhs, names);
            collect_mutated_expr(checker, rhs, names);
        }
        ast::Expr::Conditional { condition, when_true, when_false, .. } => {
            collect_mutated_expr(checker, condition, names);
            collect_mutated_expr(checker, when_true, names);
            collect_mutated_expr(checker, when_false, names);
        }
        ast::Expr::Unary { operand, .. } | ast::Expr::Cast { expr: operand, .. } => {
            collect_mutated_expr(checker, operand, names)
        }
        ast::Expr::Await { operand, .. } => collect_mutated_expr(checker, operand, names),
        ast::Expr::ImportCall { specifier, .. } => collect_mutated_expr(checker, specifier, names),
        ast::Expr::Yield { value, .. } => collect_mutated_expr(checker, value, names),
        ast::Expr::Call { args, .. } | ast::Expr::New { args, .. } => {
            for arg in args { collect_mutated_expr(checker, arg, names); }
        }
        ast::Expr::ComputedCall { callee, args, .. } => {
            collect_mutated_expr(checker, callee, names);
            for arg in args { collect_mutated_expr(checker, arg, names); }
        }
        ast::Expr::MethodCall { receiver, args, .. } => {
            collect_mutated_expr(checker, receiver, names);
            for arg in args { collect_mutated_expr(checker, arg, names); }
        }
        ast::Expr::OptionalMethodCall { object, args, .. } => {
            collect_mutated_expr(checker, object, names);
            for arg in args { collect_mutated_expr(checker, arg, names); }
        }
        ast::Expr::StaticMethodCall { args, .. } => {
            for arg in args { collect_mutated_expr(checker, arg, names); }
        }
        ast::Expr::StaticField { .. } => {}
        ast::Expr::Field { object, .. } | ast::Expr::Index { object, .. } => {
            collect_mutated_expr(checker, object, names)
        }
        ast::Expr::PrivateField { object, .. } => collect_mutated_expr(checker, object, names),
        ast::Expr::OptionalField { object, .. } => collect_mutated_expr(checker, object, names),
        ast::Expr::OptionalIndex { object, index, .. } => {
            collect_mutated_expr(checker, object, names);
            collect_mutated_expr(checker, index, names);
        }
        ast::Expr::OptionalCall { callee, args, .. } => {
            collect_mutated_expr(checker, callee, names);
            for arg in args { collect_mutated_expr(checker, arg, names); }
        }
        ast::Expr::FieldAssign { object, value, .. } => {
            collect_mutated_expr(checker, object, names);
            collect_mutated_expr(checker, value, names);
        }
        ast::Expr::PrivateFieldAssign { object, value, .. } => {
            collect_mutated_expr(checker, object, names);
            collect_mutated_expr(checker, value, names);
        }
        ast::Expr::IndexAssign { object, index, value, .. } => {
            collect_mutated_expr(checker, object, names);
            collect_mutated_expr(checker, index, names);
            collect_mutated_expr(checker, value, names);
        }
        ast::Expr::Function(_) | ast::Expr::Ident(_) | ast::Expr::Number { .. }
        | ast::Expr::BigInt { .. }
        | ast::Expr::String { .. } | ast::Expr::Template { .. } | ast::Expr::RegexLiteral { .. }
        | ast::Expr::Bool { .. } | ast::Expr::Null { .. } => {}
        ast::Expr::Spread { expr, .. } => collect_mutated_expr(checker, expr, names),
        ast::Expr::ObjectLiteral { properties, .. } => {
            for property in properties {
                if let ast::PropertyKey::Computed(key) = &property.key {
                    collect_mutated_expr(checker, key, names);
                }
                collect_mutated_expr(checker, &property.value, names);
            }
        }
    }
}
