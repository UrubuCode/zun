//! JIT eligibility.
//!
//! The typed backend compiles numeric and boolean method bodies: every local,
//! parameter, and the return must be a JIT value type (any `int` width, `f32`,
//! `f64`, or `bool`), and the body may use only opcodes the translator handles.
//! Heap operations (strings, objects, fields, `callvirt`, builtins) and the
//! float-remainder libcall stay on the interpreter. Eligibility is a
//! whole-call-graph fixpoint, so a JIT frame never calls back into the
//! interpreter.

use std::collections::{HashMap, HashSet};
use zun_il::{decode_all, JitKind, Opcode, Operand};
use zun_module::{read_body, Module};

/// Whether an opcode is handled by the typed translator.
fn opcode_supported(op: Opcode) -> bool {
    matches!(
        op,
        Opcode::Nop
            | Opcode::ConstI32
            | Opcode::ConstI64
            | Opcode::ConstF64
            | Opcode::ConstTrue
            | Opcode::ConstFalse
            | Opcode::Conv
            | Opcode::LoadLocal
            | Opcode::StoreLocal
            | Opcode::Pop
            | Opcode::Dup
            | Opcode::AddI32
            | Opcode::SubI32
            | Opcode::MulI32
            | Opcode::DivI32
            | Opcode::RemI32
            | Opcode::NegI32
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

/// Whether a value-type code is one the typed JIT can represent.
fn kind_supported(byte: u8) -> bool {
    !matches!(JitKind::from_byte(byte), JitKind::Other)
}

/// Compute the set of JIT-eligible method RIDs for `module`.
pub fn eligible_methods(module: &Module) -> HashSet<u32> {
    let mut candidate: HashMap<u32, Vec<u32>> = HashMap::new();
    for (i, method) in module.tables.method_defs.iter().enumerate() {
        let rid = i as u32 + 1;
        if method.body_offset == u32::MAX {
            continue;
        }
        let Ok(body) = read_body(&module.code, method.body_offset) else {
            continue;
        };
        // Return and every local must be a representable JIT type.
        if !kind_supported(body.return_kind) && body.returns_value {
            continue;
        }
        if !body.local_kinds.iter().all(|&k| kind_supported(k)) {
            continue;
        }
        let Ok(instrs) = decode_all(body.code) else {
            continue;
        };
        if !instrs.iter().all(|i| opcode_supported(i.opcode)) {
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
