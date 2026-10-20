//! Name resolution and type checking.
//!
//! Two passes over the AST: first collect every function's signature (so calls
//! can be forward references), then check each body with a lexical scope stack.
//! Errors poison the offending expression with [`Ty::Error`], which suppresses
//! follow-on mismatch diagnostics instead of cascading.

use crate::hir;
use crate::hir::{IntKind, Ty};
use std::collections::{HashMap, HashSet};
use zun_common::{Diagnostic, Span};
use zun_syntax::ast;

/// Check `program`, producing HIR and appending diagnostics.
pub fn check(
    source: &str,
    program: &ast::Program,
    diagnostics: &mut Vec<Diagnostic>,
) -> hir::Program {
    check_impl(source, program, diagnostics, &[], false)
}

/// A source-level function signature imported from another module.
#[derive(Debug, Clone)]
pub struct ExternalFunction {
    /// Module identity containing the export.
    pub module: String,
    /// Exported function name.
    pub name: String,
    /// Parameter types in declaration order.
    pub params: Vec<Ty>,
    /// Return type.
    pub return_ty: Ty,
}

/// Check a program with source-module imports supplied by the linker.
pub fn check_with_externals(
    source: &str,
    program: &ast::Program,
    diagnostics: &mut Vec<Diagnostic>,
    external_functions: &[ExternalFunction],
) -> hir::Program {
    check_impl(source, program, diagnostics, external_functions, true)
}

fn check_impl(
    source: &str,
    program: &ast::Program,
    diagnostics: &mut Vec<Diagnostic>,
    external_functions: &[ExternalFunction],
    allow_external_modules: bool,
) -> hir::Program {
    let external_by_name: HashMap<_, _> = external_functions
        .iter()
        .map(|function| (function.name.clone(), function.clone()))
        .collect();
    let mut checker = Checker {
        source,
        diagnostics,
        signatures: Vec::new(),
        by_name: HashMap::new(),
        classes: Vec::new(),
        classes_by_name: HashMap::new(),
        methods_by_class: Vec::new(),
        imports: HashMap::new(),
        external_functions: external_by_name,
        closures: Vec::new(),
    };
    checker.resolve_imports(program, allow_external_modules);
    // Classes first so field and signature annotations can name them, then
    // function and method signatures so calls can be forward references, then
    // bodies. Methods are checked as functions with an implicit `this` param.
    checker.collect_classes(program);
    let mut classes = checker.check_classes(program);
    checker.collect_signatures(program);

    let mut functions: Vec<hir::Function> = Vec::new();
    // Top-level functions occupy the first func indices (see collect_signatures).
    for (i, f) in program.functions.iter().enumerate() {
        functions.push(checker.check_function(i, f, None));
    }
    // Methods follow, in (class, method) order; record each class's method
    // func indices for the HIR (their MethodDef RIDs are contiguous in order).
    let mut next = program.functions.len();
    for (class_idx, class) in program.classes.iter().enumerate() {
        for m in &class.methods {
            classes[class_idx].methods.push(next as u32);
            functions.push(checker.check_function(next, m, Some(class_idx as u32)));
            next += 1;
        }
    }

    let main = checker.by_name.get("main").copied();
    hir::Program {
        functions,
        classes,
        main,
        closures: checker.closures,
    }
}

/// A collected function signature.
struct Signature {
    name: String,
    params: Vec<Ty>,
    return_ty: Ty,
}

/// A collected class: its base and its own (not inherited) fields.
struct ClassInfo {
    base: Option<u32>,
    base_name: Option<String>,
    own_fields: Vec<(String, Ty)>,
}

struct Checker<'a> {
    source: &'a str,
    diagnostics: &'a mut Vec<Diagnostic>,
    signatures: Vec<Signature>,
    by_name: HashMap<String, u32>,
    classes: Vec<ClassInfo>,
    classes_by_name: HashMap<String, u32>,
    /// Per class index: method name -> global func index.
    methods_by_class: Vec<HashMap<String, u32>>,
    /// Imported intrinsic name -> (dispatch id, its native signature).
    imports: HashMap<String, (u32, &'static zun_natives::NativeFn)>,
    /// Imported source-module function signatures by local name.
    external_functions: HashMap<String, ExternalFunction>,
    closures: Vec<hir::ClosureFunction>,
}

/// One lexical scope: name -> local slot.
type Scope = HashMap<String, hir::LocalSlot>;

/// Per-function checking state.
struct FunctionCx {
    locals: Vec<Ty>,
    scopes: Vec<Scope>,
    return_ty: Ty,
    loop_depth: usize,
    mutated: HashSet<hir::LocalSlot>,
    mutated_names: HashSet<String>,
    boxed: HashSet<hir::LocalSlot>,
    captures: HashMap<String, hir::Capture>,
}

