//! Exception-catch matching shared by the interpreter bridge and the JIT.
//!
//! Both execution tiers must agree on whether a thrown type is caught by a
//! handler, so there is exactly one base-chain walk, and it lives here.

use zun_metadata::TableKind;

use crate::Module;

/// Whether a thrown object of type `throw_rid` is caught by a handler for
/// `catch_rid` (the same type or a subclass), by walking the module's `TypeDef`
/// base chain.
///
/// The walk is bounded (64 steps) so a malformed cyclic hierarchy cannot loop
/// forever. A non-`TypeDef` (external `TypeRef`) or null base ends the chain: the
/// hierarchy above it is not verifiable here, so it is not a match. A `throw_rid`
/// that names no `TypeDef` (including 0) matches only when it equals `catch_rid`.
pub fn type_is_caught(module: &Module, throw_rid: u32, catch_rid: u32) -> bool {
    let mut current = throw_rid;
    for _ in 0..64 {
        if current == catch_rid {
            return true;
        }
        let Some(ty) = (current as usize)
            .checked_sub(1)
            .and_then(|index| module.tables.type_defs.get(index))
        else {
            return false;
        };
        if ty.base.table() != Some(TableKind::TypeDef) || ty.base.rid() == 0 {
            return false;
        }
        current = ty.base.rid();
    }
    false
}

#[cfg(test)]
mod tests {
    use zun_metadata::{StringIndex, Token, TypeDef};

    use super::*;

    fn type_def(base: Token) -> TypeDef {
        TypeDef {
            name: StringIndex(0),
            namespace: StringIndex(0),
            base,
            field_start: 0,
            field_end: 0,
            method_start: 0,
            method_end: 0,
        }
    }

    fn module_with(bases: Vec<Token>) -> Module {
        let mut module = Module::new();
        module.tables.type_defs = bases.into_iter().map(type_def).collect();
        module
    }

    fn base(rid: u32) -> Token {
        if rid == 0 {
            Token::null(TableKind::TypeDef)
        } else {
            Token::new(TableKind::TypeDef, rid)
        }
    }

    #[test]
    fn same_type_is_caught() {
        let module = module_with(vec![base(0)]);
        assert!(type_is_caught(&module, 1, 1));
    }

    #[test]
    fn direct_and_transitive_subclass_are_caught() {
        // 1 <- 2 <- 3
        let module = module_with(vec![base(0), base(1), base(2)]);
        assert!(type_is_caught(&module, 2, 1));
        assert!(type_is_caught(&module, 3, 2));
        assert!(type_is_caught(&module, 3, 1));
    }

    #[test]
    fn base_is_not_caught_by_subclass_handler() {
        let module = module_with(vec![base(0), base(1)]);
        assert!(!type_is_caught(&module, 1, 2));
    }

    #[test]
    fn unrelated_types_are_not_caught() {
        let module = module_with(vec![base(0), base(0)]);
        assert!(!type_is_caught(&module, 1, 2));
        assert!(!type_is_caught(&module, 2, 1));
    }

    #[test]
    fn non_typedef_base_ends_the_chain() {
        // Type 2 derives from an external TypeRef(1); a handler for TypeDef 1
        // must not match through it.
        let module = module_with(vec![base(0), Token::new(TableKind::TypeRef, 1)]);
        assert!(!type_is_caught(&module, 2, 1));
    }

    #[test]
    fn rid_zero_never_panics_and_only_matches_itself() {
        let module = module_with(vec![base(0)]);
        assert!(!type_is_caught(&module, 0, 1));
        assert!(type_is_caught(&module, 0, 0));
    }

    #[test]
    fn out_of_range_rid_is_not_caught() {
        let module = module_with(vec![base(0)]);
        assert!(!type_is_caught(&module, 9, 1));
    }

    #[test]
    fn cyclic_hierarchy_terminates() {
        // 1 <- 2 <- 1 : neither reaches 3, and the walk must stop.
        let module = module_with(vec![base(2), base(1), base(0)]);
        assert!(!type_is_caught(&module, 1, 3));
        // A cycle that does contain the target still matches.
        assert!(type_is_caught(&module, 1, 2));
    }
}
