use super::*;

pub(super) fn is_number_literal(expr: &hir::Expr) -> bool {
    matches!(expr.kind, hir::ExprKind::Number(_)) && expr.ty == Ty::F64
}

fn fits_int(v: f64, k: IntKind) -> bool {
    if v.fract() != 0.0 { return false; }
    let (lo, hi) = int_range(k);
    v >= lo && v <= hi
}

fn int_range(k: IntKind) -> (f64, f64) {
    if k.signed {
        let hi = 2f64.powi(k.bits as i32 - 1);
        (-hi, hi - 1.0)
    } else {
        (0.0, 2f64.powi(k.bits as i32) - 1.0)
    }
}

pub(super) fn coerce_numeric_literal(expr: &mut hir::Expr, expected: Ty) {
    // Untyped (JS) values flowing into a declared `string`/`boolean` slot are
    // unboxed implicitly, as TypeScript treats `any` as assignable to anything.
    if expr.ty == Ty::Any && matches!(expected, Ty::String | Ty::Boolean) {
        let span = expr.span;
        let inner = std::mem::replace(
            expr,
            hir::Expr { kind: hir::ExprKind::Null, ty: Ty::Any, span },
        );
        *expr = hir::Expr {
            kind: hir::ExprKind::Convert { value: Box::new(inner) },
            ty: expected,
            span,
        };
        return;
    }
    // JS `number` (f64) and the 64-bit timestamp type used by Date interchange
    // implicitly: a non-literal i64 widens to a `number` parameter and a
    // `number` narrows to an i64 slot (`new Date(ms)`).
    if !is_number_literal(expr)
        && matches!(
            (expr.ty, expected),
            (Ty::Int(IntKind { bits: 64, signed: true }), Ty::F64)
                | (Ty::F64, Ty::Int(IntKind { bits: 64, signed: true }))
        )
    {
        let span = expr.span;
        let inner = std::mem::replace(
            expr,
            hir::Expr { kind: hir::ExprKind::Null, ty: Ty::Any, span },
        );
        *expr = hir::Expr {
            kind: hir::ExprKind::Convert { value: Box::new(inner) },
            ty: expected,
            span,
        };
        return;
    }
    if !is_number_literal(expr) { return; }
    let v = match expr.kind { hir::ExprKind::Number(v) => v, _ => return };
    match expected {
        Ty::Float(_) => expr.ty = expected,
        Ty::Int(k) if fits_int(v, k) => expr.ty = expected,
        _ => {}
    }
}

pub(super) fn unify_numeric_literal(a: &mut hir::Expr, b: &mut hir::Expr) {
    if (a.ty.is_int() || a.ty.is_float()) && is_number_literal(b) {
        coerce_numeric_literal(b, a.ty);
    } else if (b.ty.is_int() || b.ty.is_float()) && is_number_literal(a) {
        coerce_numeric_literal(a, b.ty);
    }
}
