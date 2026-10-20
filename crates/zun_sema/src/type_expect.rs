use super::Checker;
use crate::hir;
use crate::hir::Ty;

impl<'a> Checker<'a> {
    pub(super) fn expect_ty(&mut self, expr: &hir::Expr, expected: Ty) -> Ty {
        if expr.ty.is_error() {
            Ty::Error
        } else if expr.ty == expected {
            expected
        } else {
            self.error(
                expr.span,
                format!("expected {expected:?}, found {:?}", expr.ty),
            );
            Ty::Error
        }
    }
}
