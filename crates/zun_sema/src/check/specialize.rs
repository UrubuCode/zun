//! Static monomorphic specialization emission (Phase 3b).
//!
//! Phase 3a (`crate::prove`) classifies each top-level function as a
//! monomorphization candidate or not. This pass turns a conservative subset of
//! those candidates into concrete specializations and routes the agreeing
//! intra-module call sites to them, so hot typed code leaves the `any` path.
//!
//! A specialization is **just another** [`hir::Function`]: it is synthesized by
//! re-running the checker over the original AST body with the parameter
//! signature overridden to the concrete call-site-agreed types, then appended
//! to [`hir::Program::functions`] (it gets its own MethodDef RID = index + 1 in
//! `zun_ilgen`, exactly like a closure or a regex matcher). The generic function
//! stays untouched and present; it remains the public entry and the always-safe
//! fallback. The `.zm`/metadata format is **not** changed: no new table, no
//! signature blob — the verifier pairing is a separate, approval-gated step.
//!
//! # Soundness
//!
//! Re-running the real checker (rather than hand-retyping HIR) means the
//! specialized body is typed by exactly the same rules as any other body:
//! boxing, dynamic-vs-typed member access, and coercions are all decided
//! correctly for the concrete parameter types. On top of that, this pass gates
//! emission hard and keeps only the generic when anything is uncertain:
//!
//! - only functions the analysis already certified `MONOMORPHIC` are touched
//!   (reassigned / captured / read-as-value / exported / spread / default /
//!   rest / generator / async / polymorphic / any-arg all stay generic);
//! - re-checking must not raise a single new diagnostic (any error aborts the
//!   one specialization and the diagnostics are rolled back);
//! - re-checking must not introduce any new closure, regex matcher, or codegen
//!   site (so a body that builds a closure stays generic);
//! - the specialized return type must be `void` or a concrete (non-`any`) type.
//!   An `any` return is intentionally excluded in this pass: a direct call does
//!   not box its return value, so an `any`-returning generic and its typed
//!   specialization could hand the caller different runtime representations
//!   (raw vs boxed). A concrete or `void` return has one representation, so
//!   routing is observably identical.
//!
//! A call site is routed only when its fixed argument arity and every argument
//! type match the specialization's concrete parameters exactly; every other
//! call site keeps targeting the generic.

use std::collections::BTreeMap;

use crate::check::Checker;
use crate::hir::{self, Expr, ExprKind, FuncIndex, Stmt, Ty};
use crate::prove::{ty_label, ProveReport, Verdict};
use zun_syntax::ast;

/// One accepted specialization: the generic it specializes, the concrete
/// parameter types that select it, and the new function's index.
struct Specialization {
    concrete_params: Vec<Ty>,
    spec_index: FuncIndex,
}

