//! Expression checking: a thin dispatcher over the per-kind helpers in the
//! submodules (`check_expr` is a single `match` over `ast::Expr`).

use crate::check::*;

mod assign;
mod call;
mod call_builtins;
mod call_coerce;
mod call_dispatch;
mod call_indirect;
mod call_sugar;
mod field;
mod ident;
mod literal;
mod mcall_dispatch;
mod mcall_buffers;
mod mcall_string;
mod mcall_managed;
mod mcall_namespaces;
mod mcall_number;
mod mcall_statics;
mod member;
mod method_call;
mod new;
mod new_builtin;
mod new_class;
mod new_realm;
mod operators;
mod optional;
mod spread_optional;

impl Checker<'_> {
    pub(in crate::check) fn check_expr(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let span = expr.span();
        match expr {
            ast::Expr::Await { .. } => self.check_await(cx, expr),
            ast::Expr::ImportCall { .. } => self.check_import_call(cx, expr),
            ast::Expr::ObjectLiteral { .. } => self.check_object_literal(cx, expr),
            ast::Expr::ArrayLiteral { elements, .. } => self.lower_array_literal(cx, elements, span),
            ast::Expr::Yield { .. } => self.check_yield(cx, expr),
            ast::Expr::Number { value, .. } => hir::Expr {
                kind: hir::ExprKind::Number(*value),
                ty: Ty::F64,
                span,
            },
            ast::Expr::BigInt { .. } => self.check_bigint_literal(cx, expr),
            ast::Expr::String { value, .. } => hir::Expr {
                kind: hir::ExprKind::String(value.clone()),
                ty: Ty::String,
                span,
            },
            ast::Expr::RegexLiteral { .. } => self.check_regex_literal(cx, expr),
            ast::Expr::Template { .. } => self.check_template(cx, expr),
            ast::Expr::TaggedTemplate { .. } => self.check_tagged_template_expr(cx, expr),
            ast::Expr::Bool { value, .. } => hir::Expr {
                kind: hir::ExprKind::Boolean(*value),
                ty: Ty::Boolean,
                span,
            },
            ast::Expr::Null { .. } => hir::Expr {
                kind: hir::ExprKind::Null,
                ty: Ty::Any,
                span,
            },
            ast::Expr::Ident(name) if self.text(name) == "new.target" => self.check_new_target(cx, expr),
            ast::Expr::Ident(_) => self.check_ident(cx, expr),
            ast::Expr::Assign { .. } => self.check_assign(cx, expr),
            ast::Expr::Binary { .. } => self.check_binary_expr(cx, expr),
            ast::Expr::Conditional { .. } => self.check_conditional(cx, expr),
            ast::Expr::Unary { .. } => self.check_unary(cx, expr),
            ast::Expr::Call { .. } => self.check_call(cx, expr),
            ast::Expr::MethodCall { .. } => self.check_method_call(cx, expr),
            ast::Expr::StaticMethodCall { .. } => self.check_static_method_call(cx, expr),
            ast::Expr::StaticField { .. } => self.check_static_field(cx, expr),
            ast::Expr::PrivateField { .. } => self.check_private_field(cx, expr),
            ast::Expr::PrivateFieldAssign { .. } => {
                self.error(span, "private field assignment is not lowered yet");
                self.poison(span)
            }
            ast::Expr::Cast { .. } => self.check_cast(cx, expr),
            ast::Expr::NewDynamic { .. } => self.check_new_dynamic(cx, expr),
            ast::Expr::New { .. } => self.check_new(cx, expr),
            ast::Expr::Field { .. } => self.check_field(cx, expr),
            ast::Expr::ChainRoot { .. } => self.check_chain_root(cx, expr),
            ast::Expr::TempBind { .. } => self.check_temp_bind(cx, expr),
            ast::Expr::OptionalChain { .. } => self.check_optional_chain(cx, expr),
            ast::Expr::OptionalField { .. } => self.check_optional_field(cx, expr),
            ast::Expr::OptionalIndex { .. } => self.check_optional_index(cx, expr),
            ast::Expr::OptionalCall { .. } => self.check_optional_call(cx, expr),
            ast::Expr::OptionalMethodCall { .. } => self.check_optional_method_call(cx, expr),
            ast::Expr::Index { .. } => self.check_index(cx, expr),
            ast::Expr::IndexAssign { .. } => self.check_index_assign(cx, expr),
            ast::Expr::FieldAssign { .. } => self.check_field_assign(cx, expr),
            ast::Expr::Spread { expr, .. } => {
                self.error(span, "spread arguments are not lowered yet");
                self.check_expr(cx, expr);
                self.poison(span)
            }
            ast::Expr::Function(_) => self.check_function_expr(cx, expr),
            ast::Expr::ComputedCall { callee, args, .. } if self.is_regex_symbol_call(cx, callee, args) => self.check_regex_symbol_call_expr(cx, expr),
            ast::Expr::ComputedCall { .. } => self.check_computed_call(cx, expr),
        }
    }
}
