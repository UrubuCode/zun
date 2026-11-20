use super::*;

impl FunctionCx {
    pub(super) fn declare(&mut self, name: &str, ty: Ty) -> hir::LocalSlot {
        let slot = self.locals.len() as hir::LocalSlot;
        self.locals.push(ty);
        self.scopes
            .last_mut()
            .expect("a scope is always open")
            .insert(name.to_string(), slot);
        slot
    }

    pub(super) fn lookup(&self, name: &str) -> Option<hir::LocalSlot> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }
}

impl FunctionCx {
    /// Whether reading `name` here is a temporal-dead-zone access: an
    /// enclosing block of this function declares it with `let`/`const`
    /// further down, and neither that block nor one nested in it has declared
    /// it yet. Such a read always throws when it is evaluated -- there is no
    /// path that reaches it after the declaration without re-entering the
    /// block, which creates a fresh binding.
    pub(super) fn in_dead_zone(&self, name: &str) -> bool {
        for (scope_index, names) in self.pending_lexical.iter().rev() {
            if !names.contains(name) {
                continue;
            }
            let declared = self
                .scopes
                .iter()
                .skip(*scope_index)
                .any(|scope| scope.contains_key(name));
            return !declared;
        }
        false
    }
}
