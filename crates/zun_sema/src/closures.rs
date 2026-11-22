//! Closure capture analysis extracted from the semantic checker.

use super::*;

impl Checker<'_> {
    pub(crate) fn check_closure_expr(
        &mut self,
        outer: &mut FunctionCx,
        function: &ast::FunctionExpr,
        span: Span,
    ) -> hir::Expr {
        // Contextual typing (TS-style): take any parameter-type hint parked by
        // the argument-checking site. It applies only to THIS closure — taken
        // once so a nested/sibling closure does not inherit it.
        let param_hint = self.closure_param_hint.take();
        let self_name = self.closure_self_name.take();
        let captures = self.analyze_closure_captures(outer, function, self_name.as_deref());
        let takes_this = std::mem::take(&mut self.closure_takes_this);
        let is_method = std::mem::take(&mut self.closure_is_method);
        let mut saw_default = false;
        for parameter in &function.params {
            if parameter.is_rest {
                continue;
            }
            if parameter.default_value.is_some() {
                saw_default = true;
            } else if saw_default {
                self.error(
                    parameter.name.span,
                    "a required parameter cannot follow a default parameter",
                );
            }
        }
        for capture in &captures {
            // Only captures read from the enclosing frame's own locals box a
            // slot there; an inherited capture's `outer_slot` indexes a frame
            // further out and must not touch this frame's slots.
            if capture.mode == hir::CaptureMode::ByCell
                && matches!(capture.source, hir::CaptureSource::Local(_))
            {
                outer.boxed.insert(capture.outer_slot);
            }
        }
        let mut inner = FunctionCx {
            locals: Vec::new(),
            scopes: vec![Scope::new()],
            return_ty: if function.is_async || function.is_generator {
                Ty::Any
            } else {
                function
                    .return_type
                    .as_ref()
                    .map(|annotation| self.resolve_annotation(annotation))
                    .unwrap_or(Ty::Any)
            },
            is_generator: function.is_async || function.is_generator,
            loop_depth: 0,
            loop_labels: Vec::new(),
            switch_depth: 0,
            block_labels: Vec::new(),
            mutated: HashSet::new(),
            mutated_names: HashSet::new(),
            tdz_names: HashSet::new(),
            pending_lexical: Vec::new(),
            param_slot_count: 0,
            boxed: HashSet::new(),
            captures: captures
                .iter()
                .map(|capture| (capture.name.clone(), capture.clone()))
                .collect(),
            this_class: function
                .this_class
                .as_ref()
                .and_then(|name| self.classes_by_name.get(self.text(name)).copied())
                .or(outer.this_class),
            // An arrow inherits `new.target` lexically, exactly as it inherits
            // `this`, so an arrow written inside a constructor must answer the
            // constructing class. A `function` expression has its own, which is
            // `undefined` unless it is itself `new`ed, so only arrows inherit.
            is_constructor: function.is_arrow && outer.is_constructor,
            is_module_scope: false,
            finally_depth: 0,
            number_arrays: HashSet::new(),
        };
        // The hidden environment is always local zero. User parameters and
        // ordinary locals follow it, matching the indirect-call ABI.
        inner.declare("__closure_environment", Ty::DynamicObject);
        if takes_this {
            inner.declare("this", Ty::DynamicObject);
        }
        let mut declared_params: Vec<(String, hir::LocalSlot)> = Vec::new();
        for (index, parameter) in function.params.iter().enumerate() {
            // Explicit annotation wins; otherwise a contextual hint types the
            // parameter (e.g. a Promise executor's `res`/`rej` become
            // `Function`); otherwise an untyped parameter is dynamic (`any`),
            // matching JS (member/arithmetic access resolves at run time).
            let ty = parameter
                .ty
                .as_ref()
                .map(|annotation| self.resolve_annotation(annotation))
                .or_else(|| {
                    param_hint
                        .as_ref()
                        .and_then(|hints| hints.get(index).copied())
                })
                .unwrap_or(Ty::Any);
            let slot = inner.declare(self.text(&parameter.name), ty);
            declared_params.push((self.text(&parameter.name).to_string(), slot));
        }
        // A named closure (`const f = () => … f() …`, a nested `function f`)
        // can call itself: bind its own name to a function value made from
        // its own method and environment.
        // A parameter default is inside the function's scope, so
        // `function d(x = typeof d) {}` needs the self-binding too. Only the
        // body text was consulted, and the outer `let d` the desugaring
        // creates is captured by value *before* it is assigned, so the default
        // read `undefined`.
        let mentions_self = |name: &str| {
            let in_body = self
                .source
                .get(function.body.span.start as usize..function.body.span.end as usize)
                .map(|body| body.contains(name))
                .unwrap_or(true);
            let in_defaults = function.params.iter().any(|parameter| {
                parameter.default_value.as_deref().is_some_and(|value| {
                    let span = value.span();
                    self.source
                        .get(span.start as usize..span.end as usize)
                        .is_some_and(|text| text.contains(name))
                })
            });
            in_body || in_defaults
        };
        let mut self_slot: Option<u16> = None;
        let self_binding = match self_name.as_deref() {
            Some(name)
                if function.params.iter().all(|p| self.text(&p.name) != name)
                    && mentions_self(name) =>
            {
                let slot = inner.declare(name, Ty::Function(hir::DYNAMIC_FUNCTION));
                self_slot = Some(slot);
                Some(hir::Stmt::Let {
                    slot,
                    init: hir::Expr {
                        kind: hir::ExprKind::SelfClosure,
                        ty: Ty::Function(hir::DYNAMIC_FUNCTION),
                        span,
                    },
                })
            }
            _ => None,
        };
        self.collect_mutated_names(&function.body.statements, &mut inner.mutated_names);
        for (pname, slot) in declared_params {
            if !function
                .params
                .iter()
                .any(|p| p.is_rest || p.default_value.is_some())
                && inner.mutated_names.contains(&pname)
                && inner.mutated_names.contains(&format!("{pname}"))
                && !matches!(inner.locals[slot as usize], Ty::Function(_))
            {
                inner.boxed.insert(slot);
            }
        }
        let mut body = self.check_block(&mut inner, &function.body);
        if let Some(binding) = self_binding {
            body.insert(0, binding);
        }
        self.rewrite_boxed_lets(&mut body, &inner.boxed);
        boxed_locals::box_parameters(
            &mut body,
            &inner.boxed,
            &inner.locals,
            function.params.len() + usize::from(takes_this),
        );
        // An expression-bodied arrow whose body is a void call (`() => log(x)`)
        // returns nothing: evaluate it as a statement, then return bare.
        let body: Vec<hir::Stmt> = if function.return_type.is_none() {
            body.into_iter()
                .flat_map(|statement| match statement {
                    hir::Stmt::Return(Some(value)) if value.ty == Ty::Void => {
                        vec![hir::Stmt::Expr(value), hir::Stmt::Return(None)]
                    }
                    other => vec![other],
                })
                .collect()
        } else {
            body
        };
        let return_ty = if function.is_async || function.is_generator {
            Ty::Any
        } else if function.return_type.is_some() {
            inner.return_ty
        } else {
            first_return_ty(&body).unwrap_or(Ty::Void)
        };
        // The implicit `arguments` object is built ahead of the default-value
        // prologue, so a default initializer sees it.
        if let Some(slot) = self.injected_arguments_slot(&body) {
            inner.scopes[0].insert("arguments".to_string(), slot);
        }
        let function_index = self.closures.len() as u32;
        let defaults = function
            .params
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                parameter.default_value.as_deref().map(|value| {
                    let mut checked = self.check_expr(&mut inner, value);
                    let expected = inner.locals[index + 1 + usize::from(takes_this)];
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
            .collect::<Vec<Option<hir::Expr>>>();
        let mut defaults = defaults;
        // The `Let` that binds the self-name is inserted at the top of the
        // *body*, which runs after the default prologue. A default that reads
        // the name therefore needs the slot filled first, so the assignment
        // rides the earliest default expression.
        if let Some(slot) = self_slot {
            if let Some(first) = defaults.iter_mut().flatten().next() {
                let span = first.span;
                let ty = first.ty;
                let value = std::mem::replace(
                    first,
                    hir::Expr {
                        kind: hir::ExprKind::Null,
                        ty: Ty::Void,
                        span,
                    },
                );
                *first = hir::Expr {
                    kind: hir::ExprKind::Sequence {
                        effect: Box::new(hir::Expr {
                            kind: hir::ExprKind::Assign {
                                slot,
                                value: Box::new(hir::Expr {
                                    kind: hir::ExprKind::SelfClosure,
                                    ty: Ty::Function(hir::DYNAMIC_FUNCTION),
                                    span,
                                }),
                            },
                            ty: Ty::Function(hir::DYNAMIC_FUNCTION),
                            span,
                        }),
                        value: Box::new(value),
                    },
                    ty,
                    span,
                };
            }
        }
        if takes_this {
            defaults.insert(0, None);
        }
        self.closures.push(hir::ClosureFunction {
            name: {
                // An arrow function has no `prototype` and is not
                // constructible; the runtime reads that off the `@arrow`
                // marker (it never combines with `@this`).
                let arrow = if function.is_arrow { "@arrow" } else { "" };
                // A method shorthand owns no `prototype` either; its marker
                // sits before `@this` so the `@this>` suffix stays intact.
                let method = if is_method { "@method" } else { "" };
                let rest_suffix = if function.params.iter().any(|p| p.is_rest) {
                    format!("{arrow}@rest")
                } else {
                    arrow.to_string()
                };
                // The same source closure can be checked more than once (e.g. each
                // arm of a runtime-key dispatch chain re-checks its arguments);
                // keep every checked copy uniquely named.
                let copies = self
                    .closures
                    .iter()
                    .filter(|c| {
                        c.name.starts_with(&format!("<closure@{}", span.start))
                            && c.name[("<closure@".len())..]
                                .trim_start_matches(|ch: char| ch.is_ascii_digit())
                                .starts_with(['@', '>', '#', ':'])
                    })
                    .count();
                let unique = if copies == 0 {
                    String::new()
                } else {
                    format!("#{copies}")
                };
                // A named function expression's own name is its `name`. It rides
                // the synthetic name as a `:<name>` segment placed before every
                // marker, so the `@this>` / `@rest>` suffix tests all over the
                // runtime keep matching.
                let written = match function.written_name {
                    Some(name) => format!(":{}", self.text(&name)),
                    None => String::new(),
                };
                // `@async` / `@generator` make a function expression's brand
                // readable (`[object GeneratorFunction]`) and tell the runtime
                // it owns no `prototype`. They sit **before** `{method}` on
                // purpose: a dozen runtime tests are end-anchored on
                // `@this>` / `@rest>`, so a trailing marker would break them.
                let kind = if function.is_generator {
                    "@generator"
                } else if function.is_async {
                    "@async"
                } else {
                    ""
                };
                if takes_this {
                    format!(
                        "<closure@{}{unique}{written}{kind}{method}@this{rest_suffix}>",
                        span.start
                    )
                } else {
                    format!(
                        "<closure@{}{unique}{written}{kind}{method}{rest_suffix}>",
                        span.start
                    )
                }
            },
            param_count: function.params.len() as u16 + u16::from(takes_this),
            rest_param: function
                .params
                .iter()
                .position(|parameter| parameter.is_rest)
                .map(|index| index as u16 + u16::from(takes_this)),
            defaults,
            locals: inner.locals,
            return_ty,
            is_async: function.is_async,
            is_generator: function.is_generator,
            captures: captures.clone(),
            body,
        });
        let capture_values = captures
            .into_iter()
            .map(|capture| hir::ClosureCapture {
                value: Box::new(hir::Expr {
                    kind: match capture.source {
                        hir::CaptureSource::Local(slot)
                            if capture.mode == hir::CaptureMode::ByCell =>
                        {
                            hir::ExprKind::CellRef { slot }
                        }
                        hir::CaptureSource::Local(slot) => hir::ExprKind::Local(slot),
                        hir::CaptureSource::Environment {
                            environment_slot,
                            mode: hir::CaptureMode::ByCell,
                        } => hir::ExprKind::CaptureRef { environment_slot },
                        hir::CaptureSource::Environment {
                            environment_slot,
                            mode: hir::CaptureMode::ByValue,
                        } => hir::ExprKind::CaptureLoad {
                            environment_slot,
                            ty: capture.ty,
                            mode: hir::CaptureMode::ByValue,
                        },
                    },
                    ty: capture.ty,
                    span,
                }),
                capture,
            })
            .collect();
        hir::Expr {
            kind: hir::ExprKind::Closure {
                function: function_index,
                captures: capture_values,
            },
            ty: Ty::Function(hir::DYNAMIC_FUNCTION),
            span,
        }
    }

    fn analyze_closure_captures(
        &self,
        cx: &FunctionCx,
        function: &ast::FunctionExpr,
        self_name: Option<&str>,
    ) -> Vec<hir::Capture> {
        let mut parameters: HashSet<String> = function
            .params
            .iter()
            .map(|param| self.text(&param.name).to_string())
            .collect();
        if self.closure_takes_this {
            parameters.insert("this".to_string());
        }
        if let Some(name) = self_name {
            parameters.insert(name.to_string());
        }
        let mut uses = CaptureUse::default();
        // A default-value initializer runs in the closure's own prologue, so the
        // enclosing locals it reads are captures just like the body's.
        for default in function.params.iter().filter_map(|param| param.default_value.as_deref()) {
            self.collect_capture_expr(cx, &parameters, default, &mut uses);
        }
        for statement in &function.body.statements {
            self.collect_capture_stmt(cx, &parameters, statement, &mut uses);
        }
        let local_capture_count = uses.slots.len();
        uses.slots
            .into_iter()
            .enumerate()
            .map(|(environment_slot, outer_slot)| {
                let name = cx
                    .scopes
                    .iter()
                    .rev()
                    .find_map(|scope| {
                        scope
                            .iter()
                            .find_map(|(name, &slot)| (slot == outer_slot).then(|| name.clone()))
                    })
                    .unwrap_or_else(|| format!("capture{outer_slot}"));
                hir::Capture {
                    name: name.clone(),
                    outer_slot,
                    environment_slot: environment_slot as u16,
                    ty: cx.locals[outer_slot as usize],
                    mode: if cx.mutated.contains(&outer_slot)
                        || (cx.mutated_names.contains(&name)
                            && (cx.boxed.contains(&outer_slot)
                                || (outer_slot as usize) >= cx.param_slot_count))
                        || uses.mutated.contains(&outer_slot)
                    {
                        hir::CaptureMode::ByCell
                    } else {
                        hir::CaptureMode::ByValue
                    },
                    source: hir::CaptureSource::Local(outer_slot),
                }
            })
            .chain(
                uses.inherited
                    .into_iter()
                    .enumerate()
                    .map(move |(offset, mut capture)| {
                        // The capture was found in the enclosing closure's own
                        // environment; read it from that environment slot.
                        let source_slot = capture.environment_slot;
                        capture.environment_slot = (local_capture_count + offset) as u16;
                        capture.source = hir::CaptureSource::Environment {
                            environment_slot: source_slot,
                            mode: capture.mode,
                        };
                        capture
                    }),
            )
            .collect()
    }

    fn collect_capture_stmt(
        &self,
        cx: &FunctionCx,
        parameters: &HashSet<String>,
        statement: &ast::Stmt,
        uses: &mut CaptureUse,
    ) {
        match statement {
            ast::Stmt::Let { init, .. }
            | ast::Stmt::Destructure { init, .. }
            | ast::Stmt::Expr(init) => self.collect_capture_expr(cx, parameters, init, uses),
            ast::Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                self.collect_capture_expr(cx, parameters, cond, uses);
                for statement in &then_block.statements {
                    self.collect_capture_stmt(cx, parameters, statement, uses);
                }
                if let Some(block) = else_block {
                    for statement in &block.statements {
                        self.collect_capture_stmt(cx, parameters, statement, uses);
                    }
                }
            }
            ast::Stmt::While { cond, body, .. } => {
                self.collect_capture_expr(cx, parameters, cond, uses);
                for statement in &body.statements {
                    self.collect_capture_stmt(cx, parameters, statement, uses);
                }
            }
            ast::Stmt::DoWhile { body, cond, .. } => {
                for statement in &body.statements {
                    self.collect_capture_stmt(cx, parameters, statement, uses);
                }
                self.collect_capture_expr(cx, parameters, cond, uses);
            }
            ast::Stmt::For {
                init,
                extra_init,
                cond,
                update,
                body,
                ..
            } => {
                if let Some(init) = init {
                    self.collect_capture_stmt(cx, parameters, init, uses);
                }
                for extra in extra_init {
                    self.collect_capture_stmt(cx, parameters, extra, uses);
                }
                if let Some(cond) = cond {
                    self.collect_capture_expr(cx, parameters, cond, uses);
                }
                if let Some(update) = update {
                    self.collect_capture_expr(cx, parameters, update, uses);
                }
                for statement in &body.statements {
                    self.collect_capture_stmt(cx, parameters, statement, uses);
                }
            }
            ast::Stmt::ForOf { iterable, body, .. } => {
                self.collect_capture_expr(cx, parameters, iterable, uses);
                for statement in &body.statements {
                    self.collect_capture_stmt(cx, parameters, statement, uses);
                }
            }
            ast::Stmt::ForIn { object, body, .. } => {
                self.collect_capture_expr(cx, parameters, object, uses);
                for statement in &body.statements {
                    self.collect_capture_stmt(cx, parameters, statement, uses);
                }
            }
            ast::Stmt::Switch {
                discriminant,
                cases,
                ..
            } => {
                self.collect_capture_expr(cx, parameters, discriminant, uses);
                for case in cases {
                    if let Some(test) = &case.test {
                        self.collect_capture_expr(cx, parameters, test, uses);
                    }
                    for statement in &case.body.statements {
                        self.collect_capture_stmt(cx, parameters, statement, uses);
                    }
                }
            }
            ast::Stmt::Return { value, .. } => {
                if let Some(value) = value {
                    self.collect_capture_expr(cx, parameters, value, uses);
                }
            }
            ast::Stmt::Throw { value, .. } => {
                self.collect_capture_expr(cx, parameters, value, uses)
            }
            ast::Stmt::Try {
                body,
                catches,
                finally,
                ..
            } => {
                for statement in &body.statements {
                    self.collect_capture_stmt(cx, parameters, statement, uses);
                }
                for catch in catches {
                    for statement in &catch.body.statements {
                        self.collect_capture_stmt(cx, parameters, statement, uses);
                    }
                }
                if let Some(block) = finally {
                    for statement in &block.statements {
                        self.collect_capture_stmt(cx, parameters, statement, uses);
                    }
                }
            }
            ast::Stmt::Labeled { statement, .. } => {
                self.collect_capture_stmt(cx, parameters, statement, uses);
            }
            ast::Stmt::Break { .. } | ast::Stmt::Continue { .. } => {}
        }
    }

    fn collect_capture_expr(
        &self,
        cx: &FunctionCx,
        parameters: &HashSet<String>,
        expr: &ast::Expr,
        uses: &mut CaptureUse,
    ) {
        let mut add_name = |name: &ast::Name, mutated: bool| {
            let text = self.text(name);
            // `new.target` is lowered through `this` (the runtime class of the
            // receiver), so inside an arrow it is a *use* of the enclosing
            // `this` even though its text never spells it. Without this the
            // arrow captured nothing, `cx.lookup("this")` answered `None` in
            // `check_new_target`, and `new.target` fell through to `undefined`
            // while the same expression directly in the constructor was right.
            // `new.target` and `super` are both lowered through `this` -- the
            // receiver's runtime class, and its home object -- so each is a
            // *use* of the enclosing `this` even though neither text spells
            // it. A closure whose only free name was one of them captured
            // nothing, and the reader then found no `this`.
            let text = if text == "new.target" || text == "super" {
                "this"
            } else {
                text
            };
            if parameters.contains(text) {
                return;
            }
            if let Some(slot) = cx.lookup(text) {
                if uses.slot_set.insert(slot) {
                    uses.slots.push(slot);
                }
                if mutated {
                    uses.mutated.insert(slot);
                }
            } else if let Some(capture) = cx.captures.get(text) {
                if uses.inherited_names.insert(text.to_owned()) {
                    uses.inherited.push(capture.clone());
                }
            }
        };
        match expr {
            ast::Expr::Ident(name) => add_name(name, false),
            ast::Expr::Assign { target, value, .. } => {
                add_name(target, true);
                self.collect_capture_expr(cx, parameters, value, uses);
            }
            ast::Expr::Binary { lhs, rhs, .. } => {
                self.collect_capture_expr(cx, parameters, lhs, uses);
                self.collect_capture_expr(cx, parameters, rhs, uses);
            }
            ast::Expr::Conditional {
                condition,
                when_true,
                when_false,
                ..
            } => {
                self.collect_capture_expr(cx, parameters, condition, uses);
                self.collect_capture_expr(cx, parameters, when_true, uses);
                self.collect_capture_expr(cx, parameters, when_false, uses);
            }
            ast::Expr::Unary { operand, .. } | ast::Expr::Cast { expr: operand, .. } => {
                self.collect_capture_expr(cx, parameters, operand, uses)
            }
            ast::Expr::Await { operand, .. } => {
                self.collect_capture_expr(cx, parameters, operand, uses)
            }
            ast::Expr::ImportCall { specifier, .. } => {
                self.collect_capture_expr(cx, parameters, specifier, uses)
            }
            ast::Expr::Yield { value, .. } => {
                self.collect_capture_expr(cx, parameters, value, uses)
            }
            ast::Expr::Call { callee, args, .. } => {
                // A bare-name callee may be a captured callable value.
                add_name(callee, false);
                for arg in args {
                    self.collect_capture_expr(cx, parameters, arg, uses);
                }
            }
            ast::Expr::ComputedCall { callee, args, .. }
            | ast::Expr::NewDynamic { callee, args, .. } => {
                self.collect_capture_expr(cx, parameters, callee, uses);
                for arg in args {
                    self.collect_capture_expr(cx, parameters, arg, uses);
                }
            }
            ast::Expr::MethodCall { receiver, args, .. } => {
                self.collect_capture_expr(cx, parameters, receiver, uses);
                for arg in args {
                    self.collect_capture_expr(cx, parameters, arg, uses);
                }
            }
            ast::Expr::StaticMethodCall { args, .. } => {
                for arg in args {
                    self.collect_capture_expr(cx, parameters, arg, uses);
                }
            }
            ast::Expr::StaticField { .. } => {}
            ast::Expr::OptionalMethodCall { object, args, .. } => {
                self.collect_capture_expr(cx, parameters, object, uses);
                for arg in args {
                    self.collect_capture_expr(cx, parameters, arg, uses);
                }
            }
            ast::Expr::New { class, args, .. } => {
                // `new C(...)` with `C` an enclosing local that holds a
                // constructor value (a run-time class, a proxy) reads that
                // local; a declared class or a realm global is resolved by
                // name and captures nothing.
                let callee = self.text(class);
                if !self.classes_by_name.contains_key(callee)
                    && !self.realm_globals.contains_key(callee)
                {
                    add_name(class, false);
                }
                for arg in args {
                    self.collect_capture_expr(cx, parameters, arg, uses);
                }
            }
            ast::Expr::Field { object, .. } => {
                self.collect_capture_expr(cx, parameters, object, uses)
            }
            ast::Expr::PrivateField { object, .. } => {
                self.collect_capture_expr(cx, parameters, object, uses)
            }
            ast::Expr::OptionalField { object, .. } => {
                self.collect_capture_expr(cx, parameters, object, uses)
            }
            ast::Expr::OptionalIndex { object, index, .. } => {
                self.collect_capture_expr(cx, parameters, object, uses);
                self.collect_capture_expr(cx, parameters, index, uses);
            }
            ast::Expr::ChainRoot { .. } => {}
            ast::Expr::TempBind { value, body, .. } => {
                self.collect_capture_expr(cx, parameters, value, uses);
                self.collect_capture_expr(cx, parameters, body, uses);
            }
            ast::Expr::OptionalChain { object, body, .. } => {
                self.collect_capture_expr(cx, parameters, object, uses);
                self.collect_capture_expr(cx, parameters, body, uses);
            }
            ast::Expr::OptionalCall { callee, args, .. } => {
                self.collect_capture_expr(cx, parameters, callee, uses);
                for arg in args {
                    self.collect_capture_expr(cx, parameters, arg, uses);
                }
            }
            ast::Expr::Spread { expr, .. } => self.collect_capture_expr(cx, parameters, expr, uses),
            ast::Expr::FieldAssign { object, value, .. } => {
                self.collect_capture_expr(cx, parameters, object, uses);
                self.collect_capture_expr(cx, parameters, value, uses);
            }
            ast::Expr::PrivateFieldAssign { object, value, .. } => {
                self.collect_capture_expr(cx, parameters, object, uses);
                self.collect_capture_expr(cx, parameters, value, uses);
            }
            ast::Expr::Index { object, index, .. } => {
                self.collect_capture_expr(cx, parameters, object, uses);
                self.collect_capture_expr(cx, parameters, index, uses);
            }
            ast::Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                self.collect_capture_expr(cx, parameters, object, uses);
                self.collect_capture_expr(cx, parameters, index, uses);
                self.collect_capture_expr(cx, parameters, value, uses);
            }
            ast::Expr::Function(function) => {
                let nested_parameters: HashSet<String> = function
                    .params
                    .iter()
                    .map(|parameter| self.text(&parameter.name).to_string())
                    .collect();
                for statement in &function.body.statements {
                    self.collect_capture_stmt(cx, &nested_parameters, statement, uses);
                }
            }
            ast::Expr::RegexLiteral { .. } => {}
            ast::Expr::Template { parts, .. } => {
                for part in parts {
                    if let ast::TemplatePart::Expr(expression) = part {
                        self.collect_capture_expr(cx, parameters, expression, uses);
                    }
                }
            }
            ast::Expr::TaggedTemplate { tag, exprs, .. } => {
                self.collect_capture_expr(cx, parameters, tag, uses);
                for expression in exprs {
                    self.collect_capture_expr(cx, parameters, expression, uses);
                }
            }
            ast::Expr::ArrayLiteral { elements, .. } => {
                for element in elements {
                    match element {
                        ast::ArrayElement::Expr(expr) | ast::ArrayElement::Spread(expr) => {
                            self.collect_capture_expr(cx, parameters, expr, uses);
                        }
                        ast::ArrayElement::Hole => {}
                    }
                }
            }
            ast::Expr::ObjectLiteral { properties, .. } => {
                for member in properties {
                    match member {
                        ast::ObjectMember::Property(property) => {
                            if let ast::PropertyKey::Computed(key) = &property.key {
                                self.collect_capture_expr(cx, parameters, key, uses);
                            }
                            self.collect_capture_expr(cx, parameters, &property.value, uses);
                        }
                        ast::ObjectMember::Spread { expr, .. } => {
                            self.collect_capture_expr(cx, parameters, expr, uses);
                        }
                    }
                }
            }
            ast::Expr::Number { .. }
            | ast::Expr::BigInt { .. }
            | ast::Expr::String { .. }
            | ast::Expr::Bool { .. }
            | ast::Expr::Null { .. } => {}
        }
    }
}

