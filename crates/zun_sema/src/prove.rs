//! Static type-flow analysis for monomorphization candidacy (Phase 3a).
//!
//! This is a purely additive, read-only analysis: it never changes the checked
//! [`hir::Program`], the emitted IL, or the metadata. Given the checked HIR, it
//! computes, per top-level function:
//!
//! - the parameter types implied by the *agreement* of all intra-module call
//!   sites (if every internal call passes the same concrete, non-`any` type for
//!   a parameter, that parameter is monomorphic at that type; otherwise `any`);
//! - an inferred return type via a conservative fixpoint that widens recursion
//!   and higher-order functions to `any`;
//! - a specialization-candidate verdict: a candidate iff at least one parameter
//!   becomes concrete *and* no bail applies.
//!
//! The analysis is deliberately conservative — prove-or-bail. When in doubt it
//! bails to `any`. It emits nothing; it only classifies. Specialized method
//! emission is a later phase (gated behind a separate metadata-marking
//! proposal), so this module is a measurement and planning tool.

use std::collections::BTreeMap;

use crate::hir::{self, Expr, ExprKind, FuncIndex, Program, Stmt, Ty};

/// Why a function cannot be a monomorphization candidate. Modeled as a
/// conjunction of conservative heuristics (RTS-style): the first reason that
/// applies, in the fixed order below, is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BailReason {
    /// The function is read as a value (passed as an argument, stored, its
    /// metadata inspected, etc.): its concrete entry point must stay generic.
    ReadAsValue,
    /// The function value is captured by a nested closure.
    CapturedByClosure,
    /// The function binding is reassigned.
    Reassigned,
    /// The function is a public entry point (`main`), which must stay generic.
    Exported,
    /// The function has spread/default/rest parameters.
    VariadicOrDefault,
    /// The function body uses runtime code generation (`eval` / dynamic import).
    DynamicCodegen,
    /// The function is a generator.
    Generator,
    /// The function is `async`.
    Async,
    /// The function is a method reached virtually.
    VirtualMethod,
    /// Intra-module call sites disagree on a parameter's concrete type.
    PolymorphicCallSites,
    /// At least one call site passes an `any`-typed argument.
    AnyArgument,
    /// No parameter would become concrete (no internal call sites, or the
    /// function takes no parameters).
    NoConcreteParam,
}

impl BailReason {
    /// A short, stable, human-readable label for reports.
    pub fn label(self) -> &'static str {
        match self {
            BailReason::ReadAsValue => "read as a value",
            BailReason::CapturedByClosure => "captured by a closure",
            BailReason::Reassigned => "binding reassigned",
            BailReason::Exported => "exported entry point",
            BailReason::VariadicOrDefault => "spread/default/rest parameter",
            BailReason::DynamicCodegen => "uses eval/dynamic codegen",
            BailReason::Generator => "generator",
            BailReason::Async => "async",
            BailReason::VirtualMethod => "virtual method",
            BailReason::PolymorphicCallSites => "polymorphic call sites",
            BailReason::AnyArgument => "any-typed argument",
            BailReason::NoConcreteParam => "no concrete parameter",
        }
    }
}

/// The verdict for one analyzed function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A monomorphization candidate: the inferred (concrete-where-known)
    /// parameter types.
    Monomorphic(Vec<Ty>),
    /// Not a candidate; the first applicable bail reason.
    Generic(BailReason),
}

/// The classification of one top-level function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionVerdict {
    /// The function's name.
    pub name: String,
    /// The declared parameter types (as the checker recorded them).
    pub declared_params: Vec<Ty>,
    /// The inferred parameter types (concrete where call sites agree).
    pub inferred_params: Vec<Ty>,
    /// The inferred return type (widened to `any` for recursion / higher order).
    pub inferred_return: Ty,
    /// The specialization verdict.
    pub verdict: Verdict,
}

/// The full analysis result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProveReport {
    /// One entry per analyzed top-level function, in function-index order.
    pub functions: Vec<FunctionVerdict>,
    /// Number of monomorphization candidates.
    pub candidate_count: usize,
    /// Number of generic (non-candidate) functions.
    pub generic_count: usize,
    /// Bail-reason histogram, ordered by [`BailReason`].
    pub bail_histogram: BTreeMap<BailReason, usize>,
    /// Count of intra-module call sites (to analyzed functions) that pass at
    /// least one `any`-typed argument — i.e. where `any` persists.
    pub any_call_sites: usize,
}

