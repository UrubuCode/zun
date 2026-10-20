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
        let stmts = block
            .statements
            .iter()
            .map(|s| self.check_stmt(cx, s))
            .collect();
        cx.scopes.pop();
        stmts
    }
}