impl Checker<'_> {
    /// Emit monomorphic specializations for the certified candidates in
    /// `report` and route the agreeing intra-module call sites to them.
    ///
    /// `ast_program` is the (possibly field-init-hoisted) source program whose
    /// top-level functions occupy the leading indices of
    /// `program.functions`. `top_level_count` is how many of those leading
    /// functions are the module's own top-level functions (the only ones the
    /// analysis classified).
    pub(in crate::check) fn emit_specializations(
        &mut self,
        ast_program: &ast::Program,
        program: &mut hir::Program,
        report: &ProveReport,
        top_level_count: usize,
    ) {
        // Generic function index -> accepted specializations for it.
        let mut routes: BTreeMap<FuncIndex, Vec<Specialization>> = BTreeMap::new();

        for (index, verdict) in report.functions.iter().enumerate() {
            if index >= top_level_count || index >= ast_program.functions.len() {
                break;
            }
            let Verdict::Monomorphic(concrete) = &verdict.verdict else {
                continue;
            };
            let func = index as FuncIndex;

            // The generic's current parameter types. Skip if the concrete types
            // add nothing (an already fully-typed function): specializing would
            // only duplicate it.
            let param_count = program.functions[index].param_count as usize;
            let generic_params = &program.functions[index].locals[..param_count];
            if generic_params == concrete.as_slice() {
                continue;
            }

            // Representation gate: a direct call does not box its return value,
            // so only `void` or a concrete (non-`any`) return is observably
            // identical between the generic and its typed specialization.
            let return_ty = program.functions[index].return_ty;
            if return_ty == Ty::Any || return_ty == Ty::Error {
                continue;
            }

            if let Some(spec) = self.try_specialize(ast_program, func, concrete) {
                program.functions.push(spec);
                routes.entry(func).or_default().push(Specialization {
                    concrete_params: concrete.clone(),
                    spec_index: (program.functions.len() - 1) as FuncIndex,
                });
            }
        }

        if routes.is_empty() {
            return;
        }

        // Route every agreeing intra-module direct call site (over the whole
        // program, specializations included) to the matching specialization.
        for function in &mut program.functions {
            for stmt in &mut function.body {
                route_stmt(stmt, &routes);
            }
            for default in function.defaults.iter_mut().flatten() {
                route_expr(default, &routes);
            }
        }
    }

    /// Re-check one top-level function with its parameter signature overridden
    /// to `concrete`, returning the specialized [`hir::Function`] if every
    /// emission-time soundness gate passes, else `None` (keep only the generic).
    fn try_specialize(
        &mut self,
        ast_program: &ast::Program,
        func: FuncIndex,
        concrete: &[Ty],
    ) -> Option<hir::Function> {
        let index = func as usize;
        let ast_fn = &ast_program.functions[index];
        if ast_fn.params.len() != concrete.len() {
            return None;
        }

        // Snapshot every checker side table re-checking might extend, so a
        // rejected specialization leaves no trace.
        let diag_mark = self.diagnostics.len();
        let closure_mark = self.closures.len();
        let regex_mark = self.regex_matchers.len();
        let codegen_mark = self.codegen_sites.len();

        // Override the parameter signature with the concrete types; keep the
        // declared return type (so the specialized body returns the exact same
        // representation as the generic). The original signature is restored
        // unconditionally afterwards.
        let saved_params = std::mem::replace(&mut self.signatures[index].params, concrete.to_vec());
        let specialized = self.check_function(index, ast_fn, None);
        self.signatures[index].params = saved_params;

        let introduced_diag = self.diagnostics.len() != diag_mark;
        let introduced_closure = self.closures.len() != closure_mark;
        let introduced_regex = self.regex_matchers.len() != regex_mark;
        let introduced_codegen = self.codegen_sites.len() != codegen_mark;

        if introduced_diag || introduced_closure || introduced_regex || introduced_codegen {
            // Roll back anything the aborted re-check appended.
            self.diagnostics.truncate(diag_mark);
            self.closures.truncate(closure_mark);
            self.regex_matchers.truncate(regex_mark);
            self.codegen_sites.truncate(codegen_mark);
            return None;
        }

        // The specialized parameter slots must be exactly the concrete types
        // (a last defensive check; `check_function` reads them from the
        // overridden signature).
        let param_count = specialized.param_count as usize;
        if specialized.locals.get(..param_count) != Some(concrete) {
            return None;
        }

        let mut specialized = specialized;
        specialized.name = mangled_name(&specialized.name, concrete);
        Some(specialized)
    }
}

/// The specialization's stable, module-unique debug name: `name$mono$t1_t2`.
/// It is reached only by `Call <token>`, never by name, so collisions with a
/// real identifier are impossible (`$` cannot appear in source identifiers that
/// reach this point, and the generic name is already unique at top level).
fn mangled_name(name: &str, concrete: &[Ty]) -> String {
    let types = concrete
        .iter()
        .map(|ty| ty_label(*ty))
        .collect::<Vec<_>>()
        .join("_");
    format!("{name}$mono${types}")
}

/// Rewrite a matching direct [`ExprKind::Call`] to target the specialization.
fn route_call(func: &mut FuncIndex, args: &[Expr], routes: &BTreeMap<FuncIndex, Vec<Specialization>>) {
    let Some(specs) = routes.get(func) else {
        return;
    };
    for spec in specs {
        if spec.concrete_params.len() == args.len()
            && spec
                .concrete_params
                .iter()
                .zip(args)
                .all(|(expected, arg)| *expected == arg.ty)
        {
            *func = spec.spec_index;
            return;
        }
    }
}

