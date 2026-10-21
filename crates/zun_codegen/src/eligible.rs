//! JIT eligibility.
//!
//! The typed backend compiles numeric and boolean method bodies: every local,
//! parameter, and the return must be a JIT value type (any `int` width, `f32`,
//! `f64`, or `bool`), and the body may use only opcodes the translator handles.
//! Heap operations (strings, objects, fields, `callvirt`, and unsupported
//! builtins) and the
//! float-remainder libcall stay on the interpreter. Numeric array loads are a
//! narrow read-only exception. Eligibility is a
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
            | Opcode::ConstStr
            | Opcode::Conv
            | Opcode::UnboxAny
            | Opcode::BoxAny
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
            | Opcode::ArrayLength
            | Opcode::NewArray
            | Opcode::LoadArray
            | Opcode::StoreArray
            | Opcode::ArrayPop
            | Opcode::ArrayPush
    );
    supported
        || (op == Opcode::CallBuiltin
            && (date_builtin(instr)
                || math_builtin(instr)
                || collection_builtin(instr)
                || string_builtin(instr)))
}

/// Checked operations are not eligible until Cranelift can route their
/// exceptions through the same handler model as the interpreter. Keep this
/// rule centralized instead of growing type- or container-specific allowlists.
fn requires_exception_bridge(op: Opcode) -> bool {
    matches!(op, Opcode::CastClass)
}

fn string_builtin(instr: &zun_il::Instruction) -> bool {
    let Operand::Imm32(id) = instr.operand else { return false; };
    matches!(
        zun_natives::by_id(id as u32).map(|f| f.name),
        Some("length" | "contains" | "includes" | "charCodeAt" | "startsWith" | "endsWith" | "indexOf" | "lastIndexOf" | "count" | "compare" | "concat" | "substring" | "trim" | "trimStart" | "trimEnd" | "toLowerCase" | "toUpperCase" | "repeat" | "reverse" | "padStart" | "padEnd" | "replace" | "toInt" | "toFloat" | "toBoolean")
    )
}

fn collection_builtin(instr: &zun_il::Instruction) -> bool {
    let Operand::Imm32(id) = instr.operand else {
        return false;
    };
    matches!(
        zun_natives::by_id(id as u32).map(|f| f.name),
        Some(
            "mapSize"
                | "mapHasF64"
                | "setSize"
                | "setHasF64"
                | "setAddF64"
                | "mapDeleteF64"
                | "setDeleteF64"
                | "mapSetF64"
                | "mapGetF64Or"
                | "mapHasI64"
                | "mapDeleteI64"
                | "setAddI64"
                | "setHasI64"
                | "setDeleteI64"
                | "mapSetI64"
                | "mapGetI64Or"
                | "bufferLength"
                | "bufferGetU8"
                | "bufferReadU32"
                | "bufferReadF64"
                | "bufferSetU8"
                | "bufferWriteU32"
                | "bufferWriteF64"
                | "dataViewGetU32"
                | "dataViewLength"
                | "dataViewByteLength"
                | "dataViewGetU16"
                | "dataViewGetF64"
                | "dataViewSetU32"
                | "dataViewSetU16"
                | "dataViewSetF64"
                | "u8ArrayGet"
                | "u8ArrayLength"
                | "u8ArrayByteLength"
                | "u8ArraySet"
                | "u16ArrayGet"
                | "u16ArrayLength"
                | "u16ArrayByteLength"
                | "u16ArraySet"
                | "u32ArrayGet"
                | "u32ArrayLength"
                | "u32ArrayByteLength"
                | "u32ArraySet"
                | "f64ArrayGet"
                | "f64ArrayLength"
                | "f64ArrayByteLength"
                | "f64ArraySet",
        )
    )
}

fn date_builtin(instr: &zun_il::Instruction) -> bool {
    let Operand::Imm32(id) = instr.operand else {
        return false;
    };
    matches!(
        zun_natives::by_id(id as u32).map(|f| f.name),
        Some(
            "isValid"
                | "year"
                | "month"
                | "day"
                | "hour"
                | "minute"
                | "second"
                | "millisecond"
                | "weekday"
                | "dayOfYear"
        )
    )
}

