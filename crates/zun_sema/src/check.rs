//! Name resolution and type checking.
//!
//! Two passes over the AST: first collect every function's signature (so calls
//! can be forward references), then check each body with a lexical scope stack.
//! Errors poison the offending expression with [`Ty::Error`], which suppresses
//! follow-on mismatch diagnostics instead of cascading.

use crate::hir;
use crate::hir::{Builtin, Ty};
use std::collections::HashMap;
use zun_common::{Diagnostic, Span};
use zun_syntax::ast;

/// Check `program`, producing HIR and appending diagnostics.
pub fn check(
    source: &str,
    program: &ast::Program,
    diagnostics: &mut Vec<Diagnostic>,
) -> hir::Program {
    let mut checker = Checker {
        source,
        diagnostics,
        signatures: Vec::new(),
        by_name: HashMap::new(),
        classes: Vec::new(),
        classes_by_name: HashMap::new(),
    };
    // Classes first so field and signature annotations can name them, then
    // function signatures so calls can be forward references, then bodies.
    checker.collect_classes(program);
    let classes = checker.check_classes(program);
    checker.collect_signatures(program);
    let functions = program
        .functions
        .iter()
        .enumerate()
        .map(|(index, f)| checker.check_function(index, f))
        .collect();
    let main = checker.by_name.get("main").copied();
    hir::Program {
        functions,
        classes,
        main,
    }
}

/// A collected function signature.
struct Signature {
    name: String,
    params: Vec<Ty>,
    return_ty: Ty,
}

/// A collected class layout: field name -> (slot, type).
struct ClassInfo {
    fields: Vec<(String, Ty)>,
}

struct Checker<'a> {
    source: &'a str,
    diagnostics: &'a mut Vec<Diagnostic>,
    signatures: Vec<Signature>,
    by_name: HashMap<String, u32>,
    classes: Vec<ClassInfo>,
    classes_by_name: HashMap<String, u32>,
}

/// One lexical scope: name -> local slot.
type Scope = HashMap<String, hir::LocalSlot>;

/// Per-function checking state.
struct FunctionCx {
    locals: Vec<Ty>,
    scopes: Vec<Scope>,
    return_ty: Ty,
}

impl FunctionCx {
    fn declare(&mut self, name: &str, ty: Ty) -> hir::LocalSlot {
        let slot = self.locals.len() as hir::LocalSlot;
        self.locals.push(ty);
        self.scopes
            .last_mut()
            .expect("a scope is always open")
            .insert(name.to_string(), slot);
        slot
    }

    fn lookup(&self, name: &str) -> Option<hir::LocalSlot> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }
}

