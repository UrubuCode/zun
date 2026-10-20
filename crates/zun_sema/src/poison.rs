use super::Checker;
use crate::hir;
use zun_common::Span;

impl<'a> Checker<'a> {
    pub(super) fn poison(&mut self, span: Span) -> hir::Expr {
        hir::Expr {
            kind: hir::ExprKind::Error,
            ty: hir::Ty::Error,
            span,
        }
    }
}
