//! Small instruction-selection helpers extracted from IL generation.

use zun_il::Opcode;
use zun_sema::hir::Ty;

use super::numkind_of;

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
        // NOTE: `Ty::Function` stays `Other` here. The language does not express
        // calling a stored function value (`f(n)` on a local is rejected by sema
        // as "unknown function"), so a `Function`-typed local never reaches the
        // JIT; only the transient `(expr)(args)` form does, and that carries the
        // function value on the operand stack, not in a local. Stored/closure
        // function values need sema support first.
        None => JitKind::Other,
    };
    kind.to_byte()
}

/// The field value-type code uses distinct codes for slot decoding: strings
/// carry a handle (12) and `any` fields are carried out-of-line in the heap's
/// `any_fields` side table (14), since an `Any` does not fit the 8-byte slot.
pub(crate) fn field_kind_byte(ty: Ty) -> u8 {
    match ty {
        Ty::String => 12,
        Ty::Any => 14,
        _ => jitkind_byte(ty),
    }
}
