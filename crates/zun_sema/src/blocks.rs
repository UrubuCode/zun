use super::{Checker, FunctionCx, Scope};
use crate::hir;
use zun_syntax::ast;

impl<'a> Checker<'a> {
    pub(super) fn check_block(
        &mut self,
        cx: &mut FunctionCx,
        block: &ast::Block,
    ) -> Vec<hir::Stmt> {
        cx.scopes.push(Scope::new());
        let mut stmts: Vec<hir::Stmt> = Vec::with_capacity(block.statements.len());
        for statement in &block.statements {
            let lowered = self.check_stmt(cx, statement);
            // A top-level `let`/`const` stays a typed local, but is also mirrored
            // to the realm global store so function bodies can read it via
            // `globalGet` (JS module scope). The mirror reads the local back, so
            // its value and type in `<main>` are unaffected.
            if cx.is_module_scope {
                if let ast::Stmt::Let { name, .. } = statement {
                    let name_text = self.text(name).to_string();
                    if self.module_globals.contains(&name_text) {
                        if let hir::Stmt::Let { slot, .. } | hir::Stmt::BoxedLet { slot, .. } =
                            &lowered
                        {
                            let slot = *slot;
                            let slot_ty = cx.locals[slot as usize];
                            // Function values do not box to `Any`, so they cannot
                            // ride the realm global store; such a binding stays a
                            // local only (it is still callable within `<main>`).
                            if !matches!(slot_ty, crate::hir::Ty::Function(_)) {
                                let span = name.span;
                                let read = hir::Expr {
                                    kind: if cx.boxed.contains(&slot) {
                                        hir::ExprKind::CellLoad { slot }
                                    } else {
                                        hir::ExprKind::Local(slot)
                                    },
                                    ty: slot_ty,
                                    span,
                                };
                                stmts.push(lowered);
                                stmts.push(hir::Stmt::Expr(
                                    self.global_set_expr(&name_text, read, span),
                                ));
                                continue;
                            }
                        }
                    }
                }
            }
            stmts.push(lowered);
        }
        cx.scopes.pop();
        stmts
    }
}