/// Collected facts about one call site to a direct (`Call`) target.
struct CallSite {
    arg_types: Vec<Ty>,
}

/// Mutable accumulator threaded through the HIR walk.
#[derive(Default)]
struct Facts {
    /// Per target function index: the fixed-arity direct call sites.
    calls: BTreeMap<FuncIndex, Vec<CallSite>>,
    /// Function indices whose value is read (`FunctionRef`) at module scope.
    read_as_value: std::collections::BTreeSet<FuncIndex>,
    /// Function indices whose value is read inside a closure body.
    read_in_closure: std::collections::BTreeSet<FuncIndex>,
    /// Function indices called through a direct call graph edge (for recursion).
    call_edges: BTreeMap<FuncIndex, std::collections::BTreeSet<FuncIndex>>,
    /// Function indices whose body performs a dynamic import.
    dynamic_codegen: std::collections::BTreeSet<FuncIndex>,
}

/// Run the monomorphization analysis over a checked program.
///
/// `user_top_level` is the number of leading entries in `program.functions` that
/// are the module's own top-level functions (the checker places them first, in
/// source order). Only those functions are classified; call sites are collected
/// from the whole program so agreement reflects every internal caller.
pub fn prove_program(program: &Program, user_top_level: usize) -> ProveReport {
    prove_program_with(program, user_top_level, false)
}

/// Like [`prove_program`], but `partial` enables **partial specialization**
/// candidacy. Under the default (`partial == false`) a parameter that is `any`
/// at even one intra-module call site bails the whole function
/// ([`BailReason::AnyArgument`]). Under `partial == true` that parameter is
/// instead classified over its *non-`any`* call sites alone: if they all agree
/// on one concrete type it becomes concrete, and only the statically-concrete
/// sites are later routed to the typed twin while the `any` sites keep calling
/// the generic fallback. This is sound without a guard because routing is purely
/// static per call site (see `check::specialize::route_call`): an `any` argument
/// never matches a concrete parameter, so it is never routed. The flip stays off
/// by default (gated by `ZUN_PARTIAL_SPEC` on the compile path) until the phase-3
/// gates pass.
pub fn prove_program_with(program: &Program, user_top_level: usize, partial: bool) -> ProveReport {
    let analyzed = user_top_level.min(program.functions.len());

    // Collect call sites and value-read facts across every function body, every
    // closure body, and each function's default-argument expressions.
    let mut facts = Facts::default();
    for (index, function) in program.functions.iter().enumerate() {
        let owner = index as FuncIndex;
        for default in function.defaults.iter().flatten() {
            walk_expr(default, owner, false, &mut facts);
        }
        for stmt in &function.body {
            walk_stmt(stmt, owner, false, &mut facts);
        }
    }
    for closure in &program.closures {
        for default in closure.defaults.iter().flatten() {
            walk_expr(default, FuncIndex::MAX, true, &mut facts);
        }
        for stmt in &closure.body {
            walk_stmt(stmt, FuncIndex::MAX, true, &mut facts);
        }
    }

    // The recursion fixpoint: a function is recursive if it can reach itself
    // through direct-call edges.
    let recursive = recursive_set(&facts.call_edges);

    // Count call sites (to analyzed functions) that carry an `any` argument.
    let mut any_call_sites = 0usize;
    for (target, sites) in &facts.calls {
        if (*target as usize) >= analyzed {
            continue;
        }
        for site in sites {
            if site.arg_types.contains(&Ty::Any) {
                any_call_sites += 1;
            }
        }
    }

    let mut functions = Vec::with_capacity(analyzed);
    let mut candidate_count = 0usize;
    let mut generic_count = 0usize;
    let mut bail_histogram: BTreeMap<BailReason, usize> = BTreeMap::new();

    for index in 0..analyzed {
        let func = index as FuncIndex;
        let function = &program.functions[index];
        let declared_params = function.locals[..function.param_count as usize].to_vec();

        let inferred_return = infer_return(program, func, &recursive, &facts);
        let verdict = classify(program, func, &declared_params, &facts, partial);

        let inferred_params = match &verdict {
            Verdict::Monomorphic(params) => params.clone(),
            Verdict::Generic(_) => declared_params.clone(),
        };

        match &verdict {
            Verdict::Monomorphic(_) => candidate_count += 1,
            Verdict::Generic(reason) => {
                generic_count += 1;
                *bail_histogram.entry(*reason).or_insert(0) += 1;
            }
        }

        functions.push(FunctionVerdict {
            name: function.name.clone(),
            declared_params,
            inferred_params,
            inferred_return,
            verdict,
        });
    }

    ProveReport {
        functions,
        candidate_count,
        generic_count,
        bail_histogram,
        any_call_sites,
    }
}

