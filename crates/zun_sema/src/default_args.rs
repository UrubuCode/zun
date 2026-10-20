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
            args.push(hir::Expr {
                kind: hir::ExprKind::Null,
                ty: sig.params[args.len()],
                span,
            });
        }
    }
}