#[derive(Default)]
struct CaptureUse {
    slots: Vec<hir::LocalSlot>,
    mutated: HashSet<hir::LocalSlot>,
    inherited: Vec<hir::Capture>,
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
    fn rewrite_boxed_lets(&self, statements: &mut [hir::Stmt], boxed: &HashSet<hir::LocalSlot>) {
        for statement in statements {
            match statement {
                hir::Stmt::Let { slot, init } if boxed.contains(slot) => {
                    let span = init.span;
                    let slot = *slot;
                    let value = std::mem::replace(init, hir::Expr {
                        kind: hir::ExprKind::Error,
                        ty: Ty::Error,
                        span,
                    });
                    *statement = hir::Stmt::BoxedLet { slot, init: value };
                }
                hir::Stmt::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    self.rewrite_boxed_lets(then_body, boxed);
                    self.rewrite_boxed_lets(else_body, boxed);
                }
                hir::Stmt::While { body, .. } => self.rewrite_boxed_lets(body, boxed),
                hir::Stmt::Try {
                    body,
                    catches,
                    finally_body,
                } => {
                    self.rewrite_boxed_lets(body, boxed);
                    for catch in catches {
                        self.rewrite_boxed_lets(&mut catch.body, boxed);
                    }
                    if let Some(finally_body) = finally_body {
                        self.rewrite_boxed_lets(finally_body, boxed);
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_mutated_names(&self, statements: &[ast::Stmt], names: &mut HashSet<String>) {
        for statement in statements {
            match statement {
                ast::Stmt::Let { init, .. } | ast::Stmt::Expr(init) => {
                    self.collect_mutated_expr(init, names)
                }
                ast::Stmt::If {
                    cond,
                    then_block,
                    else_block,
                    ..
                } => {
                    self.collect_mutated_expr(cond, names);
                    self.collect_mutated_names(&then_block.statements, names);
                    if let Some(block) = else_block {
                        self.collect_mutated_names(&block.statements, names);
                    }
                }
                ast::Stmt::While { cond, body, .. } => {
                    self.collect_mutated_expr(cond, names);
                    self.collect_mutated_names(&body.statements, names);
                }
                ast::Stmt::Return { value, .. } => {
                    if let Some(value) = value {
                        self.collect_mutated_expr(value, names);
                    }
                }
                ast::Stmt::Throw { value, .. } => self.collect_mutated_expr(value, names),
                ast::Stmt::Try {
                    body,
                    catches,
                    finally,
                    ..
                } => {
                    self.collect_mutated_names(&body.statements, names);
                    for catch in catches {
                        self.collect_mutated_names(&catch.body.statements, names);
                    }
                    if let Some(block) = finally {
                        self.collect_mutated_names(&block.statements, names);
                    }
                }
                ast::Stmt::Break { .. } | ast::Stmt::Continue { .. } => {}
            }
        }
    }

    fn collect_mutated_expr(&self, expr: &ast::Expr, names: &mut HashSet<String>) {
        match expr {
            ast::Expr::Assign { target, value, .. } => {
                names.insert(self.text(target).to_string());
                self.collect_mutated_expr(value, names);
            }
            ast::Expr::Binary { lhs, rhs, .. } => {
                self.collect_mutated_expr(lhs, names);
                self.collect_mutated_expr(rhs, names);
            }
            ast::Expr::Unary { operand, .. } | ast::Expr::Cast { expr: operand, .. } => {
                self.collect_mutated_expr(operand, names)
            }
            ast::Expr::Call { args, .. } | ast::Expr::New { args, .. } => {
                for arg in args {
                    self.collect_mutated_expr(arg, names);
                }
            }
            ast::Expr::MethodCall { receiver, args, .. } => {
                self.collect_mutated_expr(receiver, names);
                for arg in args {
                    self.collect_mutated_expr(arg, names);
                }
            }
            ast::Expr::Field { object, .. } | ast::Expr::Index { object, .. } => {
                self.collect_mutated_expr(object, names)
            }
            ast::Expr::FieldAssign { object, value, .. } => {
                self.collect_mutated_expr(object, names);
                self.collect_mutated_expr(value, names);
            }
            ast::Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                self.collect_mutated_expr(object, names);
                self.collect_mutated_expr(index, names);
                self.collect_mutated_expr(value, names);
            }
            ast::Expr::Function(_) | ast::Expr::Ident(_) | ast::Expr::Number { .. }
            | ast::Expr::String { .. }
            | ast::Expr::Bool { .. }
            | ast::Expr::Null { .. } => {}
        }
    }

    fn check_closure_expr(
        &mut self,
        outer: &mut FunctionCx,
        function: &ast::FunctionExpr,
        span: Span,
    ) -> hir::Expr {
        let captures = self.analyze_closure_captures(outer, function);
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
            loop_depth: 0,
            mutated: HashSet::new(),
            mutated_names: HashSet::new(),
            boxed: HashSet::new(),
            captures: captures
                .iter()
                .map(|capture| (capture.name.clone(), capture.clone()))
                .collect(),
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
        self.closures.push(hir::ClosureFunction {
            name: format!("<closure@{}>", span.start),
            param_count: function.params.len() as u16,
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
            .chain(uses.inherited.into_iter().enumerate().map(move |(offset, mut capture)| {
                let source_slot = match capture.source {
                    hir::CaptureSource::Environment { environment_slot, .. } => environment_slot,
                    hir::CaptureSource::Local(slot) => slot as u16,
                };
                capture.environment_slot = (local_capture_count + offset) as u16;
                capture.source = hir::CaptureSource::Environment {
                    environment_slot: source_slot,
                    mode: capture.mode,
                };
                capture
            }))
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
            ast::Stmt::Let { init, .. } | ast::Stmt::Expr(init) => {
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
            ast::Expr::Unary { operand, .. } | ast::Expr::Cast { expr: operand, .. } => {
                self.collect_capture_expr(cx, parameters, operand, uses)
            }
            ast::Expr::Call { args, .. } => {
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
            ast::Expr::New { args, .. } => {
                for arg in args {
                    self.collect_capture_expr(cx, parameters, arg, uses);
                }
            }
            ast::Expr::Field { object, .. } => {
                self.collect_capture_expr(cx, parameters, object, uses)
            }
            ast::Expr::FieldAssign { object, value, .. } => {
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
            ast::Expr::Number { .. }
            | ast::Expr::String { .. }
            | ast::Expr::Bool { .. }
            | ast::Expr::Null { .. } => {}
        }
    }

    fn text(&self, name: &ast::Name) -> &str {
        &self.source[name.span.start as usize..name.span.end as usize]
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(
            Diagnostic::error(message)
                .with_code("SEM0001")
                .with_span(span),
        );
    }

    /// Resolve a written type annotation to a [`Ty`].
    fn resolve_annotation(&mut self, annotation: &ast::TypeAnnotation) -> Ty {
        let text = self.text(&annotation.name).to_string();
        match text.as_str() {
            "number" | "float" | "f64" => Ty::F64,
            "f32" => Ty::F32,
            "int" | "i32" => Ty::Int(IntKind::I32),
            "i8" => Ty::Int(IntKind::I8),
            "i16" => Ty::Int(IntKind::I16),
            "i64" => Ty::Int(IntKind::I64),
            "u8" => Ty::Int(IntKind::U8),
            "u16" => Ty::Int(IntKind::U16),
            "u32" => Ty::Int(IntKind::U32),
            "u64" => Ty::Int(IntKind::U64),
            "string" => Ty::String,
            "boolean" => Ty::Boolean,
            "Array" | "array" => Ty::Array,
            "dynamic" | "DynamicObject" => Ty::DynamicObject,
            "any" => Ty::Any,
            "Function" => Ty::Function(hir::DYNAMIC_FUNCTION),
            "Predicate" => Ty::Function(hir::DYNAMIC_PREDICATE),
            "Reducer" => Ty::Function(hir::DYNAMIC_REDUCER),
            "void" => Ty::Void,
            other => {
                if let Some(&func) = self.by_name.get(other) {
                    return Ty::Function(func);
                }
                match self.classes_by_name.get(other) {
                    Some(&index) => Ty::Instance(index),
                    None => {
                        self.error(annotation.name.span, format!("unknown type `{other}`"));
                        Ty::Error
                    }
                }
            }
        }
    }

    /// Resolve `import { ... } from "zun:..."` declarations to intrinsics,
    /// binding each imported name to its signature and dispatch id.
    fn resolve_imports(&mut self, program: &ast::Program, allow_external_modules: bool) {
        for import in &program.imports {
            let Some(module) = zun_natives::module(&import.module) else {
                if allow_external_modules && !import.module.starts_with("zun:") {
                    continue;
                }
                self.error(import.span, format!("unknown module `{}`", import.module));
                continue;
            };
            let _ = module;
            for name in &import.names {
                let text = self.text(name).to_string();
                match zun_natives::resolve(&import.module, &text) {
                    Some((id, f)) => {
                        self.imports.insert(text, (id, f));
                    }
                    None => self.error(
                        name.span,
                        format!("`{}` has no export named `{text}`", import.module),
                    ),
                }
            }
        }
    }

    /// First pass over classes: register names (so annotations resolve) and
    /// remember each base's name for later resolution.
    fn collect_classes(&mut self, program: &ast::Program) {
        for c in &program.classes {
            let name = self.text(&c.name).to_string();
            let index = self.classes.len() as u32;
            if self.classes_by_name.insert(name, index).is_some() {
                self.error(c.name.span, "duplicate class");
            }
            let base_name = c.base.as_ref().map(|b| self.text(b).to_string());
            self.classes.push(ClassInfo {
                base: None,
                base_name,
                own_fields: Vec::new(),
            });
        }
        // Resolve base names to indices now that all classes are known.
        for (i, c) in program.classes.iter().enumerate() {
            if let Some(base_name) = self.classes[i].base_name.clone() {
                match self.classes_by_name.get(&base_name) {
                    Some(&b) if b != i as u32 => self.classes[i].base = Some(b),
                    Some(_) => {
                        if let Some(b) = &c.base {
                            self.error(b.span, "a class cannot extend itself");
                        }
                    }
                    None => {
                        if let Some(b) = &c.base {
                            self.error(b.span, format!("unknown base class `{base_name}`"));
                        }
                    }
                }
            }
        }
    }

    /// Second pass: resolve each class's own field types, then build the HIR
    /// classes with the full (inherited-first) field layout.
    fn check_classes(&mut self, program: &ast::Program) -> Vec<hir::Class> {
        for (index, c) in program.classes.iter().enumerate() {
            let mut own = Vec::new();
            for f in &c.fields {
                let fname = self.text(&f.name).to_string();
                let ty = self.resolve_annotation(&f.ty);
                own.push((fname, ty));
            }
            self.classes[index].own_fields = own;
        }

        let mut out = Vec::with_capacity(program.classes.len());
        for (index, c) in program.classes.iter().enumerate() {
            let fields = self
                .full_fields(index as u32)
                .into_iter()
                .map(|(name, ty)| hir::ClassField { name, ty })
                .collect();
            out.push(hir::Class {
                name: self.text(&c.name).to_string(),
                base: self.classes[index].base,
                fields,
                methods: Vec::new(), // filled after signatures assign indices
            });
        }
        out
    }

    /// The full field layout of a class: inherited fields first (base's full
    /// layout), then the class's own fields, in slot order.
    fn full_fields(&self, class: u32) -> Vec<(String, Ty)> {
        let info = &self.classes[class as usize];
        let mut fields = match info.base {
            Some(b) => self.full_fields(b),
            None => Vec::new(),
        };
        fields.extend(info.own_fields.iter().cloned());
        fields
    }

    /// Find a field's slot and type, searching inherited fields too.
    fn field_of(&self, class: u32, name: &str) -> Option<(u16, Ty)> {
        self.full_fields(class)
            .into_iter()
            .enumerate()
            .find(|(_, (n, _))| n == name)
            .map(|(slot, (_, ty))| (slot as u16, ty))
    }

    /// Resolve a method name to its function index, searching base classes.
    fn method_of(&self, class: u32, name: &str) -> Option<u32> {
        if let Some(&f) = self.methods_by_class[class as usize].get(name) {
            return Some(f);
        }
        match self.classes[class as usize].base {
            Some(b) => self.method_of(b, name),
            None => None,
        }
    }

    /// Whether `sub` is `sup` or a (transitive) subclass of it.
    fn is_subclass(&self, sub: u32, sup: u32) -> bool {
        if sub == sup {
            return true;
        }
        match self.classes[sub as usize].base {
            Some(b) => self.is_subclass(b, sup),
            None => false,
        }
    }

    /// Whether a value of type `from` may be used where `to` is expected.
    /// Instances are assignable to a base type; otherwise types must match.
    fn is_assignable(&self, from: Ty, to: Ty) -> bool {
        match (from, to) {
            (Ty::Instance(d), Ty::Instance(b)) => self.is_subclass(d, b),
            (Ty::Function(_), Ty::Function(expected)) if expected == hir::DYNAMIC_FUNCTION => true,
            (Ty::Function(_), Ty::Function(expected)) if expected == hir::DYNAMIC_PREDICATE => true,
            (Ty::Function(_), Ty::Function(expected)) if expected == hir::DYNAMIC_REDUCER => true,
            (_, Ty::Any) => true,
            (a, b) => a == b,
        }
    }

    fn collect_signatures(&mut self, program: &ast::Program) {
        // Register every top-level name before resolving annotations. This
        // permits a function-typed parameter to refer to a function declared
        // later in the source file.
        for (index, f) in program.functions.iter().enumerate() {
            let name = self.text(&f.name).to_string();
            if self.by_name.insert(name, index as u32).is_some() {
                self.error(f.name.span, "duplicate function declaration");
            }
        }
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
            self.signatures.push(Signature {
                name,
                params,
                return_ty,
            });
        }

        // Method signatures follow the top-level functions, in (class, method)
        // order, each with an implicit leading `this` parameter.
        self.methods_by_class = vec![HashMap::new(); program.classes.len()];
        for (class_idx, class) in program.classes.iter().enumerate() {
            for m in &class.methods {
                let name = self.text(&m.name).to_string();
                let receiver_ty = match self.text(&class.name) {
                    "String" => Ty::String,
                    "Number" => Ty::F64,
                    "Array" => Ty::Array,
                    _ => Ty::Instance(class_idx as u32),
                };
                let mut params = vec![receiver_ty];
                for p in &m.params {
                    params.push(match &p.ty {
                        Some(t) => self.resolve_annotation(t),
                        None => {
                            self.error(p.name.span, "methods require a type on every parameter");
                            Ty::Error
                        }
                    });
                }
                let return_ty = match &m.return_type {
                    Some(t) => self.resolve_annotation(t),
                    None => Ty::Void,
                };
                let func_index = self.signatures.len() as u32;
                if self.methods_by_class[class_idx]
                    .insert(name.clone(), func_index)
                    .is_some()
                {
                    self.error(m.name.span, format!("duplicate method `{name}`"));
                }
                self.signatures.push(Signature {
                    name,
                    params,
                    return_ty,
                });
            }
        }
    }

    /// Check a function or method body. `this_class` is `Some` for methods,
    /// which bind `this` as the first local slot.
    fn check_function(
        &mut self,
        index: usize,
        f: &ast::Function,
        this_class: Option<u32>,
    ) -> hir::Function {
        let sig = &self.signatures[index];
        let return_ty = sig.return_ty;
        let param_tys = sig.params.clone();
        let name = sig.name.clone();

        let mut cx = FunctionCx {
            locals: Vec::new(),
            scopes: vec![Scope::new()],
            return_ty,
            loop_depth: 0,
            mutated: HashSet::new(),
            mutated_names: HashSet::new(),
            boxed: HashSet::new(),
            captures: HashMap::new(),
        };
        // For a method, slot 0 is the implicit `this`; the declared params
        // follow it. For a free function, the params occupy every slot.
        let mut param_tys_iter = param_tys.iter();
        if this_class.is_some() {
            let this_ty = *param_tys_iter.next().expect("method has a this param");
            cx.declare("this", this_ty);
        }
        for (p, &ty) in f.params.iter().zip(param_tys_iter) {
            let pname = self.text(&p.name).to_string();
            cx.declare(&pname, ty);
        }
        self.collect_mutated_names(&f.body.statements, &mut cx.mutated_names);

        let body = if f.is_extern {
            let Some((intrinsic_id, intrinsic)) = self.imports.get(&name).copied() else {
                self.error(
                    f.name.span,
                    format!("extern method `{name}` must be bound to an imported intrinsic"),
                );
                return hir::Function {
                    name,
                    param_count: param_tys.len() as u16,
                    locals: cx.locals,
                    return_ty,
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
            body
        };
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
                let mut init = self.check_expr(cx, init);
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
                        declared
                    }
                    None => init.ty,
                };
                let name_text = self.text(name).to_string();
                let slot = cx.declare(&name_text, var_ty);
                if cx.boxed.contains(&slot) {
                    hir::Stmt::BoxedLet { slot, init }
                } else {
                    hir::Stmt::Let { slot, init }
                }
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
                cx.loop_depth += 1;
                let body = self.check_block(cx, body);
                cx.loop_depth -= 1;
                hir::Stmt::While { cond, body }
            }
            ast::Stmt::Break { span } => {
                if cx.loop_depth == 0 {
                    self.error(*span, "`break` is only valid inside a loop");
                }
                hir::Stmt::Break
            }
            ast::Stmt::Continue { span } => {
                if cx.loop_depth == 0 {
                    self.error(*span, "`continue` is only valid inside a loop");
                }
                hir::Stmt::Continue
            }
            ast::Stmt::Return { value, span } => {
                let value = value.as_ref().map(|v| {
                    let mut e = self.check_expr(cx, v);
                    coerce_numeric_literal(&mut e, cx.return_ty);
                    e
                });
                match (&value, cx.return_ty) {
                    (None, Ty::Void) => {}
                    (None, expected) if !expected.is_error() => {
                        self.error(*span, format!("expected a {expected:?} return value"));
                    }
                    (Some(v), expected) => {
                        if !expected.is_error()
                            && !v.ty.is_error()
                            && !self.is_assignable(v.ty, expected)
                        {
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
            ast::Stmt::Throw { value, .. } => hir::Stmt::Throw(self.check_expr(cx, value)),
            ast::Stmt::Try {
                body,
                catches,
                finally,
                ..
            } => {
                let body = self.check_block(cx, body);
                let catches = catches
                    .iter()
                    .map(|catch| {
                        let ty = self.resolve_annotation(&catch.ty);
                        cx.scopes.push(Scope::new());
                        let slot = cx.declare(self.text(&catch.binding), ty);
                        let body = self.check_block(cx, &catch.body);
                        cx.scopes.pop();
                        hir::CatchClause { slot, ty, body }
                    })
                    .collect();
                let finally_body = finally.as_ref().map(|block| self.check_block(cx, block));
                hir::Stmt::Try {
                    body,
                    catches,
                    finally_body,
                }
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
            ast::Expr::Null { .. } => {
                self.error(span, "`null` is not supported in phase 1");
                self.poison(span)
            }
            ast::Expr::Ident(name) => {
                let text = self.text(name).to_string();
                match cx.lookup(&text) {
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
                        self.error(span, format!("unknown variable `{text}`"));
                        self.poison(span)
                    }
                }
            }
            ast::Expr::Assign { target, value, .. } => {
                let mut value = self.check_expr(cx, value);
                let text = self.text(target).to_string();
                match cx.lookup(&text) {
                    Some(slot) => {
                        cx.mutated.insert(slot);
                        let slot_ty = cx.locals[slot as usize];
                        coerce_numeric_literal(&mut value, slot_ty);
                        if !slot_ty.is_error()
                            && !value.ty.is_error()
                            && !self.is_assignable(value.ty, slot_ty)
                        {
                            self.error(
                                value.span,
                                format!("cannot assign {:?} to {slot_ty:?}", value.ty),
                            );
                        }
                        hir::Expr {
                            ty: slot_ty,
                            kind: if cx.boxed.contains(&slot) {
                                hir::ExprKind::CellStore {
                                    slot,
                                    value: Box::new(value),
                                }
                            } else {
                                hir::ExprKind::Assign {
                                    slot,
                                    value: Box::new(value),
                                }
                            },
                            span,
                        }
                    }
                    None => {
                        if let Some(capture) = cx.captures.get(&text).cloned() {
                            cx.mutated.insert(capture.outer_slot);
                            if capture.mode == hir::CaptureMode::ByValue {
                                self.error(
                                    target.span,
                                    "cannot assign to a read-only closure capture",
                                );
                                return self.poison(span);
                            }
                            return hir::Expr {
                                kind: hir::ExprKind::CaptureStore {
                                    environment_slot: capture.environment_slot,
                                    ty: capture.ty,
                                    mode: capture.mode,
                                    value: Box::new(value),
                                },
                                ty: capture.ty,
                                span,
                            };
                        }
                        self.error(target.span, format!("unknown variable `{text}`"));
                        self.poison(span)
                    }
                }
            }
            ast::Expr::Binary { op, lhs, rhs, .. } => {
                let mut lhs = self.check_expr(cx, lhs);
                let mut rhs = self.check_expr(cx, rhs);
                // Let an integral number literal take on an Int sibling's type
                // so `x + 1` type-checks when `x: int` (and vice versa).
                unify_numeric_literal(&mut lhs, &mut rhs);
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
                // Fold `-<number literal>` into a single negative number literal
                // so it stays coercible to an int context (e.g. `abs(-7)`).
                if let ast::UnaryOp::Neg = op {
                    if let ast::Expr::Number { value, .. } = operand.as_ref() {
                        return hir::Expr {
                            kind: hir::ExprKind::Number(-value),
                            ty: Ty::F64,
                            span,
                        };
                    }
                }
                let operand = self.check_expr(cx, operand);
                let ty = match op {
                    // Negation keeps the operand's numeric type; `!` needs bool.
                    ast::UnaryOp::Neg if operand.ty.is_int() || operand.ty.is_float() => operand.ty,
                    ast::UnaryOp::Neg => {
                        self.error(operand.span, format!("cannot negate {:?}", operand.ty));
                        Ty::Error
                    }
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
                let mut args: Vec<hir::Expr> =
                    args.iter().map(|a| self.check_expr(cx, a)).collect();

                if let Some(slot) = cx.lookup(&name) {
                    let function_ty = cx.locals[slot as usize];
                    if let Ty::Function(func) = function_ty {
                        let return_ty = if func == hir::DYNAMIC_FUNCTION {
                            if args.len() != 1 {
                                self.error(span, format!("generic Function callback expects exactly one argument, found {}", args.len()));
                            }
                            Ty::Any
                        } else if func == hir::DYNAMIC_PREDICATE {
                            if args.len() != 1 {
                                self.error(span, format!("generic Predicate callback expects exactly one argument, found {}", args.len()));
                            }
                            Ty::Boolean
                        } else if func == hir::DYNAMIC_REDUCER {
                            if args.len() != 2 {
                                self.error(span, format!("generic Reducer callback expects exactly two arguments, found {}", args.len()));
                            }
                            Ty::Any
                        } else {
                            let (param_tys, return_ty) = {
                                let sig = &self.signatures[func as usize];
                                (sig.params.clone(), sig.return_ty)
                            };
                            if args.len() != param_tys.len() {
                                self.error(
                                    span,
                                    format!(
                                        "function value expects {} argument(s), found {}",
                                        param_tys.len(),
                                        args.len()
                                    ),
                                );
                            } else {
                                for (arg, &expected) in args.iter_mut().zip(&param_tys) {
                                    coerce_numeric_literal(arg, expected);
                                    if !expected.is_error()
                                        && !arg.ty.is_error()
                                        && !self.is_assignable(arg.ty, expected)
                                    {
                                        self.error(
                                            arg.span,
                                            format!(
                                                "argument type {:?} does not match {expected:?}",
                                                arg.ty
                                            ),
                                        );
                                    }
                                }
                            }
                            return_ty
                        };
                        let callee_expr = hir::Expr {
                            kind: hir::ExprKind::Local(slot),
                            ty: function_ty,
                            span,
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::IndirectCall {
                                callee: Box::new(callee_expr),
                                args,
                                func,
                            },
                            ty: return_ty,
                            span,
                        };
                    }
                }

                // `print` is the one implicit intrinsic: one argument of any
                // primitive type, returns void.
                if name == "print" {
                    if args.len() != 1 {
                        self.error(span, "print takes exactly one argument");
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id: zun_natives::PRINT,
                            args,
                        },
                        ty: Ty::Void,
                        span,
                    };
                }

                if name == "dynamic_new" {
                    if !args.is_empty() {
                        self.error(span, "dynamic_new takes no arguments");
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::DynamicNew,
                        ty: Ty::DynamicObject,
                        span,
                    };
                }

                if name == "box_any" {
                    if args.len() != 1 {
                        self.error(span, "box_any takes exactly one argument");
                        return self.poison(span);
                    }
                    let value = args.pop().unwrap();
                    return hir::Expr {
                        kind: hir::ExprKind::BoxAny {
                            value: Box::new(value),
                        },
                        ty: Ty::Any,
                        span,
                    };
                }

                let dynamic_property = |checker: &mut Checker<'_>,
                                        args: &mut Vec<hir::Expr>,
                                        span: Span| {
                    if args.len() != 2
                        || args[0].ty != Ty::DynamicObject
                        || args[1].ty != Ty::String
                    {
                        checker.error(span, "dynamic property access expects (dynamic, string)");
                        return None;
                    }
                    let name = match &args[1].kind {
                        hir::ExprKind::String(name) => name.clone(),
                        _ => {
                            checker.error(
                                args[1].span,
                                "dynamic property name must be a string literal",
                            );
                            return None;
                        }
                    };
                    Some((args.remove(0), name))
                };

                if name == "dynamic_get" {
                    let Some((object, property)) = dynamic_property(self, &mut args, span) else {
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::DynamicGet {
                            object: Box::new(object),
                            name: property,
                        },
                        ty: Ty::Any,
                        span,
                    };
                }

                if name == "dynamic_set" {
                    if args.len() != 3
                        || args[0].ty != Ty::DynamicObject
                        || args[1].ty != Ty::String
                        || args[2].ty != Ty::Any
                    {
                        self.error(
                            span,
                            "dynamic property assignment expects (dynamic, string, any)",
                        );
                        return self.poison(span);
                    }
                    let property = match &args[1].kind {
                        hir::ExprKind::String(name) => name.clone(),
                        _ => {
                            self.error(
                                args[1].span,
                                "dynamic property name must be a string literal",
                            );
                            return self.poison(span);
                        }
                    };
                    let object = args.remove(0);
                    args.remove(0);
                    let value = args.remove(0);
                    return hir::Expr {
                        kind: hir::ExprKind::DynamicSet {
                            object: Box::new(object),
                            name: property,
                            value: Box::new(value),
                        },
                        ty: Ty::Any,
                        span,
                    };
                }

                // An imported intrinsic: check against its native signature.
                if let Some((intrinsic_id, intrinsic)) = self.imports.get(&name).copied() {
                    let param_tys: Vec<Ty> =
                        intrinsic.params.iter().map(|&t| native_ty(t)).collect();
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
                        for (arg, &expected) in args.iter_mut().zip(&param_tys) {
                            coerce_numeric_literal(arg, expected);
                            if !expected.is_error()
                                && !arg.ty.is_error()
                                && !self.is_assignable(arg.ty, expected)
                            {
                                self.error(
                                    arg.span,
                                    format!(
                                        "argument type {:?} does not match {expected:?}",
                                        arg.ty
                                    ),
                                );
                            }
                        }
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id: intrinsic_id,
                            args,
                        },
                        ty: native_ty(intrinsic.ret),
                        span,
                    };
                }

                if let Some(external) = self.external_functions.get(&name).cloned() {
                    if args.len() != external.params.len() {
                        self.error(
                            span,
                            format!(
                                "`{name}` expects {} argument(s), found {}",
                                external.params.len(),
                                args.len()
                            ),
                        );
                    } else {
                        for (arg, &expected) in args.iter_mut().zip(&external.params) {
                            coerce_numeric_literal(arg, expected);
                            if !expected.is_error()
                                && !arg.ty.is_error()
                                && !self.is_assignable(arg.ty, expected)
                            {
                                self.error(
                                    arg.span,
                                    format!(
                                        "argument type {:?} does not match {expected:?}",
                                        arg.ty
                                    ),
                                );
                            }
                        }
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::ExternalCall {
                            module: external.module,
                            name: external.name,
                            args,
                        },
                        ty: external.return_ty,
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
                    for (arg, &expected) in args.iter_mut().zip(&param_tys) {
                        coerce_numeric_literal(arg, expected);
                        if !expected.is_error()
                            && !arg.ty.is_error()
                            && !self.is_assignable(arg.ty, expected)
                        {
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
            ast::Expr::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                let receiver = self.check_expr(cx, receiver);
                let mname = self.text(method).to_string();
                if receiver.ty == Ty::Array && mname == "pop" {
                    if !args.is_empty() {
                        self.error(span, "Array.pop expects no arguments");
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::ArrayPop {
                            object: Box::new(receiver),
                        },
                        ty: Ty::Any,
                        span,
                    };
                }
                if receiver.ty == Ty::Array && mname == "push" {
                    if args.len() != 1 {
                        self.error(span, "Array.push expects one argument");
                        return self.poison(span);
                    }
                    let value = self.check_expr(cx, &args[0]);
                    return hir::Expr {
                        kind: hir::ExprKind::ArrayPush {
                            object: Box::new(receiver),
                            value: Box::new(value),
                        },
                        ty: Ty::Int(IntKind::U64),
                        span,
                    };
                }
                let class = match receiver.ty {
                    Ty::String => match self.classes_by_name.get("String") {
                        Some(&class) => class,
                        None => {
                            self.error(
                                receiver.span,
                                "String methods are not available without class String",
                            );
                            return self.poison(span);
                        }
                    },
                    Ty::F64 => match self.classes_by_name.get("Number") {
                        Some(&class) => class,
                        None => {
                            self.error(
                                receiver.span,
                                "Number methods are not available without class Number",
                            );
                            return self.poison(span);
                        }
                    },
                    Ty::Array => match self.classes_by_name.get("Array") {
                        Some(&class) => class,
                        None => {
                            self.error(
                                receiver.span,
                                "Array methods are not available without class Array",
                            );
                            return self.poison(span);
                        }
                    },
                    Ty::Instance(c) => c,
                    Ty::Error => return self.poison(span),
                    other => {
                        self.error(receiver.span, format!("type {other:?} has no methods"));
                        return self.poison(span);
                    }
                };
                let Some(func) = self.method_of(class, &mname) else {
                    self.error(method.span, format!("no method `{mname}` on this type"));
                    return self.poison(span);
                };
                let (param_tys, return_ty) = {
                    let sig = &self.signatures[func as usize];
                    (sig.params.clone(), sig.return_ty)
                };
                // param_tys[0] is `this`; the remaining are the declared params.
                let mut call_args = vec![receiver];
                for a in args {
                    call_args.push(self.check_expr(cx, a));
                }
                if call_args.len() != param_tys.len() {
                    self.error(
                        span,
                        format!(
                            "`{mname}` expects {} argument(s), found {}",
                            param_tys.len() - 1,
                            call_args.len() - 1
                        ),
                    );
                } else {
                    for (arg, &expected) in call_args.iter_mut().zip(&param_tys).skip(1) {
                        coerce_numeric_literal(arg, expected);
                        if !expected.is_error()
                            && !arg.ty.is_error()
                            && !self.is_assignable(arg.ty, expected)
                        {
                            self.error(
                                arg.span,
                                format!("argument type {:?} does not match {expected:?}", arg.ty),
                            );
                        }
                    }
                }
                hir::Expr {
                    kind: hir::ExprKind::MethodCall {
                        func,
                        args: call_args,
                    },
                    ty: return_ty,
                    span,
                }
            }
            ast::Expr::Cast { expr, ty, .. } => {
                let value = self.check_expr(cx, expr);
                let target = self.resolve_annotation(ty);
                // Numeric conversions may also explicitly unbox an `any` value.
                let numeric = |t: Ty| t.is_int() || t.is_float();
                if !value.ty.is_error()
                    && !((numeric(value.ty) && numeric(target))
                        || (value.ty == Ty::Any && (numeric(target) || target == Ty::Array)))
                {
                    self.error(span, format!("cannot convert {:?} to {target:?}", value.ty));
                    return self.poison(span);
                }
                hir::Expr {
                    kind: hir::ExprKind::Convert {
                        value: Box::new(value),
                    },
                    ty: target,
                    span,
                }
            }
            ast::Expr::New { class, args, .. } => {
                let name = self.text(class).to_string();
                if name == "Array" || name == "array" {
                    if args.len() != 1 {
                        self.error(span, "`Array` expects one length argument");
                        return self.poison(span);
                    }
                    let mut length = self.check_expr(cx, &args[0]);
                    coerce_numeric_literal(&mut length, Ty::INT);
                    if !length.ty.is_int() {
                        self.error(length.span, "array length must be an integer");
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::ArrayNew {
                            length: Box::new(length),
                        },
                        ty: Ty::Array,
                        span,
                    };
                }
                let Some(&class_idx) = self.classes_by_name.get(&name) else {
                    self.error(class.span, format!("unknown class `{name}`"));
                    return self.poison(span);
                };
                let field_tys: Vec<Ty> = self
                    .full_fields(class_idx)
                    .into_iter()
                    .map(|(_, ty)| ty)
                    .collect();
                let mut args: Vec<hir::Expr> =
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
                    for (arg, &expected) in args.iter_mut().zip(&field_tys) {
                        coerce_numeric_literal(arg, expected);
                        if !expected.is_error()
                            && !arg.ty.is_error()
                            && !self.is_assignable(arg.ty, expected)
                        {
                            self.error(
                                arg.span,
                                format!(
                                    "field initializer type {:?} does not match {expected:?}",
                                    arg.ty
                                ),
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
                if object.ty == Ty::Array && fname == "length" {
                    return hir::Expr {
                        kind: hir::ExprKind::ArrayLength {
                            object: Box::new(object),
                        },
                        ty: Ty::Int(IntKind::U64),
                        span,
                    };
                }
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
            ast::Expr::Index { object, index, .. } => {
                let object = self.check_expr(cx, object);
                let mut index = self.check_expr(cx, index);
                coerce_numeric_literal(&mut index, Ty::INT);
                if object.ty != Ty::Array && !object.ty.is_error() {
                    self.error(object.span, "indexing requires an Array");
                }
                if !index.ty.is_int() && !index.ty.is_error() {
                    self.error(index.span, "array index must be an integer");
                }
                hir::Expr {
                    kind: hir::ExprKind::ArrayGet {
                        object: Box::new(object),
                        index: Box::new(index),
                    },
                    ty: Ty::Any,
                    span,
                }
            }
            ast::Expr::IndexAssign {
                object,
                index,
                value,
                ..
            } => {
                let object = self.check_expr(cx, object);
                let mut index = self.check_expr(cx, index);
                let value = self.check_expr(cx, value);
                let value_ty = value.ty;
                coerce_numeric_literal(&mut index, Ty::INT);
                if object.ty != Ty::Array && !object.ty.is_error() {
                    self.error(object.span, "indexing requires an Array");
                }
                if !index.ty.is_int() && !index.ty.is_error() {
                    self.error(index.span, "array index must be an integer");
                }
                hir::Expr {
                    kind: hir::ExprKind::ArraySet {
                        object: Box::new(object),
                        index: Box::new(index),
                        value: Box::new(value),
                    },
                    ty: value_ty,
                    span,
                }
            }
            ast::Expr::FieldAssign {
                object,
                field,
                value,
                ..
            } => {
                let object = self.check_expr(cx, object);
                let mut value = self.check_expr(cx, value);
                let fname = self.text(field).to_string();
                match object.ty {
                    Ty::Instance(class) => match self.field_of(class, &fname) {
                        Some((slot, ty)) => {
                            coerce_numeric_literal(&mut value, ty);
                            if !ty.is_error()
                                && !value.ty.is_error()
                                && !self.is_assignable(value.ty, ty)
                            {
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
            ast::Expr::Function(function) => self.check_closure_expr(cx, function, span),
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
        // Both numeric operands must be the same numeric type (no implicit
        // int/float mixing; conversions are explicit — PLAN.md §4.2).
        // Both operands must be the same numeric type, exact width included.
        let same_numeric = |a: Ty, b: Ty| {
            matches!((a, b),
            (Ty::Int(x), Ty::Int(y)) if x == y)
                || matches!((a, b), (Ty::Float(x), Ty::Float(y)) if x == y)
        };
        match op {
            Add => {
                // `+` is numeric addition or string concatenation.
                if same_numeric(lhs.ty, rhs.ty) {
                    lhs.ty
                } else if lhs.ty == Ty::String && rhs.ty == Ty::String {
                    Ty::String
                } else {
                    self.error(span, format!("cannot add {:?} and {:?}", lhs.ty, rhs.ty));
                    Ty::Error
                }
            }
            Sub | Mul | Div | Rem => {
                if same_numeric(lhs.ty, rhs.ty) {
                    lhs.ty
                } else {
                    self.error(
                        span,
                        format!(
                            "arithmetic requires matching numbers, found {:?} and {:?}",
                            lhs.ty, rhs.ty
                        ),
                    );
                    Ty::Error
                }
            }
            Lt | Le | Gt | Ge => {
                if same_numeric(lhs.ty, rhs.ty) {
                    Ty::Boolean
                } else {
                    self.error(
                        span,
                        format!(
                            "comparison requires matching numbers, found {:?} and {:?}",
                            lhs.ty, rhs.ty
                        ),
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

/// Map an intrinsic signature type to the checker's `Ty`.
fn native_ty(t: zun_natives::NativeTy) -> Ty {
    use zun_natives::NativeTy as N;
    match t {
        N::I8 => Ty::Int(IntKind::I8),
        N::I16 => Ty::Int(IntKind::I16),
        N::I32 => Ty::Int(IntKind::I32),
        N::I64 => Ty::Int(IntKind::I64),
        N::U8 => Ty::Int(IntKind::U8),
        N::U16 => Ty::Int(IntKind::U16),
        N::U32 => Ty::Int(IntKind::U32),
        N::U64 => Ty::Int(IntKind::U64),
        N::F32 => Ty::F32,
        N::F64 => Ty::F64,
        N::Bool => Ty::Boolean,
        N::Str => Ty::String,
        N::Void => Ty::Void,
        N::Opaque => Ty::Any,
    }
}

/// Whether `expr` is a default (f64) number literal — a candidate to adopt a
/// different numeric type from its context.
fn is_number_literal(expr: &hir::Expr) -> bool {
    matches!(expr.kind, hir::ExprKind::Number(_)) && expr.ty == Ty::F64
}

/// Whether `v` fits exactly in the integer kind `k`.
fn fits_int(v: f64, k: IntKind) -> bool {
    if v.fract() != 0.0 {
        return false;
    }
    let (lo, hi) = int_range(k);
    v >= lo && v <= hi
}

/// Inclusive `(min, max)` of an integer kind, as f64 (exact up to 2^53).
fn int_range(k: IntKind) -> (f64, f64) {
    if k.signed {
        let hi = 2f64.powi(k.bits as i32 - 1);
        (-hi, hi - 1.0)
    } else {
        (0.0, 2f64.powi(k.bits as i32) - 1.0)
    }
}

/// Retype a number literal to `expected` when that is a numeric type the
/// literal can represent: any float, or an integer kind the value fits.
fn coerce_numeric_literal(expr: &mut hir::Expr, expected: Ty) {
    if !is_number_literal(expr) {
        return;
    }
    let v = match expr.kind {
        hir::ExprKind::Number(v) => v,
        _ => return,
    };
    match expected {
        Ty::Float(_) => expr.ty = expected,
        Ty::Int(k) if fits_int(v, k) => expr.ty = expected,
        _ => {}
    }
}

/// If one side of a binary op has a concrete numeric type and the other is a
/// default number literal that can take it, retype the literal to match — so
/// `x + 1` checks when `x: u8` (and `1 + x`).
fn unify_numeric_literal(a: &mut hir::Expr, b: &mut hir::Expr) {
    if (a.ty.is_int() || a.ty.is_float()) && is_number_literal(b) {
        coerce_numeric_literal(b, a.ty);
    } else if (b.ty.is_int() || b.ty.is_float()) && is_number_literal(a) {
        coerce_numeric_literal(a, b.ty);
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
        assert_eq!(add.return_ty, Ty::F64);
    }

    #[test]
    fn let_infers_type_from_initializer() {
        let hir = check_ok("function f() { let x = 1; let s = \"hi\"; }");
        assert_eq!(hir.functions[0].locals, vec![Ty::F64, Ty::String]);
    }

    #[test]
    fn shadowing_gets_a_fresh_slot() {
        let hir =
            check_ok("function f() { let x = 1; if (x > 0) { let x = \"inner\"; print(x); } }");
        assert_eq!(hir.functions[0].locals, vec![Ty::F64, Ty::String]);
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
        let msg = first_error("function g(n: number) {} function f() { g(\"s\"); }");
        assert!(msg.contains("does not match"), "{msg}");
    }

    #[test]
    fn wrong_return_type_diagnoses() {
        assert!(first_error("function f(): number { return \"s\"; }").contains("does not match"));
    }

    #[test]
    fn inner_scope_names_do_not_leak() {
        let msg = first_error("function f(x: boolean) { if (x) { let inner = 1; } print(inner); }");
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
    fn methods_check_and_this_resolves_fields() {
        let hir = check_ok(
            "class Counter {\n\
                n: number;\n\
                inc() { this.n = this.n + 1; }\n\
                get(): number { return this.n; }\n\
             }\n\
             function main() {\n\
                let c = new Counter(0);\n\
                c.inc(); c.inc();\n\
                print(c.get());\n\
             }",
        );
        // 1 top-level function + 2 methods.
        assert_eq!(hir.functions.len(), 3);
    }

    #[test]
    fn method_with_params_checks_arity() {
        let msg = first_error(
            "class Adder { base: number; add(x: number): number { return this.base + x; } }\n\
             function main() { let a = new Adder(10); print(a.add()); }",
        );
        assert!(msg.contains("argument(s)"), "{msg}");
    }

    #[test]
    fn unknown_method_diagnoses() {
        let msg = first_error(
            "class C { x: number; }\n\
             function main() { let c = new C(1); c.nope(); }",
        );
        assert!(msg.contains("no method"), "{msg}");
    }

    #[test]
    fn inherited_fields_and_methods_resolve() {
        check_ok(
            "class Animal { legs: number; describe(): number { return this.legs; } }\n\
             class Dog extends Animal { tailLen: number; }\n\
             function main() {\n\
                let d = new Dog(4, 20);\n\
                print(d.describe());\n\
                print(d.tailLen);\n\
             }",
        );
    }

    #[test]
    fn subclass_is_assignable_to_base() {
        check_ok(
            "class Shape { sides: number; }\n\
             class Square extends Shape { size: number; }\n\
             function area(s: Shape): number { return s.sides; }\n\
             function main() { let sq = new Square(4, 10); print(area(sq)); }",
        );
    }

    #[test]
    fn base_is_not_assignable_to_subclass() {
        let msg = first_error(
            "class Shape { sides: number; }\n\
             class Square extends Shape { size: number; }\n\
             function f(sq: Square) {}\n\
             function main() { let s = new Shape(4); f(s); }",
        );
        assert!(msg.contains("does not match"), "{msg}");
    }

    #[test]
    fn unknown_base_class_diagnoses() {
        let msg = first_error("class Dog extends Missing { n: number; }");
        assert!(msg.contains("unknown base class"), "{msg}");
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
    fn int_and_float_are_distinct() {
        let hir = check_ok("function main() { let a: int = 1; let b: float = 1.0; let c = 2; }");
        // a:int, b:float(Number), c defaults to Number.
        assert_eq!(hir.functions[0].locals, vec![Ty::INT, Ty::F64, Ty::F64]);
    }

    #[test]
    fn int_arithmetic_stays_int() {
        let hir = check_ok("function f(): int { let x: int = 10; return x * 2 + 5; }");
        assert_eq!(hir.functions[0].return_ty, Ty::INT);
    }

    #[test]
    fn mixing_int_and_float_is_an_error() {
        let msg =
            first_error("function f() { let x: int = 1; let y: float = 2.0; let z = x + y; }");
        assert!(
            msg.contains("cannot add") || msg.contains("matching"),
            "{msg}"
        );
    }

    #[test]
    fn non_integral_literal_not_coerced_to_int() {
        let msg = first_error("function f() { let x: int = 1.5; }");
        assert!(msg.contains("does not match"), "{msg}");
    }

    #[test]
    fn int_param_accepts_integer_literal() {
        check_ok(
            "function inc(n: int): int { return n + 1; }\n\
             function main() { print(inc(41)); }",
        );
    }

    #[test]
    fn any_is_an_explicit_dynamic_type() {
        let hir = check_ok("function identity(value: any): any { return value; }");
        assert_eq!(hir.functions[0].param_count, 1);
        assert_eq!(hir.functions[0].return_ty, Ty::Any);
    }

    #[test]
    fn array_is_a_managed_core_type() {
        let hir = check_ok("function identity(values: Array): Array { return values; }");
        assert_eq!(hir.functions[0].locals, vec![Ty::Array]);
        assert_eq!(hir.functions[0].return_ty, Ty::Array);
    }

    #[test]
    fn array_constructor_checks_integer_length() {
        let hir = check_ok("function main(): Array { return new Array(2); }");
        assert_eq!(hir.functions[0].return_ty, Ty::Array);
    }

    #[test]
    fn rejects_incompatible_named_function_callback() {
        let msg = first_error(
            "function takes_text(value: string): number { return 1.0; }\n\
             function apply(f: takes_text, value: number): number { return f(value); }",
        );
        assert!(msg.contains("does not match"), "{msg}");
    }

    #[test]
    fn rejects_named_function_callback_with_incompatible_return() {
        let msg = first_error(
            "function returns_text(value: number): string { return \"value\"; }\n\
             function apply(f: returns_text, value: number): number { return f(value); }",
        );
        assert!(
            msg.contains("return type") || msg.contains("does not match"),
            "{msg}"
        );
    }

    #[test]
    fn any_does_not_implicitly_enable_arithmetic() {
        let msg = first_error("function f(value: any) { let result = value + 1; }");
        assert!(msg.contains("cannot add"), "{msg}");
    }

    #[test]
    fn dynamic_property_builtins_are_type_checked() {
        let hir = check_ok(
            "function main(): any {\n\
                let object = dynamic_new();\n\
                dynamic_set(object, \"answer\", box_any(41));\n\
                return dynamic_get(object, \"answer\");\n\
             }",
        );
        assert_eq!(hir.functions[0].return_ty, Ty::Any);
        assert_eq!(hir.functions[0].locals, vec![Ty::DynamicObject]);
    }

    #[test]
    fn checks_external_function_call_from_linker_signature() {
        let source = "import { answer } from \"core\"; function main(): int { return answer(41); }";
        let mut diagnostics = Vec::new();
        let ast = parse(source, &mut diagnostics);
        let hir = check_with_externals(
            source,
            &ast,
            &mut diagnostics,
            &[ExternalFunction {
                module: "core".into(),
                name: "answer".into(),
                params: vec![Ty::INT],
                return_ty: Ty::INT,
            }],
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let hir::Stmt::Return(Some(expr)) = &hir.functions[0].body[0] else {
            panic!("expected return statement")
        };
        assert!(matches!(
            expr.kind,
            hir::ExprKind::ExternalCall { ref module, ref name, .. }
                if module == "core" && name == "answer"
        ));
        assert_eq!(expr.ty, Ty::INT);
    }

    #[test]
    fn error_does_not_cascade() {
        // `y` is unknown; the addition and the assignment must not pile on
        // extra diagnostics.
        let (_, diags) = check_src("function f() { let x = y + 1; }");
        assert_eq!(diags.len(), 1, "{diags:?}");
    }

    #[test]
    fn extern_method_requires_an_explicit_intrinsic_binding() {
        let (_, diagnostics) = check_src(
            "class Service { extern missing(): int; }\nfunction main(): int { return 0; }",
        );
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic
                .message
                .contains("must be bound to an imported intrinsic")),
            "{diagnostics:?}"
        );
    }
}
