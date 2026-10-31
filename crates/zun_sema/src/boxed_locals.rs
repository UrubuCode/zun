use super::*;

pub(super) fn rewrite_boxed_lets(
    statements: &mut [hir::Stmt],
    boxed: &HashSet<hir::LocalSlot>,
) {
    for statement in statements {
        match statement {
            hir::Stmt::Let { slot, init } if boxed.contains(slot) => {
                let span = init.span;
                let slot = *slot;
                let value = std::mem::replace(
                    init,
                    hir::Expr {
                        kind: hir::ExprKind::Error,
                        ty: Ty::Error,
                        span,
                    },
                );
                *statement = hir::Stmt::BoxedLet { slot, init: value };
            }
            hir::Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                rewrite_boxed_lets(then_body, boxed);
                rewrite_boxed_lets(else_body, boxed);
            }
            hir::Stmt::While { body, .. }
            | hir::Stmt::DoWhile { body, .. }
            | hir::Stmt::ForOf { body, .. } => rewrite_boxed_lets(body, boxed),
            hir::Stmt::Group(body) => rewrite_boxed_lets(body, boxed),
            hir::Stmt::Labeled { body, .. } => {
                rewrite_boxed_lets(std::slice::from_mut(body.as_mut()), boxed)
            }
            hir::Stmt::For { init, cond, update, body } => {
                let mut loop_cells: Vec<hir::LocalSlot> = Vec::new();
                if let Some(init) = init {
                    rewrite_boxed_lets(std::slice::from_mut(init.as_mut()), boxed);
                    collect_boxed_lets(init, &mut loop_cells);
                }
                rewrite_boxed_lets(body, boxed);
                // Per-iteration bindings: before the update runs, every
                // closure-captured loop variable moves into a fresh cell
                // holding the current value, so closures created in earlier
                // iterations keep observing their own iteration's value.
                let span = cond.span;
                for slot in loop_cells {
                    let renew = hir::Expr {
                        kind: hir::ExprKind::Assign {
                            slot,
                            value: Box::new(hir::Expr {
                                kind: hir::ExprKind::DynamicObjectLiteral {
                                    entries: vec![hir::ObjectEntry::Data {
                                        key: hir::ObjectKey::Named("$value".to_string()),
                                        value: hir::Expr {
                                            kind: hir::ExprKind::BoxAny {
                                                value: Box::new(hir::Expr {
                                                    kind: hir::ExprKind::CellLoad { slot },
                                                    ty: Ty::Any,
                                                    span,
                                                }),
                                            },
                                            ty: Ty::Any,
                                            span,
                                        },
                                    }],
                                },
                                ty: Ty::DynamicObject,
                                span,
                            }),
                        },
                        ty: Ty::DynamicObject,
                        span,
                    };
                    *update = Some(match update.take() {
                        Some(old) => {
                            let ty = old.ty;
                            let span = old.span;
                            hir::Expr {
                                kind: hir::ExprKind::Sequence {
                                    effect: Box::new(renew),
                                    value: Box::new(old),
                                },
                                ty,
                                span,
                            }
                        }
                        None => renew,
                    });
                }
            }
            hir::Stmt::Try {
                body,
                catches,
                finally_body,
            } => {
                rewrite_boxed_lets(body, boxed);
                for catch in catches {
                    rewrite_boxed_lets(&mut catch.body, boxed);
                }
                if let Some(finally_body) = finally_body {
                    rewrite_boxed_lets(finally_body, boxed);
                }
            }
            _ => {}
        }
    }
}

/// Collect the cell slots declared by a (possibly grouped) rewritten `for` init.
fn collect_boxed_lets(statement: &hir::Stmt, out: &mut Vec<hir::LocalSlot>) {
    match statement {
        hir::Stmt::BoxedLet { slot, .. } => out.push(*slot),
        hir::Stmt::Group(body) => {
            for inner in body {
                collect_boxed_lets(inner, out);
            }
        }
        _ => {}
    }
}

/// A parameter that closures capture and mutate lives in a shared cell: wrap
/// its incoming value in a fresh cell at function entry.
pub(super) fn box_parameters(
    body: &mut Vec<hir::Stmt>,
    boxed: &HashSet<hir::LocalSlot>,
    locals: &[Ty],
    param_count: usize,
) {
    let mut slots: Vec<hir::LocalSlot> = boxed
        .iter()
        .copied()
        .filter(|slot| (*slot as usize) < param_count)
        .collect();
    slots.sort_unstable();
    for (index, slot) in slots.into_iter().enumerate() {
        let ty = locals.get(slot as usize).copied().unwrap_or(Ty::Any);
        body.insert(
            index,
            hir::Stmt::BoxedLet {
                slot,
                init: hir::Expr {
                    kind: hir::ExprKind::Local(slot),
                    ty,
                    span: zun_common::Span::new(0, 0),
                },
            },
        );
    }
}
