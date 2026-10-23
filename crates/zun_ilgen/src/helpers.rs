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
        None => JitKind::Other,
    };
    kind.to_byte()
}

/// The field value-type code uses a distinct string code for slot decoding.
pub(crate) fn field_kind_byte(ty: Ty) -> u8 {
    match ty {
        Ty::String => 12,
        _ => jitkind_byte(ty),
    }
}
