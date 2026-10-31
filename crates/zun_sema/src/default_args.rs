use super::{Checker, Signature};
use crate::hir;
use zun_common::Span;

impl<'a> Checker<'a> {
    /// Pad omitted trailing optional arguments with the null sentinel consumed
    /// by the callee's default-parameter prologue.
    pub(super) fn pad_default_args(
        &mut self,
        args: &mut Vec<hir::Expr>,
        sig: &Signature,
        span: Span,
    ) {
        let required = sig.params.len().saturating_sub(sig.optional_count);
        if args.len() < required || args.len() > sig.params.len() {
            self.error(
                span,
                format!(
                    "`{}` expects {} to {} argument(s), found {}",
                    sig.name,
                    required,
                    sig.params.len(),
                    args.len()
                ),
            );
            return;
        }
        while args.len() < sig.params.len() {
            let ty = sig.params[args.len()];
            args.push(missing_arg(ty, span));
        }
    }
}

/// The placeholder for an omitted argument: undefined for dynamic params (so
/// an explicit null is distinguishable and does not trigger the default),
/// the null sentinel for typed params.
pub(super) fn missing_arg(ty: hir::Ty, span: Span) -> hir::Expr {
    if ty == hir::Ty::Any {
        hir::Expr {
            kind: hir::ExprKind::Unary {
                op: hir::UnaryOp::Void,
                operand: Box::new(hir::Expr { kind: hir::ExprKind::Null, ty: hir::Ty::Any, span }),
            },
            ty: hir::Ty::Any,
            span,
        }
    } else {
        hir::Expr { kind: hir::ExprKind::Null, ty, span }
    }
}
