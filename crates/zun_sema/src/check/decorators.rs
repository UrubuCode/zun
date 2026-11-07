use crate::check::*;

impl Checker<'_> {
    /// The signature of a bare `@dec` decorator whose callee is a top-level
    /// `(any, DecoratorContext)` function: the typed-context decorator form.
    pub(in crate::check) fn typed_decorator_signature(
        &self,
        decorator: &ast::Decorator,
    ) -> Option<&Signature> {
        if decorator.args.is_some() {
            return None;
        }
        let ctx_class = *self.classes_by_name.get("DecoratorContext")?;
        let span = decorator.callee.span();
        let callee = self.source.get(span.start as usize..span.end as usize)?;
        let signature = self.signatures.get(*self.by_name.get(callee)? as usize)?;
        (signature.params.len() == 2
            && signature.rest_param.is_none()
            && matches!(signature.params[0], Ty::Any)
            && signature.params[1] == Ty::Instance(ctx_class))
        .then_some(signature)
    }

    /// Decide, before any body is checked, which instance methods are routed
    /// through the `class_named_methods` side table (a typed-context decorator
    /// returns a function, so it may replace the method) and which classes need
    /// the `new` hooks (instance `addInitializer`, field transforms). Only
    /// decorated elements opt in: undecorated methods and classes keep static
    /// dispatch and plain `new`.
    pub(in crate::check) fn collect_decorator_routing(&mut self, program: &ast::Program) {
        let mut routed: HashMap<u32, u32> = HashMap::new();
        let mut own_hooked: HashSet<u32> = HashSet::new();
        for (class_idx, class) in program.classes.iter().enumerate() {
            let class_idx = class_idx as u32;
            for method in &class.methods {
                if method.is_static {
                    continue;
                }
                let name = self.text(&method.name).to_string();
                let Some(&func) = self
                    .methods_by_class
                    .get(class_idx as usize)
                    .and_then(|m| m.get(&name))
                else {
                    continue;
                };
                let rest = self.signatures[func as usize].rest_param.is_some();
                for decorator in &method.decorators {
                    let Some(signature) = self.typed_decorator_signature(decorator) else {
                        continue;
                    };
                    own_hooked.insert(class_idx);
                    if !rest && matches!(signature.return_ty, Ty::Function(_)) {
                        routed.insert(func, class_idx);
                    }
                }
            }
            for field in &class.fields {
                if field.is_static || field.is_private {
                    continue;
                }
                if field
                    .decorators
                    .iter()
                    .any(|decorator| self.typed_decorator_signature(decorator).is_some())
                {
                    own_hooked.insert(class_idx);
                }
            }
        }
        let mut hooked = HashSet::new();
        for class in 0..self.classes.len() as u32 {
            let mut current = Some(class);
            while let Some(c) = current {
                if own_hooked.contains(&c) {
                    hooked.insert(class);
                    break;
                }
                current = self.classes.get(c as usize).and_then(|info| info.base);
            }
        }
        self.routed_methods = routed;
        self.hooked_classes = hooked;
    }

    /// Register, ahead of every decorator, the original body of each routed
    /// (decorated, function-returning) instance method in `class_named_methods`,
    /// plus the undecorated overrides of that method in subclasses, so a routed
    /// call site finds the runtime class's own implementation.
    pub(in crate::check) fn register_routed_methods(
        &self,
        program: &ast::Program,
        body: &mut Vec<hir::Stmt>,
        span: Span,
    ) {
        let Some((set_method, _)) = zun_natives::resolve("collections", "classSetMethod") else {
            return;
        };
        let mut routed: Vec<(u32, u32)> =
            self.routed_methods.iter().map(|(&f, &c)| (f, c)).collect();
        routed.sort_unstable();
        let register = |class: u32, name: &str, func: u32| {
            hir::Stmt::Expr(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: set_method,
                    args: vec![
                        hir::Expr {
                            kind: hir::ExprKind::Number((class + 1) as f64),
                            ty: Ty::Int(IntKind::U32),
                            span,
                        },
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
            })
        };
        for (func, class) in routed {
            let Some(name) = self.method_name_of(class, func) else {
                continue;
            };
            body.push(register(class, &name, func));
            // Undecorated overrides in subclasses.
            for (sub, sub_class) in program.classes.iter().enumerate() {
                let sub = sub as u32;
                if sub == class || !self.is_subclass(sub, class) {
                    continue;
                }
                if !sub_class.methods.iter().any(|m| self.text(&m.name) == name) {
                    continue;
                }
                let Some(&own) = self
                    .methods_by_class
                    .get(sub as usize)
                    .and_then(|m| m.get(&name))
                else {
                    continue;
                };
                if !self.routed_methods.contains_key(&own) {
                    body.push(register(sub, &name, own));
                }
            }
        }
    }

    /// Lower a call to a routed (decorator-replaceable) instance method as an
    /// indirect call: `classBoundMethod(receiver, declaringClass, name)` looks the
    /// receiver's runtime-class entry up in `class_named_methods` and binds the
    /// receiver as `this`; the bound function value is then invoked with the
    /// remaining arguments. The dynamic call yields `Any`, converted back to the
    /// method's return type (a `void` method's result is left as `Any`).
    pub(in crate::check) fn routed_method_call(
        &self,
        func: u32,
        owner: u32,
        call_args: &mut Vec<hir::Expr>,
        return_ty: Ty,
        span: Span,
    ) -> Option<hir::Expr> {
        if matches!(return_ty, Ty::Function(_)) {
            return None;
        }
        let (bound_id, _) = zun_natives::resolve("collections", "classBoundMethod")?;
        let name = self.method_name_of(owner, func)?;
        let receiver = call_args.remove(0);
        let boxed = hir::Expr {
            kind: hir::ExprKind::BoxAny {
                value: Box::new(receiver),
            },
            ty: Ty::Any,
            span,
        };
        let callee = hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id: bound_id,
                args: vec![
                    boxed,
                    hir::Expr {
                        kind: hir::ExprKind::Number((owner + 1) as f64),
                        ty: Ty::Int(IntKind::U32),
                        span,
                    },
                    hir::Expr {
                        kind: hir::ExprKind::String(name),
                        ty: Ty::String,
                        span,
                    },
                ],
            },
            ty: Ty::Function(hir::DYNAMIC_FUNCTION),
            span,
        };
        let call = hir::Expr {
            kind: hir::ExprKind::IndirectCall {
                callee: Box::new(callee),
                args: std::mem::take(call_args),
                func: hir::DYNAMIC_FUNCTION,
            },
            ty: Ty::Any,
            span,
        };
        if matches!(return_ty, Ty::Void | Ty::Error) {
            return Some(call);
        }
        Some(hir::Expr {
            kind: hir::ExprKind::Convert {
                value: Box::new(call),
            },
            ty: return_ty,
            span,
        })
    }

    /// The source-level name of method `func` declared by `class`.
    pub(in crate::check) fn method_name_of(&self, class: u32, func: u32) -> Option<String> {
        self.methods_by_class
            .get(class as usize)?
            .iter()
            .find(|(_, &index)| index == func)
            .map(|(name, _)| name.clone())
    }

    pub(in crate::check) fn append_runtime_decorator_calls(
        &self,
        program: &ast::Program,
        body: &mut Vec<hir::Stmt>,
        span: Span,
    ) {
        self.register_routed_methods(program, body, span);
        for (class_idx, class) in program.classes.iter().enumerate() {
            let class_idx = class_idx as u32;
            let class_name = self.text(&class.name).to_string();
            let full_fields = self.full_fields(class_idx);
            // Field decorators run at class-definition time with
            // `kind: "field"`. Private (`#x`) fields stay deferred. A decorator
            // returning a function installs it as the field's initializer
            // transform, applied per instance by the `new` hook.
            for field in &class.fields {
                if field.is_private {
                    continue;
                }
                let field_name = self.text(&field.name).to_string();
                let slot = full_fields
                    .iter()
                    .position(|(name, _, _, _)| *name == field_name)
                    .map_or(-1, |slot| slot as i32);
                for decorator in field.decorators.iter().rev() {
                    let target = DecoratorTarget {
                        class_idx,
                        slot,
                        replaces_method: false,
                        transforms_field: !field.is_static && slot >= 0,
                    };
                    if let Some(stmt) = self.runtime_decorator_call(
                        decorator,
                        || decorator_null_value(span),
                        "field",
                        &field_name,
                        field.is_static,
                        target,
                        span,
                    ) {
                        body.push(stmt);
                    }
                }
            }
            // Member decorators first (TC39: before the class decorator).
            for method in &class.methods {
                let method_name = self.text(&method.name).to_string();
                let Some(&func_index) = self
                    .methods_by_class
                    .get(class_idx as usize)
                    .and_then(|m| m.get(&method_name))
                else {
                    continue;
                };
                let routed = self.routed_methods.contains_key(&func_index);
                // Bottom-to-top per element. A routed method hands its current
                // body to each decorator as a `functionBox`ed `value`.
                for decorator in method.decorators.iter().rev() {
                    let make_value = || {
                        if routed {
                            self.method_value_expr(class_idx, &method_name, span)
                        } else {
                            decorator_null_value(span)
                        }
                    };
                    let target = DecoratorTarget {
                        class_idx,
                        slot: -1,
                        replaces_method: routed,
                        transforms_field: false,
                    };
                    if let Some(stmt) = self.runtime_decorator_call(
                        decorator,
                        make_value,
                        "method",
                        &method_name,
                        method.is_static,
                        target,
                        span,
                    ) {
                        body.push(stmt);
                    }
                }
            }
            // Class decorators after members, bottom-to-top.
            for decorator in class.decorators.iter().rev() {
                let target = DecoratorTarget {
                    class_idx,
                    slot: -1,
                    replaces_method: false,
                    transforms_field: false,
                };
                if let Some(stmt) = self.runtime_decorator_call(
                    decorator,
                    || decorator_null_value(span),
                    "class",
                    &class_name,
                    false,
                    target,
                    span,
                ) {
                    body.push(stmt);
                }
            }
        }
    }

    /// `classMethodBox(classRid, name)`: the method's current body as a
    /// `functionBox` record in an `Any`, the `value` a method decorator receives.
    pub(in crate::check) fn method_value_expr(
        &self,
        class_idx: u32,
        name: &str,
        span: Span,
    ) -> hir::Expr {
        let Some((id, _)) = zun_natives::resolve("collections", "classMethodBox") else {
            return decorator_null_value(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![
                    hir::Expr {
                        kind: hir::ExprKind::Number((class_idx + 1) as f64),
                        ty: Ty::Int(IntKind::U32),
                        span,
                    },
                    hir::Expr {
                        kind: hir::ExprKind::String(name.to_string()),
                        ty: Ty::String,
                        span,
                    },
                ],
            },
            ty: Ty::Any,
            span,
        }
    }

    /// Build one statement for a runtime decorator: a direct `dec(value,
    /// context)` call for a bare `@dec` whose callee is a `(any, any)`
    /// top-level function, or, for a factory `@dec(args)`, `applyDecorator(
    /// dec(args), value, context)`. Returns `None` when the decorator is inert
    /// (callee is not a resolvable top-level function, or a factory with
    /// non-literal arguments or a non-function return).
    pub(in crate::check) fn runtime_decorator_call(
        &self,
        decorator: &ast::Decorator,
        make_value: impl Fn() -> hir::Expr,
        kind: &str,
        name: &str,
        is_static: bool,
        target: DecoratorTarget,
        span: Span,
    ) -> Option<hir::Stmt> {
        let callee_span = decorator.callee.span();
        let callee = self
            .source
            .get(callee_span.start as usize..callee_span.end as usize)?;
        // Must resolve to a top-level function value (else inert / macro).
        let dec_index = *self.by_name.get(callee)?;
        let signature = self.signatures.get(dec_index as usize)?;

        // Typed context: `(value: any, context: DecoratorContext)`. Lowered to
        // `ctx = make(..); dec(null, ctx); finish(ctx)` so `addInitializer`
        // callbacks live in typed class fields (functions cannot cross `Any`).
        if decorator.args.is_none() {
            if let Some(stmt) = self.typed_context_decorator(
                dec_index,
                signature,
                make_value(),
                kind,
                name,
                is_static,
                target,
                span,
            ) {
                return Some(stmt);
            }
        }

        let context = hir::Expr {
            kind: hir::ExprKind::DynamicObjectLiteral {
                entries: vec![
                    decorator_context_entry(
                        "kind",
                        hir::ExprKind::String(kind.to_string()),
                        Ty::String,
                        span,
                    ),
                    decorator_context_entry(
                        "name",
                        hir::ExprKind::String(name.to_string()),
                        Ty::String,
                        span,
                    ),
                    decorator_context_entry(
                        "static",
                        hir::ExprKind::Boolean(is_static),
                        Ty::Boolean,
                        span,
                    ),
                ],
            },
            ty: Ty::DynamicObject,
            span,
        };
        let context_arg = hir::Expr {
            kind: hir::ExprKind::BoxAny {
                value: Box::new(context),
            },
            ty: Ty::Any,
            span,
        };

        match &decorator.args {
            None => {
                // Bare `@dec`: the callee is itself the decorator and must be
                // exactly `(value: any, context: any)`.
                if signature.params.len() != 2
                    || signature.rest_param.is_some()
                    || !matches!(signature.params[0], Ty::Any)
                    || !matches!(signature.params[1], Ty::Any)
                {
                    return None;
                }
                Some(hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::Call {
                        func: dec_index,
                        args: vec![make_value(), context_arg],
                    },
                    // The return value is discarded by the expression statement.
                    ty: signature.return_ty,
                    span,
                }))
            }
            Some(args) => {
                // Decorator factory `@dec(args)`: evaluate `dec(args)` to produce
                // the actual decorator function value, then invoke it with
                // `(value, context)` via `applyDecorator`. The factory must take
                // exactly `args` (literal) parameters and return a function value.
                if signature.rest_param.is_some() || signature.params.len() != args.len() {
                    return None;
                }
                if !matches!(signature.return_ty, Ty::Function(_) | Ty::Any) {
                    return None;
                }
                let mut lowered = Vec::with_capacity(args.len());
                for arg in args {
                    lowered.push(self.lower_decorator_literal_arg(arg, span)?);
                }
                let factory_call = hir::Expr {
                    kind: hir::ExprKind::Call {
                        func: dec_index,
                        args: lowered,
                    },
                    ty: signature.return_ty,
                    span,
                };
                let (apply_id, _) = zun_natives::resolve("collections", "applyDecorator")?;
                Some(hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: apply_id,
                        args: vec![factory_call, make_value(), context_arg],
                    },
                    ty: Ty::Void,
                    span,
                }))
            }
        }
    }

    /// Lowering for a decorator whose second parameter is the prelude's typed
    /// `DecoratorContext`. Uses module-initializer local slot 0.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::check) fn typed_context_decorator(
        &self,
        dec_index: u32,
        signature: &Signature,
        value: hir::Expr,
        kind: &str,
        name: &str,
        is_static: bool,
        target: DecoratorTarget,
        span: Span,
    ) -> Option<hir::Stmt> {
        let ctx_class = *self.classes_by_name.get("DecoratorContext")?;
        let ctx_ty = Ty::Instance(ctx_class);
        if signature.params.len() != 2
            || signature.rest_param.is_some()
            || !matches!(signature.params[0], Ty::Any)
            || signature.params[1] != ctx_ty
        {
            return None;
        }
        let make = self.method_of(ctx_class, "make")?;
        let finish = self.method_of(ctx_class, "finish")?;
        let expr = |kind: hir::ExprKind, ty: Ty| hir::Expr { kind, ty, span };
        let local = || expr(hir::ExprKind::Local(0), ctx_ty);
        let call = expr(
            hir::ExprKind::Call {
                func: dec_index,
                args: vec![value, local()],
            },
            signature.return_ty,
        );
        let class_rid = |ty: Ty| expr(hir::ExprKind::Number((target.class_idx + 1) as f64), ty);
        // A function returned by a method decorator replaces the method; one
        // returned by a field decorator becomes its initializer transform.
        let returns_function = matches!(signature.return_ty, Ty::Function(_));
        let result_stmt = if returns_function && target.replaces_method {
            let (id, _) = zun_natives::resolve("collections", "classReplaceMethod")?;
            hir::Stmt::Expr(expr(
                hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![
                        class_rid(Ty::Int(IntKind::U32)),
                        expr(hir::ExprKind::String(name.to_string()), Ty::String),
                        call,
                    ],
                },
                Ty::Void,
            ))
        } else if returns_function && target.transforms_field {
            let (id, _) = zun_natives::resolve("collections", "classSetFieldTransform")?;
            hir::Stmt::Expr(expr(
                hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![
                        class_rid(Ty::Int(IntKind::U32)),
                        expr(
                            hir::ExprKind::Number(target.slot as f64),
                            Ty::Int(IntKind::U32),
                        ),
                        call,
                    ],
                },
                Ty::Void,
            ))
        } else {
            hir::Stmt::Expr(call)
        };
        Some(hir::Stmt::Group(vec![
            hir::Stmt::Let {
                slot: 0,
                init: expr(
                    hir::ExprKind::StaticCall {
                        func: make,
                        args: vec![
                            expr(hir::ExprKind::String(kind.to_string()), Ty::String),
                            expr(hir::ExprKind::String(name.to_string()), Ty::String),
                            expr(hir::ExprKind::Boolean(is_static), Ty::Boolean),
                            class_rid(Ty::F64),
                            expr(hir::ExprKind::Number(target.slot as f64), Ty::F64),
                        ],
                    },
                    ctx_ty,
                ),
            },
            result_stmt,
            hir::Stmt::Expr(expr(
                hir::ExprKind::StaticCall {
                    func: finish,
                    args: vec![local()],
                },
                Ty::Void,
            )),
        ]))
    }

    /// Lower a decorator-factory argument. Only literals are supported (the
    /// module initializer has no local scope); anything else makes the whole
    /// decorator inert.
    pub(in crate::check) fn lower_decorator_literal_arg(
        &self,
        arg: &ast::Expr,
        span: Span,
    ) -> Option<hir::Expr> {
        Some(match arg {
            ast::Expr::Number { value, .. } => hir::Expr {
                kind: hir::ExprKind::Number(*value),
                ty: Ty::F64,
                span,
            },
            ast::Expr::String { value, .. } => hir::Expr {
                kind: hir::ExprKind::String(value.clone()),
                ty: Ty::String,
                span,
            },
            ast::Expr::Bool { value, .. } => hir::Expr {
                kind: hir::ExprKind::Boolean(*value),
                ty: Ty::Boolean,
                span,
            },
            _ => return None,
        })
    }
}

