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
            // Static initializers of classes declared above this statement run
            // here, in declaration order, with the module bindings in scope.
            if cx.is_module_scope && cx.scopes.len() == 2 && !self.pending_static_classes.is_empty()
            {
                if let Some(start) = Self::stmt_start(statement) {
                    while let Some(&class_idx) = self.pending_static_classes.front() {
                        if self.classes[class_idx as usize].span_start >= start {
                            break;
                        }
                        self.pending_static_classes.pop_front();
                        let init = self.class_static_initializers(cx, class_idx);
                        stmts.extend(init);
                    }
                }
            }
            let lowered = self.check_stmt(cx, statement);
            // A top-level `let`/`const` stays a typed local, but is also mirrored
            // to the realm global store so function bodies can read it via
            // `globalGet` (JS module scope). The mirror reads the local back, so
            // its value and type in `<main>` are unaffected.
            if cx.is_module_scope && cx.scopes.len() == 2 {
                if let ast::Stmt::Let { name, .. } = statement {
                    let name_text = self.text(name).to_string();
                    if self.module_globals.contains(&name_text)
                        && self.globals_read_in_functions.contains(&name_text)
                    {
                        if let hir::Stmt::Let { slot, init: _ }
                        | hir::Stmt::BoxedLet { slot, init: _ } = &lowered
                        {
                            let slot = *slot;
                            let slot_ty = cx.locals[slot as usize];
                            // Function values ride the global object function slot (see
                            // `globalSet`), so function-valued bindings are mirrored too and
                            // callable from other functions.
                            {
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
        // Anything still pending (a declaration at the tail) runs at the end.
        if cx.is_module_scope && cx.scopes.len() == 2 {
            while !self.pending_static_classes.is_empty() {
                let class_idx = self
                    .pending_static_classes
                    .pop_front()
                    .expect("pending static class");
                let init = self.class_static_initializers(cx, class_idx);
                stmts.extend(init);
            }
        }
        cx.scopes.pop();
        stmts
    }
}
