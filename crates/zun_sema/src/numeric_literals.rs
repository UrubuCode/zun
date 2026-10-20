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
