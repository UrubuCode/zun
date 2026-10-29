//! Deriving a compiler external-function surface from a loaded `zun_abi`
//! extension.
//!
//! A native extension (e.g. the `zun-node` library) registers its functions
//! with the host, each carrying a stable `(module, key, params, ret)` signature.
//! To compile a program that imports from such a module, the compiler needs the
//! matching [`ExternalFunction`] declarations. Rather than hardcode any module's
//! surface, derive them from the extension's own registered descriptors, so the
//! compiler stays extension-agnostic and the surface can never drift from what
//! the extension actually provides.

use zun_abi::{OwnedFunction, ZunTy, ZunTyRepr};
use zun_sema::check::ExternalFunction;
use zun_sema::hir::IntKind;
use zun_sema::Ty;

/// Map a stable ABI scalar type to the checker's `Ty`. Returns `None` for a
/// carrier the source language cannot yet name (so that function is skipped
/// rather than given a wrong type).
fn ty_of(repr: ZunTyRepr) -> Option<Ty> {
    let tag = repr.0;
    Some(if tag == ZunTy::Void as u32 {
        Ty::Void
    } else if tag == ZunTy::Bool as u32 {
        Ty::Boolean
    } else if tag == ZunTy::I32 as u32 {
        Ty::Int(IntKind::I32)
    } else if tag == ZunTy::U32 as u32 {
        Ty::Int(IntKind::U32)
    } else if tag == ZunTy::I64 as u32 {
        Ty::Int(IntKind::I64)
    } else if tag == ZunTy::U64 as u32 {
        Ty::Int(IntKind::U64)
    } else if tag == ZunTy::F64 as u32 {
        Ty::F64
    } else if tag == ZunTy::Str as u32 {
        Ty::String
    } else if tag == ZunTy::Opaque as u32 {
        Ty::Any
    } else {
        return None;
    })
}

/// Derive the [`ExternalFunction`] declarations for `module` from a loaded
/// extension's registered descriptors (`functions`, e.g. an extension's
/// `OwnedRegistry::functions` or a `LoadedExtension::functions()`). Functions
/// belonging to other modules, or carrying a type the source language cannot
/// name, are skipped.
pub fn external_functions(functions: &[OwnedFunction], module: &str) -> Vec<ExternalFunction> {
    let mut externals = Vec::new();
    for function in functions {
        if function.module != module {
            continue;
        }
        let params: Option<Vec<Ty>> = function.params.iter().copied().map(ty_of).collect();
        let (Some(params), Some(return_ty)) = (params, ty_of(function.ret)) else {
            continue;
        };
        externals.push(ExternalFunction {
            module: function.module.clone(),
            name: function.key.clone(),
            params,
            return_ty,
        });
    }
    externals
}

#[cfg(test)]
mod tests {
    use super::*;
    use zun_abi::ZunValue;

    unsafe extern "C" fn noop(_: *const ZunValue, _: usize, _: *mut ZunValue) -> i32 {
        0
    }

    fn descriptor(module: &str, key: &str, params: Vec<ZunTy>, ret: ZunTy) -> OwnedFunction {
        OwnedFunction {
            module: module.to_owned(),
            key: key.to_owned(),
            params: params.into_iter().map(ZunTyRepr::from).collect(),
            ret: ZunTyRepr::from(ret),
            invoke: noop,
        }
    }

    #[test]
    fn derives_declarations_for_the_requested_module_only() {
        let functions = vec![
            descriptor("node:fs", "readFileUtf8", vec![ZunTy::Str], ZunTy::Str),
            descriptor("node:fs", "statSize", vec![ZunTy::Str], ZunTy::I64),
            descriptor("fs", "readFileUtf8", vec![ZunTy::Str], ZunTy::Str),
        ];
        let externals = external_functions(&functions, "node:fs");
        assert_eq!(externals.len(), 2, "only node:fs functions");
        let read = externals.iter().find(|f| f.name == "readFileUtf8").unwrap();
        assert_eq!(read.module, "node:fs");
        assert_eq!(read.params, vec![Ty::String]);
        assert_eq!(read.return_ty, Ty::String);
        let stat = externals.iter().find(|f| f.name == "statSize").unwrap();
        assert_eq!(stat.return_ty, Ty::Int(IntKind::I64));
    }
}
