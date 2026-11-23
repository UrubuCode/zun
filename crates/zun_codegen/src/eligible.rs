//! JIT eligibility.
//!
//! The typed backend compiles numeric and boolean method bodies: every local,
//! parameter, and the return must be a JIT value type (any `int` width, `f32`,
//! `f64`, or `bool`), and the body may use only opcodes the translator handles.
//! Heap operations (strings, objects, fields, `callvirt`, and unsupported
//! builtins) stay on the interpreter; float remainder is compiled through the
//! `zun_jit_math_fmod` libcall. Numeric array loads are a
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
            // A nullable class reference's `null`: the zero handle. The translator
            // refuses it wherever the target is not an `Object` slot.
            | Opcode::ConstNull
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
            | Opcode::AndI32
            | Opcode::OrI32
            | Opcode::XorI32
            | Opcode::NotI32
            | Opcode::ShlI32
            | Opcode::ShrI32
            | Opcode::UShrI32
            | Opcode::PowI32
            | Opcode::AddF64
            | Opcode::SubF64
            | Opcode::MulF64
            | Opcode::DivF64
            // Float remainder lowers to the `zun_jit_math_fmod` libcall (no
            // Cranelift frem); it cannot throw, so no exception bridge is needed.
            | Opcode::RemF64
            // Float math machine ops: each lowers to a single Cranelift
            // instruction, none can throw, so none needs the exception bridge.
            | Opcode::SqrtF64
            | Opcode::AbsF64
            | Opcode::NegF64
            | Opcode::FloorF64
            | Opcode::CeilF64
            | Opcode::TruncF64
            | Opcode::MinF64
            | Opcode::MaxF64
            | Opcode::RoundF64
            | Opcode::SignF64
            | Opcode::MinI32
            | Opcode::MaxI32
            | Opcode::ClzI32
            | Opcode::CtzI32
            | Opcode::PopcntI32
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
            // String `+`: lowers to the `zun_jit_string_concat` libcall, the
            // same managed-string helper the `.concat` method uses. Both
            // operands must be JIT string handles.
            | Opcode::Concat
            // Class-static field access: lowers to the `zun_jit_static_get`/`set`
            // helpers over the shared heap statics store. The field's declared
            // kind must be JIT-representable (checked in `instr_fields_ok`).
            | Opcode::LoadStatic
            | Opcode::StoreStatic
            // Explicit `throw`: lowered as a same-frame branch to the catch
            // handler. `exception_regions_jit_safe` gates the surrounding method
            // so the throw is always routed to an in-frame catch-all.
            | Opcode::Throw
            // First-class function reference + indirect call. Only reachable
            // transiently (a no-closure `LoadFunction` immediately invoked); both
            // require a bridge (checked in `eligible_impl`), and `CallIndirect`
            // dispatches through it to the target method.
            | Opcode::LoadFunction
            | Opcode::CallIndirect
    );
    supported
        || (op == Opcode::CallBuiltin
            && (date_builtin(instr)
                || math_builtin(instr)
                || collection_builtin(instr)
                || allocation_builtin(instr)
                || string_builtin(instr)))
}

/// Checked operations are not eligible until Cranelift can route their
/// exceptions through the same handler model as the interpreter. Keep this
/// rule centralized instead of growing type- or container-specific allowlists.
fn requires_exception_bridge(op: Opcode) -> bool {
    matches!(op, Opcode::CastClass)
}

/// Opcodes allowed inside a protected (`try`) region for the same-frame
/// exception slice. The whitelist admits only operations that cannot raise a
/// *catchable* exception that would need cross-frame unwinding: integer
/// division by zero faults the VM (it is not a catchable throw), so arithmetic
/// is fine, but any call-type op or `CastClass` could surface a thrown value
/// from elsewhere. `BoxAny`/dynamic ops are excluded so the thrown value can
/// never be an `Any` carrier the typed catch edge cannot thread.
fn try_region_op_allowed(op: Opcode) -> bool {
    matches!(
        op,
        Opcode::Nop
            | Opcode::ConstI32
            | Opcode::ConstI64
            | Opcode::ConstF64
            | Opcode::ConstTrue
            | Opcode::ConstFalse
            | Opcode::ConstStr
            | Opcode::ConstNull
            | Opcode::Conv
            | Opcode::LoadLocal
            | Opcode::StoreLocal
            | Opcode::LoadField
            | Opcode::StoreField
            | Opcode::LoadStatic
            | Opcode::StoreStatic
            | Opcode::Pop
            | Opcode::Dup
            | Opcode::AddI32
            | Opcode::SubI32
            | Opcode::MulI32
            | Opcode::DivI32
            | Opcode::RemI32
            | Opcode::NegI32
            | Opcode::AndI32
            | Opcode::OrI32
            | Opcode::XorI32
            | Opcode::NotI32
            | Opcode::ShlI32
            | Opcode::ShrI32
            | Opcode::UShrI32
            | Opcode::PowI32
            | Opcode::AddF64
            | Opcode::SubF64
            | Opcode::MulF64
            | Opcode::DivF64
            | Opcode::RemF64
            | Opcode::MinI32
            | Opcode::MaxI32
            | Opcode::CmpEq
            | Opcode::CmpNe
            | Opcode::CmpLt
            | Opcode::CmpLe
            | Opcode::CmpGt
            | Opcode::CmpGe
            | Opcode::Branch
            | Opcode::BranchTrue
            | Opcode::BranchFalse
            | Opcode::NewObj
            | Opcode::Concat
            | Opcode::Throw
    )
}

/// Whether a method's exception regions are limited to the same-frame slice the
/// typed JIT can lower. The conditions guarantee a `throw` always lands in this
/// frame's catch, so it compiles to a plain branch with no cross-frame unwinding:
///
/// - exactly one handler, a typed `catch` (no `finally`, no catch-all);
/// - exactly one `throw`, inside the protected region, whose value is a freshly
///   constructed object of *exactly* the catch's declared type (`new T` then
///   `throw`), so it is unconditionally caught here (no type-mismatch escape);
/// - every instruction in the protected region is on the `try_region_op_allowed`
///   whitelist (no call-type op whose callee could surface a thrown value).
///
/// A typed catch binds the exception to a concrete object local the typed JIT
/// represents; a catch-all binds it to `any`, which the `local_kinds` gate
/// already rejects.
fn exception_regions_jit_safe(
    module: &Module,
    body: &zun_module::MethodBody<'_>,
    instrs: &[zun_il::Instruction],
    bridge_available: bool,
) -> bool {
    if body.handlers.len() != 1 {
        return false;
    }
    let handler = &body.handlers[0];
    if handler.kind != zun_module::HandlerKind::Catch {
        return false;
    }
    let Some(catch_type) = handler.catch_type else {
        return false;
    };
    let mut throw_count = 0usize;
    let mut call_in_try = false;
    let mut offset = 0usize;
    let mut prev_newobj_type: Option<u32> = None;
    for instr in instrs {
        let in_try = (handler.try_start as usize) <= offset && offset < handler.try_end as usize;
        if instr.opcode == Opcode::Throw {
            throw_count += 1;
            // An explicit throw must be inside the region and throw a freshly
            // constructed object caught by this frame's handler (so it never
            // escapes to an outer/cross-frame handler).
            match prev_newobj_type {
                Some(throw_rid)
                    if in_try && zun_module::type_is_caught(module, throw_rid, catch_type.rid()) => {}
                _ => return false,
            }
        }
        if in_try {
            // With a bridge, a call inside the region is allowed: a throw from it
            // is routed at run time to this catch (or propagated). `CallBuiltin`
            // stays out — a JIT builtin can't signal a throw through the slot.
            let call_in_region =
                bridge_available && matches!(instr.opcode, Opcode::Call | Opcode::CallVirt);
            if call_in_region {
                call_in_try = true;
            } else if !try_region_op_allowed(instr.opcode) {
                return false;
            }
            // A field access on a null reference raises a TypeError through the same
            // pending-exception slot, so the catch can be reached without a `throw`.
            if bridge_available && matches!(instr.opcode, Opcode::LoadField | Opcode::StoreField) {
                call_in_try = true;
            }
        }
        prev_newobj_type = match (instr.opcode, instr.operand) {
            (Opcode::NewObj, Operand::Token(token)) => Some(token.rid()),
            _ => None,
        };
        offset += instr.encoded_len();
    }
    // The handler must be able to receive a throw: an explicit one, or (with a
    // bridge) a call in the region whose throw is routed at run time. Otherwise
    // the catch block is an unreachable, parameterless stub.
    throw_count >= 1 || call_in_try
}

