use super::{Checker, FunctionCx};
use crate::hir;
use zun_syntax::ast;

impl<'a> Checker<'a> {
    pub(super) fn check_condition(
        &mut self,
        cx: &mut FunctionCx,
        cond: &ast::Expr,
    ) -> hir::Expr {
        // JS evaluates any value for truthiness in a condition position; the
        // branch opcodes coerce at run time, so no type is rejected here.
        self.check_expr(cx, cond)
    }
}
