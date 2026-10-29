//! Small instruction-selection helpers extracted from IL generation.

use zun_il::Opcode;
use zun_sema::hir::{self, Ty};

use super::numkind_of;

/// Whether `ty` is a *concrete* function type — a specific top-level function,
/// not one of the erased dynamic sentinels (`DYNAMIC_FUNCTION`/`_PREDICATE`/
/// `_REDUCER`, which also cover bound functions and closures). Only a concrete
/// function value is representable in the JIT as a bare one-word method rid; the
/// dynamic ones may carry a closure environment or bound arguments and stay on
/// Tier 0.
fn is_concrete_function(ty: Ty) -> bool {
    matches!(ty, Ty::Function(index) if index < hir::DYNAMIC_REDUCER)
}

/// Choose between the integer and float opcode for a numeric operation.
pub(crate) fn int_or_float(is_int: bool, int_op: Opcode, float_op: Opcode) -> Opcode {
    if is_int {
        int_op
    } else {
        float_op
    }
}

pub(crate) fn jitkind_byte(ty: Ty) -> u8 {
    use zun_il::JitKind;
    let kind = match numkind_of(ty) {
        Some(n) => JitKind::Num(n),
        None if ty == Ty::Boolean => JitKind::Bool,
        None if ty == Ty::String => JitKind::String,
        None
            if matches!(
                ty,
                Ty::Instance(_)
                    | Ty::Buffer
                    | Ty::BufferView(_)
                    | Ty::Map
                    | Ty::Set
                    | Ty::WeakMap
                    | Ty::WeakSet
                    | Ty::WeakRef
            ) => JitKind::Object,
        None if ty == Ty::Array => JitKind::Array,
        // A concrete (non-dynamic) function value: a stored/typed function local
        // or return. Carried as a one-word method rid (`LoadFunction` → rid,
        // `CallIndirect` dispatches it through the bridge). The eligibility gate
        // keeps this sound by refusing a method that could put a *closure* or
        // bound function into such a slot (function-typed param, or a call that
        // returns a function). Dynamic function types stay `Other` (Tier 0).
        None if is_concrete_function(ty) => JitKind::Function,
        None => JitKind::Other,
    };
    kind.to_byte()
}

/// The field value-type code uses distinct codes for slot decoding: strings
/// carry a handle (12) and `any` fields are carried out-of-line in the heap's
/// `any_fields` side table (14), since an `Any` does not fit the 8-byte slot.
pub(crate) fn field_kind_byte(ty: Ty) -> u8 {
    use zun_il::JitKind;
    match ty {
        Ty::String => 12,
        Ty::Any => 14,
        // A function-typed field keeps the opaque `Other` slot code. The typed
        // JIT carries a function value only as a transient operand or a local
        // rid, never in a heap field slot, so field decoding is unchanged by the
        // concrete-function local representation above.
        Ty::Function(_) => JitKind::Other.to_byte(),
        _ => jitkind_byte(ty),
    }
}
