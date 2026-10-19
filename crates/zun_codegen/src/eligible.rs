//! JIT eligibility.
//!
//! The Tier-1 backend compiles a numeric subset: method bodies in which every
//! value — numbers and booleans alike — is represented as an `f64`. This covers
//! arithmetic, comparisons, control flow, and calls among such methods, which
//! is where native code pays off most. Anything touching the heap (strings,
//! objects, field access, `callvirt`, builtins) or `rem.f64` (a libcall) stays
//! on the always-correct interpreter.
//!
//! Eligibility is a property of the whole call graph: a method is eligible only
//! if every method it calls is eligible too, so a JIT frame never has to call
//! back into the interpreter.

use std::collections::{HashMap, HashSet};
use zun_il::{decode_all, Opcode, Operand};
use zun_module::{read_body, Module};

/// Whether an opcode is part of the numeric (all-`f64`) subset.
fn opcode_is_numeric(op: Opcode) -> bool {
    matches!(
        op,
        Opcode::Nop
            | Opcode::ConstF64
            | Opcode::ConstTrue
            | Opcode::ConstFalse
            | Opcode::LoadLocal
            | Opcode::StoreLocal
            | Opcode::Pop
            | Opcode::Dup
            | Opcode::AddF64
            | Opcode::SubF64
            | Opcode::MulF64
            | Opcode::DivF64
            | Opcode::CmpEq
            | Opcode::CmpNe
            | Opcode::CmpLt
            | Opcode::CmpLe
            | Opcode::CmpGt
            | Opcode::CmpGe
            | Opcode::Branch
            | Opcode::BranchTrue
            | Opcode::BranchFalse
            | Opcode::Ret
            | Opcode::Call
    )
}

/// Compute the set of JIT-eligible method RIDs for `module`.
///
/// A method is eligible if its body decodes, uses only numeric opcodes, and
/// every method it calls is itself eligible. The call-graph constraint is
/// solved by removing any method that calls an ineligible one until the set is
/// stable.
pub fn eligible_methods(module: &Module) -> HashSet<u32> {
    // Candidates: methods whose own opcodes are all numeric.
    let mut candidate: HashMap<u32, Vec<u32>> = HashMap::new(); // rid -> callees
    for (i, method) in module.tables.method_defs.iter().enumerate() {
        let rid = i as u32 + 1;
        if method.body_offset == u32::MAX {
            continue;
        }
        let Ok(body) = read_body(&module.code, method.body_offset) else {
            continue;
        };
        let Ok(instrs) = decode_all(body.code) else {
            continue;
        };
        if !instrs.iter().all(|i| opcode_is_numeric(i.opcode)) {
            continue;
        }
        let callees = instrs
            .iter()
            .filter(|i| i.opcode == Opcode::Call)
            .filter_map(|i| match i.operand {
                Operand::Token(t) => Some(t.rid()),
                _ => None,
            })
            .collect();
        candidate.insert(rid, callees);
    }

    // Fixpoint: drop any candidate that calls a method not (yet) known
    // eligible, repeating until the set stops shrinking.
    let mut eligible: HashSet<u32> = candidate.keys().copied().collect();
    loop {
        let prev = eligible.clone();
        eligible.retain(|rid| candidate[rid].iter().all(|c| prev.contains(c)));
        if eligible.len() == prev.len() {
            break;
        }
    }
    eligible
}
