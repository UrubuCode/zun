use super::Checker;
use crate::hir;
use zun_common::Span;

impl<'a> Checker<'a> {
    pub(super) fn poison(&mut self, span: Span) -> hir::Expr {
        hir::Expr {
            kind: hir::ExprKind::Error,
            ty: hir::Ty::Error,
            span,
        }
    }

    /// Whether a type is a built-in iterable that a collection constructor can
    /// drain. Mirrors the for-of iterator dispatch (`check_stmt`'s `ForOf`):
    /// arrays, maps, sets, strings, typed arrays, and `any` (resolved at
    /// runtime). `Array` is listed for completeness; the collection
    /// constructors route it to their dedicated array fast path.
    pub(super) fn is_iterable_ty(ty: hir::Ty) -> bool {
        matches!(
            ty,
            hir::Ty::Array
                | hir::Ty::Map
                | hir::Ty::Set
                | hir::Ty::String
                | hir::Ty::Any
                | hir::Ty::BufferView(_)
                // A user object implementing `[Symbol.iterator]`: a dynamic
                // object literal, or a typed class instance. The runtime
                // `from_iterable` native drives the protocol (and throws if the
                // value turns out not to be iterable, as JS does).
                | hir::Ty::DynamicObject
                | hir::Ty::Instance(_)
        )
    }
}
