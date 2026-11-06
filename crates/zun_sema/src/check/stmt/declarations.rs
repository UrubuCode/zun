use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_destructure_stmt(&mut self, cx: &mut FunctionCx, stmt: &ast::Stmt) -> hir::Stmt {
        let ast::Stmt::Destructure { pattern, init, .. } = stmt else { unreachable!() };
        let value = self.check_expr(cx, init);
        let mut out = Vec::new();
        self.bind_pattern(cx, pattern, value, &mut out);
        hir::Stmt::Group(out)
    }

    pub(in crate::check) fn check_let_stmt(&mut self, cx: &mut FunctionCx, stmt: &ast::Stmt) -> hir::Stmt {
        let ast::Stmt::Let { name, ty, init, .. } = stmt else { unreachable!() };
        if matches!(init, ast::Expr::Function(_)) && !cx.mutated_names.contains(self.text(name)) {
            self.closure_self_name = Some(self.text(name).to_string());
        }
        // The adapter's injected `arguments = [p1, ..., ...rest]` (name and
        // literal share one span): trimmed to the real call-site count.
        let injected_arguments = self.text(name) == "arguments"
            && matches!(init, ast::Expr::ArrayLiteral { span: literal, .. } if *literal == name.span);
        let mut init = self.check_expr(cx, init);
        self.closure_self_name = None;
        if injected_arguments && init.ty == Ty::Array {
            if let Some((trim_id, _)) = zun_natives::resolve("collections", "argumentsObject") {
                let (span_of, ty) = (init.span, init.ty);
                init = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id: trim_id, args: vec![init] },
                    ty,
                    span: span_of,
                };
            }
        }
        let declared = ty.as_ref().map(|t| self.resolve_annotation(t));
        // `let x: int = 1` — retype an integral number literal to the
        // declared numeric type.
        if let Some(d) = declared {
            coerce_numeric_literal(&mut init, d);
        }
        let var_ty = match declared {
            Some(declared) => {
                if !declared.is_error()
                    && !init.ty.is_error()
                    && !self.is_assignable(init.ty, declared)
                {
                    self.error(
                        init.span,
                        format!(
                            "initializer type {:?} does not match declared type {declared:?}",
                            init.ty
                        ),
                    );
                }
                init = Self::unbox_to(init, declared);
                declared
            }
            // An unannotated reassigned variable initialised from an integer-typed
            // value (`let hi = arr.length - 1`) is a JS `number`: widen to f64 so
            // later float assignments type-check.
            None if init.ty.is_int() && cx.mutated_names.contains(self.text(name)) => {
                let span = init.span;
                init = hir::Expr { kind: hir::ExprKind::Convert { value: Box::new(init) }, ty: Ty::F64, span };
                Ty::F64
            }
            None => init.ty,
        };
        let name_text = self.text(name).to_string();
        // A function-valued variable that is reassigned somewhere holds
        // an arbitrary function: calls go through the dynamic path.
        let reassigned_fn = declared.is_none()
            && matches!(var_ty, Ty::Function(_))
            && cx.mutated_names.contains(&name_text)
            && cx.mutated_names.contains(&format!("{name_text}"));
        let boxed_fn = reassigned_fn
            && cx.mutated_names.contains(&name_text)
            && cx.mutated_names.contains(&format!("{name_text}"));
        if reassigned_fn {
            self.reassigned_fn_names.insert(name_text.clone());
        }
        let var_ty = if reassigned_fn { Ty::Function(hir::DYNAMIC_FUNCTION) } else { var_ty };
        let slot = cx.declare(&name_text, var_ty);
        // A variable that a closure may capture and that is assigned
        // somewhere is boxed from its declaration, so every access
        // lowered before the capturing closure already uses the cell.
        if cx.mutated_names.contains(&name_text)
            && cx.mutated_names.contains(&format!("\u{1}{name_text}"))
            && (boxed_fn || !matches!(var_ty, Ty::Function(_)))
        {
            cx.boxed.insert(slot);
        }
        if cx.boxed.contains(&slot) {
            hir::Stmt::BoxedLet { slot, init }
        } else {
            hir::Stmt::Let { slot, init }
        }
    }
}