/// Classify one function, applying structural bails first and then the
/// call-site agreement analysis.
fn classify(
    program: &Program,
    func: FuncIndex,
    declared_params: &[Ty],
    facts: &Facts,
    partial: bool,
) -> Verdict {
    let function = &program.functions[func as usize];

    // Structural bails, in a fixed, deterministic order.
    if function.is_generator {
        return Verdict::Generic(BailReason::Generator);
    }
    if function.is_async {
        return Verdict::Generic(BailReason::Async);
    }
    if function.rest_param.is_some() || function.defaults.iter().any(Option::is_some) {
        return Verdict::Generic(BailReason::VariadicOrDefault);
    }
    if program.main == Some(func) {
        return Verdict::Generic(BailReason::Exported);
    }
    if facts.read_in_closure.contains(&func) {
        return Verdict::Generic(BailReason::CapturedByClosure);
    }
    if facts.read_as_value.contains(&func) {
        return Verdict::Generic(BailReason::ReadAsValue);
    }
    if facts.dynamic_codegen.contains(&func) {
        return Verdict::Generic(BailReason::DynamicCodegen);
    }
    if declared_params.is_empty() {
        return Verdict::Generic(BailReason::NoConcreteParam);
    }

    // Call-site agreement. Each parameter position accumulates the concrete
    // types observed across every direct call site.
    let sites = facts.calls.get(&func);
    let Some(sites) = sites else {
        // No intra-module call sites: nothing to specialize on.
        return Verdict::Generic(BailReason::NoConcreteParam);
    };

    let mut inferred = vec![Ty::Any; declared_params.len()];
    let mut concrete_count = 0usize;
    for (position, slot) in inferred.iter_mut().enumerate() {
        let mut agreed: Option<Ty> = None;
        let mut saw_arg = false;
        let mut saw_any = false;
        let mut polymorphic = false;
        for site in sites {
            let Some(arg) = site.arg_types.get(position) else {
                continue;
            };
            saw_arg = true;
            if *arg == Ty::Any || *arg == Ty::Error {
                saw_any = true;
                continue;
            }
            match agreed {
                None => agreed = Some(*arg),
                Some(existing) if existing == *arg => {}
                Some(_) => polymorphic = true,
            }
        }
        // Under the default policy, a single `any` call site bails the whole
        // function. Under partial specialization the `any` sites are simply not
        // routed (they stay on the generic), so the parameter is classified over
        // its concrete sites alone and only truly disagreeing concrete sites
        // bail.
        if saw_any && !partial {
            return Verdict::Generic(BailReason::AnyArgument);
        }
        if polymorphic {
            return Verdict::Generic(BailReason::PolymorphicCallSites);
        }
        if saw_arg {
            if let Some(ty) = agreed {
                *slot = ty;
                concrete_count += 1;
            }
        }
    }

    if concrete_count >= 1 {
        Verdict::Monomorphic(inferred)
    } else {
        Verdict::Generic(BailReason::NoConcreteParam)
    }
}

/// Infer a function's return type via a conservative widening rule: a recursive
/// or higher-order function widens to `any`; otherwise the checker's declared
/// return type stands.
fn infer_return(
    program: &Program,
    func: FuncIndex,
    recursive: &std::collections::BTreeSet<FuncIndex>,
    _facts: &Facts,
) -> Ty {
    let function = &program.functions[func as usize];
    let higher_order = function.return_ty.is_dynamic_function()
        || matches!(function.return_ty, Ty::Function(_))
        || function.locals[..function.param_count as usize]
            .iter()
            .any(|ty| matches!(ty, Ty::Function(_)));
    if recursive.contains(&func) || higher_order {
        Ty::Any
    } else {
        function.return_ty
    }
}

