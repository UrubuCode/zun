use super::{Checker, FunctionCx};
use crate::hir;
use crate::hir::Ty;
use zun_syntax::ast;

impl<'a> Checker<'a> {
    pub(super) fn check_condition(
        &mut self,
        cx: &mut FunctionCx,
        cond: &ast::Expr,
    ) -> hir::Expr {
        let cond = self.check_expr(cx, cond);
        if !cond.ty.is_error() && cond.ty != Ty::Boolean {
            self.error(
                cond.span,
                format!("condition must be boolean, found {:?}", cond.ty),
            );
        }
        cond
    }
}
