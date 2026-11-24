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
            // `undefined`: only ever an `any` (the `Void` tag).
            | Opcode::ConstUndef
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
    // Every kind is representable now: an `any` is the (tag, bits, obj) triple of
    // `translate/any.rs`. The predicate stays the one place to refuse a new kind.
    let _ = JitKind::from_byte(byte);
    true
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

 /// Whether a method's signature can cross the bridge's packed-value ABI. Every
/// kind can: a scalar or reference is one word, an `any` two (tag, payload), and
/// an `any` result comes back in a carrier after the arguments.
fn bridge_marshalable(module: &Module, rid: u32) -> bool {
    let Some(method) = module.tables.method_defs.get(rid as usize - 1) else {
        return false;
    };
    method.body_offset != u32::MAX && read_body(&module.code, method.body_offset).is_ok()
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
    if const_null_modes(module, &body, &instrs, start..end).is_none() {
        return Some("const.null/const.undef whose use is not a lowered form".into());
    }
    if let Some(why) = unbox_any_use_rejection(&instrs, start..end) {
        return Some(why);
    }
    if let Some(why) = any_operand_forms_rejection(module, &body, &instrs, start..end) {
        return Some(why);
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

/// Whether instruction `index` pushes an `any` value (a `JitKind::Other` stack
/// entry in the translator): an `any` local, a call returning one, a `box.any`,
/// or an element / `Map.get` read. Used to decide what a `null` is compared with.
fn pushes_any(
    module: &Module,
    body: &zun_module::MethodBody<'_>,
    instrs: &[zun_il::Instruction],
    index: usize,
) -> bool {
    let instr = &instrs[index];
    match (instr.opcode, instr.operand) {
        (Opcode::LoadLocal, Operand::Local(slot)) => body
            .local_kinds
            .get(slot as usize)
            .is_some_and(|&kind| JitKind::from_byte(kind) == JitKind::Other),
        (Opcode::BoxAny, _) => true,
        (Opcode::Call | Opcode::CallVirt, Operand::Token(token)) => module
            .tables
            .method_defs
            .get((token.rid() as usize).wrapping_sub(1))
            .and_then(|method| read_body(&module.code, method.body_offset).ok())
            .is_some_and(|callee| {
                callee.returns_value && JitKind::from_byte(callee.return_kind) == JitKind::Other
            }),
        (Opcode::LoadArray, _) => array_load_is_any(body, instrs, index),
        (Opcode::CallBuiltin, Operand::Imm32(id)) => {
            zun_natives::by_id(id as u32).map(|f| f.name) == Some("mapGet")
        }
        _ => false,
    }
}

/// Whether the `ldelem` at `index` is lowered as an `any` read. The element kind
/// is not in the instruction; the translator reads an `any` when the element is
/// unboxed next, when the method returns an `any`, or when the method has any
/// `any` local (a typed read is only sound in the narrow all-scalar shapes).
pub(crate) fn array_load_is_any(
    body: &zun_module::MethodBody<'_>,
    instrs: &[zun_il::Instruction],
    index: usize,
) -> bool {
    (body.returns_value && JitKind::from_byte(body.return_kind) == JitKind::Other)
        || instrs
            .get(index + 1)
            .is_some_and(|next| next.opcode == Opcode::UnboxAny)
        || body
            .local_kinds
            .iter()
            .any(|&kind| JitKind::from_byte(kind) == JitKind::Other)
}

/// The instruction that produced the value `depth` places below the top of the
/// operand stack just before instruction `at`, found by walking backwards over
/// straight-line code. `None` when a branch, a duplicating instruction or an
/// unknown stack effect is in the way.
fn producer_of(
    module: &Module,
    instrs: &[zun_il::Instruction],
    at: usize,
    mut depth: usize,
) -> Option<usize> {
    for k in (0..at).rev() {
        let instr = &instrs[k];
        if matches!(instr.operand, Operand::Branch(_)) || instr.opcode == Opcode::Dup {
            return None;
        }
        let (pops, pushes) = match (instr.opcode, instr.operand) {
            (Opcode::Call | Opcode::CallVirt, Operand::Token(token)) => {
                let method = module
                    .tables
                    .method_defs
                    .get((token.rid() as usize).wrapping_sub(1))?;
                let callee = read_body(&module.code, method.body_offset).ok()?;
                (callee.param_count as usize, usize::from(callee.returns_value))
            }
            (Opcode::StoreField, _) => (2, 0),
            (Opcode::StoreStatic, _) => (1, 0),
            (Opcode::NewObj, Operand::Token(token)) => {
                let ty = module
                    .tables
                    .type_defs
                    .get((token.rid() as usize).wrapping_sub(1))?;
                ((ty.field_end - ty.field_start) as usize, 1)
            }
            (op, _) => match op.stack_effect() {
                zun_il::StackEffect::Fixed { pops, pushes } => (pops as usize, pushes as usize),
                _ => return None,
            },
        };
        if depth < pushes {
            return Some(k);
        }
        depth = depth - pushes + pops;
    }
    None
}

/// What a `const.null` / `const.undef` stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NullMode {
    /// The zero handle of a nullable class reference.
    Reference,
    /// The `null` / `undefined` value of an `any`.
    Any,
}

/// Classify every `const.null` / `const.undef` whose byte offset is in `within`
/// by what *consumes* it, or `None` when some use is not one the typed JIT
/// lowers.
///
/// `null` is the zero handle of a class reference, so it is only a reference where
/// one is: compared with one, or stored, passed or returned into a slot of class
/// type. Stored, passed, returned or compared into an `any` slot it is the `null`
/// value of that `any` (`undefined` can only ever be the latter). Anywhere else
/// (a number parameter that defaults when `null` is passed) zero means something
/// else, and the translator would refuse it; saying so here keeps eligibility
/// equal to what is lowered.
///
/// The rule is about what *consumes* each value, not about where it sits: the
/// operand stack is followed forward from the constant (by declared stack effect,
/// with call arity read from metadata, across `dup`, and along an unconditional
/// `br`) until an instruction pops it, and the slot it lands in must be of one
/// kind. Conditional control flow between the constant and its consumer, or an
/// instruction whose effect is not known here, declines the method.
pub(crate) fn const_null_modes(
    module: &Module,
    body: &zun_module::MethodBody<'_>,
    instrs: &[zun_il::Instruction],
    within: std::ops::Range<usize>,
) -> Option<HashMap<usize, NullMode>> {
    let field_is_object = |token: zun_metadata::Token| {
        module
            .tables
            .field_defs
            .get((token.rid() as usize).wrapping_sub(1))
            .is_some_and(|field| JitKind::from_byte(field.kind) == JitKind::Object)
    };
    let callee_of = |token: zun_metadata::Token| {
        module
            .tables
            .method_defs
            .get((token.rid() as usize).wrapping_sub(1))
            .and_then(|method| read_body(&module.code, method.body_offset).ok())
    };
    let mut offsets = Vec::with_capacity(instrs.len());
    let mut running = 0usize;
    for instr in instrs {
        offsets.push(running);
        running += instr.encoded_len();
    }
    let slot_mode = |kind: Option<JitKind>| match kind {
        Some(JitKind::Object) => Some(Some(NullMode::Reference)),
        Some(JitKind::Other) => Some(Some(NullMode::Any)),
        _ => None,
    };
    let mut modes = HashMap::new();
    for (i, instr) in instrs.iter().enumerate() {
        if !matches!(instr.opcode, Opcode::ConstNull | Opcode::ConstUndef)
            || !within.contains(&offsets[i])
        {
            continue;
        }
        let is_undef = instr.opcode == Opcode::ConstUndef;
        // Depths (from the top of the stack) of the live copies of the constant.
        let mut tracked: Vec<usize> = vec![0];
        let mut mode: Option<NullMode> = None;
        let mut j = i + 1;
        let mut steps = 0usize;
        while !tracked.is_empty() {
            steps += 1;
            let next = instrs.get(j)?;
            if steps > 256 {
                return None;
            }
            // An unconditional `br` keeps the stack: follow it.
            if next.opcode == Opcode::Branch {
                let Operand::Branch(disp) = next.operand else {
                    return None;
                };
                let target = offsets[j] as i64 + next.encoded_len() as i64 + i64::from(disp);
                j = offsets.binary_search(&usize::try_from(target).ok()?).ok()?;
                continue;
            }
            if matches!(next.operand, Operand::Branch(_)) {
                return None;
            }
            // Arity of the instruction: how many operands it pops, and how many
            // values it leaves.
            let (pops, pushes) = match (next.opcode, next.operand) {
                (Opcode::Ret, _) => (1, 0),
                (Opcode::StoreField, _) => (2, 0),
                (Opcode::StoreStatic, _) => (1, 0),
                (Opcode::Call | Opcode::CallVirt, Operand::Token(token)) => {
                    let callee = callee_of(token)?;
                    (callee.param_count as usize, usize::from(callee.returns_value))
                }
                (Opcode::NewObj, Operand::Token(token)) => {
                    let ty = module
                        .tables
                        .type_defs
                        .get((token.rid() as usize).wrapping_sub(1))?;
                    ((ty.field_end - ty.field_start) as usize, 1)
                }
                (op, _) => match op.stack_effect() {
                    zun_il::StackEffect::Fixed { pops, pushes } => (pops as usize, pushes as usize),
                    _ => return None,
                },
            };
            let mut next_tracked = Vec::new();
            for &depth in &tracked {
                if pops <= depth {
                    next_tracked.push(depth - pops + pushes);
                    continue;
                }
                // `next` pops this copy, as operand `index` of its `pops` operands.
                let index = pops - 1 - depth;
                let consumed: Option<Option<NullMode>> = match (next.opcode, next.operand) {
                    (Opcode::Dup, _) => {
                        next_tracked.push(0);
                        next_tracked.push(1);
                        continue;
                    }
                    (Opcode::Pop, _) => Some(None),
                    (Opcode::Ret, _) => {
                        if depth == 0 && body.returns_value {
                            slot_mode(Some(JitKind::from_byte(body.return_kind)))
                        } else {
                            None
                        }
                    }
                    (Opcode::StoreLocal, Operand::Local(slot)) => slot_mode(
                        body.local_kinds
                            .get(slot as usize)
                            .map(|&kind| JitKind::from_byte(kind)),
                    ),
                    (Opcode::StoreField, Operand::Token(token)) => {
                        (index == 1 && field_is_object(token)).then_some(Some(NullMode::Reference))
                    }
                    (Opcode::StoreStatic, Operand::Token(token)) => {
                        field_is_object(token).then_some(Some(NullMode::Reference))
                    }
                    (Opcode::NewObj, Operand::Token(token)) => module
                        .tables
                        .type_defs
                        .get((token.rid() as usize).wrapping_sub(1))
                        .and_then(|ty| module.tables.field_defs.get(ty.field_start as usize + index - 1))
                        .is_some_and(|field| JitKind::from_byte(field.kind) == JitKind::Object)
                        .then_some(Some(NullMode::Reference)),
                    (Opcode::Call | Opcode::CallVirt, Operand::Token(token)) => {
                        let callee = callee_of(token)?;
                        slot_mode(
                            callee
                                .local_kinds
                                .get(index)
                                .map(|&kind| JitKind::from_byte(kind)),
                        )
                    }
                    // `p === null`: the other operand is whatever produced the value
                    // beside the constant on the operand stack.
                    (Opcode::CmpEq | Opcode::CmpNe, _) => {
                        // The constant is operand `index` of two; the other is `1 - index`,
                        // at depth `index` from the top (the top operand is depth 0).
                        let other_depth = index;
                        match producer_of(module, instrs, j, other_depth) {
                            Some(p) if pushes_object(module, body, &instrs[p]) => {
                                Some(Some(NullMode::Reference))
                            }
                            Some(p) if pushes_any(module, body, instrs, p) => {
                                Some(Some(NullMode::Any))
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                match consumed? {
                    None => {}
                    Some(wanted) => {
                        if (is_undef && wanted == NullMode::Reference)
                            || mode.is_some_and(|seen| seen != wanted)
                        {
                            return None;
                        }
                        mode = Some(wanted);
                    }
                }
            }
            tracked = next_tracked;
            j += 1;
        }
        modes.insert(i, mode.unwrap_or(NullMode::Reference));
    }
    Some(modes)
}

/// Each `unbox.any` is lowered by looking at what consumes its result (the
/// static kind is not in the instruction): `conv` or a branch (truthiness). A bare
/// `ret` / `stloc` hands the unboxed value on unchanged, which a typed slot cannot
/// represent, and any other consumer needs a coercion or a throw (`castclass`)
/// that has no inline form yet.
/// Mirrors `translate::any::unbox_use`.
fn unbox_any_use_rejection(
    instrs: &[zun_il::Instruction],
    within: std::ops::Range<usize>,
) -> Option<String> {
    let mut offset = 0usize;
    for (i, instr) in instrs.iter().enumerate() {
        let here = offset;
        offset += instr.encoded_len();
        if instr.opcode != Opcode::UnboxAny || !within.contains(&here) {
            continue;
        }
        let ok = instrs.get(i + 1).is_some_and(|next| match (next.opcode, next.operand) {
            (Opcode::Conv, Operand::Imm32(imm)) => zun_il::NumKind::from_imm(imm).is_some(),
            (Opcode::BranchTrue | Opcode::BranchFalse, _) => true,
            _ => false,
        });
        if !ok {
            let consumer = instrs
                .get(i + 1)
                .map_or("nothing".to_string(), |next| format!("{:?}", next.opcode));
            return Some(format!(
                "unbox.any consumed by {consumer}, not a conv or a branch at {here:#06x}"
            ));
        }
    }
    None
}

/// Whether instruction `index` pushes a number or a bool (a scalar the translator
/// can compare with, or box into, an `any` without a helper).
fn produces_scalar(
    module: &Module,
    body: &zun_module::MethodBody<'_>,
    instrs: &[zun_il::Instruction],
    index: usize,
) -> bool {
    let instr = &instrs[index];
    let scalar_kind = |kind: u8| matches!(JitKind::from_byte(kind), JitKind::Num(_) | JitKind::Bool);
    match (instr.opcode, instr.operand) {
        (Opcode::LoadLocal, Operand::Local(slot)) => body
            .local_kinds
            .get(slot as usize)
            .is_some_and(|&kind| scalar_kind(kind)),
        (Opcode::LoadField | Opcode::LoadStatic, Operand::Token(token)) => module
            .tables
            .field_defs
            .get((token.rid() as usize).wrapping_sub(1))
            .is_some_and(|field| scalar_kind(field.kind)),
        (Opcode::Call | Opcode::CallVirt, Operand::Token(token)) => module
            .tables
            .method_defs
            .get((token.rid() as usize).wrapping_sub(1))
            .and_then(|method| read_body(&module.code, method.body_offset).ok())
            .is_some_and(|callee| callee.returns_value && scalar_kind(callee.return_kind)),
        (
            Opcode::ConstI32
            | Opcode::ConstI64
            | Opcode::ConstF64
            | Opcode::ConstTrue
            | Opcode::ConstFalse
            | Opcode::Conv
            | Opcode::CmpEq
            | Opcode::CmpNe
            | Opcode::CmpLt
            | Opcode::CmpLe
            | Opcode::CmpGt
            | Opcode::CmpGe
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
            | Opcode::NegF64
            | Opcode::SqrtF64
            | Opcode::AbsF64
            | Opcode::FloorF64
            | Opcode::CeilF64
            | Opcode::TruncF64
            | Opcode::MinF64
            | Opcode::MaxF64
            | Opcode::RoundF64
            | Opcode::SignF64
            | Opcode::MinI32
            | Opcode::MaxI32
            | Opcode::ArrayLength,
            _,
        ) => true,
        _ => false,
    }
}

/// Whether instruction `index` pushes a string handle.
fn produces_string(
    module: &Module,
    body: &zun_module::MethodBody<'_>,
    instrs: &[zun_il::Instruction],
    index: usize,
) -> bool {
    let instr = &instrs[index];
    let string_kind = |kind: u8| JitKind::from_byte(kind) == JitKind::String;
    match (instr.opcode, instr.operand) {
        (Opcode::ConstStr | Opcode::Concat, _) => true,
        (Opcode::LoadLocal, Operand::Local(slot)) => body
            .local_kinds
            .get(slot as usize)
            .is_some_and(|&kind| string_kind(kind)),
        (Opcode::LoadField | Opcode::LoadStatic, Operand::Token(token)) => module
            .tables
            .field_defs
            .get((token.rid() as usize).wrapping_sub(1))
            .is_some_and(|field| string_kind(field.kind)),
        (Opcode::Call | Opcode::CallVirt, Operand::Token(token)) => module
            .tables
            .method_defs
            .get((token.rid() as usize).wrapping_sub(1))
            .and_then(|method| read_body(&module.code, method.body_offset).ok())
            .is_some_and(|callee| callee.returns_value && string_kind(callee.return_kind)),
        _ => false,
    }
}

/// How an `any` meets the other operand of a comparison, and what may be boxed
/// into one. Mirrors `translate::any::compare_any` / `box_to_any`:
///
/// - `===` / `!==` of an `any` is lowered against a literal `null` / `undefined`
///   and against a number or bool (also one just boxed); two arbitrary `any`s can
///   hold objects whose equality goes through function boxes, constructor records
///   or BigInts, or strings whose handles are not canonical, so that form stays on
///   Tier 0. A relational operator on an `any` coerces (`ToPrimitive`), also Tier 0.
/// - `box.any` lowers a number, a bool, a class reference or an `any`; a string or
///   a function value has no inline form.
fn any_operand_forms_rejection(
    module: &Module,
    body: &zun_module::MethodBody<'_>,
    instrs: &[zun_il::Instruction],
    within: std::ops::Range<usize>,
) -> Option<String> {
    let mut offset = 0usize;
    for (i, instr) in instrs.iter().enumerate() {
        let here = offset;
        offset += instr.encoded_len();
        if !within.contains(&here) {
            continue;
        }
        match instr.opcode {
            Opcode::BoxAny => {
                let ok = producer_of(module, instrs, i, 0).is_some_and(|p| {
                    produces_string(module, body, instrs, p)
                        || produces_scalar(module, body, instrs, p)
                        || pushes_object(module, body, &instrs[p])
                        || pushes_any(module, body, instrs, p)
                });
                if !ok {
                    return Some(format!(
                        "box.any of a value that is not a number, bool, class reference or any at {here:#06x}"
                    ));
                }
            }
            Opcode::CmpEq
            | Opcode::CmpNe
            | Opcode::CmpLt
            | Opcode::CmpLe
            | Opcode::CmpGt
            | Opcode::CmpGe => {
                let top = producer_of(module, instrs, i, 0);
                let below = producer_of(module, instrs, i, 1);
                let is_any =
                    |p: Option<usize>| p.is_some_and(|p| pushes_any(module, body, instrs, p));
                if !is_any(top) && !is_any(below) {
                    continue;
                }
                let relational = !matches!(instr.opcode, Opcode::CmpEq | Opcode::CmpNe);
                // The decidable other side: a literal null/undefined, a scalar, or a
                // box.any of a scalar.
                let decidable = |p: Option<usize>| {
                    p.is_some_and(|p| {
                        matches!(instrs[p].opcode, Opcode::ConstNull | Opcode::ConstUndef)
                            || produces_scalar(module, body, instrs, p)
                            || (instrs[p].opcode == Opcode::BoxAny
                                && producer_of(module, instrs, p, 0)
                                    .is_some_and(|q| produces_scalar(module, body, instrs, q)))
                    })
                };
                let ok = !relational
                    && ((is_any(top) && decidable(below)) || (is_any(below) && decidable(top)));
                if !ok {
                    return Some(format!(
                        "comparison of an any with an operand that is not a literal null/undefined, number or bool at {here:#06x}"
                    ));
                }
            }
            Opcode::Call | Opcode::CallVirt => {
                let Operand::Token(token) = instr.operand else {
                    continue;
                };
                let Some(callee) = module
                    .tables
                    .method_defs
                    .get((token.rid() as usize).wrapping_sub(1))
                    .and_then(|method| read_body(&module.code, method.body_offset).ok())
                else {
                    continue;
                };
                let params = callee.param_count as usize;
                for arg in 0..params {
                    let any_arg = producer_of(module, instrs, i, params - 1 - arg)
                        .is_some_and(|p| pushes_any(module, body, instrs, p));
                    let want = callee.local_kinds.get(arg).map(|&k| JitKind::from_byte(k));
                    if any_arg && want != Some(JitKind::Other) {
                        return Some(format!(
                            "an any value passed to a typed parameter at {here:#06x}"
                        ));
                    }
                }
            }
            Opcode::StoreLocal => {
                let Operand::Local(slot) = instr.operand else {
                    continue;
                };
                let local_is_any = body
                    .local_kinds
                    .get(slot as usize)
                    .is_some_and(|&k| JitKind::from_byte(k) == JitKind::Other);
                let value_is_any = producer_of(module, instrs, i, 0)
                    .is_some_and(|p| pushes_any(module, body, instrs, p));
                if value_is_any && !local_is_any {
                    return Some(format!("an any value stored into a typed local at {here:#06x}"));
                }
            }
            Opcode::Ret => {
                let value_is_any = producer_of(module, instrs, i, 0)
                    .is_some_and(|p| pushes_any(module, body, instrs, p));
                let returns_any = body.returns_value
                    && JitKind::from_byte(body.return_kind) == JitKind::Other;
                if value_is_any && !returns_any {
                    return Some(format!("an any value returned as a typed value at {here:#06x}"));
                }
            }
            // An element access takes an array and an index, never an `any`.
            Opcode::LoadArray | Opcode::StoreArray | Opcode::ArrayPush => {
                let value_slots = usize::from(matches!(instr.opcode, Opcode::StoreArray | Opcode::ArrayPush));
                let operands = match instr.opcode {
                    Opcode::LoadArray => 2,
                    Opcode::StoreArray => 3,
                    _ => 2,
                };
                for depth in value_slots..operands {
                    if producer_of(module, instrs, i, depth)
                        .is_some_and(|p| pushes_any(module, body, instrs, p))
                    {
                        return Some(format!(
                            "an any used as an array or index operand at {here:#06x}"
                        ));
                    }
                }
            }
            // A builtin other than `Map.get` / `clear` takes typed operands.
            Opcode::CallBuiltin => {
                let Operand::Imm32(id) = instr.operand else {
                    continue;
                };
                let name = zun_natives::by_id(id as u32).map(|f| f.name);
                if matches!(name, Some("mapGet" | "mapClear" | "setClear")) {
                    continue;
                }
                let Some((_, arg_count, _, _, _)) =
                    crate::translate::builtins_table::builtin_desc(name)
                else {
                    continue;
                };
                for depth in 0..arg_count {
                    if producer_of(module, instrs, i, depth)
                        .is_some_and(|p| pushes_any(module, body, instrs, p))
                    {
                        return Some(format!("a builtin takes an any operand at {here:#06x}"));
                    }
                }
            }
            op if !any_aware(op) => {
                // Every other operator reads its operands as numbers or references.
                if let zun_il::StackEffect::Fixed { pops, .. } = op.stack_effect() {
                    for depth in 0..pops as usize {
                        if producer_of(module, instrs, i, depth)
                            .is_some_and(|p| pushes_any(module, body, instrs, p))
                        {
                            return Some(format!("{op:?} on an any operand at {here:#06x}"));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
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

    let mut eligible: HashSet<u32> = candidate.keys().copied().collect();
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
    // Locally-fine methods dropped by the fixpoint: name the blocking callee.
    for (rid, callees) in &candidate {
        if eligible.contains(rid) {
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
    if !kind_supported(body.return_kind) && body.returns_value {
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
    if !body.local_kinds.iter().all(|&k| kind_supported(k)) {
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
    if !kind_supported(body.return_kind) && body.returns_value {
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
    if const_null_modes(module, &body, &instrs, 0..usize::MAX).is_none() {
        return Err("const.null/const.undef whose use is not a lowered form".into());
    }
    if let Some(why) = unbox_any_use_rejection(&instrs, 0..usize::MAX) {
        return Err(why);
    }
    if let Some(why) = any_operand_forms_rejection(module, &body, &instrs, 0..usize::MAX) {
        return Err(why);
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
    if const_null_modes(module, &body, &instrs, 0..usize::MAX).is_none() {
        out.push("const.null/const.undef whose use is not a lowered form".into());
    }
    if let Some(why) = unbox_any_use_rejection(&instrs, 0..usize::MAX) {
        let why = why.split(" at ").next().unwrap_or(&why).to_string();
        if !out.contains(&why) {
            out.push(why);
        }
    }
    if let Some(why) = any_operand_forms_rejection(module, &body, &instrs, 0..usize::MAX) {
        // Without the offset, so the census groups methods by the form.
        let why = why.split(" at ").next().unwrap_or(&why).to_string();
        if !out.contains(&why) {
            out.push(why);
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

/// The opcodes that have a lowered form for an `any` operand (mirrors
/// `Translator::refuse_any_operands`); every other operator reads its operands as
/// numbers or references.
fn any_aware(op: Opcode) -> bool {
    matches!(
        op,
        Opcode::Nop
            | Opcode::LoadLocal
            | Opcode::StoreLocal
            | Opcode::Pop
            | Opcode::Dup
            | Opcode::BranchTrue
            | Opcode::BranchFalse
            | Opcode::Branch
            | Opcode::Ret
            | Opcode::Call
            | Opcode::CallVirt
            | Opcode::CallIndirect
            | Opcode::CallBuiltin
            | Opcode::CmpEq
            | Opcode::CmpNe
            | Opcode::BoxAny
            | Opcode::UnboxAny
            | Opcode::LoadArray
            | Opcode::StoreArray
            | Opcode::ArrayPush
    )
}
