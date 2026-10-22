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

#[path = "closures.rs"]
mod closures;
#[path = "class_layout.rs"]
mod class_layout;
#[path = "classes.rs"]
mod classes;
#[path = "conditions.rs"]
mod conditions;
#[path = "blocks.rs"]
mod blocks;
#[path = "default_args.rs"]
mod default_args;
#[path = "diagnostics.rs"]
mod diagnostics;
#[path = "module_resolution.rs"]
mod module_resolution;
#[path = "native_types.rs"]
mod native_types;
#[path = "numeric_literals.rs"]
mod numeric_literals;
#[path = "function_context.rs"]
mod function_context;
#[path = "boxed_locals.rs"]
mod boxed_locals;
#[path = "capture_analysis.rs"]
mod capture_analysis;
#[path = "poison.rs"]
mod poison;
#[path = "type_expect.rs"]
mod type_expect;

use numeric_literals::{coerce_numeric_literal, unify_numeric_literal};
#[path = "types.rs"]
mod types;

use native_types::native_ty;

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
    let mut external_by_name: HashMap<String, ExternalFunction> = HashMap::new();
    for function in external_functions {
        if let Some(previous) = external_by_name.get(&function.name) {
            if previous.module != function.module {
                diagnostics.push(
                    Diagnostic::error(format!(
                        "external symbol `{}` is ambiguous between `{}` and `{}`",
                        function.name, previous.module, function.module
                    ))
                    .with_code("SEM0001"),
                );
                continue;
            }
        }
        external_by_name.insert(function.name.clone(), function.clone());
    }
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
        type_aliases: HashMap::new(),
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
            functions.push(checker.check_function(
                next,
                m,
                (!m.is_static).then_some(class_idx as u32),
            ));
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
    optional_count: usize,
    rest_param: Option<usize>,
    is_generator: bool,
    is_static: bool,
}

/// A collected class: its base and its own (not inherited) fields.
struct ClassInfo {
    base: Option<u32>,
    base_name: Option<String>,
    own_fields: Vec<(String, Ty, bool, bool)>,
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
    /// Imported default type bindings over the frozen runtime floor.
    type_aliases: HashMap<String, Ty>,
    closures: Vec<hir::ClosureFunction>,
}

/// One lexical scope: name -> local slot.
type Scope = HashMap<String, hir::LocalSlot>;

/// Per-function checking state.
struct FunctionCx {
    locals: Vec<Ty>,
    scopes: Vec<Scope>,
    return_ty: Ty,
    is_generator: bool,
    loop_depth: usize,
    loop_labels: Vec<String>,
    mutated: HashSet<hir::LocalSlot>,
    mutated_names: HashSet<String>,
    boxed: HashSet<hir::LocalSlot>,
    captures: HashMap<String, hir::Capture>,
    this_class: Option<u32>,
}

#[derive(Default)]
struct CaptureUse {
    slots: Vec<hir::LocalSlot>,
    mutated: HashSet<hir::LocalSlot>,
    inherited: Vec<hir::Capture>,
}

