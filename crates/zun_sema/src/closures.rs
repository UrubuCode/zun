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
        let captures = self.analyze_closure_captures(outer, function);
        let mut saw_default = false;
        for parameter in &function.params {
            if parameter.is_rest {
                self.error(parameter.name.span, "rest parameters are not lowered yet");
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
            mutated: HashSet::new(),
            mutated_names: HashSet::new(),
            boxed: HashSet::new(),
            captures: captures
                .iter()
                .map(|capture| (capture.name.clone(), capture.clone()))
                .collect(),
            this_class: outer.this_class,
        };
        // The hidden environment is always local zero. User parameters and
        // ordinary locals follow it, matching the indirect-call ABI.
        inner.declare("__closure_environment", Ty::DynamicObject);
        for parameter in &function.params {
            let ty = parameter
                .ty
                .as_ref()
                .map(|annotation| self.resolve_annotation(annotation))
                .unwrap_or(Ty::F64);
            inner.declare(self.text(&parameter.name), ty);
        }
        let body = self.check_block(&mut inner, &function.body);
        let return_ty = if function.return_type.is_some() {
            inner.return_ty
        } else {
            body.iter()
                .find_map(|statement| match statement {
                    hir::Stmt::Return(Some(value)) => Some(value.ty),
                    _ => None,
                })
                .unwrap_or(Ty::Void)
        };
        let function_index = self.closures.len() as u32;
        let defaults = function
            .params
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                parameter.default_value.as_deref().map(|value| {
                    let mut checked = self.check_expr(&mut inner, value);
                    let expected = inner.locals[index + 1];
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
        self.closures.push(hir::ClosureFunction {
            name: format!("<closure@{}>", span.start),
            param_count: function.params.len() as u16,
            rest_param: function
                .params
                .iter()
                .position(|parameter| parameter.is_rest)
                .map(|index| index as u16),
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
    ) -> Vec<hir::Capture> {
        let parameters: HashSet<String> = function
            .params
            .iter()
            .map(|param| self.text(&param.name).to_string())
            .collect();
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
            ast::Expr::ComputedCall { callee, args, .. } => {
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
