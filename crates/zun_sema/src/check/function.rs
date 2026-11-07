use crate::check::*;

impl Checker<'_> {
    /// Check a function or method body. `this_class` is `Some` for methods,
    /// which bind `this` as the first local slot.
    pub(in crate::check) fn check_function(
        &mut self,
        index: usize,
        f: &ast::Function,
        this_class: Option<u32>,
    ) -> hir::Function {
        let mut saw_default = false;
        for (index, parameter) in f.params.iter().enumerate() {
            if parameter.is_rest && index + 1 != f.params.len() {
                self.error(parameter.name.span, "rest parameter must be last");
            }
            if parameter.default_value.is_some() {
                saw_default = true;
            } else if saw_default && !parameter.is_rest {
                self.error(
                    parameter.name.span,
                    "a required parameter cannot follow a default parameter",
                );
            }
        }
        let sig = &self.signatures[index];
        let return_ty = sig.return_ty;
        let param_tys = sig.params.clone();
        let name = sig.name.clone();
        if f.is_async && !f.is_generator
            && return_ty != Ty::Promise {
                self.error(
                    f.name.span,
                    "async functions must declare a Promise return type",
                );
            }
            // A body without suspension points follows the .NET fast path:
            // execute synchronously and return the already-created Promise.
            // Bodies containing await remain gated until state-machine IL is
            // available, so the front end never emits an invalid suspension.
        // An async body is checked (and lowered) as a coroutine returning `any`;
        // the Promise typing lives only on the signature that callers see.
        let return_ty = if f.is_async { Ty::Any } else { return_ty };

        let mut cx = FunctionCx {
            locals: Vec::new(),
            scopes: vec![Scope::new()],
            return_ty,
            is_generator: f.is_generator || f.is_async,
            loop_depth: 0,
            loop_labels: Vec::new(),
            switch_depth: 0,
            block_labels: Vec::new(),
            mutated: HashSet::new(),
            mutated_names: HashSet::new(),
            tdz_names: HashSet::new(),
            param_slot_count: 0,
            boxed: HashSet::new(),
            captures: HashMap::new(),
            this_class,
            is_constructor: this_class.is_some() && self.text(&f.name) == "constructor",
            is_module_scope: false,
            finally_depth: 0,
        };
        // For a method, slot 0 is the implicit `this`; the declared params
        // follow it. For a free function, the params occupy every slot.
        let mut param_tys_iter = param_tys.iter();
        self.cur_static_class = self.static_this.get(&(index as u32)).copied();
        if this_class.is_some() || self.cur_static_class.is_some() {
            let this_ty = *param_tys_iter.next().expect("method has a this param");
            cx.declare("this", this_ty);
        }
        let mut declared_params: Vec<(String, hir::LocalSlot)> = Vec::new();
        for (p, &ty) in f.params.iter().zip(param_tys_iter) {
            let pname = self.text(&p.name).to_string();
            let slot = cx.declare(&pname, ty);
            declared_params.push((pname, slot));
        }
        self.collect_mutated_names(&f.body.statements, &mut cx.mutated_names);
        // A closure created in a default expression captures the parameter scope.
        cx.param_slot_count = cx.locals.len();
        let mut default_names: HashSet<String> = HashSet::new();
        for parameter in &f.params {
            if let Some(default_value) = parameter.default_value.as_deref() {
                capture_analysis::collect_mutated_expr(self, default_value, &mut default_names);
            }
        }
        cx.mutated_names.extend(default_names.iter().cloned());
        for (pname, slot) in declared_params {
            // A body-level `var` of the same name is a separate binding (the
            // parameter scope keeps the argument for default-value closures), so
            // body closures never capture the parameter itself.
            let shadowed_by_body = f.params.iter().any(|p| p.default_value.is_some() || p.is_rest)
                && f.body.statements.iter().any(
                    |statement| matches!(statement, ast::Stmt::Let { name, .. } if self.text(name) == pname),
                );
            let needs_cell = if shadowed_by_body {
                default_names.contains(&pname) && default_names.contains(&format!("{pname}"))
            } else {
                cx.mutated_names.contains(&pname) && cx.mutated_names.contains(&format!("{pname}"))
            };
            if needs_cell && !matches!(cx.locals[slot as usize], Ty::Function(_)) {
                cx.boxed.insert(slot);
            }
        }

        let body = if f.is_extern && f.external_module.is_some() {
            let module = f.external_module.as_deref().unwrap_or_default();
            let external = self
                .external_functions
                .get(&name)
                .cloned()
                .unwrap_or_else(|| {
                    self.error(
                        f.name.span,
                        format!("extern function `{name}` is missing a linker signature"),
                    );
                    ExternalFunction {
                        module: module.to_owned(),
                        name: name.clone(),
                        params: param_tys.clone(),
                        return_ty,
                    }
                });
            if external.module != module {
                self.error(
                    f.name.span,
                    format!(
                        "extern function `{name}` declares module `{module}` but linker resolved `{}`",
                        external.module
                    ),
                );
            }
            if external.params != param_tys {
                self.error(
                    f.name.span,
                    format!("extern function `{name}` parameter types do not match the linker signature"),
                );
            }
            if external.return_ty != return_ty {
                self.error(
                    f.name.span,
                    format!(
                        "extern function `{name}` return type does not match the linker signature"
                    ),
                );
            }
            let args = param_tys
                .iter()
                .enumerate()
                .map(|(slot, &ty)| hir::Expr {
                    kind: hir::ExprKind::Local(slot as hir::LocalSlot),
                    ty,
                    span: f.name.span,
                })
                .collect();
            vec![hir::Stmt::Return(Some(hir::Expr {
                kind: hir::ExprKind::ExternalCall {
                    module: external.module,
                    name: external.name,
                    args,
                },
                ty: return_ty,
                span: f.name.span,
            }))]
        } else if f.is_extern {
            let Some((intrinsic_id, intrinsic)) = self.imports.get(&name).copied() else {
                self.error(
                    f.name.span,
                    format!("extern method `{name}` must be bound to an imported intrinsic"),
                );
                let local_count = cx.locals.len();
                return hir::Function {
                    name,
                    param_count: param_tys.len() as u16,
                    rest_param: f
                        .params
                        .iter()
                        .position(|parameter| parameter.is_rest)
                        .map(|index| index as u16 + u16::from(this_class.is_some())),
                    defaults: (0..f.params.len()).map(|_| None).collect(),
                    locals: cx.locals,
                    return_ty,
                    is_async: f.is_async,
                    is_static: f.is_static,
                    is_generator: f.is_generator,
                    async_plan: f.is_async.then(|| hir::AsyncStateMachinePlan {
                        state_count: 1 + count_awaits_in_block(&f.body) as u16,
                        await_count: count_awaits_in_block(&f.body) as u16,
                        hoisted_locals: (0..local_count as u16).collect(),
                    }),
                    generator_plan: f.is_generator.then(|| hir::GeneratorStateMachinePlan {
                        state_count: 1 + count_yields_in_block(&f.body) as u16,
                        yield_count: count_yields_in_block(&f.body) as u16,
                        hoisted_locals: (0..local_count as u16).collect(),
                    }),
                    body: Vec::new(),
                };
            };
            let native_params: Vec<Ty> = intrinsic.params.iter().map(|&t| native_ty(t)).collect();
            let signature_matches = native_params.len() == param_tys.len()
                && native_params
                    .iter()
                    .zip(&param_tys)
                    .all(|(native, actual)| {
                        *native == Ty::Any && matches!(actual, Ty::Instance(_))
                            || *native == *actual
                    });
            if !signature_matches {
                self.error(
                    f.name.span,
                    format!(
                        "extern method `{name}` signature does not match intrinsic `{}`",
                        intrinsic.name
                    ),
                );
            }
            if native_ty(intrinsic.ret) != return_ty {
                self.error(
                    f.name.span,
                    format!(
                        "extern method `{name}` return type does not match intrinsic `{}`",
                        intrinsic.name
                    ),
                );
            }
            let args = (0..param_tys.len())
                .map(|slot| hir::Expr {
                    kind: hir::ExprKind::Local(slot as hir::LocalSlot),
                    ty: param_tys[slot],
                    span: f.name.span,
                })
                .collect();
            vec![hir::Stmt::Return(Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: intrinsic_id,
                    args,
                },
                ty: return_ty,
                span: f.name.span,
            }))]
        } else {
            let mut body = self.check_block(&mut cx, &f.body);
            self.rewrite_boxed_lets(&mut body, &cx.boxed);
            boxed_locals::box_parameters(&mut body, &cx.boxed, &cx.locals, param_tys.len());
            // A derived constructor that touches `this` in a top-level
            // statement before its `super(...)` call throws a ReferenceError.
            if cx.is_constructor
                && this_class.is_some_and(|c| self.classes[c as usize].base.is_some())
            {
                let mut this_before_super = false;
                for statement in &f.body.statements {
                    let ast::Stmt::Expr(expr) = statement else {
                        continue;
                    };
                    let is_super_call = matches!(expr, ast::Expr::Call { callee, .. } if self.text(callee) == "super");
                    if is_super_call {
                        break;
                    }
                    let text = &self.source[expr.span().start as usize..expr.span().end as usize];
                    if text
                        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
                        .any(|word| word == "this")
                    {
                        this_before_super = true;
                        break;
                    }
                }
                if this_before_super {
                    if let Some(&class_idx) = self.classes_by_name.get("ReferenceError") {
                        let message = ast::Expr::String {
                            value: "Must call super constructor in derived class before accessing 'this' or returning from derived constructor".to_string(),
                            span: f.name.span,
                        };
                        let thrown = self.check_error_new(
                            &mut cx,
                            class_idx,
                            "ReferenceError",
                            std::slice::from_ref(&message),
                            f.name.span,
                        );
                        body.insert(0, hir::Stmt::Throw(thrown));
                    }
                }
            }
            body
        };
        let defaults = f
            .params
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                parameter.default_value.as_deref().map(|value| {
                    cx.tdz_names = f.params[index..]
                        .iter()
                        .map(|p| self.text(&p.name).to_string())
                        .collect();
                    let mut checked = self.check_expr(&mut cx, value);
                    cx.tdz_names.clear();
                    let expected = param_tys[index + usize::from(this_class.is_some())];
                    coerce_numeric_literal(&mut checked, expected);
                    if !checked.ty.is_error() && !self.is_assignable(checked.ty, expected) {
                        self.error(
                            checked.span,
                            format!(
                                "default expression type {:?} does not match {expected:?}",
                                checked.ty
                            ),
                        );
                    }
                    checked
                })
            })
            .collect();
        let local_count = cx.locals.len();
        hir::Function {
            name,
            param_count: param_tys.len() as u16,
            rest_param: f
                .params
                .iter()
                .position(|parameter| parameter.is_rest)
                .map(|index| index as u16 + u16::from(this_class.is_some())),
            defaults,
            locals: cx.locals,
            return_ty,
            is_async: f.is_async,
            is_static: f.is_static,
            is_generator: f.is_generator,
            async_plan: f.is_async.then(|| hir::AsyncStateMachinePlan {
                state_count: 1 + count_awaits_in_block(&f.body) as u16,
                await_count: count_awaits_in_block(&f.body) as u16,
                hoisted_locals: (0..local_count as u16).collect(),
            }),
            generator_plan: f.is_generator.then(|| hir::GeneratorStateMachinePlan {
                state_count: 1 + count_yields_in_block(&f.body) as u16,
                yield_count: count_yields_in_block(&f.body) as u16,
                hoisted_locals: (0..local_count as u16).collect(),
            }),
            body,
        }
    }

    /// A collision-free hidden local name (contains a space, which no source
    /// identifier can), unique per slot index within the function.
    pub(in crate::check) fn fresh_temp_name(&self, cx: &FunctionCx) -> String {
        format!(" destr#{}", cx.locals.len())
    }

    /// Wrap a read with a destructuring default: `coalesceUndefined(read, d)`
    /// returns `d` when `read` is `undefined` (eager default evaluation).
    pub(in crate::check) fn apply_default(
        &mut self,
        cx: &mut FunctionCx,
        read: hir::Expr,
        default: Option<&ast::Expr>,
        out: &mut Vec<hir::Stmt>,
    ) -> hir::Expr {
        let Some(default) = default else { return read };
        let span = read.span;
        let Some((id, _)) = zun_natives::resolve("array", "anyIsUndefined") else {
            self.error(span, "missing `anyIsUndefined` intrinsic");
            return read;
        };
        // The read happens once (into a temp); the default expression is
        // evaluated lazily, only when the read is `undefined`.
        let temp = cx.declare(&self.fresh_temp_name(cx), Ty::Any);
        out.push(hir::Stmt::Let {
            slot: temp,
            init: Self::box_to_any(read),
        });
        let default = self.check_expr(cx, default);
        let default = Self::box_to_any(default);
        let temp_read = || hir::Expr {
            kind: hir::ExprKind::Local(temp),
            ty: Ty::Any,
            span,
        };
        hir::Expr {
            kind: hir::ExprKind::Conditional {
                condition: Box::new(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![temp_read()],
                    },
                    ty: Ty::Boolean,
                    span,
                }),
                when_true: Box::new(default),
                when_false: Box::new(temp_read()),
            },
            ty: Ty::Any,
            span,
        }
    }
}