/// Compute the set of functions that can reach themselves through direct-call
/// edges (a transitive-closure fixpoint).
fn recursive_set(
    edges: &BTreeMap<FuncIndex, std::collections::BTreeSet<FuncIndex>>,
) -> std::collections::BTreeSet<FuncIndex> {
    // Transitive closure by iteration to a fixpoint.
    let mut reach: BTreeMap<FuncIndex, std::collections::BTreeSet<FuncIndex>> = edges.clone();
    loop {
        let mut changed = false;
        let keys: Vec<FuncIndex> = reach.keys().copied().collect();
        for key in keys {
            let successors: Vec<FuncIndex> = reach[&key].iter().copied().collect();
            let mut additions = std::collections::BTreeSet::new();
            for succ in successors {
                if let Some(next) = reach.get(&succ) {
                    for target in next {
                        if !reach[&key].contains(target) {
                            additions.insert(*target);
                        }
                    }
                }
            }
            if !additions.is_empty() {
                let entry = reach.get_mut(&key).unwrap();
                for target in additions {
                    entry.insert(target);
                }
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    reach
        .into_iter()
        .filter(|(key, targets)| targets.contains(key))
        .map(|(key, _)| key)
        .collect()
}

fn walk_stmt(stmt: &Stmt, owner: FuncIndex, in_closure: bool, facts: &mut Facts) {
    match stmt {
        Stmt::Let { init, .. } | Stmt::BoxedLet { init, .. } => {
            walk_expr(init, owner, in_closure, facts)
        }
        Stmt::Group(stmts) => {
            for s in stmts {
                walk_stmt(s, owner, in_closure, facts);
            }
        }
        Stmt::Expr(expr) => walk_expr(expr, owner, in_closure, facts),
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            walk_expr(cond, owner, in_closure, facts);
            walk_block(then_body, owner, in_closure, facts);
            walk_block(else_body, owner, in_closure, facts);
        }
        Stmt::While { cond, body } | Stmt::DoWhile { body, cond } => {
            walk_expr(cond, owner, in_closure, facts);
            walk_block(body, owner, in_closure, facts);
        }
        Stmt::For {
            init,
            cond,
            update,
            body,
        } => {
            if let Some(init) = init {
                walk_stmt(init, owner, in_closure, facts);
            }
            walk_expr(cond, owner, in_closure, facts);
            if let Some(update) = update {
                walk_expr(update, owner, in_closure, facts);
            }
            walk_block(body, owner, in_closure, facts);
        }
        Stmt::ForOf {
            iterator,
            done,
            next,
            body,
            ..
        } => {
            walk_expr(iterator, owner, in_closure, facts);
            walk_expr(done, owner, in_closure, facts);
            walk_expr(next, owner, in_closure, facts);
            walk_block(body, owner, in_closure, facts);
        }
        Stmt::Labeled { body, .. } => walk_stmt(body, owner, in_closure, facts),
        Stmt::Break(_) | Stmt::Continue(_) => {}
        Stmt::Return(expr) => {
            if let Some(expr) = expr {
                walk_expr(expr, owner, in_closure, facts);
            }
        }
        Stmt::Throw(expr) => walk_expr(expr, owner, in_closure, facts),
        Stmt::Try {
            body,
            catches,
            finally_body,
        } => {
            walk_block(body, owner, in_closure, facts);
            for catch in catches {
                walk_block(&catch.body, owner, in_closure, facts);
            }
            if let Some(finally_body) = finally_body {
                walk_block(finally_body, owner, in_closure, facts);
            }
        }
    }
}

fn walk_block(stmts: &[Stmt], owner: FuncIndex, in_closure: bool, facts: &mut Facts) {
    for stmt in stmts {
        walk_stmt(stmt, owner, in_closure, facts);
    }
}

fn walk_expr(expr: &Expr, owner: FuncIndex, in_closure: bool, facts: &mut Facts) {
    match &expr.kind {
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
        | ExprKind::Error => {}

        ExprKind::FunctionRef { func } => {
            if in_closure {
                facts.read_in_closure.insert(*func);
            } else {
                facts.read_as_value.insert(*func);
            }
        }

        ExprKind::Closure { captures, .. } => {
            for capture in captures {
                walk_expr(&capture.value, owner, in_closure, facts);
            }
        }

        ExprKind::CellStore { value, .. }
        | ExprKind::CaptureStore { value, .. }
        | ExprKind::Assign { value, .. }
        | ExprKind::Yield { value }
        | ExprKind::BoxAny { value }
        | ExprKind::StaticFieldSet { value, .. }
        | ExprKind::Convert { value } => walk_expr(value, owner, in_closure, facts),

        ExprKind::Await { operand } | ExprKind::Unary { operand, .. } => {
            walk_expr(operand, owner, in_closure, facts)
        }

        ExprKind::ImportDynamic { specifier } => {
            facts.dynamic_codegen.insert(owner);
            walk_expr(specifier, owner, in_closure, facts);
        }

        ExprKind::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, owner, in_closure, facts);
            walk_expr(rhs, owner, in_closure, facts);
        }

        ExprKind::Conditional {
            condition,
            when_true,
            when_false,
        } => {
            walk_expr(condition, owner, in_closure, facts);
            walk_expr(when_true, owner, in_closure, facts);
            walk_expr(when_false, owner, in_closure, facts);
        }

        ExprKind::Call { func, args } => {
            record_direct_call(owner, *func, args, facts);
            walk_args(args, owner, in_closure, facts);
        }
        ExprKind::CallRest { func, args } | ExprKind::GeneratorCall { func, args, .. } => {
            // Rest/generator targets bail structurally; record the edge for
            // recursion but not as a fixed-arity call site.
            facts.call_edges.entry(owner).or_default().insert(*func);
            walk_args(args, owner, in_closure, facts);
        }
        ExprKind::CallRestArray { func, args } => {
            facts.call_edges.entry(owner).or_default().insert(*func);
            walk_expr(args, owner, in_closure, facts);
        }
        ExprKind::CallRestArrayFixed {
            func,
            fixed_args,
            args,
        } => {
            facts.call_edges.entry(owner).or_default().insert(*func);
            walk_args(fixed_args, owner, in_closure, facts);
            walk_expr(args, owner, in_closure, facts);
        }

        ExprKind::FunctionBind { callee, args, .. } => {
            walk_expr(callee, owner, in_closure, facts);
            walk_args(args, owner, in_closure, facts);
        }
        ExprKind::IndirectCall { callee, args, .. }
        | ExprKind::IndirectCallRest { callee, args, .. }
        | ExprKind::OptionalIndirectCall { callee, args, .. } => {
            walk_expr(callee, owner, in_closure, facts);
            walk_args(args, owner, in_closure, facts);
        }

        ExprKind::CallBuiltin { args, .. } | ExprKind::OptionalCallBuiltin { args, .. } => {
            walk_args(args, owner, in_closure, facts)
        }

        ExprKind::Sequence { effect, value } => {
            walk_expr(effect, owner, in_closure, facts);
            walk_expr(value, owner, in_closure, facts);
        }

        ExprKind::DynamicGet { object, .. }
        | ExprKind::OptionalDynamicGet { object, .. }
        | ExprKind::ArrayLength { object }
        | ExprKind::ArrayPop { object } => walk_expr(object, owner, in_closure, facts),

        ExprKind::DynamicGetValue { object, key }
        | ExprKind::OptionalDynamicGetValue { object, key } => {
            walk_expr(object, owner, in_closure, facts);
            walk_expr(key, owner, in_closure, facts);
        }

        ExprKind::DynamicSet { object, value, .. } => {
            walk_expr(object, owner, in_closure, facts);
            walk_expr(value, owner, in_closure, facts);
        }
        ExprKind::DynamicSetValue { object, key, value } => {
            walk_expr(object, owner, in_closure, facts);
            walk_expr(key, owner, in_closure, facts);
            walk_expr(value, owner, in_closure, facts);
        }

        ExprKind::DynamicObjectLiteral { entries } => {
            for entry in entries {
                match entry {
                    hir::ObjectEntry::Data { key, value } => {
                        if let hir::ObjectKey::Computed(key) = key {
                            walk_expr(key, owner, in_closure, facts);
                        }
                        walk_expr(value, owner, in_closure, facts);
                    }
                    hir::ObjectEntry::Install { args, .. } => {
                        walk_args(args, owner, in_closure, facts)
                    }
                }
            }
        }

        ExprKind::RegexMatch { args, .. }
        | ExprKind::MethodCall { args, .. }
        | ExprKind::StaticCall { args, .. }
        | ExprKind::MethodCallRest { args, .. }
        | ExprKind::New { args, .. } => walk_args(args, owner, in_closure, facts),

        ExprKind::ExternalCall { args, .. } => walk_args(args, owner, in_closure, facts),

        ExprKind::NewWithConstructor { defaults, args, .. } => {
            walk_args(defaults, owner, in_closure, facts);
            walk_args(args, owner, in_closure, facts);
        }

        ExprKind::MethodCallRestArray { receiver, args, .. } => {
            walk_expr(receiver, owner, in_closure, facts);
            walk_expr(args, owner, in_closure, facts);
        }
        ExprKind::MethodCallRestArrayFixed {
            receiver,
            fixed_args,
            args,
            ..
        } => {
            walk_expr(receiver, owner, in_closure, facts);
            walk_args(fixed_args, owner, in_closure, facts);
            walk_expr(args, owner, in_closure, facts);
        }

        ExprKind::ArrayNew { length } => walk_expr(length, owner, in_closure, facts),

        ExprKind::ArrayLiteral { elements } => {
            for element in elements.iter().flatten() {
                walk_expr(element, owner, in_closure, facts);
            }
        }
        ExprKind::ArraySpreadLiteral { parts } => {
            for part in parts {
                match part {
                    hir::ArrayPart::Element(expr) => walk_expr(expr, owner, in_closure, facts),
                    hir::ArrayPart::Hole => {}
                    hir::ArrayPart::Spread { source, .. } => {
                        walk_expr(source, owner, in_closure, facts)
                    }
                }
            }
        }

        ExprKind::FieldGet { object, .. } => walk_expr(object, owner, in_closure, facts),
        ExprKind::FieldSet { object, value, .. } => {
            walk_expr(object, owner, in_closure, facts);
            walk_expr(value, owner, in_closure, facts);
        }
        ExprKind::ArrayGet { object, index } => {
            walk_expr(object, owner, in_closure, facts);
            walk_expr(index, owner, in_closure, facts);
        }
        ExprKind::ArraySet {
            object,
            index,
            value,
        } => {
            walk_expr(object, owner, in_closure, facts);
            walk_expr(index, owner, in_closure, facts);
            walk_expr(value, owner, in_closure, facts);
        }
        ExprKind::ArrayPush { object, value } => {
            walk_expr(object, owner, in_closure, facts);
            walk_expr(value, owner, in_closure, facts);
        }
    }
}

