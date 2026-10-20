use super::*;

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
        ast::Expr::Field { object, .. } | ast::Expr::Index { object, .. } => {
            collect_mutated_expr(checker, object, names)
        }
        ast::Expr::OptionalField { object, .. } => collect_mutated_expr(checker, object, names),
        ast::Expr::FieldAssign { object, value, .. } => {
            collect_mutated_expr(checker, object, names);
            collect_mutated_expr(checker, value, names);
        }
        ast::Expr::IndexAssign { object, index, value, .. } => {
            collect_mutated_expr(checker, object, names);
            collect_mutated_expr(checker, index, names);
            collect_mutated_expr(checker, value, names);
        }
        ast::Expr::Function(_) | ast::Expr::Ident(_) | ast::Expr::Number { .. }
        | ast::Expr::String { .. } | ast::Expr::Template { .. }
        | ast::Expr::Bool { .. } | ast::Expr::Null { .. } => {}
        ast::Expr::Spread { expr, .. } => collect_mutated_expr(checker, expr, names),
    }
}