/// The `value` argument passed to a runtime decorator, as a `null` of type
/// `Any`.
///
/// Ideally a method decorator would receive the method as a first-class
/// function value (a handle to its MethodDef RID) and a class decorator the
/// constructor. Neither can cross the `Any` boundary here: `Value::Function` is
/// deliberately excluded from `Any` (so closure environments stay visible to
/// precise GC-root handling, see `Value::box_any`), and there is no first-class
/// runtime class/constructor value in this typed-class model. Delivering a live
/// `value` is therefore deferred together with return-value replacement (both
/// need the dispatch-model change). The decorator still receives the full
/// `context`, which is the observable Stage-3 behavior.
pub(in crate::check) fn decorator_null_value(span: Span) -> hir::Expr {
    hir::Expr {
        kind: hir::ExprKind::Null,
        ty: Ty::Any,
        span,
    }
}

/// One named property of a runtime decorator's `context` object. The value is
/// boxed to `Any`, as `DynamicObjectLiteral` stores require.
pub(in crate::check) fn decorator_context_entry(
    name: &str,
    value: hir::ExprKind,
    value_ty: Ty,
    span: Span,
) -> hir::ObjectEntry {
    hir::ObjectEntry::Data {
        key: hir::ObjectKey::Named(name.to_string()),
        value: hir::Expr {
            kind: hir::ExprKind::BoxAny {
                value: Box::new(hir::Expr {
                    kind: value,
                    ty: value_ty,
                    span,
                }),
            },
            ty: Ty::Any,
            span,
        },
    }
}
