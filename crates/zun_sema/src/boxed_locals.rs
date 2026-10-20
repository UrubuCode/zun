use super::*;

pub(super) fn rewrite_boxed_lets(
    statements: &mut [hir::Stmt],
    boxed: &HashSet<hir::LocalSlot>,
) {
    for statement in statements {
        match statement {
            hir::Stmt::Let { slot, init } if boxed.contains(slot) => {
                let span = init.span;
                let slot = *slot;
                let value = std::mem::replace(
                    init,
                    hir::Expr {
                        kind: hir::ExprKind::Error,
                        ty: Ty::Error,
                        span,
                    },
                );
                *statement = hir::Stmt::BoxedLet { slot, init: value };
            }
            hir::Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                rewrite_boxed_lets(then_body, boxed);
                rewrite_boxed_lets(else_body, boxed);
            }
            hir::Stmt::While { body, .. } | hir::Stmt::DoWhile { body, .. } => {
                rewrite_boxed_lets(body, boxed)
            }
            hir::Stmt::For { init, body, .. } => {
                if let Some(init) = init {
                    rewrite_boxed_lets(std::slice::from_mut(init.as_mut()), boxed);
                }
                rewrite_boxed_lets(body, boxed);
            }
            hir::Stmt::Try {
                body,
                catches,
                finally_body,
            } => {
                rewrite_boxed_lets(body, boxed);
                for catch in catches {
                    rewrite_boxed_lets(&mut catch.body, boxed);
                }
                if let Some(finally_body) = finally_body {
                    rewrite_boxed_lets(finally_body, boxed);
                }
            }
            _ => {}
        }
    }
}