impl Checker<'_> {
    fn rewrite_boxed_lets(&self, statements: &mut [hir::Stmt], boxed: &HashSet<hir::LocalSlot>) {
        boxed_locals::rewrite_boxed_lets(statements, boxed);
    }

    fn collect_mutated_names(&self, statements: &[ast::Stmt], names: &mut HashSet<String>) {
        capture_analysis::collect_mutated_names(self, statements, names);
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
                optional_count: f
                    .params
                    .iter()
                    .rev()
                    .take_while(|p| p.default_value.is_some())
                    .count(),
                rest_param: f.params.iter().position(|parameter| parameter.is_rest),
                is_generator: f.is_generator,
                is_static: false,
            });
        }

        // Method signatures follow the top-level functions, in (class, method)
        // order. Instance methods have an implicit leading `this`; static
        // methods intentionally do not.
        self.methods_by_class = vec![HashMap::new(); program.classes.len()];
        for (class_idx, class) in program.classes.iter().enumerate() {
            for m in &class.methods {
                let base_name = self.text(&m.name).to_string();
                let name = match m.accessor {
                    Some(ast::AccessorKind::Get) => format!("get_{base_name}"),
                    Some(ast::AccessorKind::Set) => format!("set_{base_name}"),
                    None => base_name,
                };
                let receiver_ty = match self.text(&class.name) {
                    "String" => Ty::String,
                    "Number" => Ty::F64,
                    "Array" => Ty::Array,
                    "Map" => Ty::Map,
                    "Set" => Ty::Set,
                    "WeakMap" => Ty::WeakMap,
                    "RegExp" => Ty::RegExp,
                    "BigInt" => Ty::BigInt,
                    "FinalizationRegistry" => Ty::FinalizationRegistry,
                    _ => Ty::Instance(class_idx as u32),
                };
                let mut params = if m.is_static { Vec::new() } else { vec![receiver_ty] };
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
                    optional_count: m
                        .params
                        .iter()
                        .rev()
                        .take_while(|p| p.default_value.is_some())
                        .count(),
                    rest_param: m
                        .params
                        .iter()
                        .position(|parameter| parameter.is_rest)
                        .map(|index| index + 1),
                    is_generator: false,
                    is_static: m.is_static,
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
        let mut saw_default = false;
        for (index, parameter) in f.params.iter().enumerate() {
            if parameter.is_rest && index + 1 != f.params.len() {
                self.error(parameter.name.span, "rest parameter must be last");
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
        let sig = &self.signatures[index];
        let return_ty = sig.return_ty;
        let param_tys = sig.params.clone();
        let name = sig.name.clone();
        if f.is_async {
            if return_ty != Ty::Promise {
                self.error(
                    f.name.span,
                    "async functions must declare a Promise return type",
                );
            }
            // A body without suspension points follows the .NET fast path:
            // execute synchronously and return the already-created Promise.
            // Bodies containing await remain gated until state-machine IL is
            // available, so the front end never emits an invalid suspension.
        }

        let mut cx = FunctionCx {
            locals: Vec::new(),
            scopes: vec![Scope::new()],
            return_ty,
            is_generator: f.is_generator,
            loop_depth: 0,
            loop_labels: Vec::new(),
            mutated: HashSet::new(),
            mutated_names: HashSet::new(),
            boxed: HashSet::new(),
            captures: HashMap::new(),
            this_class,
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
                    format!("extern function `{name}` return type does not match the linker signature"),
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
            body
        };
        let defaults = f
            .params
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                parameter.default_value.as_deref().map(|value| {
                    let mut checked = self.check_expr(&mut cx, value);
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

    fn check_stmt(&mut self, cx: &mut FunctionCx, stmt: &ast::Stmt) -> hir::Stmt {
        match stmt {
            ast::Stmt::ArrayDestructure { names, init, .. } => {
                let init = self.check_expr(cx, init);
                if init.ty != Ty::Array && !init.ty.is_error() {
                    self.error(init.span, "array destructuring requires an Array initializer");
                }
                let slots = names
                    .iter()
                    .map(|name| {
                        let name = self.text(name).to_string();
                        cx.declare(&name, Ty::Any)
                    })
                    .collect();
                hir::Stmt::ArrayDestructure { slots, init }
            }
            ast::Stmt::ObjectDestructure { names, init, .. } => {
                let init = self.check_expr(cx, init);
                if init.ty != Ty::Any
                    && init.ty != Ty::DynamicObject
                    && !init.ty.is_error()
                {
                    self.error(
                        init.span,
                        "object destructuring requires a dynamic object initializer",
                    );
                }
                let bindings = names
                    .iter()
                    .map(|name| {
                        let text = self.text(name).to_string();
                        let slot = cx.declare(&text, Ty::Any);
                        (slot, text)
                    })
                    .collect();
                hir::Stmt::ObjectDestructure { bindings, init }
            }
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
            ast::Stmt::DoWhile { body, cond, .. } => {
                cx.loop_depth += 1;
                let body = self.check_block(cx, body);
                cx.loop_depth -= 1;
                let cond = self.check_condition(cx, cond);
                hir::Stmt::DoWhile { body, cond }
            }
            ast::Stmt::For {
                init,
                cond,
                update,
                body,
                ..
            } => {
                let init = init
                    .as_ref()
                    .map(|stmt| Box::new(self.check_stmt(cx, stmt)));
                let cond = cond
                    .as_ref()
                    .map(|expr| self.check_condition(cx, expr))
                    .unwrap_or(hir::Expr {
                        kind: hir::ExprKind::Boolean(true),
                        ty: Ty::Boolean,
                        span: body.span,
                    });
                let update = update.as_ref().map(|expr| self.check_expr(cx, expr));
                cx.loop_depth += 1;
                let body = self.check_block(cx, body);
                cx.loop_depth -= 1;
                hir::Stmt::For {
                    init,
                    cond,
                    update,
                    body,
                }
            }
            ast::Stmt::ForOf {
                name,
                iterable,
                body,
                span: _,
            } => {
                let iterable_expr = self.check_expr(cx, iterable);
                let (new_name, done_name, next_name) = match iterable_expr.ty {
                    Ty::Array => ("arrayIteratorNew", "arrayIteratorDone", "arrayIteratorNext"),
                    Ty::Map => ("mapIteratorNew", "mapIteratorDone", "mapIteratorNext"),
                    Ty::Set => ("setIteratorNew", "setIteratorDone", "setIteratorNext"),
                    Ty::Any => ("generatorIteratorNew", "generatorIteratorDone", "generatorIteratorNext"),
                    Ty::Error => ("arrayIteratorNew", "arrayIteratorDone", "arrayIteratorNext"),
                    other => {
                        self.error(
                            iterable_expr.span,
                            format!("type {other:?} is not iterable"),
                        );
                        ("arrayIteratorNew", "arrayIteratorDone", "arrayIteratorNext")
                    }
                };
                let span = iterable_expr.span;
                let Some((new_id, _)) = zun_natives::resolve("zun:buffer", new_name) else {
                    self.error(span, "missing iterator constructor intrinsic");
                    return hir::Stmt::Expr(self.poison(span));
                };
                let Some((done_id, _)) = zun_natives::resolve("zun:buffer", done_name) else {
                    self.error(span, "missing iterator completion intrinsic");
                    return hir::Stmt::Expr(self.poison(span));
                };
                let Some((next_id, _)) = zun_natives::resolve("zun:buffer", next_name) else {
                    self.error(span, "missing iterator next intrinsic");
                    return hir::Stmt::Expr(self.poison(span));
                };
                let iterator = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: new_id,
                        args: vec![iterable_expr],
                    },
                    ty: Ty::Any,
                    span,
                };
                let iterator_slot = cx.declare("__for_of_iterator", Ty::Any);
                let done = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: done_id,
                        args: vec![hir::Expr {
                            kind: hir::ExprKind::Local(iterator_slot),
                            ty: Ty::Any,
                            span,
                        }],
                    },
                    ty: Ty::Boolean,
                    span,
                };
                let next = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: next_id,
                        args: vec![hir::Expr {
                            kind: hir::ExprKind::Local(iterator_slot),
                            ty: Ty::Any,
                            span,
                        }],
                    },
                    ty: Ty::Any,
                    span,
                };
                cx.scopes.push(Scope::new());
                let binding = cx.declare(self.text(name), Ty::Any);
                let body = self.check_block(cx, body);
                cx.scopes.pop();
                hir::Stmt::ForOf {
                    iterator,
                    iterator_slot,
                    done,
                    next,
                    binding,
                    body,
                }
            }
            ast::Stmt::Labeled { label, statement, .. } => {
                let label_text = self.text(label).to_string();
                let labels_loop = matches!(
                    statement.as_ref(),
                    ast::Stmt::While { .. }
                        | ast::Stmt::DoWhile { .. }
                        | ast::Stmt::For { .. }
                        | ast::Stmt::ForOf { .. }
                );
                if labels_loop {
                    cx.loop_labels.push(label_text.clone());
                }
                let body = self.check_stmt(cx, statement);
                if labels_loop {
                    cx.loop_labels.pop();
                }
                hir::Stmt::Labeled {
                    label: label_text,
                    body: Box::new(body),
                }
            }
            ast::Stmt::Break { label, span } => {
                if cx.loop_depth == 0 {
                    self.error(*span, "`break` is only valid inside a loop");
                } else if let Some(label) = label {
                    let name = self.text(label);
                    if !cx.loop_labels.iter().any(|candidate| candidate == name) {
                        self.error(*span, format!("unknown break label `{name}`"));
                    }
                }
                hir::Stmt::Break(label.map(|name| self.text(&name).to_string()))
            }
            ast::Stmt::Continue { label, span } => {
                if cx.loop_depth == 0 {
                    self.error(*span, "`continue` is only valid inside a loop");
                } else if let Some(label) = label {
                    let name = self.text(label);
                    if !cx.loop_labels.iter().any(|candidate| candidate == name) {
                        self.error(*span, format!("unknown continue label `{name}`"));
                    }
                }
                hir::Stmt::Continue(label.map(|name| self.text(&name).to_string()))
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
                        if count_awaits_in_block(&catch.body) != 0 {
                            self.error(
                                catch.body.span,
                                "await is not allowed inside a catch handler",
                            );
                        }
                        cx.scopes.push(Scope::new());
                        let slot = cx.declare(self.text(&catch.binding), ty);
                        let body = self.check_block(cx, &catch.body);
                        cx.scopes.pop();
                        hir::CatchClause { slot, ty, body }
                    })
                    .collect();
                let finally_body = finally.as_ref().map(|block| {
                    if count_awaits_in_block(block) != 0 {
                        self.error(
                            block.span,
                            "await is not allowed inside a finally handler",
                        );
                    }
                    self.check_block(cx, block)
                });
                hir::Stmt::Try {
                    body,
                    catches,
                    finally_body,
                }
            }
        }
    }

    fn check_expr(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let span = expr.span();
        match expr {
            ast::Expr::Await { operand, .. } => {
                let checked = self.check_expr(cx, operand);
                if checked.ty != Ty::Promise && checked.ty != Ty::Any {
                    self.error(
                        checked.span,
                        "await operand must have Promise type",
                    );
                }
                let span = checked.span;
                hir::Expr {
                    kind: hir::ExprKind::Await {
                        operand: Box::new(checked),
                    },
                    ty: Ty::Any,
                    span,
                }
            }
            ast::Expr::Yield { value, .. } => {
                if !cx.is_generator {
                    self.error(span, "yield is only valid inside a generator function");
                }
                let checked = self.check_expr(cx, value);
                hir::Expr {
                    kind: hir::ExprKind::Yield {
                        value: Box::new(checked),
                    },
                    ty: Ty::Any,
                    span,
                }
            }
            ast::Expr::Number { value, .. } => hir::Expr {
                kind: hir::ExprKind::Number(*value),
                ty: Ty::F64,
                span,
            },
            ast::Expr::BigInt { value, .. } => {
                let Some((id, native)) = zun_natives::resolve("zun:collections", "bigIntNew") else {
                    self.error(span, "BigInt literal intrinsic is unavailable");
                    return self.poison(span);
                };
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![hir::Expr {
                            kind: hir::ExprKind::String(value.clone()),
                            ty: Ty::String,
                            span,
                        }],
                    },
                    ty: native_ty(native.ret),
                    span,
                }
            }
            ast::Expr::String { value, .. } => hir::Expr {
                kind: hir::ExprKind::String(value.clone()),
                ty: Ty::String,
                span,
            },
            ast::Expr::Template { parts, .. } => {
                let mut result = hir::Expr {
                    kind: hir::ExprKind::String(String::new()),
                    ty: Ty::String,
                    span,
                };
                for part in parts {
                    let next = match part {
                        ast::TemplatePart::Text(text) => hir::Expr {
                            kind: hir::ExprKind::String(text.clone()),
                            ty: Ty::String,
                            span,
                        },
                        ast::TemplatePart::Expr(expression) => {
                            let value = self.check_expr(cx, expression);
                            if value.ty == Ty::F64 {
                                if let Some((id, _)) = zun_natives::resolve("zun:number", "toString") {
                                    hir::Expr {
                                        kind: hir::ExprKind::CallBuiltin { id, args: vec![value] },
                                        ty: Ty::String,
                                        span,
                                    }
                                } else {
                                    self.error(span, "number template conversion is unavailable");
                                    self.poison(span)
                                }
                            } else {
                                if value.ty != Ty::String && !value.ty.is_error() {
                                    self.error(value.span, "template interpolation must have type string or number");
                                }
                                value
                            }
                        }
                    };
                    result = hir::Expr {
                        kind: hir::ExprKind::Binary {
                            op: ast::BinaryOp::Add,
                            lhs: Box::new(result),
                            rhs: Box::new(next),
                        },
                        ty: Ty::String,
                        span,
                    };
                }
                result
            }
            ast::Expr::Bool { value, .. } => hir::Expr {
                kind: hir::ExprKind::Boolean(*value),
                ty: Ty::Boolean,
                span,
            },
            ast::Expr::Null { .. } => hir::Expr {
                kind: hir::ExprKind::Null,
                ty: Ty::Any,
                span,
            },
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
                        if text == "globalThis" || text == "global" {
                            return hir::Expr { kind: hir::ExprKind::RealmGlobal, ty: Ty::DynamicObject, span };
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
                if lhs.ty == Ty::BigInt && rhs.ty == Ty::BigInt {
                    let intrinsic = match op {
                        ast::BinaryOp::Add => Some("bigIntAdd"),
                        ast::BinaryOp::Sub => Some("bigIntSub"),
                        ast::BinaryOp::Mul => Some("bigIntMul"),
                        ast::BinaryOp::Div => Some("bigIntDiv"),
                        ast::BinaryOp::Rem => Some("bigIntRem"),
                        ast::BinaryOp::Eq => Some("bigIntEq"),
                        ast::BinaryOp::Ne => Some("bigIntNe"),
                        ast::BinaryOp::Lt => Some("bigIntLt"),
                        ast::BinaryOp::Le => Some("bigIntLe"),
                        ast::BinaryOp::Gt => Some("bigIntGt"),
                        ast::BinaryOp::Ge => Some("bigIntGe"),
                        _ => None,
                    };
                    if let Some(name) = intrinsic {
                        if let Some((id, native)) = zun_natives::resolve("zun:collections", name) {
                            return hir::Expr {
                                kind: hir::ExprKind::CallBuiltin {
                                    id,
                                    args: vec![lhs, rhs],
                                },
                                ty: native_ty(native.ret),
                                span,
                            };
                        }
                    }
                    self.error(span, "BigInt supports arithmetic and comparison operators");
                    return self.poison(span);
                }
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
            ast::Expr::Conditional {
                condition,
                when_true,
                when_false,
                ..
            } => {
                let condition = self.check_condition(cx, condition);
                let when_true = self.check_expr(cx, when_true);
                let when_false = self.check_expr(cx, when_false);
                let ty = if when_true.ty == when_false.ty {
                    when_true.ty
                } else {
                    self.error(span, "conditional branches must have matching types");
                    Ty::Error
                };
                hir::Expr {
                    kind: hir::ExprKind::Conditional {
                        condition: Box::new(condition),
                        when_true: Box::new(when_true),
                        when_false: Box::new(when_false),
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
                if *op == ast::UnaryOp::Neg && operand.ty == Ty::BigInt {
                    if let Some((id, native)) = zun_natives::resolve("zun:collections", "bigIntNeg") {
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id,
                                args: vec![operand],
                            },
                            ty: native_ty(native.ret),
                            span,
                        };
                    }
                }
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
                if let Some(ast::Expr::Spread { expr, .. }) = args.last() {
                    if let Some(&func) = self.by_name.get(&name) {
                        let rest_param = self.signatures[func as usize].rest_param;
                        if rest_param == Some(args.len() - 1) {
                            let mut fixed_args = Vec::with_capacity(args.len() - 1);
                            for argument in &args[..args.len() - 1] {
                                fixed_args.push(self.check_expr(cx, argument));
                            }
                            let array = self.check_expr(cx, expr);
                            if array.ty != Ty::Array && !array.ty.is_error() {
                                self.error(array.span, "spread call requires an Array value");
                                return self.poison(span);
                            }
                            let params = self.signatures[func as usize].params.clone();
                            for (argument, &expected) in fixed_args.iter_mut().zip(&params) {
                                coerce_numeric_literal(argument, expected);
                                if !expected.is_error()
                                    && !argument.ty.is_error()
                                    && !self.is_assignable(argument.ty, expected)
                                {
                                    self.error(
                                        argument.span,
                                        format!("argument type {:?} does not match {expected:?}", argument.ty),
                                    );
                                }
                            }
                            let return_ty = self.signatures[func as usize].return_ty;
                            return hir::Expr {
                                kind: if fixed_args.is_empty() {
                                    hir::ExprKind::CallRestArray {
                                        func,
                                        args: Box::new(array),
                                    }
                                } else {
                                    hir::ExprKind::CallRestArrayFixed {
                                        func,
                                        fixed_args,
                                        args: Box::new(array),
                                    }
                                },
                                ty: return_ty,
                                span,
                            };
                        }
                    }
                }
                let mut args: Vec<hir::Expr> =
                    args.iter().map(|a| self.check_expr(cx, a)).collect();

                if let Some(&func) = self.by_name.get(&name) {
                    if self.signatures[func as usize].is_generator {
                        let Some((native_id, _)) = zun_natives::resolve("zun:buffer", "generatorIteratorNewArgs") else {
                            self.error(span, "missing generator iterator intrinsic");
                            return self.poison(span);
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::GeneratorCall { func, args, native_id },
                            ty: Ty::Any,
                            span,
                        };
                    }
                }

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
                            let (param_tys, return_ty, optional_count, signature_name) = {
                                let sig = &self.signatures[func as usize];
                                (
                                    sig.params.clone(),
                                    sig.return_ty,
                                    sig.optional_count,
                                    sig.name.clone(),
                                )
                            };
                            let sig = Signature {
                                name: signature_name,
                                params: param_tys.clone(),
                                return_ty,
                                optional_count,
                                rest_param: self.signatures[func as usize].rest_param,
                                is_generator: self.signatures[func as usize].is_generator,
                                is_static: false,
                            };
                            if let Some(rest_index) = sig.rest_param {
                                if args.len() < rest_index {
                                    self.error(
                                        span,
                                        format!(
                                            "function value expects at least {} argument(s), found {}",
                                            rest_index,
                                            args.len()
                                        ),
                                    );
                                }
                                for (arg, &expected) in args.iter_mut().zip(&param_tys).take(rest_index) {
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
                                return hir::Expr {
                                    kind: hir::ExprKind::IndirectCallRest {
                                        callee: Box::new(hir::Expr {
                                            kind: hir::ExprKind::Local(slot),
                                            ty: function_ty,
                                            span,
                                        }),
                                        args,
                                        func,
                                    },
                                    ty: return_ty,
                                    span,
                                };
                            }
                            if args.len() < param_tys.len().saturating_sub(optional_count)
                                || args.len() > param_tys.len()
                            {
                                self.error(
                                    span,
                                    format!(
                                        "function value expects {} to {} argument(s), found {}",
                                        param_tys.len().saturating_sub(optional_count),
                                        param_tys.len(),
                                        args.len()
                                    ),
                                );
                            } else {
                                self.pad_default_args(&mut args, &sig, span);
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

                if name == "dynamicSetFunction" {
                    if args.len() != 3 {
                        self.error(span, "dynamicSetFunction expects an object, name, and function");
                        return self.poison(span);
                    }
                    let Some((id, native)) = zun_natives::resolve("zun:collections", &name) else {
                        return self.poison(span);
                    };
                    if args[0].ty != Ty::DynamicObject
                        || args[1].ty != Ty::String
                        || !matches!(args[2].ty, Ty::Function(_))
                    {
                        self.error(span, "dynamicSetFunction expects (dynamic, string, function)");
                        return self.poison(span);
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args },
                        ty: native_ty(native.ret),
                        span,
                    };
                } else if name == "dynamic_new" {
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

                if name == "dynamic_get" || name == "reflect_get" {
                    if name == "reflect_get"
                        && args.len() == 2
                        && args[0].ty == Ty::DynamicObject
                        && args[1].ty == Ty::String
                        && !matches!(args[1].kind, hir::ExprKind::String(_))
                    {
                        let key = args.pop().unwrap();
                        let object = args.pop().unwrap();
                        return hir::Expr {
                            kind: hir::ExprKind::DynamicGetValue {
                                object: Box::new(object),
                                key: Box::new(key),
                            },
                            ty: Ty::Any,
                            span,
                        };
                    }
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

                if name == "dynamic_set" || name == "reflect_set" {
                    if name == "reflect_set"
                        && args.len() == 3
                        && args[0].ty == Ty::DynamicObject
                        && args[1].ty == Ty::String
                        && args[2].ty == Ty::Any
                        && !matches!(args[1].kind, hir::ExprKind::String(_))
                    {
                        let value = args.pop().unwrap();
                        let key = args.pop().unwrap();
                        let object = args.pop().unwrap();
                        return hir::Expr {
                            kind: hir::ExprKind::DynamicSetValue {
                                object: Box::new(object),
                                key: Box::new(key),
                                value: Box::new(value),
                            },
                            ty: Ty::Any,
                            span,
                        };
                    }
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
                let (param_tys, return_ty, optional_count, signature_name) = {
                    let sig = &self.signatures[func as usize];
                    (
                        sig.params.clone(),
                        sig.return_ty,
                        sig.optional_count,
                        sig.name.clone(),
                    )
                };
                let sig = Signature {
                    name: signature_name,
                    params: param_tys.clone(),
                    return_ty,
                    optional_count,
                    rest_param: self.signatures[func as usize].rest_param,
                    is_generator: self.signatures[func as usize].is_generator,
                    is_static: false,
                };
                let rest_index = sig.rest_param;
                if let Some(rest_index) = rest_index {
                    if args.len() < rest_index {
                        self.error(
                            span,
                            format!(
                                "`{name}` expects at least {} argument(s), found {}",
                                rest_index,
                                args.len()
                            ),
                        );
                    } else {
                        for (arg, &expected) in args.iter_mut().take(rest_index).zip(&param_tys) {
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
                } else if args.len() < param_tys.len().saturating_sub(optional_count)
                    || args.len() > param_tys.len()
                {
                    self.error(
                        span,
                        format!(
                            "`{name}` expects {} to {} argument(s), found {}",
                            param_tys.len().saturating_sub(optional_count),
                            param_tys.len(),
                            args.len()
                        ),
                    );
                } else {
                    self.pad_default_args(&mut args, &sig, span);
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
                    kind: if rest_index.is_some() {
                        hir::ExprKind::CallRest { func, args }
                    } else {
                        hir::ExprKind::Call { func, args }
                    },
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
                if let ast::Expr::Ident(class) = receiver.as_ref() {
                    let class_name = self.text(class).to_string();
                    if let Some(&class_index) = self.classes_by_name.get(&class_name) {
                        if let Some(func) = self.method_of(class_index, self.text(method)) {
                            let (param_tys, return_ty) = {
                                let sig = &self.signatures[func as usize];
                                (sig.params.clone(), sig.return_ty)
                            };
                            if !self.signatures[func as usize].is_static {
                                self.error(
                                    method.span,
                                    format!("instance method `{}` must be called through an object", self.text(method)),
                                );
                                return self.poison(span);
                            }
                            if param_tys.len() == args.len() {
                                let mut checked = Vec::with_capacity(args.len());
                                for (arg, &expected) in args.iter().zip(&param_tys) {
                                    let value = self.check_expr(cx, arg);
                                    if !expected.is_error()
                                        && !value.ty.is_error()
                                        && !self.is_assignable(value.ty, expected)
                                    {
                                        self.error(
                                            value.span,
                                            format!("argument type {:?} does not match {expected:?}", value.ty),
                                        );
                                    }
                                    checked.push(value);
                                }
                                return hir::Expr {
                                    kind: hir::ExprKind::StaticCall { func, args: checked },
                                    ty: return_ty,
                                    span,
                                };
                            }
                        }
                    }
                }
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
                let managed_method = match (receiver.ty, mname.as_str()) {
                    (Ty::Map, "set") => Some(("mapSet", Ty::Void, vec![Ty::Any, Ty::Any])),
                    (Ty::Map, "get") => Some(("mapGet", Ty::Any, vec![Ty::Any])),
                    (Ty::Map, "has") => Some(("mapHas", Ty::Boolean, vec![Ty::Any])),
                    (Ty::Map, "size") => Some(("mapSize", Ty::Int(IntKind::U64), vec![])),
                    (Ty::Map, "delete") => Some(("mapDelete", Ty::Boolean, vec![Ty::Any])),
                    (Ty::Map, "clear") => Some(("mapClear", Ty::Void, vec![])),
                    (Ty::Map, "values") => Some(("mapValues", Ty::Array, vec![])),
                    (Ty::Map, "keys") => Some(("mapKeys", Ty::Array, vec![])),
                    (Ty::Map, "entries") => Some(("mapEntries", Ty::Array, vec![])),
                    (Ty::Set, "add") => Some(("setAdd", Ty::Boolean, vec![Ty::Any])),
                    (Ty::Set, "has") => Some(("setHas", Ty::Boolean, vec![Ty::Any])),
                    (Ty::Set, "size") => Some(("setSize", Ty::Int(IntKind::U64), vec![])),
                    (Ty::Set, "delete") => Some(("setDelete", Ty::Boolean, vec![Ty::Any])),
                    (Ty::Set, "clear") => Some(("setClear", Ty::Void, vec![])),
                    (Ty::Set, "values") => Some(("setValues", Ty::Array, vec![])),
                    (Ty::Set, "keys") => Some(("setValues", Ty::Array, vec![])),
                    (Ty::Set, "entries") => Some(("setEntries", Ty::Array, vec![])),
                    (Ty::WeakMap, "set") => {
                        Some(("weakMapSet", Ty::Boolean, vec![Ty::Any, Ty::Any]))
                    }
                    (Ty::WeakMap, "get") => Some(("weakMapGet", Ty::Any, vec![Ty::Any])),
                    (Ty::WeakMap, "has") => Some(("weakMapHas", Ty::Boolean, vec![Ty::Any])),
                    (Ty::WeakMap, "delete") => Some(("weakMapDelete", Ty::Boolean, vec![Ty::Any])),
                    (Ty::RegExp, "isMatch") => Some(("regexpIsMatch", Ty::Boolean, vec![Ty::String])),
                    (Ty::RegExp, "source") => Some(("regexpSource", Ty::String, vec![])),
                    (Ty::BigInt, "add") => Some(("bigIntAdd", Ty::BigInt, vec![Ty::BigInt])),
                    (Ty::BigInt, "sub") => Some(("bigIntSub", Ty::BigInt, vec![Ty::BigInt])),
                    (Ty::BigInt, "mul") => Some(("bigIntMul", Ty::BigInt, vec![Ty::BigInt])),
                    (Ty::BigInt, "compare") => Some(("bigIntCompare", Ty::Int(IntKind::I32), vec![Ty::BigInt])),
                    (Ty::BigInt, "toString") => Some(("bigIntToString", Ty::String, vec![])),
                    (Ty::FinalizationRegistry, "register") => Some((
                        "finalizationRegister",
                        Ty::Int(IntKind::U64),
                        vec![Ty::Any, Ty::Function(hir::DYNAMIC_FUNCTION)],
                    )),
                    (Ty::FinalizationRegistry, "unregister") => Some((
                        "finalizationUnregister",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64)],
                    )),
                    (Ty::Promise, "state") => Some(("promiseState", Ty::Int(IntKind::U8), vec![])),
                    (Ty::Promise, "result") => Some(("promiseResult", Ty::Any, vec![])),
                    (Ty::Promise, "isReady") => Some(("promiseIsReady", Ty::Boolean, vec![])),
                    (Ty::Promise, "resolve") => Some(("promiseResolve", Ty::Boolean, vec![Ty::Any])),
                    (Ty::Promise, "reject") => Some(("promiseReject", Ty::Boolean, vec![Ty::String])),
                    (Ty::String, "toInt") => Some(("toInt", Ty::Int(IntKind::I32), vec![])),
                    (Ty::String, "toFloat") => Some(("toFloat", Ty::F64, vec![])),
                    (Ty::String, "toBoolean") => Some(("toBoolean", Ty::Boolean, vec![])),
                    (Ty::BufferView(hir::BufferViewKind::U8), "get") => Some((
                        "u8ArrayGet",
                        Ty::Int(IntKind::U8),
                        vec![Ty::Int(IntKind::U64)],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::U8), "set") => Some((
                        "u8ArraySet",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U8)],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::U16), "get") => Some((
                        "u16ArrayGet",
                        Ty::Int(IntKind::U16),
                        vec![Ty::Int(IntKind::U64)],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::U16), "set") => Some((
                        "u16ArraySet",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U16)],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::U32), "get") => Some((
                        "u32ArrayGet",
                        Ty::Int(IntKind::U32),
                        vec![Ty::Int(IntKind::U64)],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::U32), "set") => Some((
                        "u32ArraySet",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U32)],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::F64), "get") => {
                        Some(("f64ArrayGet", Ty::F64, vec![Ty::Int(IntKind::U64)]))
                    }
                    (Ty::BufferView(hir::BufferViewKind::F64), "set") => Some((
                        "f64ArraySet",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64), Ty::F64],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "getUint32") => Some((
                        "dataViewGetU32",
                        Ty::Int(IntKind::U32),
                        vec![Ty::Int(IntKind::U64), Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "setUint32") => Some((
                        "dataViewSetU32",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U32), Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "getUint16") => Some((
                        "dataViewGetU16",
                        Ty::Int(IntKind::U16),
                        vec![Ty::Int(IntKind::U64), Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "setUint16") => Some((
                        "dataViewSetU16",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U16), Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "getFloat64") => Some((
                        "dataViewGetF64",
                        Ty::F64,
                        vec![Ty::Int(IntKind::U64), Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "setFloat64") => Some((
                        "dataViewSetF64",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64), Ty::F64, Ty::Boolean],
                    )),
                    _ => None,
                };
                if let Some((native_name, return_ty, expected)) = managed_method {
                    if args.len() != expected.len() {
                        self.error(
                            span,
                            format!(
                                "`{mname}` expects {} argument(s), found {}",
                                expected.len(),
                                args.len()
                            ),
                        );
                        return self.poison(span);
                    }
                    let receiver_is_string = receiver.ty == Ty::String;
                    let mut call_args = vec![receiver];
                    for (arg, expected_ty) in args.into_iter().zip(expected) {
                        let mut value = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut value, expected_ty);
                        if !value.ty.is_error() && !self.is_assignable(value.ty, expected_ty) {
                            self.error(
                                value.span,
                                format!(
                                    "argument type {:?} does not match {expected_ty:?}",
                                    value.ty
                                ),
                            );
                        }
                        call_args.push(value);
                    }
                    let module = if receiver_is_string {
                        "zun:string"
                    } else if native_name.starts_with("dataView")
                        || native_name.ends_with("ArrayGet")
                        || native_name.ends_with("ArraySet")
                    {
                        "zun:buffer"
                    } else if native_name.starts_with("promise") {
                        "zun:promise"
                    } else {
                        "zun:collections"
                    };
                    let Some((id, _)) = zun_natives::resolve(module, native_name) else {
                        self.error(
                            span,
                            format!("missing collection intrinsic `{native_name}`"),
                        );
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: call_args,
                        },
                        ty: return_ty,
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
                // Static signatures omit the implicit receiver. Reaching this
                // instance-call path with that arity means `value.method()`
                // attempted to invoke a static member through an object.
                if self.signatures[func as usize].is_static {
                    self.error(
                        method.span,
                        format!("static method `{mname}` must be called through its class"),
                    );
                    return self.poison(span);
                }
                if let Some(ast::Expr::Spread { expr, .. }) = args.last() {
                    let rest_index = self.signatures[func as usize].rest_param;
                    let fixed_count = args.len().saturating_sub(1);
                    if rest_index == Some(fixed_count + 1) {
                        let mut fixed_args = Vec::with_capacity(fixed_count);
                        for argument in &args[..fixed_count] {
                            fixed_args.push(self.check_expr(cx, argument));
                        }
                        let array = self.check_expr(cx, expr);
                        if array.ty != Ty::Array && !array.ty.is_error() {
                            self.error(array.span, "spread method call requires an Array value");
                            return self.poison(span);
                        }
                        let params = self.signatures[func as usize].params.clone();
                        for (argument, &expected) in fixed_args.iter_mut().zip(params.iter().skip(1)) {
                            coerce_numeric_literal(argument, expected);
                            if !expected.is_error()
                                && !argument.ty.is_error()
                                && !self.is_assignable(argument.ty, expected)
                            {
                                self.error(
                                    argument.span,
                                    format!("argument type {:?} does not match {expected:?}", argument.ty),
                                );
                            }
                        }
                        let return_ty = self.signatures[func as usize].return_ty;
                        return hir::Expr {
                            kind: if fixed_args.is_empty() {
                                hir::ExprKind::MethodCallRestArray {
                                    func,
                                    receiver: Box::new(receiver),
                                    args: Box::new(array),
                                }
                            } else {
                                hir::ExprKind::MethodCallRestArrayFixed {
                                    func,
                                    receiver: Box::new(receiver),
                                    fixed_args,
                                    args: Box::new(array),
                                }
                            },
                            ty: return_ty,
                            span,
                        };
                    }
                }
                let (param_tys, return_ty, optional_count, signature_name) = {
                    let sig = &self.signatures[func as usize];
                    (
                        sig.params.clone(),
                        sig.return_ty,
                        sig.optional_count,
                        sig.name.clone(),
                    )
                };
                // param_tys[0] is `this`; the remaining are the declared params.
                let mut call_args = vec![receiver];
                for a in args {
                    call_args.push(self.check_expr(cx, a));
                }
                let sig = Signature {
                    name: signature_name,
                    params: param_tys.clone(),
                    return_ty,
                    optional_count,
                    rest_param: self.signatures[func as usize].rest_param,
                    is_generator: self.signatures[func as usize].is_generator,
                    is_static: false,
                };
                let rest_index = sig.rest_param;
                if rest_index.is_some() && call_args.len() < rest_index.unwrap() {
                    self.error(
                        span,
                        format!("`{mname}` expects at least {} argument(s)", rest_index.unwrap() - 1),
                    );
                } else if rest_index.is_none()
                    && (call_args.len() < param_tys.len().saturating_sub(optional_count)
                        || call_args.len() > param_tys.len())
                {
                    self.error(
                        span,
                        format!(
                            "`{mname}` expects {} to {} argument(s), found {}",
                            param_tys.len() - 1 - optional_count,
                            param_tys.len() - 1,
                            call_args.len() - 1
                        ),
                    );
                } else {
                    if rest_index.is_none() {
                        self.pad_default_args(&mut call_args, &sig, span);
                    }
                    for (arg, &expected) in call_args
                        .iter_mut()
                        .zip(&param_tys)
                        .skip(1)
                        .take(rest_index.unwrap_or(param_tys.len()) - 1)
                    {
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
                    kind: if rest_index.is_some() {
                        hir::ExprKind::MethodCallRest { func, args: call_args }
                    } else {
                        hir::ExprKind::MethodCall { func, args: call_args }
                    },
                    ty: return_ty,
                    span,
                }
            }
            ast::Expr::StaticMethodCall { class, method, args, .. } => {
                let class_name = self.text(class).to_string();
                let Some(&class_index) = self.classes_by_name.get(&class_name) else {
                    self.error(class.span, format!("unknown class `{class_name}`"));
                    return self.poison(span);
                };
                let Some(func) = self.method_of(class_index, self.text(method)) else {
                    self.error(method.span, format!("unknown static method `{}`", self.text(method)));
                    return self.poison(span);
                };
                let (param_tys, return_ty) = {
                    let sig = &self.signatures[func as usize];
                    (sig.params.clone(), sig.return_ty)
                };
                if param_tys.len() != args.len() {
                    self.error(span, format!("static method `{}` expects {} arguments", self.text(method), param_tys.len()));
                    return self.poison(span);
                }
                let mut checked = Vec::with_capacity(args.len());
                for (arg, &expected) in args.iter().zip(&param_tys) {
                    let value = self.check_expr(cx, arg);
                    if !expected.is_error() && !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                        self.error(value.span, format!("argument type {:?} does not match {expected:?}", value.ty));
                    }
                    checked.push(value);
                }
                hir::Expr {
                    kind: hir::ExprKind::StaticCall { func, args: checked },
                    ty: return_ty,
                    span,
                }
            }
            ast::Expr::StaticField { class, field, .. } => {
                let class_name = self.text(class).to_string();
                let Some(&class_index) = self.classes_by_name.get(&class_name) else {
                    self.error(class.span, format!("unknown class `{class_name}`"));
                    return self.poison(span);
                };
                let Some((field_slot, ty)) = self.static_field_of(class_index, self.text(field)) else {
                    self.error(field.span, format!("unknown static field `{}`", self.text(field)));
                    return self.poison(span);
                };
                hir::Expr { kind: hir::ExprKind::StaticFieldGet { class: class_index, field_slot }, ty, span }
            }
            ast::Expr::PrivateField { object, field, .. } => {
                let object = self.check_expr(cx, object);
                let name = self.text(field).to_string();
                let Some(class) = (match object.ty { Ty::Instance(class) => Some(class), _ => None }) else {
                    self.error(span, "private fields require an instance receiver");
                    return self.poison(span);
                };
                if cx.this_class != Some(class) {
                    self.error(field.span, "private field is only accessible inside its declaring class");
                    return self.poison(span);
                }
                let Some((field_slot, ty)) = self.private_field_of(class, &name) else {
                    self.error(field.span, format!("unknown private field `#{name}`"));
                    return self.poison(span);
                };
                hir::Expr { kind: hir::ExprKind::FieldGet { object: Box::new(object), class, field_slot }, ty, span }
            }
            ast::Expr::PrivateFieldAssign { .. } => {
                self.error(span, "private field assignment is not lowered yet");
                self.poison(span)
            }
            ast::Expr::Cast { expr, ty, .. } => {
                let value = self.check_expr(cx, expr);
                let target = self.resolve_annotation(ty);
                // Numeric conversions may also explicitly unbox an `any` value.
                let numeric = |t: Ty| t.is_int() || t.is_float();
                if !value.ty.is_error()
                    && !((numeric(value.ty) && numeric(target))
                        || (value.ty == Ty::Any
                            && (numeric(target)
                                || target == Ty::Boolean
                                || target == Ty::Array
                                || target == Ty::String
                                || matches!(target, Ty::Instance(_)))))
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
                let typed_view = match name.as_str() {
                    "Uint8Array" => Some("u8ArrayNew"),
                    "Uint16Array" => Some("u16ArrayNew"),
                    "Uint32Array" => Some("u32ArrayNew"),
                    "Float64Array" => Some("f64ArrayNew"),
                    _ => None,
                };
                if name == "ArrayBuffer" {
                    if args.len() != 1 {
                        self.error(span, "`ArrayBuffer` expects one length argument");
                        return self.poison(span);
                    }
                    let mut length = self.check_expr(cx, &args[0]);
                    coerce_numeric_literal(&mut length, Ty::INT);
                    if !length.ty.is_int() {
                        self.error(length.span, "ArrayBuffer length must be an integer");
                    }
                    let Some((id, _)) = zun_natives::resolve("zun:buffer", "bufferNew") else {
                        self.error(span, "missing ArrayBuffer intrinsic");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![length],
                        },
                        ty: Ty::Buffer,
                        span,
                    };
                }
                if name == "DataView" {
                    if args.len() != 3 {
                        self.error(span, "`DataView` expects buffer, offset, and length");
                        return self.poison(span);
                    }
                    let mut call_args: Vec<hir::Expr> =
                        args.iter().map(|arg| self.check_expr(cx, arg)).collect();
                    for arg in call_args.iter_mut().skip(1) {
                        coerce_numeric_literal(arg, Ty::INT);
                    }
                    let Some((id, _)) = zun_natives::resolve("zun:buffer", "dataViewNew") else {
                        self.error(span, "missing DataView intrinsic");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: call_args,
                        },
                        ty: Ty::BufferView(hir::BufferViewKind::DataView),
                        span,
                    };
                }
                if let Some(native_name) = typed_view {
                    if args.len() != 3 {
                        self.error(span, format!("`{name}` expects buffer, offset, and length"));
                        return self.poison(span);
                    }
                    let mut call_args: Vec<hir::Expr> =
                        args.iter().map(|arg| self.check_expr(cx, arg)).collect();
                    for arg in call_args.iter_mut().skip(1) {
                        coerce_numeric_literal(arg, Ty::INT);
                    }
                    let Some((id, _)) = zun_natives::resolve("zun:buffer", native_name) else {
                        self.error(
                            span,
                            format!("missing typed-array intrinsic `{native_name}`"),
                        );
                        return self.poison(span);
                    };
                    let view_kind = match name.as_str() {
                        "Uint8Array" => hir::BufferViewKind::U8,
                        "Uint16Array" => hir::BufferViewKind::U16,
                        "Uint32Array" => hir::BufferViewKind::U32,
                        "Float64Array" => hir::BufferViewKind::F64,
                        _ => unreachable!(),
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: call_args,
                        },
                        ty: Ty::BufferView(view_kind),
                        span,
                    };
                }
                let managed_collection = match name.as_str() {
                    "Map" => Some(("mapNew", Ty::Map)),
                    "Set" => Some(("setNew", Ty::Set)),
                    "WeakMap" => Some(("weakMapNew", Ty::WeakMap)),
                    "RegExp" => Some(("regexpNew", Ty::RegExp)),
                    "BigInt" => Some(("bigIntNew", Ty::BigInt)),
                    "FinalizationRegistry" => Some((
                        "finalizationRegistryNew",
                        Ty::FinalizationRegistry,
                    )),
                    "Proxy" => Some(("proxyNew", Ty::Proxy)),
                    "Promise" => Some(("promiseNew", Ty::Promise)),
                    _ => None,
                };
                if let Some((native_name, ty)) = managed_collection {
                    let valid_args = (name == "RegExp" && args.len() == 1)
                        || (name == "BigInt" && args.len() == 1)
                        || (name == "Proxy" && args.len() == 2);
                    if !args.is_empty() && !valid_args {
                        self.error(span, format!("`{name}` expects no constructor arguments"));
                        return self.poison(span);
                    }
                    let module = if name == "Promise" {
                        "zun:promise"
                    } else {
                        "zun:collections"
                    };
                    let Some((id, _)) = zun_natives::resolve(module, native_name) else {
                        self.error(
                            span,
                            format!("missing collection intrinsic `{native_name}`"),
                        );
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: if name == "RegExp" || name == "BigInt" || name == "Proxy" {
                                args.iter().map(|arg| self.check_expr(cx, arg)).collect()
                            } else {
                                Vec::new()
                            },
                        },
                        ty,
                        span,
                    };
                }
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
                    .map(|(_, ty, _, _)| ty)
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
                let fname = self.text(field).to_string();
                if let ast::Expr::Ident(class_name) = object.as_ref() {
                    if let Some(&class_index) = self.classes_by_name.get(self.text(class_name)) {
                        if let Some((field_slot, ty)) = self.static_field_of(class_index, &fname) {
                            return hir::Expr {
                                kind: hir::ExprKind::StaticFieldGet { class: class_index, field_slot },
                                ty,
                                span,
                            };
                        }
                    }
                }
                let object = self.check_expr(cx, object);
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
                            let getter = format!("get_{fname}");
                            let Some(func) = self.method_of(class, &getter) else {
                                self.error(field.span, format!("no field or getter `{fname}` on this type"));
                                return self.poison(span);
                            };
                            let sig = &self.signatures[func as usize];
                            if sig.params.len() != 1 {
                                self.error(field.span, format!("getter `{getter}` must take no arguments"));
                                return self.poison(span);
                            }
                            hir::Expr {
                                kind: hir::ExprKind::MethodCall { func, args: vec![object] },
                                ty: sig.return_ty,
                                span,
                            }
                        }
                    },
                    Ty::Error => self.poison(span),
                    other => {
                        self.error(object.span, format!("type {other:?} has no fields"));
                        self.poison(span)
                    }
                }
            }
            ast::Expr::OptionalField { object, field, .. } => {
                let object = self.check_expr(cx, object);
                if object.ty != Ty::Any && !object.ty.is_error() {
                    self.error(
                        object.span,
                        "optional property access requires an any receiver",
                    );
                }
                hir::Expr {
                    kind: hir::ExprKind::OptionalDynamicGet {
                        object: Box::new(object),
                        name: self.text(field).to_string(),
                    },
                    ty: Ty::Any,
                    span,
                }
            }
            ast::Expr::OptionalMethodCall {
                object,
                method,
                args,
                ..
            } => {
                let object = self.check_expr(cx, object);
                let name = self.text(method).to_string();
                let (native_name, return_ty, expected) = match (object.ty, name.as_str()) {
                    (Ty::Map, "set") => ("mapSet", Ty::Void, vec![Ty::Any, Ty::Any]),
                    (Ty::Map, "get") => ("mapGet", Ty::Any, vec![Ty::Any]),
                    (Ty::Map, "has") => ("mapHas", Ty::Boolean, vec![Ty::Any]),
                    (Ty::Map, "size") => ("mapSize", Ty::Int(IntKind::U64), vec![]),
                    (Ty::Map, "delete") => ("mapDelete", Ty::Boolean, vec![Ty::Any]),
                    (Ty::Map, "clear") => ("mapClear", Ty::Void, vec![]),
                    (Ty::Set, "add") => ("setAdd", Ty::Boolean, vec![Ty::Any]),
                    (Ty::Set, "has") => ("setHas", Ty::Boolean, vec![Ty::Any]),
                    (Ty::Set, "size") => ("setSize", Ty::Int(IntKind::U64), vec![]),
                    (Ty::Set, "delete") => ("setDelete", Ty::Boolean, vec![Ty::Any]),
                    (Ty::Set, "clear") => ("setClear", Ty::Void, vec![]),
                    (Ty::WeakMap, "set") => {
                        ("weakMapSet", Ty::Boolean, vec![Ty::Any, Ty::Any])
                    }
                    (Ty::WeakMap, "get") => ("weakMapGet", Ty::Any, vec![Ty::Any]),
                    (Ty::WeakMap, "has") => ("weakMapHas", Ty::Boolean, vec![Ty::Any]),
                    (Ty::WeakMap, "delete") => {
                        ("weakMapDelete", Ty::Boolean, vec![Ty::Any])
                    }
                    (Ty::RegExp, "isMatch") => ("regexpIsMatch", Ty::Boolean, vec![Ty::String]),
                    (Ty::RegExp, "source") => ("regexpSource", Ty::String, vec![]),
                    (Ty::Promise, "state") => ("promiseState", Ty::Int(IntKind::U8), vec![]),
                    (Ty::Promise, "result") => ("promiseResult", Ty::Any, vec![]),
                    (Ty::Promise, "isReady") => ("promiseIsReady", Ty::Boolean, vec![]),
                    (Ty::Promise, "resolve") => ("promiseResolve", Ty::Boolean, vec![Ty::Any]),
                    (Ty::Promise, "reject") => ("promiseReject", Ty::Boolean, vec![Ty::String]),
                    (Ty::Error, _) => return self.poison(span),
                    (other, _) => {
                        self.error(
                            object.span,
                            format!("optional managed method requires Map, Set, or WeakMap receiver, found {other:?}"),
                        );
                        return self.poison(span);
                    }
                };
                if args.len() != expected.len() {
                    self.error(
                        span,
                        format!("`{name}` expects {} argument(s), found {}", expected.len(), args.len()),
                    );
                    return self.poison(span);
                }
                let module = if native_name.starts_with("promise") {
                    "zun:promise"
                } else {
                    "zun:collections"
                };
                let Some((id, _)) = zun_natives::resolve(module, native_name) else {
                    self.error(span, format!("missing collection intrinsic `{native_name}`"));
                    return self.poison(span);
                };
                let mut call_args = vec![object];
                for (arg, expected_ty) in args.into_iter().zip(expected) {
                    let mut value = self.check_expr(cx, arg);
                    coerce_numeric_literal(&mut value, expected_ty);
                    if !value.ty.is_error() && !self.is_assignable(value.ty, expected_ty) {
                        self.error(
                            value.span,
                            format!("argument type {:?} does not match {expected_ty:?}", value.ty),
                        );
                    }
                    call_args.push(value);
                }
                hir::Expr {
                    kind: hir::ExprKind::OptionalCallBuiltin { id, args: call_args },
                    ty: return_ty,
                    span,
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
                if let ast::Expr::PrivateField { object: private_object, field, .. } = object.as_ref() {
                    let checked_object = self.check_expr(cx, private_object);
                    let name = self.text(field).to_string();
                    let Some(class) = (match checked_object.ty { Ty::Instance(class) => Some(class), _ => None }) else {
                        self.error(span, "private fields require an instance receiver");
                        return self.poison(span);
                    };
                    if cx.this_class != Some(class) {
                        self.error(field.span, "private field is only accessible inside its declaring class");
                        return self.poison(span);
                    }
                    let Some((field_slot, ty)) = self.private_field_of(class, &name) else {
                        self.error(field.span, format!("unknown private field `#{name}`"));
                        return self.poison(span);
                    };
                    let mut value = self.check_expr(cx, value);
                    coerce_numeric_literal(&mut value, ty);
                    if !ty.is_error() && !value.ty.is_error() && !self.is_assignable(value.ty, ty) {
                        self.error(value.span, format!("cannot assign {:?} to private field of type {ty:?}", value.ty));
                    }
                    return hir::Expr { kind: hir::ExprKind::FieldSet { object: Box::new(checked_object), class, field_slot, value: Box::new(value) }, ty, span };
                }
                let fname = self.text(field).to_string();
                if let ast::Expr::Ident(class_name) = object.as_ref() {
                    if let Some(&class_index) = self.classes_by_name.get(self.text(class_name)) {
                        if let Some((field_slot, ty)) = self.static_field_of(class_index, &fname) {
                            let mut value = self.check_expr(cx, value);
                            coerce_numeric_literal(&mut value, ty);
                            if !ty.is_error() && !value.ty.is_error() && !self.is_assignable(value.ty, ty) {
                                self.error(value.span, format!("cannot assign {:?} to static field of type {ty:?}", value.ty));
                            }
                            return hir::Expr { kind: hir::ExprKind::StaticFieldSet { class: class_index, field_slot, value: Box::new(value) }, ty, span };
                        }
                    }
                }
                let object = self.check_expr(cx, object);
                let mut value = self.check_expr(cx, value);
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
                            let setter = format!("set_{fname}");
                            let Some(func) = self.method_of(class, &setter) else {
                                self.error(field.span, format!("no field or setter `{fname}` on this type"));
                                return self.poison(span);
                            };
                            let (param_count, expected, return_ty) = {
                                let sig = &self.signatures[func as usize];
                                (sig.params.len(), sig.params.get(1).copied().unwrap_or(Ty::Error), sig.return_ty)
                            };
                            if param_count != 2 {
                                self.error(field.span, format!("setter `{setter}` must take one argument"));
                                return self.poison(span);
                            }
                            coerce_numeric_literal(&mut value, expected);
                            if !expected.is_error() && !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                                self.error(value.span, format!("cannot assign {:?} to setter argument of type {expected:?}", value.ty));
                            }
                            hir::Expr {
                                kind: hir::ExprKind::MethodCall { func, args: vec![object, value] },
                                ty: return_ty,
                                span,
                            }
                        }
                    },
                    Ty::Error => self.poison(span),
                    other => {
                        self.error(object.span, format!("type {other:?} has no fields"));
                        self.poison(span)
                    }
                }
            }
            ast::Expr::Spread { expr, .. } => {
                self.error(span, "spread arguments are not lowered yet");
                self.check_expr(cx, expr);
                self.poison(span)
            }
            ast::Expr::Function(function) => self.check_closure_expr(cx, function, span),
            ast::Expr::ComputedCall { callee, args, .. } => {
                let is_well_known_iterator = matches!(callee.as_ref(), ast::Expr::Index {
                    index,
                    ..
                } if matches!(index.as_ref(), ast::Expr::Field { object, field, .. }
                    if matches!(object.as_ref(), ast::Expr::Ident(name) if self.text(name) == "Symbol")
                        && self.text(field) == "iterator"));
                if is_well_known_iterator && args.is_empty() {
                    let ast::Expr::Index { object, .. } = callee.as_ref() else {
                        unreachable!();
                    };
                    let object = self.check_expr(cx, object);
                    let native_name = match object.ty {
                        Ty::Array => "arrayIteratorNew",
                        Ty::Map => "mapIteratorNew",
                        Ty::Set => "setIteratorNew",
                        Ty::Error => "arrayIteratorNew",
                        other => {
                            self.error(
                                object.span,
                                format!("type {other:?} has no Symbol.iterator"),
                            );
                            return self.poison(span);
                        }
                    };
                    let Some((id, _)) = zun_natives::resolve("zun:buffer", native_name) else {
                        self.error(span, "missing Symbol.iterator intrinsic");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![object],
                        },
                        ty: Ty::Any,
                        span,
                    };
                }
                let callee = self.check_expr(cx, callee);
                if let Ty::Function(func) = callee.ty {
                    let mut args: Vec<hir::Expr> =
                        args.iter().map(|argument| self.check_expr(cx, argument)).collect();
                    if matches!(
                        func,
                        hir::DYNAMIC_FUNCTION | hir::DYNAMIC_PREDICATE | hir::DYNAMIC_REDUCER
                    ) {
                        let (expected, return_ty) = match func {
                            hir::DYNAMIC_REDUCER => (2, Ty::Any),
                            hir::DYNAMIC_PREDICATE => (1, Ty::Boolean),
                            _ => (1, Ty::Any),
                        };
                        if args.len() != expected {
                            self.error(
                                span,
                                format!("dynamic function value expects {expected} argument(s), found {}", args.len()),
                            );
                        }
                        return hir::Expr {
                            kind: hir::ExprKind::IndirectCall {
                                callee: Box::new(callee),
                                args,
                                func,
                            },
                            ty: return_ty,
                            span,
                        };
                    }
                    let (param_tys, optional_count, rest_param, return_ty, signature_name) = {
                        let signature = &self.signatures[func as usize];
                        (
                            signature.params.clone(),
                            signature.optional_count,
                            signature.rest_param,
                            signature.return_ty,
                            signature.name.clone(),
                        )
                    };
                    if let Some(rest_index) = rest_param {
                        if args.len() < rest_index {
                            self.error(
                                span,
                                format!("function value expects at least {} argument(s), found {}", rest_index, args.len()),
                            );
                        }
                        for (arg, &expected) in args.iter_mut().zip(&param_tys).take(rest_index) {
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
                        return hir::Expr {
                            kind: hir::ExprKind::IndirectCallRest {
                                callee: Box::new(callee),
                                args,
                                func,
                            },
                            ty: return_ty,
                            span,
                        };
                    }
                    if args.len() < param_tys.len().saturating_sub(optional_count)
                        || args.len() > param_tys.len()
                    {
                        self.error(
                            span,
                            format!(
                                "function value expects {} to {} argument(s), found {}",
                                param_tys.len().saturating_sub(optional_count),
                                param_tys.len(),
                                args.len()
                            ),
                        );
                    } else {
                        let signature = Signature {
                            name: signature_name,
                            params: param_tys.clone(),
                            return_ty,
                            optional_count,
                            rest_param,
                            is_generator: false,
                            is_static: false,
                        };
                        self.pad_default_args(&mut args, &signature, span);
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
                    return hir::Expr {
                        kind: hir::ExprKind::IndirectCall {
                            callee: Box::new(callee),
                            args,
                            func,
                        },
                        ty: return_ty,
                        span,
                    };
                }
                self.error(span, "computed calls are not supported for this callee");
                self.poison(span)
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
        // Both numeric operands must be the same numeric type (no implicit
        // int/float mixing; conversions are explicit — PLAN.md §4.2).
        // Both operands must be the same numeric type, exact width included.
        let same_numeric = |a: Ty, b: Ty| {
            matches!((a, b),
            (Ty::Int(x), Ty::Int(y)) if x == y)
                || matches!((a, b), (Ty::Float(x), Ty::Float(y)) if x == y)
        };
        match op {
            Nullish => {
                if lhs.ty == Ty::Any {
                    rhs.ty
                } else {
                    self.error(span, "left operand of `??` must be nullable");
                    Ty::Error
                }
            }
            And | Or => {
                if lhs.ty == Ty::Boolean && rhs.ty == Ty::Boolean {
                    Ty::Boolean
                } else {
                    self.error(span, "logical operators require boolean operands");
                    Ty::Error
                }
            }
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

}

fn count_awaits_in_block(block: &ast::Block) -> usize {
    block.statements.iter().map(count_awaits_in_stmt).sum()
}

fn count_yields_in_block(block: &ast::Block) -> usize {
    block.statements.iter().map(count_yields_in_stmt).sum()
}

fn count_yields_in_stmt(stmt: &ast::Stmt) -> usize {
    use ast::Stmt::*;
    match stmt {
        Let { init, .. } | ArrayDestructure { init, .. } | ObjectDestructure { init, .. } => count_yields_in_expr(init),
        Expr(expr) => count_yields_in_expr(expr),
        If { cond, then_block, else_block, .. } => count_yields_in_expr(cond) + count_yields_in_block(then_block) + else_block.as_ref().map_or(0, count_yields_in_block),
        While { cond, body, .. } => count_yields_in_expr(cond) + count_yields_in_block(body),
        DoWhile { body, cond, .. } => count_yields_in_block(body) + count_yields_in_expr(cond),
        For { init, cond, update, body, .. } => init.as_deref().map_or(0, count_yields_in_stmt) + cond.as_ref().map_or(0, count_yields_in_expr) + update.as_ref().map_or(0, count_yields_in_expr) + count_yields_in_block(body),
        ForOf { iterable, body, .. } => count_yields_in_expr(iterable) + count_yields_in_block(body),
        Labeled { statement, .. } => count_yields_in_stmt(statement),
        Break { .. } | Continue { .. } => 0,
        Return { value, .. } => value.as_ref().map_or(0, count_yields_in_expr),
        Throw { value, .. } => count_yields_in_expr(value),
        Try { body, catches, finally, .. } => count_yields_in_block(body) + catches.iter().map(|c| count_yields_in_block(&c.body)).sum::<usize>() + finally.as_ref().map_or(0, count_yields_in_block),
    }
}

fn count_yields_in_expr(expr: &ast::Expr) -> usize {
    use ast::Expr::*;
    match expr {
        Yield { value, .. } => 1 + count_yields_in_expr(value),
        Assign { value, .. } | Await { operand: value, .. } | Unary { operand: value, .. } | Spread { expr: value, .. } | Cast { expr: value, .. } => count_yields_in_expr(value),
        Binary { lhs, rhs, .. } => count_yields_in_expr(lhs) + count_yields_in_expr(rhs),
        Conditional { condition, when_true, when_false, .. } => count_yields_in_expr(condition) + count_yields_in_expr(when_true) + count_yields_in_expr(when_false),
        Call { args, .. } | New { args, .. } => args.iter().map(count_yields_in_expr).sum(),
        ComputedCall { callee, args, .. } => count_yields_in_expr(callee) + args.iter().map(count_yields_in_expr).sum::<usize>(),
        MethodCall { receiver, args, .. } | OptionalMethodCall { object: receiver, args, .. } => count_yields_in_expr(receiver) + args.iter().map(count_yields_in_expr).sum::<usize>(),
        OptionalField { object, .. } | Field { object, .. } | PrivateField { object, .. } => count_yields_in_expr(object),
        FieldAssign { object, value, .. } | PrivateFieldAssign { object, value, .. } => count_yields_in_expr(object) + count_yields_in_expr(value),
        Index { object, index, .. } => count_yields_in_expr(object) + count_yields_in_expr(index),
        IndexAssign { object, index, value, .. } => count_yields_in_expr(object) + count_yields_in_expr(index) + count_yields_in_expr(value),
        Template { parts, .. } => parts.iter().map(|p| match p { ast::TemplatePart::Text(_) => 0, ast::TemplatePart::Expr(e) => count_yields_in_expr(e) }).sum(),
        Function(_) | Number { .. } | BigInt { .. } | String { .. } | Bool { .. } | Null { .. } | Ident(_) => 0,
        StaticMethodCall { args, .. } => args.iter().map(count_yields_in_expr).sum(),
        StaticField { .. } => 0,
    }
}

fn count_awaits_in_stmt(stmt: &ast::Stmt) -> usize {
    use ast::Stmt::*;
    match stmt {
        Let { init, .. } | ArrayDestructure { init, .. } | ObjectDestructure { init, .. } => {
            count_awaits_in_expr(init)
        }
        Expr(expr) => count_awaits_in_expr(expr),
        If { cond, then_block, else_block, .. } => {
            count_awaits_in_expr(cond)
                + count_awaits_in_block(then_block)
                + else_block.as_ref().map_or(0, count_awaits_in_block)
        }
        While { cond, body, .. } => count_awaits_in_expr(cond) + count_awaits_in_block(body),
        DoWhile { body, cond, .. } => count_awaits_in_block(body) + count_awaits_in_expr(cond),
        For { init, cond, update, body, .. } => {
            init.as_deref().map_or(0, count_awaits_in_stmt)
                + cond.as_ref().map_or(0, count_awaits_in_expr)
                + update.as_ref().map_or(0, count_awaits_in_expr)
                + count_awaits_in_block(body)
        }
        ForOf { iterable, body, .. } => count_awaits_in_expr(iterable) + count_awaits_in_block(body),
        Labeled { statement, .. } => count_awaits_in_stmt(statement),
        Break { .. } | Continue { .. } => 0,
        Return { value, .. } => value.as_ref().map_or(0, count_awaits_in_expr),
        Throw { value, .. } => count_awaits_in_expr(value),
        Try { body, catches, finally, .. } => {
            count_awaits_in_block(body)
                + catches.iter().map(|catch| count_awaits_in_block(&catch.body)).sum::<usize>()
                + finally.as_ref().map_or(0, count_awaits_in_block)
        }
    }
}

fn count_awaits_in_expr(expr: &ast::Expr) -> usize {
    use ast::Expr::*;
    match expr {
        Function(_) | Number { .. } | BigInt { .. } | String { .. } | Bool { .. } | Null { .. } | Ident(_) => 0,
        Template { parts, .. } => parts
            .iter()
            .map(|part| match part {
                ast::TemplatePart::Text(_) => 0,
                ast::TemplatePart::Expr(expr) => count_awaits_in_expr(expr),
            })
            .sum(),
        Assign { value, .. } | Await { operand: value, .. } | Yield { value, .. } | Unary { operand: value, .. }
        | Spread { expr: value, .. } | Cast { expr: value, .. } => {
            usize::from(matches!(expr, Await { .. })) + count_awaits_in_expr(value)
        }
        Binary { lhs, rhs, .. } => count_awaits_in_expr(lhs) + count_awaits_in_expr(rhs),
        Conditional { condition, when_true, when_false, .. } => {
            count_awaits_in_expr(condition)
                + count_awaits_in_expr(when_true)
                + count_awaits_in_expr(when_false)
        }
        Call { args, .. } => args.iter().map(count_awaits_in_expr).sum(),
        ComputedCall { callee, args, .. } => {
            count_awaits_in_expr(callee) + args.iter().map(count_awaits_in_expr).sum::<usize>()
        }
        MethodCall { receiver, args, .. } => {
            count_awaits_in_expr(receiver) + args.iter().map(count_awaits_in_expr).sum::<usize>()
        }
        StaticMethodCall { args, .. } => args.iter().map(count_awaits_in_expr).sum(),
        StaticField { .. } => 0,
        OptionalField { object, .. } => count_awaits_in_expr(object),
        OptionalMethodCall { object, args, .. } => {
            count_awaits_in_expr(object) + args.iter().map(count_awaits_in_expr).sum::<usize>()
        }
        New { args, .. } => args.iter().map(count_awaits_in_expr).sum(),
        Field { object, .. } | PrivateField { object, .. } => count_awaits_in_expr(object),
        FieldAssign { object, value, .. } | PrivateFieldAssign { object, value, .. } => count_awaits_in_expr(object) + count_awaits_in_expr(value),
        Index { object, index, .. } => count_awaits_in_expr(object) + count_awaits_in_expr(index),
        IndexAssign { object, index, value, .. } => {
            count_awaits_in_expr(object) + count_awaits_in_expr(index) + count_awaits_in_expr(value)
        }
    }
}

#[cfg(test)]
mod tests {
#[cfg(test)]
#[path = "../tests.rs"]
mod tests;
}