fn walk_args(args: &[Expr], owner: FuncIndex, in_closure: bool, facts: &mut Facts) {
    for arg in args {
        walk_expr(arg, owner, in_closure, facts);
    }
}

fn record_direct_call(owner: FuncIndex, func: FuncIndex, args: &[Expr], facts: &mut Facts) {
    facts.call_edges.entry(owner).or_default().insert(func);
    facts.calls.entry(func).or_default().push(CallSite {
        arg_types: args.iter().map(|arg| arg.ty).collect(),
    });
}

/// A short, stable label for a type in reports.
pub fn ty_label(ty: Ty) -> String {
    use hir::{FloatKind, IntKind};
    match ty {
        Ty::Int(IntKind {
            bits: 32,
            signed: true,
        }) => "int".to_string(),
        Ty::Int(IntKind { bits, signed }) => {
            format!("{}{}", if signed { "i" } else { "u" }, bits)
        }
        Ty::Float(FloatKind::F64) => "number".to_string(),
        Ty::Float(FloatKind::F32) => "f32".to_string(),
        Ty::String => "string".to_string(),
        Ty::Boolean => "boolean".to_string(),
        Ty::Array => "array".to_string(),
        Ty::Buffer => "buffer".to_string(),
        Ty::Map => "map".to_string(),
        Ty::Set => "set".to_string(),
        Ty::BigInt => "bigint".to_string(),
        Ty::Promise => "promise".to_string(),
        Ty::DynamicObject => "object".to_string(),
        Ty::Symbol => "symbol".to_string(),
        Ty::Any => "any".to_string(),
        Ty::Void => "void".to_string(),
        Ty::Function(_) => "function".to_string(),
        Ty::Instance(index) => format!("instance#{index}"),
        Ty::Error => "error".to_string(),
        other => format!("{other:?}"),
    }
}

