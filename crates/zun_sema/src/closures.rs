//! Closure capture analysis extracted from the semantic checker.

use super::*;

impl Checker<'_> {
    pub(crate) fn check_closure_expr(
        &mut self,
        outer: &mut FunctionCx,
        function: &ast::FunctionExpr,
        span: Span,
    ) -> hir::Expr {
        if function.is_async {
            self.error(
                span,
                "async closures are parsed but source-level async lowering is not implemented yet",
            );
        }
        if function.is_generator {
            self.error(span, "generator closures are not implemented yet");
        }
        // Contextual typing (TS-style): take any parameter-type hint parked by
        // the argument-checking site. It applies only to THIS closure — taken
        // once so a nested/sibling closure does not inherit it.
        let param_hint = self.closure_param_hint.take();
        let self_name = self.closure_self_name.take();
        let captures = self.analyze_closure_captures(outer, function, self_name.as_deref());
        let takes_this = std::mem::take(&mut self.closure_takes_this);
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
            if capture.mode == hir::CaptureMode::ByCell {
                outer.boxed.insert(capture.outer_slot);
            }
        }
        let mut inner = FunctionCx {
            locals: Vec::new(),
            scopes: vec![Scope::new()],
            return_ty: function
                .return_type
                .as_ref()
                .map(|annotation| self.resolve_annotation(annotation))
                .unwrap_or(Ty::Any),
            is_generator: false,
            loop_depth: 0,
            loop_labels: Vec::new(),
            switch_depth: 0,
            block_labels: Vec::new(),
            mutated: HashSet::new(),
            mutated_names: HashSet::new(),
            boxed: HashSet::new(),
            captures: captures
                .iter()
                .map(|capture| (capture.name.clone(), capture.clone()))
                .collect(),
            this_class: outer.this_class,
            is_constructor: false,
            is_module_scope: false,
        };
        // The hidden environment is always local zero. User parameters and
        // ordinary locals follow it, matching the indirect-call ABI.
        inner.declare("__closure_environment", Ty::DynamicObject);
        if takes_this {
            inner.declare("this", Ty::DynamicObject);
        }
        for (index, parameter) in function.params.iter().enumerate() {
            // Explicit annotation wins; otherwise a contextual hint types the
            // parameter (e.g. a Promise executor's `res`/`rej` become
            // `Function`); otherwise an untyped parameter is dynamic (`any`),
            // matching JS (member/arithmetic access resolves at run time).
            let ty = parameter
                .ty
                .as_ref()
                .map(|annotation| self.resolve_annotation(annotation))
                .or_else(|| param_hint.as_ref().and_then(|hints| hints.get(index).copied()))
                .unwrap_or(Ty::Any);
            inner.declare(self.text(&parameter.name), ty);
        }
        // A named closure (`const f = () => … f() …`, a nested `function f`)
        // can call itself: bind its own name to a function value made from
        // its own method and environment.
        let self_binding = match self_name.as_deref() {
            Some(name)
                if function.params.iter().all(|p| self.text(&p.name) != name)
                    && self.source.get(function.body.span.start as usize..function.body.span.end as usize).map(|body| body.contains(name)).unwrap_or(true) =>
            {
                let slot = inner.declare(name, Ty::Function(hir::DYNAMIC_FUNCTION));
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
        let mut body = self.check_block(&mut inner, &function.body);
        if let Some(binding) = self_binding {
            body.insert(0, binding);
        }
        self.rewrite_boxed_lets(&mut body, &inner.boxed);
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
        let return_ty = if function.return_type.is_some() {
            inner.return_ty
        } else {
            first_return_ty(&body).unwrap_or(Ty::Void)
        };
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
        if takes_this {
            defaults.insert(0, None);
        }
        self.closures.push(hir::ClosureFunction {
            name: {
                let rest_suffix = if function.params.iter().any(|p| p.is_rest) { "@rest" } else { "" };
                if takes_this {
                    format!("<closure@{}@this{rest_suffix}>", span.start)
                } else {
                    format!("<closure@{}{rest_suffix}>", span.start)
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
                        || cx.mutated_names.contains(&name)
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
                        let source_slot = match capture.source {
                            hir::CaptureSource::Environment {
                                environment_slot, ..
                            } => environment_slot,
                            hir::CaptureSource::Local(slot) => slot as u16,
                        };
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
            | ast::Stmt::Expr(init) => {
                self.collect_capture_expr(cx, parameters, init, uses)
            }
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
                cond,
                update,
                body,
                ..
            } => {
                if let Some(init) = init {
                    self.collect_capture_stmt(cx, parameters, init, uses);
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
            ast::Stmt::Switch { discriminant, cases, .. } => {
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
            if parameters.contains(text) {
                return;
            }
            if let Some(slot) = cx.lookup(text) {
                if !uses.slots.contains(&slot) {
                    uses.slots.push(slot);
                }
                if mutated {
                    uses.mutated.insert(slot);
                }
            } else if let Some(capture) = cx.captures.get(text) {
                if !uses.inherited.iter().any(|existing| existing.name == text) {
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
            ast::Expr::ComputedCall { callee, args, .. } | ast::Expr::NewDynamic { callee, args, .. } => {
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
                for arg in args { self.collect_capture_expr(cx, parameters, arg, uses); }
            }
            ast::Expr::StaticField { .. } => {}
            ast::Expr::OptionalMethodCall { object, args, .. } => {
                self.collect_capture_expr(cx, parameters, object, uses);
                for arg in args {
                    self.collect_capture_expr(cx, parameters, arg, uses);
                }
            }
            ast::Expr::New { args, .. } => {
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
        hir::Stmt::If { then_body, else_body, .. } => {
            first_return_ty(then_body).or_else(|| first_return_ty(else_body))
        }
        hir::Stmt::While { body, .. }
        | hir::Stmt::DoWhile { body, .. }
        | hir::Stmt::For { body, .. }
        | hir::Stmt::ForOf { body, .. } => first_return_ty(body),
        hir::Stmt::Labeled { body, .. } => first_return_ty(std::slice::from_ref(&**body)),
        hir::Stmt::Try { body, catches, finally_body } => first_return_ty(body)
            .or_else(|| catches.iter().find_map(|c| first_return_ty(&c.body)))
            .or_else(|| finally_body.as_ref().and_then(|b| first_return_ty(b))),
        _ => None,
    })
}
