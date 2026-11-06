use crate::check::*;

impl Checker<'_> {
    /// Evaluate non-literal `static` field initializers (`static n: number = 2 + 3`,
    /// `static #next = 1`) once, in class declaration order, before the module
    /// body. Literal initializers of a matching type are handled by the module
    /// initializer; this covers every other expression.
    pub(in crate::check) fn class_static_initializers(&mut self, cx: &mut FunctionCx, class_idx: u32) -> Vec<hir::Stmt> {
        let span = Span::new(0, 0);
        let mut out = Vec::new();
        let class_idx = class_idx as usize;
        {
            let fields = self.full_fields(class_idx as u32);
            let initializers = self.full_initializers(class_idx as u32);
            let first_own = fields.len().saturating_sub(self.classes[class_idx].own_fields.len());
            for (slot, (_, field_ty, is_static, _)) in fields.iter().enumerate() {
                if !*is_static || slot < first_own {
                    continue;
                }
                let Some(Some(init)) = initializers.get(slot) else { continue };
                let init = init.as_ref().clone();
                let field_ty = *field_ty;
                let literal = matches!(
                    (&init, field_ty),
                    (ast::Expr::Number { .. }, Ty::F64)
                        | (ast::Expr::String { .. }, Ty::String)
                        | (ast::Expr::Bool { .. }, Ty::Boolean)
                );
                // `static x;` (an implicit `void 0`) leaves the slot unset.
                if literal || matches!(init, ast::Expr::Unary { op: ast::UnaryOp::Void, .. }) {
                    continue;
                }
                let mut value = self.check_expr(cx, &init);
                coerce_numeric_literal(&mut value, field_ty);
                if field_ty == Ty::Any {
                    value = Self::box_to_any(value);
                } else if !field_ty.is_error() && !value.ty.is_error() && !self.is_assignable(value.ty, field_ty) {
                    self.error(value.span, format!("cannot assign {:?} to static field of type {field_ty:?}", value.ty));
                    continue;
                }
                out.push(hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::StaticFieldSet {
                        class: class_idx as u32,
                        field_slot: slot as u16,
                        value: Box::new(value),
                    },
                    ty: field_ty,
                    span,
                }));
            }
        }
        out
    }

    /// Emit the runtime TC39 decorator (PLAN §3.5 Stage 3, the "runtime flavor")
    /// invocations into the module-initializer `body`.
    ///
    /// ## What runs
    ///
    /// For each decorated class and each decorated method whose decorator callee
    /// is a bare `@dec` resolving to an ordinary top-level function value, this
    /// appends a single call `dec(value, context)` that executes once, before
    /// the entry point, at class-definition time. `context` is a `DynamicObject`
    /// `{ kind, name, static }`:
    ///
    /// * method decorator — `kind: "method"`, `name: <methodName>`,
    ///   `static: <bool>`.
    /// * class decorator — `kind: "class"`, `name: <className>`,
    ///   `static: false`.
    ///
    /// `value` is `null` in both cases: see [`decorator_null_value`] for why a
    /// live method/class value cannot be delivered in the current model.
    ///
    /// ## Evaluation order (TC39)
    ///
    /// Member decorators run before the class decorator, and the decorators of a
    /// single element run bottom-to-top (nearest the declaration last), so each
    /// element's list is applied in reverse source order.
    ///
    /// ## Macro-vs-runtime disambiguation
    ///
    /// Compile-time macro decorators (e.g. `@autoToString`) are consumed earlier,
    /// in `zun_compiler`'s macro-expansion pass, and their callee names are not
    /// declared functions, so they never resolve here. A decorator whose callee
    /// resolves to a top-level function value takes this runtime path; anything
    /// else (a macro name, an unknown name, `@sealed`/`@trace`) resolves to no
    /// function and stays inert — exactly the Phase-1 behavior. `zun_sema` cannot
    /// depend on `zun_compiler`'s macro registry, so "resolves to a function" is
    /// the registry-free equivalent of the coordinator's rule.
    ///
    /// ## Deferred (documented)
    ///
    /// A typed-context instance-method decorator that returns a `Function`
    /// **replaces the method**: only such decorated methods dispatch through
    /// `class_named_methods` (see `routed_methods`); the replacement receives the
    /// instance as its leading argument. A returned function from a field
    /// decorator is its initializer transform. Still deferred: class
    /// return-value replacement (`new C` targets a compile-time TypeDef, there
    /// is no first-class runtime class value to rebind); static-method
    /// replacement; a live class `value`; accessor and private-element
    /// decorators; and replacement results of decorator factories. Decorator
    /// factories `@dec(args)` are
    /// supported (literal arguments): the factory call is evaluated in the
    /// initializer and its result invoked via the `applyDecorator` native.
    /// Emit `classSetSymbolFunction(classRid, symbol, fn)` for each class
    /// computed-symbol-key method, attaching the method (as a function value)
    /// to the per-type symbol-method side table at class-definition time. The
    /// class rid is `class index + 1` (one TypeDef per class, in order).
    pub(in crate::check) fn append_class_symbol_methods(&self, body: &mut Vec<hir::Stmt>, span: Span) {
        if self.class_symbol_methods.is_empty() {
            return;
        }
        let Some((set_id, _)) = zun_natives::resolve("collections", "classSetSymbolFunction") else {
            return;
        };
        let Some((well_known_id, _)) = zun_natives::resolve("collections", "symbolWellKnown") else {
            return;
        };
        for &(class_idx, symbol_id, func_index) in &self.class_symbol_methods {
            let class_rid = hir::Expr {
                kind: hir::ExprKind::Number((class_idx + 1) as f64),
                ty: Ty::Int(IntKind::U32),
                span,
            };
            let symbol = hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: well_known_id,
                    args: vec![hir::Expr {
                        kind: hir::ExprKind::Number(symbol_id as f64),
                        ty: Ty::F64,
                        span,
                    }],
                },
                ty: Ty::Symbol,
                span,
            };
            let function = hir::Expr {
                kind: hir::ExprKind::FunctionRef { func: func_index },
                ty: Ty::Function(func_index),
                span,
            };
            body.push(hir::Stmt::Expr(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: set_id,
                    args: vec![class_rid, symbol, function],
                },
                ty: Ty::Void,
                span,
            }));
        }
    }

    /// Register, at class-definition time, (a) each class's zero-parameter
    /// instance `valueOf`/`toString` (own or inherited) via `classSetMethod`,
    /// so the interpreter floor can resolve them by name for ToPrimitive, and
    /// (b) each literal field-initializer default via `classSetFieldDefault`, so
    /// runtime construction (`Reflect.construct`, proxy `construct`) can fill
    /// absent trailing args. Non-literal initializers are not registered.
    pub(in crate::check) fn append_class_named_methods_and_defaults(&self, body: &mut Vec<hir::Stmt>, span: Span) {
        let class_rid = |class_idx: usize| hir::Expr {
            kind: hir::ExprKind::Number((class_idx + 1) as f64),
            ty: Ty::Int(IntKind::U32),
            span,
        };
        if let Some((set_method, _)) = zun_natives::resolve("collections", "classSetMethod") {
            for class_idx in 0..self.classes.len() {
                for name in ["valueOf", "toString", "get_name"] {
                    let Some(func) = self.method_of(class_idx as u32, name) else { continue };
                    let sig = &self.signatures[func as usize];
                    if sig.is_static || sig.params.len() != 1 {
                        continue;
                    }
                    // A routed (decorator-replaceable) method is already
                    // registered, and its entry may have been replaced.
                    if self.routed_methods.contains_key(&func) {
                        continue;
                    }
                    body.push(hir::Stmt::Expr(hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id: set_method,
                            args: vec![
                                class_rid(class_idx),
                                hir::Expr {
                                    kind: hir::ExprKind::String(name.to_string()),
                                    ty: Ty::String,
                                    span,
                                },
                                hir::Expr {
                                    kind: hir::ExprKind::FunctionRef { func },
                                    ty: Ty::Function(func),
                                    span,
                                },
                            ],
                        },
                        ty: Ty::Void,
                        span,
                    }));
                }
            }
        }
        // Static fields with a literal initializer (`static NONE: number = 0`)
        // are stored once at module initialization.
        for class_idx in 0..self.classes.len() {
            let fields = self.full_fields(class_idx as u32);
            let initializers = self.full_initializers(class_idx as u32);
            for (slot, (_, field_ty, is_static, _)) in fields.iter().enumerate() {
                if !*is_static {
                    continue;
                }
                let Some(Some(init)) = initializers.get(slot) else { continue };
                let (kind, ty) = match (init.as_ref(), *field_ty) {
                    (ast::Expr::Number { value, .. }, Ty::F64) => {
                        (hir::ExprKind::Number(*value), Ty::F64)
                    }
                    (ast::Expr::String { value, .. }, Ty::String) => {
                        (hir::ExprKind::String(value.clone()), Ty::String)
                    }
                    (ast::Expr::Bool { value, .. }, Ty::Boolean) => {
                        (hir::ExprKind::Boolean(*value), Ty::Boolean)
                    }
                    _ => continue,
                };
                body.push(hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::StaticFieldSet {
                        class: class_idx as u32,
                        field_slot: slot as u16,
                        value: Box::new(hir::Expr { kind, ty, span }),
                    },
                    ty,
                    span,
                }));
            }
        }
        if let Some((set_default, _)) = zun_natives::resolve("collections", "classSetFieldDefault") {
            for class_idx in 0..self.classes.len() {
                let fields = self.full_fields(class_idx as u32);
                let initializers = self.full_initializers(class_idx as u32);
                for (slot, (_, field_ty, _, _)) in fields.iter().enumerate() {
                    let Some(Some(init)) = initializers.get(slot) else { continue };
                    let (kind, ty) = match (init.as_ref(), *field_ty) {
                        (ast::Expr::Number { value, .. }, Ty::F64 | Ty::Any) => {
                            (hir::ExprKind::Number(*value), Ty::F64)
                        }
                        (ast::Expr::String { value, .. }, Ty::String | Ty::Any) => {
                            (hir::ExprKind::String(value.clone()), Ty::String)
                        }
                        (ast::Expr::Bool { value, .. }, Ty::Boolean | Ty::Any) => {
                            (hir::ExprKind::Boolean(*value), Ty::Boolean)
                        }
                        _ => continue,
                    };
                    body.push(hir::Stmt::Expr(hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id: set_default,
                            args: vec![
                                class_rid(class_idx),
                                hir::Expr {
                                    kind: hir::ExprKind::Number(slot as f64),
                                    ty: Ty::Int(IntKind::U32),
                                    span,
                                },
                                hir::Expr {
                                    kind: hir::ExprKind::BoxAny {
                                        value: Box::new(hir::Expr { kind, ty, span }),
                                    },
                                    ty: Ty::Any,
                                    span,
                                },
                            ],
                        },
                        ty: Ty::Void,
                        span,
                    }));
                }
            }
        }
    }
}