fn string_builtin(instr: &zun_il::Instruction) -> bool {
    let Operand::Imm32(id) = instr.operand else {
        return false;
    };
    matches!(
        zun_natives::by_id(id as u32).map(|f| f.name),
        Some(
            "length"
                | "contains"
                | "includes"
                | "charCodeAt"
                | "charCodeAtNum"
                | "startsWith"
                | "endsWith"
                | "indexOf"
                | "lastIndexOf"
                | "count"
                | "compare"
                | "concat"
                | "substring"
                | "trim"
                | "trimStart"
                | "trimEnd"
                | "toLowerCase"
                | "toUpperCase"
                | "reverse"
                | "padStart"
                | "padEnd"
                | "replace"
                | "toInt"
                | "toFloat"
                | "toBoolean"
        )
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
                | "mapClear"
                | "setClear"
                | "mapValues"
                | "mapKeys"
                | "mapEntries"
                | "setValues"
                | "setEntries"
                | "mapGet"
                | "mapHasI64"
                | "mapHasI32"
                | "mapGetI32Or"
                | "mapDeleteI64"
                | "setAddI64"
                | "setHasI64"
                | "setHasI32"
                | "setDeleteI64"
                | "mapSetI64"
                | "mapSetI32"
                | "mapGetI64Or"
                | "setAddI32"
                | "mapDeleteI32"
                | "setDeleteI32"
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
                | "dataViewGetInt8"
                | "dataViewGetUint8"
                | "dataViewGetInt16"
                | "dataViewGetInt32"
                | "dataViewGetFloat32"
                | "dataViewSetInt8"
                | "dataViewSetUint8"
                | "dataViewSetInt16"
                | "dataViewSetInt32"
                | "dataViewSetFloat32"
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

fn allocation_builtin(instr: &zun_il::Instruction) -> bool {
    if instr.opcode != Opcode::CallBuiltin {
        return false;
    }
    let Operand::Imm32(id) = instr.operand else {
        return false;
    };
    matches!(
        zun_natives::by_id(id as u32).map(|f| f.name),
        Some("mapNew" | "setNew")
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
/// numeric, bool, object, or an interned string handle — not anything else.
fn field_jit_ok(module: &Module, field_rid: u32) -> bool {
    module
        .tables
        .field_defs
        .get(field_rid as usize - 1)
        .map(|f| {
            matches!(
                JitKind::from_byte(f.kind),
                JitKind::Num(_) | JitKind::Bool | JitKind::Object | JitKind::String
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
        // A static field's declared type must be JIT-representable; the helpers
        // store it in the shared `Any` statics slot by that kind.
        (Opcode::LoadStatic | Opcode::StoreStatic, Operand::Token(t)) => {
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
fn callvirt_is_non_overridden(
    module: &Module,
    instr: &zun_il::Instruction,
    string_method_rids: &HashSet<u32>,
) -> bool {
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
            string_method_rids.contains(&token.rid())
                && matches!(
                    module.strings.get(method.name),
                    "length"
                            | "contains"
                            | "includes"
                            | "charCodeAt"
                            | "startsWith"
                            | "endsWith"
                            | "indexOf"
                            | "lastIndexOf"
                            | "count"
                            | "compare"
                            | "concat"
                            | "substring"
                            | "trim"
                            | "trimStart"
                            | "trimEnd"
                            | "toLowerCase"
                            | "toUpperCase"
                            | "reverse"
                            | "padStart"
                            | "padEnd"
                            | "replace"
                            | "toInt"
                            | "toFloat"
                            | "toBoolean"
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
        .is_some_and(|&kind| matches!(JitKind::from_byte(kind), JitKind::Object | JitKind::String))
}

/// Whether a value-type code is one the typed JIT can represent.
fn kind_supported(byte: u8) -> bool {
    !matches!(JitKind::from_byte(byte), JitKind::Other)
}

/// The current managed allocation lowering is safe for a non-escaping array
/// whose only operation is a length query. Mutation, element access, or a
/// returned reference still requires the interpreter's heap contract.
fn scalar_array_length_only(
    instrs: &[zun_il::Instruction],
    body: &zun_module::MethodBody<'_>,
) -> bool {
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
            matches!(
                instruction.opcode,
                Opcode::LoadArray | Opcode::StoreArray | Opcode::ArrayPop | Opcode::ArrayPush
            )
        })
        && instrs.iter().all(|instruction| {
            matches!(
                instruction.opcode,
                Opcode::Nop
                    | Opcode::ConstI32
                    | Opcode::ConstI64
                    | Opcode::ConstF64
                    | Opcode::ConstTrue
                    | Opcode::ConstFalse
                    | Opcode::Conv
                    | Opcode::UnboxAny
                    | Opcode::BoxAny
                    | Opcode::Pop
                    | Opcode::LoadLocal
                    | Opcode::StoreLocal
                    | Opcode::NewArray
                    | Opcode::LoadArray
                    | Opcode::StoreArray
                    | Opcode::ArrayPop
                    | Opcode::ArrayPush
                    | Opcode::Ret
            )
        })
}

fn scalar_i64_array_access_only(
    instrs: &[zun_il::Instruction],
    body: &zun_module::MethodBody<'_>,
) -> bool {
    body.returns_value
        && JitKind::from_byte(body.return_kind) == JitKind::Num(zun_il::NumKind::I64)
        && instrs.iter().any(|instruction| {
            matches!(
                instruction.opcode,
                Opcode::LoadArray | Opcode::StoreArray | Opcode::ArrayPop | Opcode::ArrayPush
            )
        })
        && instrs.iter().all(|instruction| {
            matches!(
                instruction.opcode,
                Opcode::Nop
                    | Opcode::ConstI32
                    | Opcode::ConstI64
                    | Opcode::ConstF64
                    | Opcode::Conv
                    | Opcode::UnboxAny
                    | Opcode::BoxAny
                    | Opcode::AddI32
                    | Opcode::CmpLt
                    | Opcode::Branch
                    | Opcode::BranchFalse
                    | Opcode::Dup
                    | Opcode::Pop
                    | Opcode::LoadLocal
                    | Opcode::StoreLocal
                    | Opcode::NewArray
                    | Opcode::LoadArray
                    | Opcode::StoreArray
                    | Opcode::ArrayPop
                    | Opcode::ArrayPush
                    | Opcode::Ret
            )
        })
}

fn scalar_i32_array_access_only(
    instrs: &[zun_il::Instruction],
    body: &zun_module::MethodBody<'_>,
) -> bool {
    body.returns_value
        && JitKind::from_byte(body.return_kind) == JitKind::Num(zun_il::NumKind::I32)
        && instrs.iter().any(|instruction| {
            matches!(
                instruction.opcode,
                Opcode::LoadArray | Opcode::StoreArray | Opcode::ArrayPop | Opcode::ArrayPush
            )
        })
        && instrs.iter().all(|instruction| {
            matches!(
                instruction.opcode,
                Opcode::Nop
                    | Opcode::ConstI32
                    | Opcode::ConstI64
                    | Opcode::ConstF64
                    | Opcode::Conv
                    | Opcode::UnboxAny
                    | Opcode::BoxAny
                    | Opcode::AddI32
                    | Opcode::CmpLt
                    | Opcode::Branch
                    | Opcode::BranchFalse
                    | Opcode::Dup
                    | Opcode::Pop
                    | Opcode::LoadLocal
                    | Opcode::StoreLocal
                    | Opcode::NewArray
                    | Opcode::LoadArray
                    | Opcode::StoreArray
                    | Opcode::ArrayPop
                    | Opcode::ArrayPush
                    | Opcode::Ret
            )
        })
}

fn scalar_narrow_array_access_only(
    instrs: &[zun_il::Instruction],
    body: &zun_module::MethodBody<'_>,
) -> bool {
    body.returns_value
        && matches!(
            JitKind::from_byte(body.return_kind),
            JitKind::Num(
                zun_il::NumKind::I8
                    | zun_il::NumKind::U8
                    | zun_il::NumKind::I16
                    | zun_il::NumKind::U16
            )
        )
        && instrs.iter().any(|instruction| {
            matches!(
                instruction.opcode,
                Opcode::LoadArray | Opcode::StoreArray | Opcode::ArrayPop | Opcode::ArrayPush
            )
        })
        && instrs.iter().all(|instruction| {
            matches!(
                instruction.opcode,
                Opcode::Nop
                    | Opcode::ConstI32
                    | Opcode::ConstI64
                    | Opcode::ConstF64
                    | Opcode::Conv
                    | Opcode::UnboxAny
                    | Opcode::BoxAny
                    | Opcode::AddI32
                    | Opcode::CmpLt
                    | Opcode::Branch
                    | Opcode::BranchFalse
                    | Opcode::Dup
                    | Opcode::Pop
                    | Opcode::LoadLocal
                    | Opcode::StoreLocal
                    | Opcode::NewArray
                    | Opcode::LoadArray
                    | Opcode::StoreArray
                    | Opcode::ArrayPop
                    | Opcode::ArrayPush
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
            matches!(
                instruction.opcode,
                Opcode::LoadArray | Opcode::StoreArray | Opcode::ArrayPop
            )
        })
        && instrs.iter().all(|instruction| {
            matches!(
                instruction.opcode,
                Opcode::Nop
                    | Opcode::ConstI32
                    | Opcode::ConstI64
                    | Opcode::ConstF64
                    | Opcode::ConstTrue
                    | Opcode::ConstFalse
                    | Opcode::UnboxAny
                    | Opcode::BoxAny
                    | Opcode::Pop
                    | Opcode::LoadLocal
                    | Opcode::StoreLocal
                    | Opcode::NewArray
                    | Opcode::LoadArray
                    | Opcode::StoreArray
                    | Opcode::ArrayPop
                    | Opcode::Ret
            )
        })
}

fn scalar_f64_array_access_only(
    instrs: &[zun_il::Instruction],
    body: &zun_module::MethodBody<'_>,
) -> bool {
    body.returns_value
        && JitKind::from_byte(body.return_kind) == JitKind::Num(zun_il::NumKind::F64)
        && instrs.iter().any(|instruction| {
            matches!(
                instruction.opcode,
                Opcode::LoadArray | Opcode::StoreArray | Opcode::ArrayPop
            )
        })
        && instrs.iter().all(|instruction| {
            matches!(
                instruction.opcode,
                Opcode::Nop
                    | Opcode::ConstI32
                    | Opcode::ConstI64
                    | Opcode::ConstF64
                    | Opcode::Conv
                    | Opcode::UnboxAny
                    | Opcode::BoxAny
                    | Opcode::Pop
                    | Opcode::LoadLocal
                    | Opcode::StoreLocal
                    | Opcode::NewArray
                    | Opcode::LoadArray
                    | Opcode::StoreArray
                    | Opcode::ArrayPop
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
        && matches!(
            JitKind::from_byte(body.return_kind),
            JitKind::Object | JitKind::Array
        )
        && instrs
            .iter()
            .any(|instruction| matches!(instruction.opcode, Opcode::LoadArray | Opcode::StoreArray))
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

fn object_array_stack_only(
    instrs: &[zun_il::Instruction],
    body: &zun_module::MethodBody<'_>,
) -> bool {
    (body.returns_value
        && matches!(
            JitKind::from_byte(body.return_kind),
            JitKind::Object | JitKind::Array
        )
        && instrs.iter().any(|i| i.opcode == Opcode::ArrayPop)
        && instrs.iter().all(|i| {
            matches!(
                i.opcode,
                Opcode::Nop
                    | Opcode::LoadLocal
                    | Opcode::StoreLocal
                    | Opcode::ArrayPop
                    | Opcode::Ret
            )
        }))
        || (body.returns_value
            && instrs.iter().any(|i| i.opcode == Opcode::ArrayPush)
            && instrs.iter().all(|i| {
                matches!(
                    i.opcode,
                    Opcode::Nop
                        | Opcode::LoadLocal
                        | Opcode::StoreLocal
                        | Opcode::ArrayPush
                        | Opcode::Ret
                )
            }))
}

fn any_array_return_only(
    instrs: &[zun_il::Instruction],
    body: &zun_module::MethodBody<'_>,
) -> bool {
    body.returns_value
        && JitKind::from_byte(body.return_kind) == JitKind::Other
        && instrs.iter().any(|i| i.opcode == Opcode::LoadArray)
        && instrs.iter().all(|i| {
            matches!(
                i.opcode,
                Opcode::Nop
                    | Opcode::ConstI32
                    | Opcode::ConstI64
                    | Opcode::LoadLocal
                    | Opcode::LoadArray
                    | Opcode::Ret
            )
        })
}

fn any_param_return_only(
    instrs: &[zun_il::Instruction],
    body: &zun_module::MethodBody<'_>,
) -> bool {
    body.returns_value
        && JitKind::from_byte(body.return_kind) == JitKind::Other
        && body.param_count > 0
        && body.local_kinds[..body.param_count as usize]
            .iter()
            .any(|&kind| JitKind::from_byte(kind) == JitKind::Other)
        && instrs
            .iter()
            .all(|i| matches!(i.opcode, Opcode::Nop | Opcode::LoadLocal | Opcode::Ret))
}

fn any_direct_call_return_only(
    instrs: &[zun_il::Instruction],
    body: &zun_module::MethodBody<'_>,
) -> bool {
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
        && instrs.iter().all(|i| {
            matches!(
                i.opcode,
                Opcode::Nop | Opcode::LoadLocal | Opcode::Call | Opcode::CallVirt | Opcode::Ret
            )
        })
}

fn any_map_get_only(instrs: &[zun_il::Instruction], body: &zun_module::MethodBody<'_>) -> bool {
    body.returns_value
        && JitKind::from_byte(body.return_kind) == JitKind::Other
        && body.param_count > 0
        && body.local_kinds[..body.param_count as usize]
            .iter()
            .any(|&kind| JitKind::from_byte(kind) == JitKind::Other)
        && instrs
            .iter()
            .filter(|i| i.opcode == Opcode::CallBuiltin)
            .count()
            == 1
        && instrs.iter().all(|i| {
            matches!(
                i.opcode,
                Opcode::Nop | Opcode::LoadLocal | Opcode::CallBuiltin | Opcode::Ret
            )
        })
}

/// Whether a method's signature can cross the bridge's packed-value ABI: every
/// parameter and the return must be a single-word JIT kind (not `Any`/`Other`,
/// which needs the two-word carrier the bridge does not yet marshal).
fn bridge_marshalable(module: &Module, rid: u32) -> bool {
    let Some(method) = module.tables.method_defs.get(rid as usize - 1) else {
        return false;
    };
    if method.body_offset == u32::MAX {
        return false;
    }
    let Ok(body) = read_body(&module.code, method.body_offset) else {
        return false;
    };
    let params_ok = body.local_kinds[..body.param_count as usize]
        .iter()
        .all(|&byte| JitKind::from_byte(byte) != JitKind::Other);
    let return_ok = !body.returns_value || JitKind::from_byte(body.return_kind) != JitKind::Other;
    params_ok && return_ok
}

/// Whether the method `rid` returns a function value. A call to such a method
/// would push a function value that may be a closure or bound function, which
/// the no-closure one-word JIT representation cannot carry. A bodyless (extern)
/// target is treated as not returning a function (externs returning closures do
/// not occur in practice).
fn call_returns_function(module: &Module, rid: u32, function_byte: u8) -> bool {
    if rid == 0 {
        return false;
    }
    let Some(method) = module.tables.method_defs.get(rid as usize - 1) else {
        return false;
    };
    if method.body_offset == u32::MAX {
        return false;
    }
    match read_body(&module.code, method.body_offset) {
        Ok(body) => body.returns_value && body.return_kind == function_byte,
        Err(_) => false,
    }
}

/// Compute the set of JIT-eligible method RIDs for `module`.
/// The half-open IL byte region `[target_pc, end)` of the loop whose header is
/// at `target_pc`, or `None` when no backedge targets it (or the body cannot be
/// decoded, or `target_pc` is not an instruction boundary). `end` is the end of
/// the furthest instruction that branches back to the header — i.e. the loop
/// body, the unit an OSR entry would compile (slice 3 of the module-JIT epic,
/// `docs/design/module-jit-osr.md`).
pub fn loop_region(module: &Module, method_rid: u32, target_pc: usize) -> Option<(usize, usize)> {
    let method = module.tables.method_defs.get(method_rid as usize - 1)?;
    let body = zun_module::read_body(&module.code, method.body_offset).ok()?;
    let instrs = decode_all(body.code).ok()?;
    let mut offset = 0usize;
    let mut header_on_boundary = false;
    let mut end: Option<usize> = None;
    for instr in &instrs {
        if offset == target_pc {
            header_on_boundary = true;
        }
        let next = offset + instr.encoded_len();
        if let Operand::Branch(displacement) = instr.operand {
            let target = next as i64 + i64::from(displacement);
            if target == target_pc as i64 && next > target_pc {
                end = Some(end.map_or(next, |e| e.max(next)));
            }
        }
        offset = next;
    }
    if !header_on_boundary {
        return None;
    }
    end.map(|e| (target_pc, e))
}

/// Whether every instruction in the loop region at `target_pc` is JIT-lowerable,
/// independent of any ineligible dynamic op in the method's head or tail. This
/// is the gate an OSR entry compiler consumes: a method whose `<main>`-style
/// body is otherwise ineligible can still have a hot loop region that is. Uses
/// the same per-instruction predicates as whole-method eligibility.
pub fn loop_region_eligible(module: &Module, method_rid: u32, target_pc: usize) -> bool {
    loop_region_rejection(module, method_rid, target_pc).is_none()
}

/// Why the loop region headed at `target_pc` cannot be compiled, or `None`
/// when it can. The reason names the first offending instruction, which is
/// what `ZUN_JIT_TRACE` prints: a loop that stays interpreted is the single
/// largest performance cliff, and the cause is otherwise invisible.
pub fn loop_region_rejection(module: &Module, method_rid: u32, target_pc: usize) -> Option<String> {
    let Some((start, end)) = loop_region(module, method_rid, target_pc) else {
        return Some("no loop region".into());
    };
    let Some(method) = module.tables.method_defs.get(method_rid as usize - 1) else {
        return Some("no method".into());
    };
    let Ok(body) = zun_module::read_body(&module.code, method.body_offset) else {
        return Some("unreadable body".into());
    };
    let Ok(instrs) = decode_all(body.code) else {
        return Some("undecodable body".into());
    };
    let string_method_rids: HashSet<u32> = module
        .tables
        .type_defs
        .iter()
        .filter(|ty| module.strings.get(ty.name) == "String")
        .flat_map(|ty| ty.method_start..ty.method_end)
        .collect();
    if !const_null_uses_ok(module, &body, &instrs, start..end) {
        return Some("const.null where the target is not a class reference".into());
    }
    let mut offset = 0usize;
    for instr in &instrs {
        let next = offset + instr.encoded_len();
        if offset >= start && offset < end {
            let why = if requires_exception_bridge(instr.opcode) {
                Some("needs the exception bridge")
            } else if !opcode_supported(instr.opcode, instr) {
                Some("opcode not lowered")
            } else if !instr_fields_ok(module, instr) {
                Some("field kind not lowered")
            } else if !callvirt_is_non_overridden(module, instr, &string_method_rids) {
                Some("overridden virtual call")
            } else {
                None
            };
            if let Some(why) = why {
                let detail = match (instr.opcode, &instr.operand) {
                    (Opcode::CallBuiltin, Operand::Imm32(id)) => zun_natives::by_id(*id as u32)
                        .map(|f| format!(" {}", f.name))
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                return Some(format!("{why}: {:?}{detail} at {offset:#06x}", instr.opcode));
            }
        }
        offset = next;
    }
    None
}

pub fn eligible_methods(module: &Module) -> HashSet<u32> {
    eligible_methods_with_literals(module, false)
}

/// Eligibility when a JIT→interpreter call bridge is available: a method may be
/// eligible even if it directly calls an **ineligible, non-throwing** method,
/// because that call is bridged to the interpreter. (Throwing ineligible callees
/// still disqualify the caller until the exceptional-return channel lands.)
pub fn eligible_methods_bridged(module: &Module, allow_const_str: bool) -> HashSet<u32> {
    eligible_impl(module, allow_const_str, true)
}

/// Compute runtime-JIT eligibility when the JIT owns stable copies of module
/// string bytes. Relocatable/AOT objects must keep the default false mode:
/// their code cannot embed a process-local module-string pointer.
pub fn eligible_methods_with_literals(module: &Module, allow_const_str: bool) -> HashSet<u32> {
    eligible_impl(module, allow_const_str, false)
}

/// Shared eligibility implementation. `bridge_available` relaxes the
/// whole-call-graph fixpoint so a method that directly calls an ineligible but
/// non-throwing method stays eligible (the call is bridged to the interpreter).
fn eligible_impl(module: &Module, allow_const_str: bool, bridge_available: bool) -> HashSet<u32> {
    analyze(module, allow_const_str, bridge_available).eligible
}

/// Why each method with a body is not JIT-eligible, keyed by method rid. The
/// text is the `Err` of the same per-method analysis `eligible_impl` decides
/// with, so the explanation cannot drift from the decision. A method rejected
/// only because of a callee reads `callee <name>: <callee reason>`.
pub fn method_rejections(
    module: &Module,
    allow_const_str: bool,
    bridge_available: bool,
) -> HashMap<u32, String> {
    analyze(module, allow_const_str, bridge_available).rejected
}

struct Analysis {
    eligible: HashSet<u32>,
    rejected: HashMap<u32, String>,
    /// Methods whose execution may run a collection: they allocate, or call a method
    /// that does, or call one the interpreter runs. See [`gc_profile`].
    collecting: HashSet<u32>,
}


/// Whether instruction `instr` pushes a class reference (an `Object`-kind value).
fn pushes_object(
    module: &Module,
    body: &zun_module::MethodBody<'_>,
    instr: &zun_il::Instruction,
) -> bool {
    let field_is_object = |rid: u32| {
        module
            .tables
            .field_defs
            .get((rid as usize).wrapping_sub(1))
            .is_some_and(|field| JitKind::from_byte(field.kind) == JitKind::Object)
    };
    match (instr.opcode, instr.operand) {
        (Opcode::LoadLocal, Operand::Local(slot)) => body
            .local_kinds
            .get(slot as usize)
            .is_some_and(|&kind| JitKind::from_byte(kind) == JitKind::Object),
        (Opcode::LoadField | Opcode::LoadStatic, Operand::Token(token)) => {
            field_is_object(token.rid())
        }
        (Opcode::NewObj | Opcode::ConstNull, _) => true,
        (Opcode::Call | Opcode::CallVirt, Operand::Token(token)) => module
            .tables
            .method_defs
            .get((token.rid() as usize).wrapping_sub(1))
            .and_then(|method| read_body(&module.code, method.body_offset).ok())
            .is_some_and(|callee| {
                callee.returns_value && JitKind::from_byte(callee.return_kind) == JitKind::Object
            }),
        _ => false,
    }
}

/// `const.null` is the zero handle of a class reference, so it is only meaningful
/// where a class reference is: compared with one, or stored, passed or returned
/// into a slot of class type. Anywhere else (an `any`, a number parameter that
/// defaults when `null` is passed) zero means something else, and the translator
/// would refuse it; saying so here keeps eligibility equal to what is lowered.
fn const_null_uses_ok(
    module: &Module,
    body: &zun_module::MethodBody<'_>,
    instrs: &[zun_il::Instruction],
    within: std::ops::Range<usize>,
) -> bool {
    let field_is_object = |token: zun_metadata::Token| {
        module
            .tables
            .field_defs
            .get((token.rid() as usize).wrapping_sub(1))
            .is_some_and(|field| JitKind::from_byte(field.kind) == JitKind::Object)
    };
    let mut offset = 0usize;
    for (i, instr) in instrs.iter().enumerate() {
        let here = offset;
        offset += instr.encoded_len();
        if instr.opcode != Opcode::ConstNull || !within.contains(&here) {
            continue;
        }
        // Several nulls in a row feed one `newobj`.
        let mut next = i + 1;
        while instrs.get(next).is_some_and(|n| n.opcode == Opcode::ConstNull) {
            next += 1;
        }
        let Some(consumer) = instrs.get(next) else {
            return false;
        };
        let ok = match (consumer.opcode, consumer.operand) {
            (Opcode::StoreLocal, Operand::Local(slot)) => body
                .local_kinds
                .get(slot as usize)
                .is_some_and(|&kind| JitKind::from_byte(kind) == JitKind::Object),
            (Opcode::StoreField | Opcode::StoreStatic, Operand::Token(token)) => {
                field_is_object(token)
            }
            (Opcode::Ret, _) => {
                body.returns_value && JitKind::from_byte(body.return_kind) == JitKind::Object
            }
            (Opcode::NewObj, _) => true,
            (Opcode::Call | Opcode::CallVirt, Operand::Token(token)) => module
                .tables
                .method_defs
                .get((token.rid() as usize).wrapping_sub(1))
                .and_then(|method| read_body(&module.code, method.body_offset).ok())
                .is_some_and(|callee| {
                    callee.param_count > 0
                        && callee
                            .local_kinds
                            .get(callee.param_count as usize - 1)
                            .is_some_and(|&kind| JitKind::from_byte(kind) == JitKind::Object)
                }),
            // `p === null`: the other operand was pushed just before.
            (Opcode::CmpEq | Opcode::CmpNe, _) => {
                next == i + 1
                    && i > 0
                    && pushes_object(module, body, &instrs[i - 1])
            }
            // `null === p`: the other operand follows and then the compare.
            _ if next == i + 1
                && pushes_object(module, body, consumer)
                && instrs
                    .get(next + 1)
                    .is_some_and(|c| matches!(c.opcode, Opcode::CmpEq | Opcode::CmpNe)) =>
            {
                true
            }
            _ => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// Whether the body keeps a managed reference live across a call that can collect,
/// counting only calls whose byte offset lies in `within`.
///
/// A collection scans the native frame that allocates and no other, so a compiled
/// frame that is suspended in such a call and holds a reference would resume on a
/// stale address once the object moved. A reference is held across the call when a
/// reference-kind local is live after it (backward liveness over the locals, loops
/// and handlers included) or when the operand stack below the call's arguments may
/// hold one (a forward walk of "may be a reference" per stack slot). The walk gives
/// up, reporting a hazard, on any opcode it has no rule for.
///
/// `collects` decides, for a call instruction, whether its callee can run a
/// collection (a callee the interpreter runs, or one that allocates).
fn holds_reference_across_collecting_call(
    module: &Module,
    body: &zun_module::MethodBody<'_>,
    instrs: &[zun_il::Instruction],
    within: std::ops::Range<usize>,
    collects: &dyn Fn(&zun_il::Instruction) -> bool,
) -> bool {
    let count = instrs.len();
    let mut offsets = Vec::with_capacity(count + 1);
    let mut index_of_offset: HashMap<usize, usize> = HashMap::new();
    let mut at = 0usize;
    for (i, instr) in instrs.iter().enumerate() {
        offsets.push(at);
        index_of_offset.insert(at, i);
        at += instr.encoded_len();
    }
    offsets.push(at);
    index_of_offset.insert(at, count);

    let branch_target = |i: usize| -> Option<usize> {
        match instrs[i].operand {
            Operand::Branch(displacement) => {
                let target = offsets[i + 1] as i64 + i64::from(displacement);
                index_of_offset.get(&(target as usize)).copied()
            }
            _ => None,
        }
    };
    // Successors of an instruction, handlers included: anything in a protected
    // range may continue at the handler.
    let successors = |i: usize| -> Vec<usize> {
        let mut out = Vec::new();
        match instrs[i].opcode {
            Opcode::Branch => out.extend(branch_target(i)),
            Opcode::BranchTrue | Opcode::BranchFalse => {
                out.extend(branch_target(i));
                out.push(i + 1);
            }
            Opcode::Ret | Opcode::Throw | Opcode::Rethrow => {}
            _ => out.push(i + 1),
        }
        for handler in &body.handlers {
            if (handler.try_start as usize) <= offsets[i] && offsets[i] < handler.try_end as usize {
                out.extend(index_of_offset.get(&(handler.handler_start as usize)).copied());
            }
        }
        out.retain(|&s| s < count);
        out
    };

    let locals = body.local_kinds.len();
    let words = locals.div_ceil(64).max(1);
    let is_reference_local: Vec<bool> = body
        .local_kinds
        .iter()
        .map(|&kind| kind_is_reference(kind))
        .collect();

    // Backward liveness of the reference locals.
    let mut live_in = vec![vec![0u64; words]; count + 1];
    loop {
        let mut changed = false;
        for i in (0..count).rev() {
            let mut out = vec![0u64; words];
            for s in successors(i) {
                for (o, w) in out.iter_mut().zip(&live_in[s]) {
                    *o |= *w;
                }
            }
            if let (Opcode::StoreLocal, Operand::Local(slot)) = (instrs[i].opcode, instrs[i].operand)
            {
                let slot = slot as usize;
                if slot < locals {
                    out[slot / 64] &= !(1u64 << (slot % 64));
                }
            }
            if let (Opcode::LoadLocal, Operand::Local(slot)) = (instrs[i].opcode, instrs[i].operand)
            {
                let slot = slot as usize;
                if slot < locals && is_reference_local[slot] {
                    out[slot / 64] |= 1u64 << (slot % 64);
                }
            }
            if out != live_in[i] {
                live_in[i] = out;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let live_after = |i: usize| -> bool {
        successors(i)
            .into_iter()
            .any(|s| live_in[s].iter().any(|&word| word != 0))
    };

    // Forward walk of the operand stack: may each slot hold a reference?
    let field_is_reference = |rid: u32| {
        module
            .tables
            .field_defs
            .get((rid as usize).wrapping_sub(1))
            .is_none_or(|field| kind_is_reference(field.kind))
    };
    let callee_body = |rid: u32| {
        module
            .tables
            .method_defs
            .get((rid as usize).wrapping_sub(1))
            .and_then(|method| read_body(&module.code, method.body_offset).ok())
    };
    // The walk covers only `within`: a loop region is entered with an empty stack,
    // and what the rest of the method does with the operand stack is not its
    // business (nor is an opcode this walk has no rule for out there).
    let first = index_of_offset.get(&within.start).copied().unwrap_or(0);
    let mut entry: Vec<Option<Vec<bool>>> = vec![None; count + 1];
    entry[first] = Some(Vec::new());
    let mut work = vec![first];
    for handler in &body.handlers {
        if !within.contains(&(handler.handler_start as usize)) {
            continue;
        }
        if let Some(&h) = index_of_offset.get(&(handler.handler_start as usize)) {
            if h < count && entry[h].is_none() {
                entry[h] = Some(vec![true]);
                work.push(h);
            }
        }
    }
    while let Some(i) = work.pop() {
        let Some(mut stack) = entry[i].clone() else {
            continue;
        };
        let instr = &instrs[i];
        // (values popped, values pushed, whether a pushed value may be a reference;
        // `None` = decided per opcode below).
        let (pops, pushes, pushes_reference): (usize, usize, Option<bool>) =
            match (instr.opcode, instr.operand) {
                (Opcode::Call | Opcode::CallVirt, Operand::Token(token)) => {
                    let Some(callee) = callee_body(token.rid()) else {
                        return true;
                    };
                    let returns_reference =
                        callee.returns_value && kind_is_reference(callee.return_kind);
                    (callee.param_count as usize, 1, Some(returns_reference))
                }
                (Opcode::NewObj, Operand::Token(token)) => {
                    let Some(ty) = module
                        .tables
                        .type_defs
                        .get((token.rid() as usize).wrapping_sub(1))
                    else {
                        return true;
                    };
                    ((ty.field_end - ty.field_start) as usize, 1, Some(true))
                }
                (Opcode::CallBuiltin, Operand::Imm32(id)) => {
                    let Some(native) = zun_natives::by_id(id as u32) else {
                        return true;
                    };
                    (native.params.len(), 1, Some(true))
                }
                (Opcode::CallIndirect, _) => return true,
                (opcode, _) => match opcode.stack_effect() {
                    zun_il::StackEffect::Fixed { pops, pushes } => {
                        let name = format!("{opcode:?}");
                        let numeric = matches!(
                            opcode,
                            Opcode::ConstI32
                                | Opcode::ConstI64
                                | Opcode::ConstF64
                                | Opcode::ConstTrue
                                | Opcode::ConstFalse
                                | Opcode::ConstStr
                                | Opcode::Conv
                                | Opcode::BoxAny
                                | Opcode::ArrayLength
                                | Opcode::CmpEq
                                | Opcode::CmpNe
                                | Opcode::CmpLt
                                | Opcode::CmpLe
                                | Opcode::CmpGt
                                | Opcode::CmpGe
                        ) || name.ends_with("I32")
                            || name.ends_with("F64");
                        (
                            pops as usize,
                            pushes as usize,
                            if numeric { Some(false) } else { None },
                        )
                    }
                    zun_il::StackEffect::Terminator => (
                        usize::from(matches!(opcode, Opcode::Ret | Opcode::Throw)),
                        0,
                        None,
                    ),
                    zun_il::StackEffect::Call => return true,
                },
            };
        if stack.len() < pops {
            return true;
        }
        let args_start = stack.len() - pops;
        let is_call = matches!(instr.opcode, Opcode::Call | Opcode::CallVirt);
        if is_call
            && within.contains(&offsets[i])
            && collects(instr)
            && (stack[..args_start].iter().any(|&held| held) || live_after(i))
        {
            return true;
        }
        let popped: Vec<bool> = stack.split_off(args_start);
        match instr.opcode {
            Opcode::Dup => {
                let top = popped.last().copied().unwrap_or(true);
                stack.push(top);
                stack.push(top);
            }
            Opcode::LoadLocal => {
                let reference = match instr.operand {
                    Operand::Local(slot) => {
                        is_reference_local.get(slot as usize).copied().unwrap_or(true)
                    }
                    _ => true,
                };
                stack.push(reference);
            }
            Opcode::LoadField | Opcode::LoadStatic => {
                let reference = match instr.operand {
                    Operand::Token(token) => field_is_reference(token.rid()),
                    _ => true,
                };
                stack.push(reference);
            }
            Opcode::ConstNull | Opcode::NewArray | Opcode::LoadArray | Opcode::ArrayPop => {
                stack.push(true)
            }
            _ => {
                for _ in 0..pushes {
                    stack.push(pushes_reference.unwrap_or(true));
                }
            }
        }
        for s in successors(i) {
            if !within.contains(&offsets[s]) {
                continue;
            }
            match &entry[s] {
                None => {
                    entry[s] = Some(stack.clone());
                    work.push(s);
                }
                Some(existing) => {
                    if existing.len() != stack.len() {
                        return true;
                    }
                    let merged: Vec<bool> =
                        existing.iter().zip(&stack).map(|(&a, &b)| a || b).collect();
                    if merged != *existing {
                        entry[s] = Some(merged);
                        work.push(s);
                    }
                }
            }
        }
    }
    false
}

/// What a method body does that matters to a moving collection.
struct GcProfile {
    /// It allocates itself, so running it can start a collection.
    allocates: bool,
}

/// Whether `instr` can start a collection from compiled code. Only `newobj` can:
/// it allocates through `zun_rt_alloc`, the one entry point that collects. The
/// array, string and collection helpers allocate without ever collecting, and a
/// `CallIndirect` runs arbitrary code through the bridge.
fn instruction_allocates(instr: &zun_il::Instruction) -> bool {
    matches!(instr.opcode, Opcode::NewObj | Opcode::CallIndirect)
}

fn kind_is_reference(byte: u8) -> bool {
    matches!(JitKind::from_byte(byte), JitKind::Object | JitKind::Array)
}

fn gc_profile(module: &Module, body_offset: u32) -> Option<GcProfile> {
    let body = read_body(&module.code, body_offset).ok()?;
    let instrs = decode_all(body.code).ok()?;
    Some(GcProfile {
        allocates: instrs.iter().any(instruction_allocates),
    })
}

/// Whether the call instruction `instr` can run a collection, given which methods
/// `may_collect`. A virtual call reaches every method of that name.
fn call_may_collect(
    module: &Module,
    instr: &zun_il::Instruction,
    methods_by_name: &HashMap<zun_metadata::StringIndex, Vec<u32>>,
    may_collect: &dyn Fn(u32) -> bool,
) -> bool {
    match (instr.opcode, instr.operand) {
        (Opcode::Call, Operand::Token(token)) => may_collect(token.rid()),
        (Opcode::CallVirt, Operand::Token(token)) => {
            let Some(method) = module.tables.method_defs.get((token.rid() as usize).wrapping_sub(1))
            else {
                return true;
            };
            may_collect(token.rid())
                || methods_by_name
                    .get(&method.name)
                    .is_some_and(|rids| rids.iter().any(|&rid| may_collect(rid)))
        }
        _ => true,
    }
}

/// The eligible methods whose execution can run a collection: they allocate, or
/// call something that does, or call a method the interpreter runs.
fn collecting_methods(
    candidate: &HashMap<u32, Vec<(u32, bool)>>,
    profiles: &HashMap<u32, GcProfile>,
    eligible: &HashSet<u32>,
) -> HashSet<u32> {
    let mut collecting: HashSet<u32> = eligible
        .iter()
        .copied()
        .filter(|rid| profiles.get(rid).is_some_and(|profile| profile.allocates))
        .collect();
    loop {
        let before = collecting.len();
        for rid in eligible {
            if collecting.contains(rid) {
                continue;
            }
            if candidate[rid]
                .iter()
                .any(|&(c, _)| !eligible.contains(&c) || collecting.contains(&c))
            {
                collecting.insert(*rid);
            }
        }
        if collecting.len() == before {
            return collecting;
        }
    }
}

/// The JIT's eligible methods together with the ones that may run a collection
/// (see [`GcProfile`]). The second set is what a loop region consults: a region
/// that keeps references in its locals must not call into it.
pub fn eligible_and_collecting(
    module: &Module,
    allow_const_str: bool,
    bridge_available: bool,
) -> (HashSet<u32>, HashSet<u32>) {
    let analysis = analyze(module, allow_const_str, bridge_available);
    (analysis.eligible, analysis.collecting)
}

/// Whether the loop at `target_pc` keeps a managed reference live across a call
/// that can collect. A collection scans only the frame that allocates, so the loop
/// frame would resume on stale addresses; such a loop stays on the interpreter.
pub fn loop_region_holds_references_across_collecting_call(
    module: &Module,
    method_rid: u32,
    target_pc: usize,
    eligible: &HashSet<u32>,
    collecting: &HashSet<u32>,
) -> bool {
    let Some((start, end)) = loop_region(module, method_rid, target_pc) else {
        return false;
    };
    let Some(method) = module.tables.method_defs.get(method_rid as usize - 1) else {
        return false;
    };
    let Ok(body) = read_body(&module.code, method.body_offset) else {
        return true;
    };
    let Ok(instrs) = decode_all(body.code) else {
        return true;
    };
    let mut methods_by_name: HashMap<_, Vec<u32>> = HashMap::new();
    for (index, method) in module.tables.method_defs.iter().enumerate() {
        methods_by_name
            .entry(method.name)
            .or_default()
            .push(index as u32 + 1);
    }
    let may_collect = |rid: u32| !eligible.contains(&rid) || collecting.contains(&rid);
    holds_reference_across_collecting_call(module, &body, &instrs, start..end, &|call| {
        call_may_collect(module, call, &methods_by_name, &may_collect)
    })
}
fn kind_name(byte: u8) -> String {
    match JitKind::from_byte(byte) {
        JitKind::Other => format!("Other(byte {byte})"),
        k => format!("{k:?}"),
    }
}

/// The first instruction `opcode_supported` refuses, formatted for a report.
fn first_unsupported(instrs: &[zun_il::Instruction]) -> Option<String> {
    let i = instrs.iter().find(|i| !opcode_supported(i.opcode, i))?;
    Some(match (i.opcode, &i.operand) {
        (Opcode::CallBuiltin, Operand::Imm32(id)) => format!(
            "builtin {}",
            zun_natives::by_id(*id as u32)
                .map(|f| f.name.to_string())
                .unwrap_or_else(|| format!("#{id}"))
        ),
        (op, _) => format!("opcode {op:?}"),
    })
}

fn analyze(module: &Module, allow_const_str: bool, bridge_available: bool) -> Analysis {
    let mut methods_by_name: HashMap<_, Vec<u32>> = HashMap::new();
    for (index, method) in module.tables.method_defs.iter().enumerate() {
        methods_by_name
            .entry(method.name)
            .or_default()
            .push(index as u32 + 1);
    }
    // Method rids owned by the `String` type, precomputed once. The CallVirt
    // non-override check only needs to know whether a method belongs to String,
    // which previously meant scanning every type_def for every CallVirt
    // instruction (O(instructions * type_defs)).
    let string_method_rids: HashSet<u32> = module
        .tables
        .type_defs
        .iter()
        .filter(|ty| module.strings.get(ty.name) == "String")
        .flat_map(|ty| ty.method_start..ty.method_end)
        .collect();
    // Callees per candidate, tagged `is_virtual`: a `CallVirt` callee must be
    // eligible (the translator lowers it through the vtable, not the bridge),
    // while a direct `Call` callee may be bridged when ineligible.
    let mut candidate: HashMap<u32, Vec<(u32, bool)>> = HashMap::new();
    let mut rejected: HashMap<u32, String> = HashMap::new();
    for (i, method) in module.tables.method_defs.iter().enumerate() {
        let rid = i as u32 + 1;
        if method.body_offset == u32::MAX {
            continue;
        }
        match method_candidate(
            module,
            method.body_offset,
            allow_const_str,
            bridge_available,
            &methods_by_name,
            &string_method_rids,
        ) {
            Ok(callees) => {
                candidate.insert(rid, callees);
            }
            Err(why) => {
                rejected.insert(rid, why);
            }
        }
    }

    let profiles: HashMap<u32, GcProfile> = candidate
        .keys()
        .filter_map(|&rid| {
            let method = &module.tables.method_defs[rid as usize - 1];
            gc_profile(module, method.body_offset).map(|profile| (rid, profile))
        })
        .collect();
    let mut eligible: HashSet<u32> = candidate.keys().copied().collect();
    let mut unsafe_to_hold: HashSet<u32> = HashSet::new();
    let mut collecting: HashSet<u32>;
    loop {
        loop {
            let prev = eligible.clone();
            // A callee must be eligible, OR (with a bridge) ineligible but provably
            // non-throwing — then the call is routed to the interpreter.
            eligible.retain(|rid| {
                candidate[rid].iter().all(|&(c, is_virtual)| {
                    if prev.contains(&c) {
                        return true;
                    }
                    // Only a direct `Call` to an ineligible method is bridged (the
                    // translator does not bridge `CallVirt`). With the exceptional-
                    // return channel, such a callee may be bridged whether or not it
                    // can throw — a throw propagates out via the pending slot — as
                    // long as its signature is marshalable by the packed-value ABI.
                    !is_virtual && bridge_available && bridge_marshalable(module, c)
                })
            });
            if eligible.len() == prev.len() {
                break;
            }
        }
        // A collection scans the native frame that allocates, and no other native
        // frame. A compiled method that keeps a managed reference across a call
        // that can collect would resume on a stale address once the object moved,
        // so it stays on the interpreter, whose frames the collector does root and
        // rewrite. A callee the interpreter runs (a bridged call) collects too.
        collecting = collecting_methods(&candidate, &profiles, &eligible);
        let may_collect = |rid: u32| !eligible.contains(&rid) || collecting.contains(&rid);
        let pruned: Vec<u32> = eligible
            .iter()
            .copied()
            .filter(|&rid| {
                let method = &module.tables.method_defs[rid as usize - 1];
                let Ok(body) = read_body(&module.code, method.body_offset) else {
                    return false;
                };
                let Ok(instrs) = decode_all(body.code) else {
                    return false;
                };
                holds_reference_across_collecting_call(
                    module,
                    &body,
                    &instrs,
                    0..usize::MAX,
                    &|call| call_may_collect(module, call, &methods_by_name, &may_collect),
                )
            })
            .collect();
        if pruned.is_empty() {
            break;
        }
        for rid in pruned {
            eligible.remove(&rid);
            unsafe_to_hold.insert(rid);
        }
    }
    // Locally-fine methods dropped by the fixpoint: name the blocking callee.
    for rid in &unsafe_to_hold {
        rejected.insert(
            *rid,
            "holds a managed reference across a call that may collect".into(),
        );
    }
    for (rid, callees) in &candidate {
        if eligible.contains(rid) || unsafe_to_hold.contains(rid) {
            continue;
        }
        let blocker = callees.iter().find(|&&(c, is_virtual)| {
            !eligible.contains(&c)
                && (is_virtual || !bridge_available || !bridge_marshalable(module, c))
        });
        let why = match blocker {
            Some(&(c, _)) => {
                let name = module
                    .tables
                    .method_defs
                    .get(c as usize - 1)
                    .map(|m| module.strings.get(m.name).to_string())
                    .unwrap_or_default();
                let root = rejected
                    .get(&c)
                    .cloned()
                    .unwrap_or_else(|| "callee chain".into());
                format!("callee {name}: {root}")
            }
            None => "callee chain".into(),
        };
        rejected.insert(*rid, why);
    }
    Analysis {
        eligible,
        rejected,
        collecting,
    }
}

/// Local (per-method) eligibility: `Ok` with the method's callees, or `Err`
/// with the first reason it cannot be compiled. Call-graph effects are applied
/// by the caller.
fn method_candidate(
    module: &Module,
    body_offset: u32,
    allow_const_str: bool,
    bridge_available: bool,
    methods_by_name: &HashMap<zun_metadata::StringIndex, Vec<u32>>,
    string_method_rids: &HashSet<u32>,
) -> Result<Vec<(u32, bool)>, String> {
    let Ok(body) = read_body(&module.code, body_offset) else {
        return Err("unreadable body".into());
    };
    // Return and every local must be a representable JIT type.
    if !kind_supported(body.return_kind)
        && body.returns_value
        && JitKind::from_byte(body.return_kind) != JitKind::Other
    {
        return Err(format!("return kind {}", kind_name(body.return_kind)));
    }
    let Ok(instrs) = decode_all(body.code) else {
        return Err("undecodable body".into());
    };
    // Exception regions are supported only for the same-frame slice the typed
    // JIT can lower (an explicit `throw` routed to a catch-all handler in this
    // frame, with no call-type op in the protected region). A cross-frame
    // throw — one unwound out of a callee — still needs the deferred Cranelift
    // unwinding bridge, so such methods stay on Tier 0.
    if !body.handlers.is_empty()
        && !exception_regions_jit_safe(module, &body, &instrs, bridge_available)
    {
        return Err("exception region shape (try/catch/finally)".into());
    }
    if !body.local_kinds.iter().all(|&k| kind_supported(k))
        && !any_param_return_only(&instrs, &body)
        && !any_direct_call_return_only(&instrs, &body)
        && !any_map_get_only(&instrs, &body)
    {
        let bad = body
            .local_kinds
            .iter()
            .position(|&k| !kind_supported(k))
            .map(|p| {
                let which = if p < body.param_count as usize {
                    "param"
                } else {
                    "local"
                };
                format!("{which} kind {}", kind_name(body.local_kinds[p]))
            })
            .unwrap_or_default();
        return Err(bad);
    }
    if !kind_supported(body.return_kind)
        && body.returns_value
        && !any_array_return_only(&instrs, &body)
        && !any_param_return_only(&instrs, &body)
        && !any_direct_call_return_only(&instrs, &body)
        && !any_map_get_only(&instrs, &body)
    {
        return Err(format!("return kind {}", kind_name(body.return_kind)));
    }
    if instrs.iter().any(|i| i.opcode == Opcode::ConstStr) {
        // Runtime JIT literals are stable interned handles. Keep the
        // relocatable/AOT path conservative, but allow the allocation and
        // field path whose handles have explicit typed representations.
        let extended_literal_path = instrs.iter().all(|i| {
            matches!(
                i.opcode,
                Opcode::Nop
                    | Opcode::ConstStr
                    | Opcode::NewObj
                    | Opcode::LoadLocal
                    | Opcode::StoreLocal
                    | Opcode::LoadField
                    | Opcode::StoreField
                    // String `+` with literal operands, and static-field
                    // access, all lower with explicit typed handles, so a
                    // literal feeding `Concat` or a `ldsfld`/`stsfld` is safe.
                    | Opcode::Concat
                    | Opcode::LoadStatic
                    | Opcode::StoreStatic
                    | Opcode::Pop
                    | Opcode::Ret
            )
        }) && !instrs.iter().any(|i| i.opcode == Opcode::CallVirt);
        let direct_literal_callvirt = instrs.iter().all(|i| {
            matches!(
                i.opcode,
                Opcode::Nop | Opcode::ConstStr | Opcode::CallVirt | Opcode::Ret
            )
        }) && instrs
            .iter()
            .filter(|i| i.opcode == Opcode::CallVirt)
            .count()
            == 1;
        let relocatable_literal_only = instrs
            .iter()
            .all(|i| matches!(i.opcode, Opcode::Nop | Opcode::ConstStr | Opcode::Ret));
        if !allow_const_str
            || (!extended_literal_path && !direct_literal_callvirt && !relocatable_literal_only)
        {
            return Err(if allow_const_str {
                "ConstStr literal mixed with an opcode outside the literal allowlist".into()
            } else {
                "ConstStr literal in AOT mode".into()
            });
        }
    }
    if let Some(i) = instrs.iter().find(|i| requires_exception_bridge(i.opcode)) {
        return Err(format!("opcode {:?} needs the exception bridge", i.opcode));
    }
    if instrs.iter().any(allocation_builtin)
        && JitKind::from_byte(body.return_kind) != JitKind::Object
    {
        return Err("allocation builtin in a method not returning Object".into());
    }
    if let Some(why) = first_unsupported(&instrs) {
        return Err(why);
    }
    if !const_null_uses_ok(module, &body, &instrs, 0..usize::MAX) {
        return Err("const.null where the target is not a class reference".into());
    }
    // Function values in the typed JIT are carried as a one-word method rid
    // (`LoadFunction` → rid, `CallIndirect` dispatches it through the bridge).
    // That is sound only for a *concrete, closure-free* function value, so a
    // method is refused if it could place a closure or bound function into a
    // Function slot: a function-typed parameter (the caller could pass one),
    // or a call that returns a function (the callee could). `MakeClosure` is
    // already excluded by `opcode_supported`, and dynamic function types map
    // to `Other` (so a Function-typed local/param here is always concrete).
    // Each `CallIndirect` must target a concrete (rid != 0), marshalable
    // signature — a dynamic rid-0 indirect call stays on Tier 0. All of this
    // needs the bridge for dispatch.
    let function_byte = JitKind::Function.to_byte();
    let uses_function_value = body.local_kinds.contains(&function_byte)
        || instrs
            .iter()
            .any(|i| matches!(i.opcode, Opcode::LoadFunction | Opcode::CallIndirect));
    if uses_function_value && !bridge_available {
        return Err("function value without a bridge".into());
    }
    if uses_function_value && body.local_kinds[..body.param_count as usize].contains(&function_byte)
    {
        return Err("function-typed parameter".into());
    }
    if uses_function_value
        && !instrs.iter().all(|i| match (i.opcode, i.operand) {
            (Opcode::CallIndirect, Operand::Token(t)) => {
                t.rid() != 0
                    && bridge_marshalable(module, t.rid())
                    && !call_returns_function(module, t.rid(), function_byte)
            }
            (Opcode::Call | Opcode::CallVirt, Operand::Token(t)) => {
                !call_returns_function(module, t.rid(), function_byte)
            }
            _ => true,
        })
    {
        return Err("function value: dynamic indirect call or call returning a function".into());
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
                && !scalar_i64_array_access_only(&instrs, &body)
                && !scalar_i32_array_access_only(&instrs, &body)
                && !scalar_narrow_array_access_only(&instrs, &body)
                && !any_array_return_only(&instrs, &body)))
    {
        return Err("LoadArray shape (array kind / return kind)".into());
    }
    if instrs.iter().any(|i| i.opcode == Opcode::StoreArray)
        && (!body.returns_value
            || (JitKind::from_byte(body.return_kind) != JitKind::Num(zun_il::NumKind::F64)
                && !object_array_access_only(&instrs, &body)
                && !object_array_stack_only(&instrs, &body)
                && !scalar_bool_array_access_only(&instrs, &body)
                && !scalar_u64_array_access_only(&instrs, &body)
                && !scalar_i64_array_access_only(&instrs, &body)
                && !scalar_i32_array_access_only(&instrs, &body)
                && !scalar_narrow_array_access_only(&instrs, &body)))
    {
        return Err("StoreArray shape (array kind / return kind)".into());
    }
    if instrs.iter().any(|i| i.opcode == Opcode::ArrayPop)
        && (!body.returns_value
            || (JitKind::from_byte(body.return_kind) != JitKind::Num(zun_il::NumKind::F64)
                && JitKind::from_byte(body.return_kind) != JitKind::Num(zun_il::NumKind::U64)
                && JitKind::from_byte(body.return_kind) != JitKind::Bool
                && !scalar_i32_array_access_only(&instrs, &body)
                && !scalar_narrow_array_access_only(&instrs, &body)
                && !scalar_bool_array_access_only(&instrs, &body)
                && !object_array_stack_only(&instrs, &body)))
    {
        return Err("ArrayPop shape (array kind / return kind)".into());
    }
    if instrs.iter().any(|i| i.opcode == Opcode::NewArray)
        && !matches!(
            JitKind::from_byte(body.return_kind),
            JitKind::Object | JitKind::Array
        )
        && !scalar_bool_array_access_only(&instrs, &body)
        && !scalar_u64_array_access_only(&instrs, &body)
        && !scalar_i32_array_access_only(&instrs, &body)
        && !scalar_narrow_array_access_only(&instrs, &body)
        && !scalar_i64_array_access_only(&instrs, &body)
        && !scalar_f64_array_access_only(&instrs, &body)
        && !scalar_array_length_only(&instrs, &body)
    {
        return Err("NewArray shape (array kind / return kind)".into());
    }
    // Reject methods whose field/newobj operands touch non-JIT types.
    if let Some(i) = instrs.iter().find(|i| !instr_fields_ok(module, i)) {
        return Err(format!(
            "field kind of {:?} {}",
            i.opcode,
            field_kind_detail(module, i)
        ));
    }
    if instrs
        .iter()
        .any(|i| !callvirt_is_non_overridden(module, i, string_method_rids))
    {
        return Err("overridden virtual call".into());
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
                    })
                    .map(|rid| (rid, true)),
            );
        } else {
            callees.push((token.rid(), false));
        }
    }
    Ok(callees)
}

/// The offending field's declared kind, for `instr_fields_ok` rejections.
fn field_kind_detail(module: &Module, instr: &zun_il::Instruction) -> String {
    let kind_of = |rid: u32| {
        module
            .tables
            .field_defs
            .get(rid as usize - 1)
            .map(|f| kind_name(f.kind))
            .unwrap_or_default()
    };
    match (instr.opcode, instr.operand) {
        (Opcode::NewObj, Operand::Token(t)) => module
            .tables
            .type_defs
            .get(t.rid() as usize - 1)
            .and_then(|ty| (ty.field_start..ty.field_end).find(|&r| !field_jit_ok(module, r)))
            .map(|r| format!("(a field of the class: {})", kind_of(r)))
            .unwrap_or_default(),
        (_, Operand::Token(t)) => format!("({})", kind_of(t.rid())),
        _ => String::new(),
    }
}


/// Every independent local blocker of a method, not just the first. The
/// first-reason report in `method_rejections` is order-sensitive (a method with
/// an `any` parameter never reaches the opcode check), so a census also needs
/// this view to rank blockers by how many methods each one touches. Built from
/// the same predicates (`opcode_supported`, `kind_supported`, `instr_fields_ok`).
pub fn method_blockers(module: &Module, method_rid: u32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Some(method) = module.tables.method_defs.get(method_rid as usize - 1) else {
        return out;
    };
    let Ok(body) = read_body(&module.code, method.body_offset) else {
        return out;
    };
    let Ok(instrs) = decode_all(body.code) else {
        return out;
    };
    if body.local_kinds[..(body.param_count as usize).min(body.local_kinds.len())]
        .iter()
        .any(|&k| !kind_supported(k))
    {
        out.push("param kind Other".into());
    }
    if body.local_kinds[(body.param_count as usize).min(body.local_kinds.len())..]
        .iter()
        .any(|&k| !kind_supported(k))
    {
        out.push("local kind Other".into());
    }
    if body.returns_value && !kind_supported(body.return_kind) {
        out.push("return kind Other".into());
    }
    if !body.handlers.is_empty() {
        out.push("has exception region".into());
    }
    for i in &instrs {
        let why = if requires_exception_bridge(i.opcode) {
            Some(format!("opcode {:?} needs the exception bridge", i.opcode))
        } else if !opcode_supported(i.opcode, i) {
            first_unsupported(std::slice::from_ref(i))
        } else if !instr_fields_ok(module, i) {
            Some(format!("field kind of {:?}", i.opcode))
        } else {
            None
        };
        if let Some(why) = why {
            if !out.contains(&why) {
                out.push(why);
            }
        }
    }
    out
}

#[cfg(test)]
mod invariant_tests {
    use super::*;
    use zun_il::{Instruction, Opcode, Operand};

    /// D5 invariant: every builtin the eligibility predicates admit must have a
    /// lowering — either a `builtin_desc` descriptor or one of the hand-lowered
    /// special cases in `op_call_builtin`. If someone adds a name to
    /// `eligible.rs` without a `builtins_table.rs` entry (or vice versa), this
    /// fails, catching the eligible/lowerable mismatch that caused the D5
    /// recompile storm.
    #[test]
    fn every_eligible_builtin_has_a_lowering() {
        // Builtins lowered by name in `op_call_builtin` rather than `builtin_desc`.
        const HAND_LOWERED: &[&str] = &["mapClear", "setClear", "mapGet"];
        for id in 0u32..4096 {
            let Some(f) = zun_natives::by_id(id) else {
                continue;
            };
            let instr = Instruction {
                opcode: Opcode::CallBuiltin,
                operand: Operand::Imm32(id as i32),
            };
            let eligible = string_builtin(&instr)
                || collection_builtin(&instr)
                || math_builtin(&instr)
                || date_builtin(&instr)
                || allocation_builtin(&instr);
            if !eligible {
                continue;
            }
            let lowerable = crate::translate::builtins_table::builtin_desc(Some(f.name)).is_some()
                || HAND_LOWERED.contains(&f.name);
            assert!(
                lowerable,
                "builtin `{}` (id {id}) is JIT-eligible in eligible.rs but has no \
                 lowering in builtins_table.rs or op_call_builtin (D5 invariant)",
                f.name
            );
        }
    }
}
