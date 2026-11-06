use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_new_target(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::Ident(_) = expr else { unreachable!() };
        // Inside a constructor `new.target` is the class that was
        // instantiated: the runtime class of `this` (the most-derived
        // class when reached through a `super(...)` chain). Everywhere
        // else it is `undefined`.
        if cx.is_constructor {
            if let Some(this_slot) = cx.lookup("this") {
                let this_expr = hir::Expr {
                    ty: cx.locals[this_slot as usize],
                    kind: hir::ExprKind::Local(this_slot),
                    span,
                };
                let boxed = hir::Expr {
                    kind: hir::ExprKind::BoxAny { value: Box::new(this_expr) },
                    ty: Ty::Any,
                    span,
                };
                let record = hir::Expr {
                    kind: hir::ExprKind::DynamicGet { object: Box::new(boxed), name: "constructor".to_string() },
                    ty: Ty::Any,
                    span,
                };
                return hir::Expr {
                    kind: hir::ExprKind::DynamicGet { object: Box::new(record), name: "__classDesc".to_string() },
                    ty: Ty::Any,
                    span,
                };
            }
        }
        let void_zero = ast::Expr::Unary {
            op: ast::UnaryOp::Void,
            operand: Box::new(ast::Expr::Number { value: 0.0, span }),
            span,
        };
        self.check_expr(cx, &void_zero)
    }

    pub(in crate::check) fn check_ident(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::Ident(name) = expr else { unreachable!() };
        let text = self.text(name).to_string();
        if cx.tdz_names.contains(&text) {
            // Reading a parameter before its initialisation is a ReferenceError.
            if let Some((id, _)) = zun_natives::resolve("collections", "globalRead") {
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![Self::string_literal(format!("{text}"), span)],
                    },
                    ty: Ty::Any,
                    span,
                };
            }
        }
        match cx.lookup(&text) {
            Some(slot) if self.reads_through_global(cx, &text, slot) => {
                let read = self.global_get_expr(&text, span);
                Self::unbox_to(read, cx.locals[slot as usize])
            }
            Some(slot) => hir::Expr {
                ty: cx.locals[slot as usize],
                kind: if cx.boxed.contains(&slot) {
                    hir::ExprKind::CellLoad { slot }
                } else {
                    hir::ExprKind::Local(slot)
                },
                span,
            },
            None => {
                if text == "globalThis" || text == "global" {
                    return hir::Expr { kind: hir::ExprKind::RealmGlobal, ty: Ty::DynamicObject, span };
                }
                // `this` outside a method (module scope, plain function):
                // strict-mode `undefined`.
                if text == "this" && !cx.captures.contains_key("this") {
                    let void_zero = ast::Expr::Unary {
                        op: ast::UnaryOp::Void,
                        operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                        span,
                    };
                    return self.check_expr(cx, &void_zero);
                }
                if let Some(capture) = cx.captures.get(&text) {
                    return hir::Expr {
                        kind: hir::ExprKind::CaptureLoad {
                            environment_slot: capture.environment_slot,
                            ty: capture.ty,
                            mode: capture.mode,
                        },
                        ty: capture.ty,
                        span,
                    };
                }
                if let Some(&func) = self.by_name.get(&text) {
                    return hir::Expr {
                        kind: hir::ExprKind::FunctionRef { func },
                        ty: Ty::Function(func),
                        span,
                    };
                }
                // `String` / `Number` / `Boolean` as first-class values
                // (callbacks such as `xs.map(String)`).
                let coerce_name = match text.as_str() {
                    "String" => Some("__coerceString"),
                    "Number" => Some("__coerceNumber"),
                    "Boolean" => Some("__coerceBoolean"),
                    _ => None,
                };
                if let Some(&func) = coerce_name.and_then(|n| self.by_name.get(n)) {
                    return hir::Expr {
                        kind: hir::ExprKind::FunctionRef { func },
                        ty: Ty::Function(func),
                        span,
                    };
                }
                // A declarative realm global read as a value yields its
                // opaque class descriptor (PLAN §3.1).
                if let Some(token) = self.realm_globals.get(&text).map(|b| b.token) {
                    return self.realm_descriptor_expr(token, span);
                }
                // A user class read as a value yields a class descriptor
                // (`USER_CLASS_FLAG | TypeDef rid`) usable by
                // `Reflect.construct` and `new Proxy(C, handler)`.
                if let Some(&class_idx) = self.classes_by_name.get(&text) {
                    let token = zun_natives::USER_CLASS_FLAG | (class_idx + 1);
                    return self.realm_descriptor_expr(token, span);
                }
                // A module global (top-level `let`/`const`) read from any
                // scope resolves through `globalGet` (dynamic).
                if self.module_globals.contains(&text) {
                    return self.global_get_expr(&text, span);
                }
                // An undeclared identifier is a run-time ReferenceError (it
                // may sit in dead code), not a compile error.
                if let Some((id, _)) = zun_natives::resolve("collections", "globalRead") {
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![Self::string_literal(text.clone(), span)],
                        },
                        ty: Ty::Any,
                        span,
                    };
                }
                self.error(span, format!("unknown variable `{text}`"));
                self.poison(span)
            }
        }
    }
}
