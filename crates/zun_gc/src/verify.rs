//! Verification of the remembered-set invariant.
//!
//! A young collection never traces an old object: it trusts that every old
//! object holding a young reference is in the remembered set. A store that
//! reaches a tenured object without passing the write barrier breaks that
//! trust silently -- the young target is swept or moved while the old object
//! still points at it. This pass checks the trust instead of assuming it: before
//! each young collection it walks every old object's strong edges (the same
//! edges the marker follows, through `Heap::children_into`) and reports each
//! one that leads to a young object without the owner being remembered.

use super::*;

/// Whether remembered-set verification runs before every young collection.
/// A newtype so `Heap` keeps `#[derive(Default)]` with the environment as the
/// default: `ZUN_GC_VERIFY=1` turns it on, `ZUN_GC_VERIFY=0` off, and unset it
/// follows `debug_assertions` (the test profile of a debug build).
#[derive(Debug, Clone, Copy)]
pub(super) struct VerifyBarriers(pub(super) bool);

impl Default for VerifyBarriers {
    fn default() -> Self {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        Self(*ON.get_or_init(|| match std::env::var("ZUN_GC_VERIFY") {
            Ok(value) => parse_stress_on(Some(&value)),
            Err(_) => cfg!(debug_assertions),
        }))
    }
}

impl Heap {
    /// Turn remembered-set verification on or off for this heap.
    pub fn set_verify_barriers(&mut self, verify: bool) {
        self.verify_barriers.0 = verify;
    }

    /// Every old object that holds a young reference without being remembered,
    /// described for a failure message. Empty when the barrier is complete.
    pub fn unremembered_old_to_young_edges(
        &self,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> Vec<String> {
        let mut violations = Vec::new();
        let mut children = Vec::new();
        for record in &self.records {
            if !record.alive || record.generation != records::Generation::Old {
                continue;
            }
            let owner = Obj(record.addr);
            if self.remembered_set.contains(&(owner.addr() as usize)) {
                continue;
            }
            children.clear();
            self.children_into(owner, ref_slots, &mut children);
            for &child in &children {
                let young = self.by_addr.get(&child).is_some_and(|&idx| {
                    self.records[idx].alive
                        && self.records[idx].generation == records::Generation::Young
                });
                if young {
                    violations.push(format!(
                        "old object {:#x} (type rid {}) holds young object {:#x} \
                         without a write barrier",
                        owner.addr() as usize,
                        owner.type_rid(),
                        child
                    ));
                    break;
                }
            }
        }
        violations
    }

    /// Panic when an old object holds an unremembered young reference.
    pub(crate) fn verify_remembered_set(
        &self,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) {
        let violations = self.unremembered_old_to_young_edges(ref_slots);
        assert!(
            violations.is_empty(),
            "remembered-set invariant broken, a store skipped the write barrier: {}",
            violations.join("; ")
        );
    }
}
