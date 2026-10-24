use crate::hir::{IntKind, Ty};

/// Map an intrinsic signature type to the checker's `Ty`.
pub(super) fn native_ty(t: zun_natives::NativeTy) -> Ty {
    use zun_natives::NativeTy as N;
    // The typed-array view types come from the master table.
    if let Some(view) = crate::typed_arrays::view_native_ty(t) {
        return view;
    }
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
        N::Buffer => Ty::Buffer,
        N::DataView => Ty::BufferView(crate::hir::BufferViewKind::DataView),
        // Answered above from the master table. They are still listed so the
        // match stays exhaustive: a kind added to the table without a
        // `NativeTy` variant, or with one, fails to build here rather than
        // reaching a `_` arm that quietly does the wrong thing.
        N::I8Array
        | N::U8Array
        | N::U8ClampedArray
        | N::I16Array
        | N::U16Array
        | N::I32Array
        | N::U32Array
        | N::F32Array
        | N::F64Array
        | N::I64Array
        | N::U64Array => unreachable!("typed-array views are answered from the table"),
        N::Map => Ty::Map,
        N::Set => Ty::Set,
        N::WeakMap => Ty::WeakMap,
        N::WeakSet => Ty::WeakSet,
        N::WeakRef => Ty::WeakRef,
        N::Promise => Ty::Promise,
        N::RegExp => Ty::RegExp,
        N::BigInt => Ty::BigInt,
        N::FinalizationRegistry => Ty::FinalizationRegistry,
        N::DynamicObject => Ty::DynamicObject,
        N::Proxy => Ty::Proxy,
        N::Function => Ty::Function(crate::hir::DYNAMIC_FUNCTION),
    }
}
