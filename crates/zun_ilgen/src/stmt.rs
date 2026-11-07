use super::*;

impl<'a, 'b> FnLowerer<'a, 'b> {
    pub(super) fn stmts(&mut self, stmts: &'a [Stmt]) {
        for s in stmts {
            self.stmt(s);
        }
    }

    pub(super) fn stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Let { slot, init } => {
                self.expr(init);
                self.emit(Instruction::new(Opcode::StoreLocal, Operand::Local(*slot)));
            }
            Stmt::Group(statements) => {
                for statement in statements {
                    self.stmt(statement);
                }
            }
            Stmt::BoxedLet { slot, init } => {
                self.emit(Instruction::simple(Opcode::NewDynamic));
                self.emit(Instruction::simple(Opcode::Dup));
                self.expr(init);
                self.emit(Instruction::simple(Opcode::BoxAny));
                self.emit_dynamic_store("$value");
                self.emit(Instruction::simple(Opcode::Pop));
                self.emit(Instruction::new(Opcode::StoreLocal, Operand::Local(*slot)));
            }
            Stmt::Expr(e) => {
                // Every expression leaves exactly one value on the stack in this
                // model — including void calls, which push a void value — so an
                // expression statement always discards one.
                self.expr(e);
                {
                    self.emit(Instruction::simple(Opcode::Pop));
                }
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => self.lower_if(cond, then_body, else_body),
            Stmt::While { cond, body } => self.lower_while(cond, body),
            Stmt::DoWhile { body, cond } => self.lower_do_while(body, cond),
            Stmt::For {
                init,
                cond,
                update,
                body,
            } => self.lower_for(init, cond, update, body),
            Stmt::ForOf {
                iterator,
                iterator_slot,
                done,
                next,
                binding,
                body,
            } => self.lower_for_of(iterator, iterator_slot, done, next, binding, body),
            Stmt::Labeled { label, body } => self.lower_labeled(label, body),
            Stmt::Break(label) => self.lower_break(label),
            Stmt::Continue(label) => self.lower_continue(label),
            Stmt::Return(value) => self.lower_return(value),
            Stmt::Throw(value) => {
                self.expr(value);
                self.emit(Instruction::simple(Opcode::Throw));
            }
            Stmt::Try {
                body,
                catches,
                finally_body,
            } => self.lower_try(body, catches, finally_body),
        }
    }

    /// `if (cond) { then } else { otherwise }`.
    fn lower_if(&mut self, cond: &hir::Expr, then_body: &'a [Stmt], else_body: &'a [Stmt]) {
        self.expr(cond);
        let to_else = self.emit_branch(Opcode::BranchFalse);
        self.stmts(then_body);
        if else_body.is_empty() {
            self.patch_to_here(to_else);
        } else {
            let to_end = self.emit_branch(Opcode::Branch);
            self.patch_to_here(to_else);
            self.stmts(else_body);
            self.patch_to_here(to_end);
        }
    }

    /// `while (cond) { body }`.
    fn lower_while(&mut self, cond: &hir::Expr, body: &'a [Stmt]) {
        let loop_start = self.code.len();
        self.expr(cond);
        let to_exit = self.emit_branch(Opcode::BranchFalse);
        self.break_patches.push(Vec::new());
        self.break_finally_depths.push(self.finally_bodies.len());
        self.loop_labels.push(self.pending_label.take());
        self.continue_targets.push(vec![loop_start]);
        self.continue_patches.push(Vec::new());
        self.stmts(body);
        self.emit_branch_back(Opcode::Branch, loop_start);
        self.patch_to_here(to_exit);
        self.continue_targets.pop();
        self.continue_patches.pop();
        self.loop_labels.pop();
        self.break_finally_depths.pop();
        for patch in self.break_patches.pop().unwrap_or_default() {
            self.patch_to_here(patch);
        }
    }

    /// `do { body } while (cond)`.
    fn lower_do_while(&mut self, body: &'a [Stmt], cond: &hir::Expr) {
        let loop_start = self.code.len();
        self.break_patches.push(Vec::new());
        self.break_finally_depths.push(self.finally_bodies.len());
        self.loop_labels.push(self.pending_label.take());
        self.continue_targets.push(vec![usize::MAX]);
        self.continue_patches.push(Vec::new());
        self.stmts(body);
        let continue_target = self.code.len();
        self.continue_targets.pop();
        self.loop_labels.pop();
        for patch in self.continue_patches.pop().unwrap_or_default() {
            self.patch_to_target(patch, continue_target);
        }
        self.expr(cond);
        self.emit_branch_back(Opcode::BranchTrue, loop_start);
        self.break_finally_depths.pop();
        for patch in self.break_patches.pop().unwrap_or_default() {
            self.patch_to_here(patch);
        }
    }

    /// `for (init; cond; update) { body }`.
    fn lower_for(
        &mut self,
        init: &'a Option<Box<Stmt>>,
        cond: &hir::Expr,
        update: &Option<hir::Expr>,
        body: &'a [Stmt],
    ) {
        if let Some(init) = init {
            self.stmt(init);
        }
        let loop_start = self.code.len();
        self.expr(cond);
        let to_exit = self.emit_branch(Opcode::BranchFalse);
        self.break_patches.push(Vec::new());
        self.break_finally_depths.push(self.finally_bodies.len());
        self.loop_labels.push(self.pending_label.take());
        self.continue_targets.push(vec![usize::MAX]);
        self.continue_patches.push(Vec::new());
        self.stmts(body);
        let update_target = self.code.len();
        self.continue_targets.pop();
        self.loop_labels.pop();
        for patch in self.continue_patches.pop().unwrap_or_default() {
            self.patch_to_target(patch, update_target);
        }
        if let Some(update) = update {
            self.expr(update);
            self.emit(Instruction::simple(Opcode::Pop));
        }
        self.emit_branch_back(Opcode::Branch, loop_start);
        self.patch_to_here(to_exit);
        self.break_finally_depths.pop();
        for patch in self.break_patches.pop().unwrap_or_default() {
            self.patch_to_here(patch);
        }
    }

    /// Iterator-protocol loop, with the iterator-close cleanup for generic iterators.
    fn lower_for_of(
        &mut self,
        iterator: &hir::Expr,
        iterator_slot: &hir::LocalSlot,
        done: &hir::Expr,
        next: &hir::Expr,
        binding: &hir::LocalSlot,
        body: &'a [Stmt],
    ) {
        self.expr(iterator);
        self.emit(Instruction::new(
            Opcode::StoreLocal,
            Operand::Local(*iterator_slot),
        ));
        let loop_start = self.code.len();
        self.expr(done);
        let to_exit = self.emit_branch(Opcode::BranchTrue);
        self.break_patches.push(Vec::new());
        self.break_finally_depths.push(self.finally_bodies.len());
        self.loop_labels.push(self.pending_label.take());
        self.continue_targets.push(vec![loop_start]);
        self.continue_patches.push(Vec::new());
        // Only the generic iteration protocol can carry a user
        // `return()` (or generator `finally`) to run on early exit.
        let generic_id = zun_natives::resolve("buffer", "generatorIteratorNew").map(|(id, _)| id);
        let needs_close = matches!(
            &iterator.kind,
            ExprKind::CallBuiltin { id, .. } if Some(*id) == generic_id
        );
        let loop_index = self.break_patches.len() - 1;
        let entry_depth = self.finally_bodies.len();
        if needs_close {
            self.finally_bodies.push(Cleanup::IterClose {
                slot: *iterator_slot,
                loop_index,
            });
        }
        let body_start = self.code.len() as u32;
        self.expr(next);
        self.emit(Instruction::new(
            Opcode::StoreLocal,
            Operand::Local(*binding),
        ));
        self.stmts(body);
        if needs_close {
            self.finally_bodies.pop();
        }
        let body_end = self.code.len() as u32;
        self.emit_branch_back(Opcode::Branch, loop_start);
        if needs_close {
            // A throw out of the body (or the binding pattern) closes
            // the iterator, swallowing any error from `return()`, and
            // then continues unwinding.
            let handler_start = self.code.len() as u32;
            self.emit_iterator_close(*iterator_slot, true);
            self.emit(Instruction::simple(Opcode::Pop));
            self.emit(Instruction::simple(Opcode::Rethrow));
            let handler_end = self.code.len() as u32;
            self.push_protected(
                entry_depth,
                true,
                body_start,
                body_end,
                handler_start,
                handler_end,
                HandlerKind::Finally,
                None,
            );
        }
        self.patch_to_here(to_exit);
        self.continue_targets.pop();
        self.continue_patches.pop();
        self.loop_labels.pop();
        self.break_finally_depths.pop();
        for patch in self.break_patches.pop().unwrap_or_default() {
            self.patch_to_here(patch);
        }
    }

    /// A labeled statement: a loop label, or a plain break target.
    fn lower_labeled(&mut self, label: &str, body: &'a Stmt) {
        let is_loop_stmt = |stmt: &Stmt| {
            matches!(
                stmt,
                Stmt::While { .. } | Stmt::DoWhile { .. } | Stmt::For { .. } | Stmt::ForOf { .. }
            )
        };
        // A `Group` ending in a loop (for-in's hoisted subject) still
        // labels that loop.
        let is_loop = match body {
            Stmt::Group(items) => items.last().is_some_and(is_loop_stmt),
            other => is_loop_stmt(other),
        };
        if is_loop {
            let previous = self.pending_label.replace(label.to_owned());
            self.stmt(body);
            self.pending_label = previous;
        } else {
            // A labeled non-loop statement (block or switch): a break
            // target only. An empty continue-target list marks it as
            // not a loop.
            self.break_patches.push(Vec::new());
            self.break_finally_depths.push(self.finally_bodies.len());
            self.loop_labels.push(Some(label.to_owned()));
            self.continue_targets.push(Vec::new());
            self.continue_patches.push(Vec::new());
            let previous = self.pending_label.take();
            self.stmt(body);
            self.pending_label = previous;
            self.continue_targets.pop();
            self.continue_patches.pop();
            self.loop_labels.pop();
            self.break_finally_depths.pop();
            for patch in self.break_patches.pop().unwrap_or_default() {
                self.patch_to_here(patch);
            }
        }
    }

    /// `break`, running pending `finally` cleanups first.
    fn lower_break(&mut self, label: &Option<String>) {
        // An unlabeled `break` targets the innermost loop or switch,
        // never a plain labeled block.
        let target = label
            .as_ref()
            .and_then(|name| {
                self.loop_labels
                    .iter()
                    .rposition(|candidate| candidate.as_ref() == Some(name))
            })
            .or_else(|| {
                (0..self.break_patches.len()).rev().find(|&i| {
                    !self.continue_targets[i].is_empty()
                        || self.loop_labels[i].as_deref() == Some("\0switch")
                })
            });
        if let Some(index) = target {
            self.run_finally_to(
                self.break_finally_depths.get(index).copied().unwrap_or(0),
                None,
            );
        }
        let patch = self.emit_branch(Opcode::Branch);
        if let Some(index) = target {
            if let Some(patches) = self.break_patches.get_mut(index) {
                patches.push(patch);
            }
        }
    }

    /// `continue`, running pending `finally` cleanups first.
    fn lower_continue(&mut self, label: &Option<String>) {
        let target_index = label
            .as_ref()
            .and_then(|name| {
                self.loop_labels
                    .iter()
                    .rposition(|candidate| candidate.as_ref() == Some(name))
            })
            .or_else(|| {
                (0..self.continue_targets.len())
                    .rev()
                    .find(|&i| !self.continue_targets[i].is_empty())
            });
        if let Some(index) = target_index {
            self.run_finally_to(
                self.break_finally_depths.get(index).copied().unwrap_or(0),
                Some(index),
            );
            let target = self.continue_targets[index]
                .last()
                .copied()
                .unwrap_or(usize::MAX);
            if target == usize::MAX {
                let patch = self.emit_branch(Opcode::Branch);
                if let Some(patches) = self.continue_patches.get_mut(index) {
                    patches.push(patch);
                }
            } else {
                self.emit_branch_back(Opcode::Branch, target);
            }
        }
    }

    /// `return`, inlining every pending cleanup before the `ret`.
    fn lower_return(&mut self, value: &Option<hir::Expr>) {
        let saved = self.finally_bodies.clone();
        for (index, cleanup) in saved.iter().enumerate().rev() {
            // While a cleanup runs, only the enclosing (outer) finally
            // bodies are active: a `return` inside a finally block must
            // not re-inline itself.
            self.finally_bodies.truncate(index);
            let hole_start = self.code.len() as u32;
            self.emit_cleanup(*cleanup);
            self.finally_holes
                .push((hole_start, self.code.len() as u32, index));
        }
        self.finally_bodies = saved;
        if let Some(v) = value {
            self.expr(v);
        }
        self.emit(Instruction::simple(Opcode::Ret));
    }

    /// try/catch/finally, emitting the protected regions and handlers.
    fn lower_try(
        &mut self,
        body: &'a [Stmt],
        catches: &'a [hir::CatchClause],
        finally_body: &'a Option<Vec<Stmt>>,
    ) {
        let entry_depth = self.finally_bodies.len();
        let has_finally_flag = finally_body.is_some();
        let try_start = self.code.len() as u32;
        if let Some(finally_body) = finally_body {
            self.finally_bodies.push(Cleanup::Block(finally_body));
        }
        self.stmts(body);
        if finally_body.is_some() {
            self.finally_bodies.pop();
        }
        let try_end = self.code.len() as u32;
        let mut exits = Vec::new();
        let mut catch_body_ranges: Vec<(u32, u32)> = Vec::new();
        if let Some(finally_body) = finally_body {
            self.stmts(finally_body);
        }
        exits.push(self.emit_branch(Opcode::Branch));

        for catch in catches {
            let handler_start = self.code.len() as u32;
            // A matched handler receives the exception object on the
            // evaluation stack; bind it before entering user code.
            self.emit(Instruction::new(
                Opcode::StoreLocal,
                Operand::Local(catch.slot),
            ));
            if let Some(finally_body) = finally_body {
                self.finally_bodies.push(Cleanup::Block(finally_body));
            }
            self.stmts(&catch.body);
            if finally_body.is_some() {
                self.finally_bodies.pop();
            }
            catch_body_ranges.push((handler_start, self.code.len() as u32));
            if let Some(finally_body) = finally_body {
                self.stmts(finally_body);
            }
            let handler_end = self.code.len() as u32;
            exits.push(self.emit_branch(Opcode::Branch));
            let catch_type = match catch.ty {
                Ty::Instance(index) => Some(Token::new(TableKind::TypeDef, index + 1)),
                _ => None,
            };
            self.push_protected(
                entry_depth,
                has_finally_flag,
                try_start,
                try_end,
                handler_start,
                handler_end,
                HandlerKind::Catch,
                catch_type,
            );
        }

        if let Some(finally_body) = finally_body {
            let handler_start = self.code.len() as u32;
            self.stmts(finally_body);
            self.emit(Instruction::simple(Opcode::Rethrow));
            let handler_end = self.code.len() as u32;
            self.push_protected(
                entry_depth,
                has_finally_flag,
                try_start,
                try_end,
                handler_start,
                handler_end,
                HandlerKind::Finally,
                None,
            );
            // An exception thrown inside a catch body also runs the finally.
            for (body_start, body_end) in catch_body_ranges {
                self.push_protected(
                    entry_depth,
                    has_finally_flag,
                    body_start,
                    body_end,
                    handler_start,
                    handler_end,
                    HandlerKind::Finally,
                    None,
                );
            }
        }
        for exit in exits {
            self.patch_to_here(exit);
        }
    }
}
