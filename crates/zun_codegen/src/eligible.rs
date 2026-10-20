//! JIT eligibility.
//!
//! The typed backend compiles numeric and boolean method bodies: every local,
//! parameter, and the return must be a JIT value type (any `int` width, `f32`,
//! `f64`, or `bool`), and the body may use only opcodes the translator handles.
//! Heap operations (strings, objects, fields, `callvirt`, and unsupported
//! builtins) and the
//! float-remainder libcall stay on the interpreter. Eligibility is a
//! whole-call-graph fixpoint, so a JIT frame never calls back into the
//! interpreter.

use std::collections::{HashMap, HashSet};
use zun_il::{decode_all, JitKind, Opcode, Operand};
use zun_module::{read_body, Module};

/// Whether an opcode is handled by the typed translator.
fn opcode_supported(op: Opcode, instr: &zun_il::Instruction) -> bool {
    let supported = matches!(
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
            | Opcode::CallVirt
            | Opcode::NewObj
            | Opcode::LoadField
            | Opcode::StoreField
    );
    supported || (op == Opcode::CallBuiltin && (date_builtin(instr) || math_builtin(instr)))
}

fn date_builtin(instr: &zun_il::Instruction) -> bool {
    let Operand::Imm32(id) = instr.operand else { return false; };
    matches!(zun_natives::by_id(id as u32).map(|f| f.name),
        Some("year" | "month" | "day" | "hour" | "minute" | "second" | "millisecond" | "weekday" | "dayOfYear"))
}

fn math_builtin(instr: &zun_il::Instruction) -> bool {
    let Operand::Imm32(id) = instr.operand else { return false; };
    matches!(zun_natives::by_id(id as u32).map(|f| f.name), Some(
        "abs" | "sqrt" | "min" | "max" | "sqrtNumber" | "minNumber" | "maxNumber"
        | "floor" | "ceil" | "round" | "floorNumber" | "ceilNumber" | "roundNumber"
        | "powNumber" | "sinNumber" | "cosNumber" | "expNumber" | "logNumber"
        | "tanNumber" | "atan2Number" | "hypotNumber"
    ))
}

/// Whether a field (by FieldDef token RID) has a JIT-representable type, i.e.
/// numeric, bool, or object — not a string or anything else.
fn field_jit_ok(module: &Module, field_rid: u32) -> bool {
    module
        .tables
        .field_defs
        .get(field_rid as usize - 1)
        .map(|f| kind_supported(f.kind))
        .unwrap_or(false)
}

/// Whether an instruction's field/type operand references only JIT types.
fn instr_fields_ok(module: &Module, instr: &zun_il::Instruction) -> bool {
    match (instr.opcode, instr.operand) {
        (Opcode::LoadField | Opcode::StoreField, Operand::Token(t)) => {
            field_jit_ok(module, t.rid())
        }
        (Opcode::NewObj, Operand::Token(t)) => {
            match module.tables.type_defs.get(t.rid() as usize - 1) {
                Some(ty) => (ty.field_start..ty.field_end).all(|r| field_jit_ok(module, r)),
                None => false,
            }
        }
        _ => true,
    }
}

/// Validate a virtual call token. Its target set is added to the call graph
/// below, so overridden implementations can be compiled for indirect calls.
fn callvirt_is_non_overridden(module: &Module, instr: &zun_il::Instruction) -> bool {
    if instr.opcode != Opcode::CallVirt {
        return true;
    }
    let Operand::Token(token) = instr.operand else {
        return false;
    };
    let Some(method) = module.tables.method_defs.get(token.rid() as usize - 1) else {
        return false;
    };
    if method.body_offset == u32::MAX {
        return false;
    }
    // CallVirt dispatches through an object vtable. Primitive-backed classes
    // (for example `Number`) still use the opcode at Tier 0, but their
    // receiver is a scalar and cannot be represented by the JIT vtable path.
    let Ok(body) = read_body(&module.code, method.body_offset) else {
        return false;
    };
    body.local_kinds
        .first()
        .is_some_and(|&kind| JitKind::from_byte(kind) == JitKind::Object)
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
        if !instrs.iter().all(|i| opcode_supported(i.opcode, i)) {
            continue;
        }
        // Reject methods whose field/newobj operands touch non-JIT types
        // (e.g. string fields).
        if !instrs.iter().all(|i| instr_fields_ok(module, i)) {
            continue;
        }
        if !instrs.iter().all(|i| callvirt_is_non_overridden(module, i)) {
            continue;
        }
        let mut callees = Vec::new();
        for instr in &instrs {
            if !matches!(instr.opcode, Opcode::Call | Opcode::CallVirt) {
                continue;
            }
            let Operand::Token(token) = instr.operand else {
                continue;
            };
            if instr.opcode == Opcode::CallVirt {
                let Some(method) = module.tables.method_defs.get(token.rid() as usize - 1) else {
                    continue;
                };
                callees.extend(module.tables.method_defs.iter().enumerate().filter_map(
                    |(index, candidate)| {
                        (candidate.name == method.name && candidate.body_offset != u32::MAX)
                            .then_some(index as u32 + 1)
                    },
                ));
            } else {
                callees.push(token.rid());
            }
        }
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
