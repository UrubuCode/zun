use crate::hir::{IntKind, Ty};

/// Map an intrinsic signature type to the checker's `Ty`.
pub(super) fn native_ty(t: zun_natives::NativeTy) -> Ty {
    use zun_natives::NativeTy as N;
    match t {
        N::I8 => Ty::Int(IntKind::I8),
        N::I16 => Ty::Int(IntKind::I16),
        N::I32 => Ty::Int(IntKind::I32),
        N::I64 => Ty::Int(IntKind::I64),
        N::U8 => Ty::Int(IntKind::U8),
        N::U16 => Ty::Int(IntKind::U16),
        N::U32 => Ty::Int(IntKind::U32),
        N::U64 => Ty::Int(IntKind::U64),
        N::F32 => Ty::F32,
        N::F64 => Ty::F64,
        N::Bool => Ty::Boolean,
        N::Str => Ty::String,
        N::Void => Ty::Void,
        N::Opaque => Ty::Any,
        N::Array => Ty::Array,
        N::Map => Ty::Map,
        N::Set => Ty::Set,
        N::WeakMap => Ty::WeakMap,
        N::Promise => Ty::Promise,
        N::RegExp => Ty::RegExp,
    }
}