fn route_stmt(stmt: &mut Stmt, routes: &BTreeMap<FuncIndex, Vec<Specialization>>) {
    match stmt {
        Stmt::Let { init, .. } | Stmt::BoxedLet { init, .. } => route_expr(init, routes),
        Stmt::Group(stmts) => {
            for s in stmts {
                route_stmt(s, routes);
            }
        }
        Stmt::Expr(expr) => route_expr(expr, routes),
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            route_expr(cond, routes);
            route_block(then_body, routes);
            route_block(else_body, routes);
        }
        Stmt::While { cond, body } | Stmt::DoWhile { body, cond } => {
            route_expr(cond, routes);
            route_block(body, routes);
        }
        Stmt::For {
            init,
            cond,
            update,
            body,
        } => {
            if let Some(init) = init {
                route_stmt(init, routes);
            }
            route_expr(cond, routes);
            if let Some(update) = update {
                route_expr(update, routes);
            }
            route_block(body, routes);
        }
        Stmt::ForOf {
            iterator,
            done,
            next,
            body,
            ..
        } => {
            route_expr(iterator, routes);
            route_expr(done, routes);
            route_expr(next, routes);
            route_block(body, routes);
        }
        Stmt::Labeled { body, .. } => route_stmt(body, routes),
        Stmt::Break(_) | Stmt::Continue(_) => {}
        Stmt::Return(expr) => {
            if let Some(expr) = expr {
                route_expr(expr, routes);
            }
        }
        Stmt::Throw(expr) => route_expr(expr, routes),
        Stmt::Try {
            body,
            catches,
            finally_body,
        } => {
            route_block(body, routes);
            for catch in catches {
                route_block(&mut catch.body, routes);
            }
            if let Some(finally_body) = finally_body {
                route_block(finally_body, routes);
            }
        }
    }
}

fn route_block(stmts: &mut [Stmt], routes: &BTreeMap<FuncIndex, Vec<Specialization>>) {
    for stmt in stmts {
        route_stmt(stmt, routes);
    }
}