impl Checker<'_> {
    fn text(&self, name: &ast::Name) -> &str {
        &self.source[name.span.start as usize..name.span.end as usize]
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(message).with_code("SEM0001").with_span(span));
    }

    /// Resolve a written type annotation to a [`Ty`].
    fn resolve_annotation(&mut self, annotation: &ast::TypeAnnotation) -> Ty {
        let text = self.text(&annotation.name).to_string();
        match text.as_str() {
            "number" => Ty::Number,
            "string" => Ty::String,
            "boolean" => Ty::Boolean,
            "void" => Ty::Void,
            other => match self.classes_by_name.get(other) {
                Some(&index) => Ty::Instance(index),
                None => {
                    self.error(annotation.name.span, format!("unknown type `{other}`"));
                    Ty::Error
                }
            },
        }
    }

    /// First pass over classes: register names so annotations can resolve them.
    fn collect_classes(&mut self, program: &ast::Program) {
        for c in &program.classes {
            let name = self.text(&c.name).to_string();
            let index = self.classes.len() as u32;
            if self.classes_by_name.insert(name, index).is_some() {
                self.error(c.name.span, "duplicate class");
            }
            // Reserve the slot; fields are resolved in check_classes once every
            // class name is known.
            self.classes.push(ClassInfo { fields: Vec::new() });
        }
    }

    /// Second pass: resolve each class's field types and build the HIR classes.
    fn check_classes(&mut self, program: &ast::Program) -> Vec<hir::Class> {
        let mut out = Vec::with_capacity(program.classes.len());
        for (index, c) in program.classes.iter().enumerate() {
            let mut fields = Vec::new();
            let mut infos = Vec::new();
            for f in &c.fields {
                let fname = self.text(&f.name).to_string();
                let ty = self.resolve_annotation(&f.ty);
                infos.push((fname.clone(), ty));
                fields.push(hir::ClassField { name: fname, ty });
            }
            self.classes[index].fields = infos;
            out.push(hir::Class {
                name: self.text(&c.name).to_string(),
                fields,
            });
        }
        out
    }

    /// Find a field's slot and type in a class layout.
    fn field_of(&self, class: u32, name: &str) -> Option<(u16, Ty)> {
        self.classes[class as usize]
            .fields
            .iter()
            .enumerate()
            .find(|(_, (n, _))| n == name)
            .map(|(slot, (_, ty))| (slot as u16, *ty))
    }

    fn collect_signatures(&mut self, program: &ast::Program) {
        for f in &program.functions {
            let name = self.text(&f.name).to_string();
            let params = f
                .params
                .iter()
                .map(|p| match &p.ty {
                    Some(t) => self.resolve_annotation(t),
                    None => {
                        self.error(
                            p.name.span,
                            "phase 1 requires a type annotation on every parameter",
                        );
                        Ty::Error
                    }
                })
                .collect();
            let return_ty = match &f.return_type {
                Some(t) => self.resolve_annotation(t),
                None => Ty::Void,
            };
            let index = self.signatures.len() as u32;
            if self.by_name.insert(name.clone(), index).is_some() {
                self.error(f.name.span, format!("duplicate function `{name}`"));
            }
            self.signatures.push(Signature {
                name,
                params,
                return_ty,
            });
        }
    }

    fn check_function(&mut self, index: usize, f: &ast::Function) -> hir::Function {
        let sig = &self.signatures[index];
        let return_ty = sig.return_ty;
        let param_tys = sig.params.clone();
        let name = sig.name.clone();

        let mut cx = FunctionCx {
            locals: Vec::new(),
            scopes: vec![Scope::new()],
            return_ty,
        };
        for (p, &ty) in f.params.iter().zip(&param_tys) {
            let pname = self.text(&p.name).to_string();
            cx.declare(&pname, ty);
        }

        let body = self.check_block(&mut cx, &f.body);
        hir::Function {
            name,
            param_count: param_tys.len() as u16,
            locals: cx.locals,
            return_ty,
            body,
        }
    }

    fn check_block(&mut self, cx: &mut FunctionCx, block: &ast::Block) -> Vec<hir::Stmt> {
        cx.scopes.push(Scope::new());
        let stmts = block
            .statements
            .iter()
            .map(|s| self.check_stmt(cx, s))
            .collect();
        cx.scopes.pop();
        stmts
    }

    fn check_stmt(&mut self, cx: &mut FunctionCx, stmt: &ast::Stmt) -> hir::Stmt {
        match stmt {
            ast::Stmt::Let { name, ty, init, .. } => {
                let init = self.check_expr(cx, init);
                let declared = ty.as_ref().map(|t| self.resolve_annotation(t));
                let var_ty = match declared {
                    Some(declared) => {
                        if !declared.is_error()
                            && !init.ty.is_error()
                            && declared != init.ty
                        {
                            self.error(
                                init.span,
                                format!(
                                    "initializer type {:?} does not match declared type {declared:?}",
                                    init.ty
                                ),
                            );
                        }
                        declared
                    }
                    None => init.ty,
                };
                let name_text = self.text(name).to_string();
                let slot = cx.declare(&name_text, var_ty);
                hir::Stmt::Let { slot, init }
            }
            ast::Stmt::Expr(e) => hir::Stmt::Expr(self.check_expr(cx, e)),
            ast::Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                let cond = self.check_condition(cx, cond);
                let then_body = self.check_block(cx, then_block);
                let else_body = else_block
                    .as_ref()
                    .map(|b| self.check_block(cx, b))
                    .unwrap_or_default();
                hir::Stmt::If {
                    cond,
                    then_body,
                    else_body,
                }
            }
            ast::Stmt::While { cond, body, .. } => {
                let cond = self.check_condition(cx, cond);
                let body = self.check_block(cx, body);
                hir::Stmt::While { cond, body }
            }
            ast::Stmt::Return { value, span } => {
                let value = value.as_ref().map(|v| self.check_expr(cx, v));
                match (&value, cx.return_ty) {
                    (None, Ty::Void) => {}
                    (None, expected) if !expected.is_error() => {
                        self.error(*span, format!("expected a {expected:?} return value"));
                    }
                    (Some(v), expected) => {
                        if !expected.is_error() && !v.ty.is_error() && v.ty != expected {
                            self.error(
                                v.span,
                                format!("return type {:?} does not match {expected:?}", v.ty),
                            );
                        }
                    }
                    _ => {}
                }
                hir::Stmt::Return(value)
            }
        }
    }

    fn check_condition(&mut self, cx: &mut FunctionCx, cond: &ast::Expr) -> hir::Expr {
        let cond = self.check_expr(cx, cond);
        if !cond.ty.is_error() && cond.ty != Ty::Boolean {
            self.error(
                cond.span,
                format!("condition must be boolean, found {:?}", cond.ty),
            );
        }
        cond
    }

    fn check_expr(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let span = expr.span();
        match expr {
            ast::Expr::Number { value, .. } => hir::Expr {
                kind: hir::ExprKind::Number(*value),
                ty: Ty::Number,
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
            ast::Expr::Null { .. } => {
                self.error(span, "`null` is not supported in phase 1");
                self.poison(span)
            }
            ast::Expr::Ident(name) => {
                let text = self.text(name).to_string();
                match cx.lookup(&text) {
                    Some(slot) => hir::Expr {
                        ty: cx.locals[slot as usize],
                        kind: hir::ExprKind::Local(slot),
                        span,
                    },
                    None => {
                        self.error(span, format!("unknown variable `{text}`"));
                        self.poison(span)
                    }
                }
            }
            ast::Expr::Assign { target, value, .. } => {
                let value = self.check_expr(cx, value);
                let text = self.text(target).to_string();
                match cx.lookup(&text) {
                    Some(slot) => {
                        let slot_ty = cx.locals[slot as usize];
                        if !slot_ty.is_error() && !value.ty.is_error() && slot_ty != value.ty {
                            self.error(
                                value.span,
                                format!("cannot assign {:?} to {slot_ty:?}", value.ty),
                            );
                        }
                        hir::Expr {
                            ty: slot_ty,
                            kind: hir::ExprKind::Assign {
                                slot,
                                value: Box::new(value),
                            },
                            span,
                        }
                    }
                    None => {
                        self.error(target.span, format!("unknown variable `{text}`"));
                        self.poison(span)
                    }
                }
            }
            ast::Expr::Binary { op, lhs, rhs, .. } => {
                let lhs = self.check_expr(cx, lhs);
                let rhs = self.check_expr(cx, rhs);
                let ty = self.binary_result(*op, &lhs, &rhs, span);
                hir::Expr {
                    kind: hir::ExprKind::Binary {
                        op: *op,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    },
                    ty,
                    span,
                }
            }
            ast::Expr::Unary { op, operand, .. } => {
                let operand = self.check_expr(cx, operand);
                let ty = match op {
                    ast::UnaryOp::Neg => self.expect_ty(&operand, Ty::Number),
                    ast::UnaryOp::Not => self.expect_ty(&operand, Ty::Boolean),
                };
                hir::Expr {
                    kind: hir::ExprKind::Unary {
                        op: *op,
                        operand: Box::new(operand),
                    },
                    ty,
                    span,
                }
            }
            ast::Expr::Call { callee, args, .. } => {
                let name = self.text(callee).to_string();
                let args: Vec<hir::Expr> =
                    args.iter().map(|a| self.check_expr(cx, a)).collect();

                // Builtins first: `print` accepts exactly one argument of any
                // primitive type and returns void.
                if name == "print" {
                    if args.len() != 1 {
                        self.error(span, "print takes exactly one argument");
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            builtin: Builtin::Print,
                            args,
                        },
                        ty: Ty::Void,
                        span,
                    };
                }

                let Some(&func) = self.by_name.get(&name) else {
                    self.error(callee.span, format!("unknown function `{name}`"));
                    return self.poison(span);
                };
                let (param_tys, return_ty) = {
                    let sig = &self.signatures[func as usize];
                    (sig.params.clone(), sig.return_ty)
                };
                if args.len() != param_tys.len() {
                    self.error(
                        span,
                        format!(
                            "`{name}` expects {} argument(s), found {}",
                            param_tys.len(),
                            args.len()
                        ),
                    );
                } else {
                    for (arg, &expected) in args.iter().zip(&param_tys) {
                        if !expected.is_error() && !arg.ty.is_error() && arg.ty != expected {
                            self.error(
                                arg.span,
                                format!("argument type {:?} does not match {expected:?}", arg.ty),
                            );
                        }
                    }
                }
                hir::Expr {
                    kind: hir::ExprKind::Call { func, args },
                    ty: return_ty,
                    span,
                }
            }
            ast::Expr::New { class, args, .. } => {
                let name = self.text(class).to_string();
                let Some(&class_idx) = self.classes_by_name.get(&name) else {
                    self.error(class.span, format!("unknown class `{name}`"));
                    return self.poison(span);
                };
                let field_tys: Vec<Ty> = self.classes[class_idx as usize]
                    .fields
                    .iter()
                    .map(|(_, ty)| *ty)
                    .collect();
                let args: Vec<hir::Expr> =
                    args.iter().map(|a| self.check_expr(cx, a)).collect();
                if args.len() != field_tys.len() {
                    self.error(
                        span,
                        format!(
                            "`{name}` has {} field(s), found {} initializer(s)",
                            field_tys.len(),
                            args.len()
                        ),
                    );
                } else {
                    for (arg, &expected) in args.iter().zip(&field_tys) {
                        if !expected.is_error() && !arg.ty.is_error() && arg.ty != expected {
                            self.error(
                                arg.span,
                                format!("field initializer type {:?} does not match {expected:?}", arg.ty),
                            );
                        }
                    }
                }
                hir::Expr {
                    kind: hir::ExprKind::New {
                        class: class_idx,
                        args,
                    },
                    ty: Ty::Instance(class_idx),
                    span,
                }
            }
            ast::Expr::Field { object, field, .. } => {
                let object = self.check_expr(cx, object);
                let fname = self.text(field).to_string();
                match object.ty {
                    Ty::Instance(class) => match self.field_of(class, &fname) {
                        Some((slot, ty)) => hir::Expr {
                            kind: hir::ExprKind::FieldGet {
                                object: Box::new(object),
                                class,
                                field_slot: slot,
                            },
                            ty,
                            span,
                        },
                        None => {
                            self.error(field.span, format!("no field `{fname}` on this type"));
                            self.poison(span)
                        }
                    },
                    Ty::Error => self.poison(span),
                    other => {
                        self.error(object.span, format!("type {other:?} has no fields"));
                        self.poison(span)
                    }
                }
            }
            ast::Expr::FieldAssign {
                object,
                field,
                value,
                ..
            } => {
                let object = self.check_expr(cx, object);
                let value = self.check_expr(cx, value);
                let fname = self.text(field).to_string();
                match object.ty {
                    Ty::Instance(class) => match self.field_of(class, &fname) {
                        Some((slot, ty)) => {
                            if !ty.is_error() && !value.ty.is_error() && ty != value.ty {
                                self.error(
                                    value.span,
                                    format!("cannot assign {:?} to field of type {ty:?}", value.ty),
                                );
                            }
                            hir::Expr {
                                kind: hir::ExprKind::FieldSet {
                                    object: Box::new(object),
                                    class,
                                    field_slot: slot,
                                    value: Box::new(value),
                                },
                                ty,
                                span,
                            }
                        }
                        None => {
                            self.error(field.span, format!("no field `{fname}` on this type"));
                            self.poison(span)
                        }
                    },
                    Ty::Error => self.poison(span),
                    other => {
                        self.error(object.span, format!("type {other:?} has no fields"));
                        self.poison(span)
                    }
                }
            }
        }
    }

    /// The result type of a binary operation, diagnosing bad operand types.
    fn binary_result(
        &mut self,
        op: ast::BinaryOp,
        lhs: &hir::Expr,
        rhs: &hir::Expr,
        span: Span,
    ) -> Ty {
        use ast::BinaryOp::*;
        if lhs.ty.is_error() || rhs.ty.is_error() {
            return Ty::Error;
        }
        match op {
            Add => {
                // `+` is numeric addition or string concatenation.
                if lhs.ty == Ty::Number && rhs.ty == Ty::Number {
                    Ty::Number
                } else if lhs.ty == Ty::String && rhs.ty == Ty::String {
                    Ty::String
                } else {
                    self.error(
                        span,
                        format!("cannot add {:?} and {:?}", lhs.ty, rhs.ty),
                    );
                    Ty::Error
                }
            }
            Sub | Mul | Div | Rem => {
                if lhs.ty == Ty::Number && rhs.ty == Ty::Number {
                    Ty::Number
                } else {
                    self.error(
                        span,
                        format!("arithmetic requires numbers, found {:?} and {:?}", lhs.ty, rhs.ty),
                    );
                    Ty::Error
                }
            }
            Lt | Le | Gt | Ge => {
                if lhs.ty == Ty::Number && rhs.ty == Ty::Number {
                    Ty::Boolean
                } else {
                    self.error(
                        span,
                        format!("comparison requires numbers, found {:?} and {:?}", lhs.ty, rhs.ty),
                    );
                    Ty::Error
                }
            }
            Eq | Ne => {
                if lhs.ty == rhs.ty {
                    Ty::Boolean
                } else {
                    self.error(
                        span,
                        format!("cannot compare {:?} with {:?}", lhs.ty, rhs.ty),
                    );
                    Ty::Error
                }
            }
        }
    }

    fn expect_ty(&mut self, expr: &hir::Expr, expected: Ty) -> Ty {
        if expr.ty.is_error() {
            Ty::Error
        } else if expr.ty == expected {
            expected
        } else {
            self.error(
                expr.span,
                format!("expected {expected:?}, found {:?}", expr.ty),
            );
            Ty::Error
        }
    }

    fn poison(&mut self, span: Span) -> hir::Expr {
        hir::Expr {
            kind: hir::ExprKind::Error,
            ty: Ty::Error,
            span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zun_syntax::parse;

    fn check_src(src: &str) -> (hir::Program, Vec<Diagnostic>) {
        let mut diags = Vec::new();
        let ast = parse(src, &mut diags);
        assert!(diags.is_empty(), "parse errors: {diags:?}");
        let hir = check(src, &ast, &mut diags);
        (hir, diags)
    }

    fn check_ok(src: &str) -> hir::Program {
        let (hir, diags) = check_src(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        hir
    }

    fn first_error(src: &str) -> String {
        let (_, diags) = check_src(src);
        assert!(!diags.is_empty(), "expected a diagnostic");
        diags[0].message.clone()
    }

    #[test]
    fn checks_typed_function_and_finds_main() {
        let hir = check_ok(
            "function add(a: number, b: number): number { return a + b; }\n\
             function main() { print(add(2, 3)); }",
        );
        assert_eq!(hir.functions.len(), 2);
        assert_eq!(hir.main, Some(1));
        let add = &hir.functions[0];
        assert_eq!(add.param_count, 2);
        assert_eq!(add.return_ty, Ty::Number);
    }

    #[test]
    fn let_infers_type_from_initializer() {
        let hir = check_ok("function f() { let x = 1; let s = \"hi\"; }");
        assert_eq!(hir.functions[0].locals, vec![Ty::Number, Ty::String]);
    }

    #[test]
    fn shadowing_gets_a_fresh_slot() {
        let hir = check_ok(
            "function f() { let x = 1; if (x > 0) { let x = \"inner\"; print(x); } }",
        );
        assert_eq!(hir.functions[0].locals, vec![Ty::Number, Ty::String]);
    }

    #[test]
    fn forward_calls_resolve() {
        check_ok("function main() { helper(); } function helper() {}");
    }

    #[test]
    fn plus_concatenates_strings() {
        let hir = check_ok("function f(): string { return \"a\" + \"b\"; }");
        assert_eq!(hir.functions[0].return_ty, Ty::String);
    }

    #[test]
    fn mixed_add_is_an_error() {
        assert!(first_error("function f() { let x = 1 + \"a\"; }").contains("cannot add"));
    }

    #[test]
    fn condition_must_be_boolean() {
        assert!(first_error("function f() { if (1) {} }").contains("must be boolean"));
    }

    #[test]
    fn unknown_variable_diagnoses() {
        assert!(first_error("function f() { print(y); }").contains("unknown variable"));
    }

    #[test]
    fn wrong_argument_type_diagnoses() {
        let msg = first_error(
            "function g(n: number) {} function f() { g(\"s\"); }",
        );
        assert!(msg.contains("does not match"), "{msg}");
    }

    #[test]
    fn wrong_return_type_diagnoses() {
        assert!(
            first_error("function f(): number { return \"s\"; }").contains("does not match")
        );
    }

    #[test]
    fn inner_scope_names_do_not_leak() {
        let msg = first_error(
            "function f(x: boolean) { if (x) { let inner = 1; } print(inner); }",
        );
        assert!(msg.contains("unknown variable"), "{msg}");
    }

    #[test]
    fn class_fields_and_new_check() {
        let hir = check_ok(
            "class Point { x: number; y: number; }\n\
             function main() { let p = new Point(1, 2); print(p.x); }",
        );
        assert_eq!(hir.classes.len(), 1);
        assert_eq!(hir.classes[0].fields.len(), 2);
    }

    #[test]
    fn field_type_flows_through_access() {
        // `p.x` is a number, so it can feed arithmetic.
        check_ok(
            "class Box { v: number; }\n\
             function main() { let b = new Box(3); let y = b.v + 1; print(y); }",
        );
    }

    #[test]
    fn wrong_new_arity_diagnoses() {
        let msg = first_error(
            "class Point { x: number; y: number; }\n\
             function main() { let p = new Point(1); }",
        );
        assert!(msg.contains("field(s)"), "{msg}");
    }

    #[test]
    fn unknown_field_diagnoses() {
        let msg = first_error(
            "class Point { x: number; }\n\
             function main() { let p = new Point(1); print(p.z); }",
        );
        assert!(msg.contains("no field"), "{msg}");
    }

    #[test]
    fn field_assignment_type_checked() {
        let msg = first_error(
            "class Box { v: number; }\n\
             function main() { let b = new Box(1); b.v = \"s\"; }",
        );
        assert!(msg.contains("cannot assign"), "{msg}");
    }

    #[test]
    fn class_typed_field_references_another_class() {
        check_ok(
            "class Inner { n: number; }\n\
             class Outer { inner: Inner; }\n\
             function main() { let i = new Inner(5); let o = new Outer(i); print(o.inner.n); }",
        );
    }

    #[test]
    fn error_does_not_cascade() {
        // `y` is unknown; the addition and the assignment must not pile on
        // extra diagnostics.
        let (_, diags) = check_src("function f() { let x = y + 1; }");
        assert_eq!(diags.len(), 1, "{diags:?}");
    }
}