/// The type of the first `return <value>` anywhere in a closure body
/// (descending into control flow, not into nested closures).
fn first_return_ty(statements: &[hir::Stmt]) -> Option<Ty> {
    statements.iter().find_map(|statement| match statement {
        hir::Stmt::Return(Some(value)) => Some(value.ty),
        hir::Stmt::Group(body) => first_return_ty(body),
        hir::Stmt::If {
            then_body,
            else_body,
            ..
        } => first_return_ty(then_body).or_else(|| first_return_ty(else_body)),
        hir::Stmt::While { body, .. }
        | hir::Stmt::DoWhile { body, .. }
        | hir::Stmt::For { body, .. }
        | hir::Stmt::ForOf { body, .. } => first_return_ty(body),
        hir::Stmt::Labeled { body, .. } => first_return_ty(std::slice::from_ref(&**body)),
        hir::Stmt::Try {
            body,
            catches,
            finally_body,
        } => first_return_ty(body)
            .or_else(|| catches.iter().find_map(|c| first_return_ty(&c.body)))
            .or_else(|| finally_body.as_ref().and_then(|b| first_return_ty(b))),
        _ => None,
    })
}

impl Checker<'_> {
    /// Whether a closure inside `init` (not itself a function expression) refers
    /// to `name`, the variable being declared (`const req = get(url, () => req.id)`).
    /// Walks the AST with the closure-capture collector, so nested parameters
    /// shadow the name and property names, strings and comments never match.
    /// Over-approximates (an inner declaration of the same name still counts);
    /// a false positive only costs static typing of the variable.
    pub(crate) fn initializer_references_name(
        &self,
        cx: &mut FunctionCx,
        name: &str,
        init: &ast::Expr,
    ) -> bool {
        if cx.is_module_scope || matches!(init, ast::Expr::Function(_)) {
            return false;
        }
        // Probe with a throwaway binding so the collector resolves the name.
        let locals_len = cx.locals.len();
        cx.scopes.push(Scope::new());
        cx.declare(name, Ty::Any);
        let mut uses = CaptureUse::default();
        self.collect_capture_expr(cx, &HashSet::new(), init, &mut uses);
        cx.scopes.pop();
        cx.locals.truncate(locals_len);
        uses.slot_set.contains(&(locals_len as hir::LocalSlot))
    }
}

impl Checker<'_> {
    /// Whether a function expression's body reads `this`, decided on the AST
    /// with the closure-capture collector. The textual scan in
    /// `check_function_expr` only sees a `this` the source spells out; a
    /// compiler-made member function (the constructor and field initializers
    /// of a run-time class, see `zun_syntax`'s `runtime_class`) reads the
    /// receiver through a synthesized reference its source text does not show.
    pub(crate) fn closure_reads_this(
        &self,
        cx: &mut FunctionCx,
        function: &ast::FunctionExpr,
    ) -> bool {
        // Probe with a throwaway binding so the collector resolves the name.
        let locals_len = cx.locals.len();
        cx.scopes.push(Scope::new());
        cx.declare("this", Ty::Any);
        let mut uses = CaptureUse::default();
        for statement in &function.body.statements {
            self.collect_capture_stmt(cx, &HashSet::new(), statement, &mut uses);
        }
        cx.scopes.pop();
        cx.locals.truncate(locals_len);
        uses.slot_set.contains(&(locals_len as hir::LocalSlot))
    }
}