fn route_expr(expr: &mut Expr, routes: &BTreeMap<FuncIndex, Vec<Specialization>>) {
    match &mut expr.kind {
        ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Boolean(_)
        | ExprKind::Null
        | ExprKind::Local(_)
        | ExprKind::CellLoad { .. }
        | ExprKind::CellRef { .. }
        | ExprKind::CaptureRef { .. }
        | ExprKind::SelfClosure
        | ExprKind::CaptureLoad { .. }
        | ExprKind::DynamicNew
        | ExprKind::RealmGlobal
        | ExprKind::StaticFieldGet { .. }
        | ExprKind::FunctionRef { .. }
        | ExprKind::Error => {}

        ExprKind::Call { func, args } => {
            route_call(func, args, routes);
            for arg in args {
                route_expr(arg, routes);
            }
        }

        ExprKind::Closure { captures, .. } => {
            for capture in captures {
                route_expr(&mut capture.value, routes);
            }
        }

        ExprKind::CellStore { value, .. }
        | ExprKind::CaptureStore { value, .. }
        | ExprKind::Assign { value, .. }
        | ExprKind::Yield { value }
        | ExprKind::BoxAny { value }
        | ExprKind::StaticFieldSet { value, .. }
        | ExprKind::Convert { value } => route_expr(value, routes),

        ExprKind::Await { operand } | ExprKind::Unary { operand, .. } => {
            route_expr(operand, routes)
        }

        ExprKind::ImportDynamic { specifier } => route_expr(specifier, routes),

        ExprKind::Binary { lhs, rhs, .. } => {
            route_expr(lhs, routes);
            route_expr(rhs, routes);
        }

        ExprKind::Conditional {
            condition,
            when_true,
            when_false,
        } => {
            route_expr(condition, routes);
            route_expr(when_true, routes);
            route_expr(when_false, routes);
        }

        ExprKind::CallRest { args, .. }
        | ExprKind::GeneratorCall { args, .. }
        | ExprKind::CallBuiltin { args, .. }
        | ExprKind::OptionalCallBuiltin { args, .. }
        | ExprKind::RegexMatch { args, .. }
        | ExprKind::MethodCall { args, .. }
        | ExprKind::StaticCall { args, .. }
        | ExprKind::MethodCallRest { args, .. }
        | ExprKind::New { args, .. }
        | ExprKind::ExternalCall { args, .. } => {
            for arg in args {
                route_expr(arg, routes);
            }
        }

        ExprKind::CallRestArray { args, .. } => route_expr(args, routes),
        ExprKind::CallRestArrayFixed { fixed_args, args, .. } => {
            for arg in fixed_args {
                route_expr(arg, routes);
            }
            route_expr(args, routes);
        }

        ExprKind::FunctionBind { callee, args, .. }
        | ExprKind::IndirectCall { callee, args, .. }
        | ExprKind::IndirectCallRest { callee, args, .. }
        | ExprKind::OptionalIndirectCall { callee, args, .. } => {
            route_expr(callee, routes);
            for arg in args {
                route_expr(arg, routes);
            }
        }

        ExprKind::Sequence { effect, value } => {
            route_expr(effect, routes);
            route_expr(value, routes);
        }

        ExprKind::DynamicGet { object, .. }
        | ExprKind::OptionalDynamicGet { object, .. }
        | ExprKind::ArrayLength { object }
        | ExprKind::ArrayPop { object } => route_expr(object, routes),

        ExprKind::DynamicGetValue { object, key }
        | ExprKind::OptionalDynamicGetValue { object, key } => {
            route_expr(object, routes);
            route_expr(key, routes);
        }

        ExprKind::DynamicSet { object, value, .. } => {
            route_expr(object, routes);
            route_expr(value, routes);
        }
        ExprKind::DynamicSetValue { object, key, value } => {
            route_expr(object, routes);
            route_expr(key, routes);
            route_expr(value, routes);
        }

        ExprKind::DynamicObjectLiteral { entries } => {
            for entry in entries {
                match entry {
                    hir::ObjectEntry::Data { key, value } => {
                        if let hir::ObjectKey::Computed(key) = key {
                            route_expr(key, routes);
                        }
                        route_expr(value, routes);
                    }
                    hir::ObjectEntry::Install { args, .. } => {
                        for arg in args {
                            route_expr(arg, routes);
                        }
                    }
                }
            }
        }

        ExprKind::NewWithConstructor { defaults, args, .. } => {
            for arg in defaults {
                route_expr(arg, routes);
            }
            for arg in args {
                route_expr(arg, routes);
            }
        }

        ExprKind::MethodCallRestArray { receiver, args, .. } => {
            route_expr(receiver, routes);
            route_expr(args, routes);
        }
        ExprKind::MethodCallRestArrayFixed {
            receiver,
            fixed_args,
            args,
            ..
        } => {
            route_expr(receiver, routes);
            for arg in fixed_args {
                route_expr(arg, routes);
            }
            route_expr(args, routes);
        }

        ExprKind::ArrayNew { length } => route_expr(length, routes),

        ExprKind::ArrayLiteral { elements } => {
            for element in elements.iter_mut().flatten() {
                route_expr(element, routes);
            }
        }
        ExprKind::ArraySpreadLiteral { parts } => {
            for part in parts {
                match part {
                    hir::ArrayPart::Element(expr) => route_expr(expr, routes),
                    hir::ArrayPart::Hole => {}
                    hir::ArrayPart::Spread { source, .. } => route_expr(source, routes),
                }
            }
        }

        ExprKind::FieldGet { object, .. } => route_expr(object, routes),
        ExprKind::FieldSet { object, value, .. } => {
            route_expr(object, routes);
            route_expr(value, routes);
        }
        ExprKind::ArrayGet { object, index } => {
            route_expr(object, routes);
            route_expr(index, routes);
        }
        ExprKind::ArraySet {
            object,
            index,
            value,
        } => {
            route_expr(object, routes);
            route_expr(index, routes);
            route_expr(value, routes);
        }
        ExprKind::ArrayPush { object, value } => {
            route_expr(object, routes);
            route_expr(value, routes);
        }
    }
}