fn math_builtin(instr: &zun_il::Instruction) -> bool {
    let Operand::Imm32(id) = instr.operand else {
        return false;
    };
    matches!(
        zun_natives::by_id(id as u32).map(|f| f.name),
        Some(
            "abs"
                | "sqrt"
                | "cbrtNumber"
                | "min"
                | "max"
                | "sqrtNumber"
                | "minNumber"
                | "maxNumber"
                | "floor"
                | "ceil"
                | "round"
                | "truncNumber"
                | "floorNumber"
                | "ceilNumber"
                | "roundNumber"
                | "powNumber"
                | "sinNumber"
                | "cosNumber"
                | "expNumber"
                | "logNumber"
                | "log10Number"
                | "log2Number"
                | "expm1Number"
                | "log1pNumber"
                | "sinhNumber"
                | "coshNumber"
                | "tanhNumber"
                | "tanNumber"
                | "atan2Number"
                | "hypotNumber"
                | "signNumber"
                | "isFinite"
                | "isNaN"
                | "isNegativeZero"
                | "isSafeInteger"
                | "asinhNumber"
                | "acoshNumber"
                | "atanhNumber"
        )
    )
}

/// Whether a field (by FieldDef token RID) has a JIT-representable type, i.e.
/// numeric, bool, or object — not a string or anything else.
fn field_jit_ok(module: &Module, field_rid: u32) -> bool {
    module
        .tables
        .field_defs
        .get(field_rid as usize - 1)
        .map(|f| {
            matches!(
                JitKind::from_byte(f.kind),
                JitKind::Num(_) | JitKind::Bool | JitKind::Object
            )
        })
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
    if module
        .tables
        .method_defs
        .get(token.rid() as usize - 1)
        .is_some_and(|method| {
            matches!(
                module.strings.get(method.name),
                "length" | "contains" | "includes" | "charCodeAt" | "startsWith" | "endsWith" | "indexOf" | "lastIndexOf" | "count" | "compare" | "concat" | "substring" | "trim" | "trimStart" | "trimEnd" | "toLowerCase" | "toUpperCase" | "repeat" | "reverse" | "padStart" | "padEnd" | "replace"
            )
        })
    {
        // Primitive-backed String.length is lowered directly by the JIT and
        // does not use an object vtable.
        return true;
    }
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

/// The current managed allocation lowering is safe for a non-escaping array
/// whose only operation is a length query. Mutation, element access, or a
/// returned reference still requires the interpreter's heap contract.
fn scalar_array_length_only(instrs: &[zun_il::Instruction], body: &zun_module::MethodBody<'_>) -> bool {
    body.returns_value
        && matches!(JitKind::from_byte(body.return_kind), JitKind::Num(kind) if kind.int_parts().is_some())
        && instrs.iter().all(|instruction| {
            matches!(
                instruction.opcode,
                Opcode::Nop
                    | Opcode::ConstI32
                    | Opcode::ConstI64
                    | Opcode::LoadLocal
                    | Opcode::StoreLocal
                    | Opcode::NewArray
                    | Opcode::ArrayLength
                    | Opcode::Ret
            )
        })
}

/// Allow a narrow integer array ABI without making general collection code
/// eligible. The helper is intentionally limited to scalar instructions and
/// one element access; resizing, iteration, and reference values remain Tier 0.
fn scalar_u64_array_access_only(
    instrs: &[zun_il::Instruction],
    body: &zun_module::MethodBody<'_>,
) -> bool {
    body.returns_value
        && JitKind::from_byte(body.return_kind) == JitKind::Num(zun_il::NumKind::U64)
        && instrs.iter().any(|instruction| {
            matches!(instruction.opcode, Opcode::LoadArray | Opcode::StoreArray | Opcode::ArrayPop)
        })
        && instrs.iter().all(|instruction| {
            matches!(
                instruction.opcode,
                Opcode::Nop
                    | Opcode::ConstI32
                    | Opcode::ConstI64
                    | Opcode::Conv
                    | Opcode::UnboxAny
                    | Opcode::BoxAny
                    | Opcode::Pop
                    | Opcode::LoadLocal
                    | Opcode::StoreLocal
                    | Opcode::LoadArray
                    | Opcode::StoreArray
                    | Opcode::ArrayPop
                    | Opcode::Ret
            )
        })
}

fn scalar_bool_array_access_only(
    instrs: &[zun_il::Instruction],
    body: &zun_module::MethodBody<'_>,
) -> bool {
    body.returns_value
        && JitKind::from_byte(body.return_kind) == JitKind::Bool
        && instrs.iter().any(|instruction| {
            matches!(instruction.opcode, Opcode::LoadArray | Opcode::StoreArray)
        })
        && instrs.iter().all(|instruction| {
            matches!(
                instruction.opcode,
                Opcode::Nop
                    | Opcode::ConstI32
                    | Opcode::ConstI64
                    | Opcode::UnboxAny
                    | Opcode::BoxAny
                    | Opcode::Pop
                    | Opcode::LoadLocal
                    | Opcode::StoreLocal
                    | Opcode::LoadArray
                    | Opcode::StoreArray
                    | Opcode::Ret
            )
        })
}

/// Allow a narrow object-array ABI. The aggregate `Any` carrier remains out
/// of the JIT; only methods whose array result/value is a managed object use
/// the single-pointer helper ABI.
fn object_array_access_only(
    instrs: &[zun_il::Instruction],
    body: &zun_module::MethodBody<'_>,
) -> bool {
    body.returns_value
        && JitKind::from_byte(body.return_kind) == JitKind::Object
        && instrs.iter().any(|instruction| {
            matches!(instruction.opcode, Opcode::LoadArray | Opcode::StoreArray)
        })
        && instrs.iter().all(|instruction| {
            matches!(
                instruction.opcode,
                Opcode::Nop
                    | Opcode::ConstI32
                    | Opcode::ConstI64
                    | Opcode::LoadLocal
                    | Opcode::StoreLocal
                    | Opcode::LoadArray
                    | Opcode::StoreArray
                    | Opcode::Pop
                    | Opcode::Ret
            )
        })
}

fn object_array_stack_only(instrs: &[zun_il::Instruction], body: &zun_module::MethodBody<'_>) -> bool {
    (body.returns_value && JitKind::from_byte(body.return_kind) == JitKind::Object
        && instrs.iter().any(|i| i.opcode == Opcode::ArrayPop)
        && instrs.iter().all(|i| matches!(i.opcode,
            Opcode::Nop | Opcode::LoadLocal | Opcode::StoreLocal | Opcode::ArrayPop | Opcode::Ret)))
        || (body.returns_value && instrs.iter().any(|i| i.opcode == Opcode::ArrayPush)
            && instrs.iter().all(|i| matches!(i.opcode,
                Opcode::Nop | Opcode::LoadLocal | Opcode::StoreLocal | Opcode::ArrayPush | Opcode::Ret)))
}

fn any_array_return_only(instrs: &[zun_il::Instruction], body: &zun_module::MethodBody<'_>) -> bool {
    body.returns_value
        && JitKind::from_byte(body.return_kind) == JitKind::Other
        && instrs.iter().any(|i| i.opcode == Opcode::LoadArray)
        && instrs.iter().all(|i| matches!(
            i.opcode,
            Opcode::Nop | Opcode::ConstI32 | Opcode::ConstI64 | Opcode::LoadLocal
                | Opcode::LoadArray | Opcode::Ret
        ))
}

fn any_param_return_only(instrs: &[zun_il::Instruction], body: &zun_module::MethodBody<'_>) -> bool {
    body.returns_value
        && JitKind::from_byte(body.return_kind) == JitKind::Other
        && body.param_count > 0
        && body.local_kinds[..body.param_count as usize]
            .iter()
            .any(|&kind| JitKind::from_byte(kind) == JitKind::Other)
        && instrs.iter().all(|i| matches!(i.opcode, Opcode::Nop | Opcode::LoadLocal | Opcode::Ret))
}

fn any_direct_call_return_only(instrs: &[zun_il::Instruction], body: &zun_module::MethodBody<'_>) -> bool {
    body.returns_value
        && JitKind::from_byte(body.return_kind) == JitKind::Other
        && body.param_count > 0
        && body.local_kinds[..body.param_count as usize]
            .iter()
            .any(|&kind| JitKind::from_byte(kind) == JitKind::Other)
        && instrs
            .iter()
            .filter(|i| matches!(i.opcode, Opcode::Call | Opcode::CallVirt))
            .count()
            == 1
        && instrs.iter().all(|i| matches!(i.opcode, Opcode::Nop | Opcode::LoadLocal | Opcode::Call | Opcode::CallVirt | Opcode::Ret))
}

/// Compute the set of JIT-eligible method RIDs for `module`.
pub fn eligible_methods(module: &Module) -> HashSet<u32> {
    eligible_methods_with_literals(module, false)
}

/// Compute runtime-JIT eligibility when the JIT owns stable copies of module
/// string bytes. Relocatable/AOT objects must keep the default false mode:
/// their code cannot embed a process-local module-string pointer.
pub fn eligible_methods_with_literals(module: &Module, allow_const_str: bool) -> HashSet<u32> {
    let mut methods_by_name: HashMap<_, Vec<u32>> = HashMap::new();
    for (index, method) in module.tables.method_defs.iter().enumerate() {
        methods_by_name
            .entry(method.name)
            .or_default()
            .push(index as u32 + 1);
    }
    let mut candidate: HashMap<u32, Vec<u32>> = HashMap::new();
    for (i, method) in module.tables.method_defs.iter().enumerate() {
        let rid = i as u32 + 1;
        if method.body_offset == u32::MAX {
            continue;
        }
        let Ok(body) = read_body(&module.code, method.body_offset) else {
            continue;
        };
        // Exception regions require interpreter unwind/landing-pad support;
        // keep them on Tier 0 until the Cranelift EH lowering is implemented.
        if !body.handlers.is_empty() {
            continue;
        }
        // Return and every local must be a representable JIT type.
        if !kind_supported(body.return_kind) && body.returns_value {
            if JitKind::from_byte(body.return_kind) != JitKind::Other {
                continue;
            }
        }
        let Ok(instrs) = decode_all(body.code) else {
            continue;
        };
        if !body.local_kinds.iter().all(|&k| kind_supported(k))
            && !any_param_return_only(&instrs, &body)
            && !any_direct_call_return_only(&instrs, &body)
        {
            continue;
        }
        if !kind_supported(body.return_kind)
            && body.returns_value
            && !any_array_return_only(&instrs, &body)
            && !any_param_return_only(&instrs, &body)
            && !any_direct_call_return_only(&instrs, &body)
        {
            continue;
        }
        if instrs.iter().any(|i| i.opcode == Opcode::ConstStr) {
            // The first literal ABI slice is deliberately limited to a direct
            // literal receiver call. More general locals/control flow need a
            // literal-handle liveness contract at every safepoint.
            if !allow_const_str
                || !instrs.iter().all(|i| {
                    matches!(i.opcode, Opcode::Nop | Opcode::ConstStr | Opcode::CallVirt | Opcode::Ret)
                })
                || instrs.iter().filter(|i| i.opcode == Opcode::CallVirt).count() != 1
            {
                continue;
            }
        }
        if instrs.iter().any(|instruction| requires_exception_bridge(instruction.opcode)) {
            continue;
        }
        if !instrs.iter().all(|i| opcode_supported(i.opcode, i)) {
            continue;
        }
        // Array access has stable f64 and narrow u64 scalar ABIs. Keep
        // reference-valued and general collection consumers in Tier 0.
        if instrs.iter().any(|i| i.opcode == Opcode::LoadArray)
            && (!body.returns_value
                || (JitKind::from_byte(body.return_kind) != JitKind::Num(zun_il::NumKind::F64)
                    && !object_array_access_only(&instrs, &body)
                    && !object_array_stack_only(&instrs, &body)
                    && !scalar_bool_array_access_only(&instrs, &body)
                    && !scalar_u64_array_access_only(&instrs, &body)
                    && !any_array_return_only(&instrs, &body)))
        {
            continue;
        }
        if instrs.iter().any(|i| i.opcode == Opcode::StoreArray)
            && (!body.returns_value
                || (JitKind::from_byte(body.return_kind) != JitKind::Num(zun_il::NumKind::F64)
                    && !object_array_access_only(&instrs, &body)
                    && !object_array_stack_only(&instrs, &body)
                    && !scalar_bool_array_access_only(&instrs, &body)
                    && !scalar_u64_array_access_only(&instrs, &body)))
        {
            continue;
        }
        if instrs.iter().any(|i| i.opcode == Opcode::ArrayPop)
            && (!body.returns_value
                || (JitKind::from_byte(body.return_kind) != JitKind::Num(zun_il::NumKind::F64)
                    && JitKind::from_byte(body.return_kind) != JitKind::Num(zun_il::NumKind::U64)
                    && JitKind::from_byte(body.return_kind) != JitKind::Bool
                    && !scalar_bool_array_access_only(&instrs, &body)
                    && !object_array_stack_only(&instrs, &body)))
        {
            continue;
        }
        if instrs.iter().any(|i| i.opcode == Opcode::NewArray)
            && (JitKind::from_byte(body.return_kind) != JitKind::Object)
            && !scalar_array_length_only(&instrs, &body)
        {
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
                callees.extend(
                    methods_by_name
                        .get(&method.name)
                        .into_iter()
                        .flatten()
                        .copied()
                        .filter(|rid| {
                            module.tables.method_defs[*rid as usize - 1].body_offset != u32::MAX
                        }),
                );
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