impl ProveReport {
    /// Render a deterministic, plain-text report.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("zun prove: monomorphization analysis\n");
        out.push_str("====================================\n\n");
        for function in &self.functions {
            let signature = format!(
                "fn {}({}) -> {}",
                function.name,
                function
                    .inferred_params
                    .iter()
                    .map(|ty| ty_label(*ty))
                    .collect::<Vec<_>>()
                    .join(", "),
                ty_label(function.inferred_return),
            );
            let verdict = match &function.verdict {
                Verdict::Monomorphic(params) => {
                    let types = params
                        .iter()
                        .map(|ty| ty_label(*ty))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("MONOMORPHIC@({types})")
                }
                Verdict::Generic(reason) => format!("GENERIC: {}", reason.label()),
            };
            out.push_str(&format!("{signature}\n    {verdict}\n"));
        }

        out.push_str("\nSUMMARY\n-------\n");
        out.push_str(&format!(
            "total functions:        {}\n",
            self.functions.len()
        ));
        out.push_str(&format!(
            "candidates:             {}\n",
            self.candidate_count
        ));
        out.push_str(&format!("generic:                {}\n", self.generic_count));
        out.push_str(&format!(
            "any-argument call sites: {}\n",
            self.any_call_sites
        ));
        if !self.bail_histogram.is_empty() {
            out.push_str("\nbail reasons:\n");
            for (reason, count) in &self.bail_histogram {
                out.push_str(&format!("    {:>4}  {}\n", count, reason.label()));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::check;
    use zun_syntax::oxc::parse_subset;

    fn prove_src(src: &str) -> ProveReport {
        let mut diags = Vec::new();
        let ast = parse_subset(src, true).expect("oxc parse");
        let user_top_level = ast.functions.len();
        let hir = check(src, &ast, &mut diags);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        prove_program(&hir, user_top_level)
    }

    fn verdict_of<'a>(report: &'a ProveReport, name: &str) -> &'a Verdict {
        &report
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("function `{name}` not analyzed"))
            .verdict
    }

    #[test]
    fn monomorphic_string_argument_is_a_candidate() {
        // Every internal call passes the same concrete `string`, so `exemplo`
        // is monomorphic at `string`.
        let report = prove_src(
            "function exemplo(valor: any): void { print(valor); }\n\
             function main(): void { const valor = \"aaaa\"; exemplo(valor); exemplo(\"bbbb\"); }",
        );
        match verdict_of(&report, "exemplo") {
            Verdict::Monomorphic(params) => {
                assert_eq!(params, &vec![Ty::String]);
            }
            other => panic!("expected monomorphic, got {other:?}"),
        }
        assert_eq!(report.candidate_count, 1);
    }

    #[test]
    fn function_read_as_value_bails() {
        // `helper` is passed as a value (its function reference is read), so it
        // must stay generic even though its one call site agrees on a type.
        let report = prove_src(
            "function helper(valor: number): number { return valor + 1; }\n\
             function apply(callback: any, x: number): number { return x; }\n\
             function main(): void { apply(helper, 2); helper(3); }",
        );
        assert_eq!(
            verdict_of(&report, "helper"),
            &Verdict::Generic(BailReason::ReadAsValue)
        );
    }

    #[test]
    fn polymorphic_call_sites_bail() {
        // Two call sites disagree on the concrete type of the parameter.
        let report = prove_src(
            "function twice(valor: any): any { return valor; }\n\
             function main(): void { twice(\"a\"); twice(1); }",
        );
        assert_eq!(
            verdict_of(&report, "twice"),
            &Verdict::Generic(BailReason::PolymorphicCallSites)
        );
    }

    #[test]
    fn main_is_an_exported_entry() {
        let report = prove_src("function main(): int { return 0; }");
        assert_eq!(
            verdict_of(&report, "main"),
            &Verdict::Generic(BailReason::Exported)
        );
    }
}
