//! Name resolution and type checking.
//!
//! Two passes over the AST: first collect every function's signature (so calls
//! can be forward references), then check each body with a lexical scope stack.
//! Errors poison the offending expression with [`Ty::Error`], which suppresses
//! follow-on mismatch diagnostics instead of cascading.

use crate::hir;
use crate::hir::{IntKind, Ty};
use std::collections::{HashMap, HashSet, VecDeque};
use zun_common::{Diagnostic, Span};
use zun_syntax::ast;

#[path = "../blocks.rs"]
mod blocks;
#[path = "../boxed_locals.rs"]
mod boxed_locals;
#[path = "../capture_analysis.rs"]
mod capture_analysis;
#[path = "../class_layout.rs"]
mod class_layout;
#[path = "../classes.rs"]
mod classes;
#[path = "../closures.rs"]
mod closures;
#[path = "../conditions.rs"]
mod conditions;
#[path = "../default_args.rs"]
mod default_args;
#[path = "../diagnostics.rs"]
mod diagnostics;
#[path = "../field_init_hoist.rs"]
mod field_init_hoist;
#[path = "../function_context.rs"]
mod function_context;
#[path = "../function_reflect.rs"]
mod function_reflect;
#[path = "../module_resolution.rs"]
mod module_resolution;
#[path = "../native_types.rs"]
mod native_types;
#[path = "../numeric_literals.rs"]
mod numeric_literals;
#[path = "../poison.rs"]
mod poison;
#[path = "../type_expect.rs"]
mod type_expect;

mod arities;
mod ast_scan;
mod binary;
mod builtins;
mod class_members;
mod construct;
mod decorators;
mod expr;
mod function;
mod helpers;
mod literals;
mod module_init;
mod operators;
mod pattern;
mod signatures;
mod specialize;
mod stmt;

use arities::*;
use ast_scan::*;
use builtins::typed_array::data_view_accessor;
use decorators::decorator_context_entry;
use numeric_literals::{coerce_numeric_literal, unify_numeric_literal};
#[path = "../types.rs"]
mod types;

use native_types::native_ty;

/// Check `program`, producing HIR and appending diagnostics.
pub fn check(
    source: &str,
    program: &ast::Program,
    diagnostics: &mut Vec<Diagnostic>,
) -> hir::Program {
    check_impl(source, program, diagnostics, &[], false, true, false, false)
}

/// Whether partial specialization is enabled on the compile path. Off by
/// default; turned on by the `ZUN_PARTIAL_SPEC` environment variable (any
/// non-empty value other than `0`). Mirrors the `ZUN_FEEDBACK` gate style. This
/// is the phase-3 "behind a flag, off by default until the gates pass" control.
fn partial_specialization_enabled() -> bool {
    std::env::var_os("ZUN_PARTIAL_SPEC")
        .map(|value| !value.is_empty() && value != "0")
        .unwrap_or(false)
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
    check_impl(
        source,
        program,
        diagnostics,
        external_functions,
        true,
        true,
        true,
        partial_specialization_enabled(),
    )
}

/// Like [`check`] but with Phase 3b monomorphic specialization enabled, for the
/// IL-producing compile path. The emitted HIR carries extra specialized
/// functions and routed call sites; the generic functions stay present and
/// unchanged. Used by the compiler; `check` itself stays specialization-free so
/// the pure analysis (`zun prove`) and the unit tests observe the original HIR.
pub fn check_specialized(
    source: &str,
    program: &ast::Program,
    diagnostics: &mut Vec<Diagnostic>,
) -> hir::Program {
    check_impl(
        source,
        program,
        diagnostics,
        &[],
        false,
        true,
        true,
        partial_specialization_enabled(),
    )
}

/// Like [`check_specialized`] but forces partial specialization on, regardless
/// of the `ZUN_PARTIAL_SPEC` environment flag. Used by the phase-3 gate suite
/// (negative soundness tests, `zun prove --partial`, before/after measurement)
/// so the flip is exercised deterministically without mutating process-global
/// environment state.
pub fn check_specialized_partial(
    source: &str,
    program: &ast::Program,
    diagnostics: &mut Vec<Diagnostic>,
) -> hir::Program {
    check_impl(source, program, diagnostics, &[], false, true, true, true)
}

/// Check a program with the `zun:core` native surface **gated**: when
/// `allow_core` is false, any `import ... from "zun:core"` is a diagnostic
/// error. The driver passes `false` for untrusted libraries (those that are not
/// internal engine code or declared modules), so the sensitive native surface
/// (I/O, stdin/stdout, env, process exit, args) is reachable only by trusted
/// code — the compile-time half of the `.env` core-capability flag.
pub fn check_with_externals_core_gated(
    source: &str,
    program: &ast::Program,
    diagnostics: &mut Vec<Diagnostic>,
    external_functions: &[ExternalFunction],
    allow_core: bool,
) -> hir::Program {
    check_impl(
        source,
        program,
        diagnostics,
        external_functions,
        true,
        allow_core,
        true,
        partial_specialization_enabled(),
    )
}

#[allow(clippy::too_many_arguments)]
fn check_impl(
    source: &str,
    program: &ast::Program,
    diagnostics: &mut Vec<Diagnostic>,
    external_functions: &[ExternalFunction],
    allow_external_modules: bool,
    allow_core: bool,
    specialize: bool,
    partial: bool,
) -> hir::Program {
    let hoisted = field_init_hoist::hoist(source, program);
    let program = hoisted.as_ref().unwrap_or(program);
    let mut external_by_name: HashMap<String, ExternalFunction> = HashMap::new();
    let mut external_by_module_name: HashMap<(String, String), ExternalFunction> = HashMap::new();
    for function in external_functions {
        external_by_module_name.insert(
            (function.module.clone(), function.name.clone()),
            function.clone(),
        );
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
        codegen_sites: Vec::new(),
        source,
        diagnostics,
        signatures: Vec::new(),
        by_name: HashMap::new(),
        classes: Vec::new(),
        classes_by_name: HashMap::new(),
        full_fields_cache: std::cell::RefCell::new(HashMap::new()),
        full_initializers_cache: std::cell::RefCell::new(HashMap::new()),
        field_index_cache: std::cell::RefCell::new(HashMap::new()),
        methods_by_class: Vec::new(),
        imports: HashMap::new(),
        namespace_imports: HashMap::new(),
        core_namespaces: HashMap::new(),
        external_functions: external_by_name,
        external_namespace_functions: external_by_module_name,
        type_aliases: HashMap::new(),
        closures: Vec::new(),
        realm_globals: crate::realm::registry().into_iter().collect(),
        regex_matchers: Vec::new(),
        class_symbol_methods: Vec::new(),
        classes_with_has_instance: HashSet::new(),
        classes_with_iterator: HashSet::new(),
        closure_param_hint: None,
        closure_takes_this: false,
        symbol_call_guard: false,
        static_this: HashMap::new(),
        cur_static_class: None,
        static_init_class: None,
        closure_self_name: None,
        routed_methods: HashMap::new(),
        hooked_classes: HashSet::new(),
        type_param_names: HashSet::new(),
        enums: HashMap::new(),
        module_globals: HashSet::new(),
        globals_read_in_functions: HashSet::new(),
        pending_static_classes: VecDeque::new(),
        arguments_functions: HashSet::new(),
        fn_mutated_globals: HashSet::new(),
        reassigned_fn_names: HashSet::new(),
        deleted_fn_meta: HashSet::new(),
        chain_roots: Vec::new(),
    };
    checker.collect_type_param_names(program);
    checker.resolve_imports(program, allow_external_modules, allow_core);
    // Classes first so field and signature annotations can name them, then
    // function and method signatures so calls can be forward references, then
    // bodies. Methods are checked as functions with an implicit `this` param.
    checker.collect_classes(program);
    checker.collect_type_aliases(program);
    checker.collect_enums(program);
    let mut classes = checker.check_classes(program);
    checker.collect_signatures(program);
    checker.collect_decorator_routing(program);
    // Top-level `let`/`const` bindings are module globals, visible inside every
    // function (JS module scope). Register their names before any function body
    // is checked so a forward reference resolves through `globalGet`.
    for statement in &program.top_level {
        if let ast::Stmt::Let { name, .. } = statement {
            checker
                .module_globals
                .insert(checker.text(name).to_string());
        }
    }
    // Slice 1 of the module-JIT epic (docs/design/module-jit-osr.md): a
    // top-level binding only needs its realm-store mirror if some function /
    // arrow / method body references it. Collect, soundly and conservatively
    // from source text, every identifier that can appear in such a body:
    //   (a) each declared function / method / symbol-method body span, and
    //   (b) each top-level statement that contains any function / arrow / object
    //       or class method node (detected structurally; a reassigned top-level
    //       function lowers to a top-level `let f = function …`, and object
    //       method shorthand `{ get() {} }` stores an `Expr::Function`).
    // Dynamic code generation (`eval` / `new Function`) can read any binding by
    // name at run time, so if the module uses it, keep every mirror. Missing a
    // reader would silently corrupt a program, so this must only ever
    // over-include.
    // Hidden computed-field key temps (`__ckN`) are only referenced from
    // synthetic spans, so the source-text scan below cannot see their readers.
    let computed_key_temps: Vec<String> = checker
        .module_globals
        .iter()
        .filter(|name| name.starts_with("__ck"))
        .cloned()
        .collect();
    if checker.source.contains("eval(") || checker.source.contains("Function(") {
        checker.globals_read_in_functions = checker.module_globals.clone();
    } else {
        let mut spans: Vec<Span> = Vec::new();
        for f in &program.functions {
            spans.push(f.span);
        }
        for class in &program.classes {
            for m in &class.methods {
                spans.push(m.span);
            }
            for sm in &class.symbol_methods {
                spans.push(sm.function.span);
            }
        }
        for statement in &program.top_level {
            if stmt_has_function(statement) {
                if let Some(span) = stmt_full_span(statement) {
                    spans.push(span);
                }
            }
        }
        for span in spans {
            if let Some(text) = checker.source.get(span.start as usize..span.end as usize) {
                collect_identifier_words(text, &mut checker.globals_read_in_functions);
            }
        }
    }
    checker.globals_read_in_functions.extend(computed_key_temps);
    {
        let mut assigned: HashSet<String> = HashSet::new();
        for function in &program.functions {
            checker.collect_mutated_names(&function.body.statements, &mut assigned);
        }
        for class in &program.classes {
            for method in &class.methods {
                checker.collect_mutated_names(&method.body.statements, &mut assigned);
            }
        }
        // Function-valued module bindings (`let f = function () {}`) that any
        // top-level code reassigns are shared through the realm store as well,
        // since closures at module scope cannot write the entry's locals.
        let mut top_assigned: HashSet<String> = HashSet::new();
        checker.collect_mutated_names(&program.top_level, &mut top_assigned);
        for statement in &program.top_level {
            if let ast::Stmt::Let {
                name,
                init: ast::Expr::Function(_),
                ..
            } = statement
            {
                let text = checker.text(name).to_string();
                if top_assigned.contains(&text) {
                    assigned.insert(text);
                }
            }
        }
        checker.fn_mutated_globals = assigned
            .into_iter()
            .filter(|name| checker.module_globals.contains(name))
            .collect();
    }

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
    // Computed-symbol-key methods, in the same (class, method) order used by
    // `collect_signatures`, lowered as static functions (no implicit `this`).
    for (class_idx, class) in program.classes.iter().enumerate() {
        for sm in &class.symbol_methods {
            functions.push(checker.check_function(
                next,
                &sm.function,
                (!sm.function.is_static).then_some(class_idx as u32),
            ));
            next += 1;
        }
    }

    // Executable top-level statements (a JS/TS module body) lower into a
    // synthetic entry that becomes the program's `main`. A program that instead
    // defines a `function main()` (classic Zun style, no top-level statements)
    // keeps that as the entry. The synthetic entry is appended last so every
    // existing function/method/class-method index is unchanged.
    let top_level_main = if program.top_level.is_empty() {
        None
    } else {
        let function = checker.check_top_level(&program.top_level);
        let index = functions.len() as u32;
        functions.push(function);
        Some(index)
    };
    let main = top_level_main.or_else(|| checker.by_name.get("main").copied());
    // Generate the module initializer only for an executable module (one with
    // an entry). It is appended last so every existing function, method, and
    // class-method index is unchanged.
    let module_init = main.and_then(|_| {
        checker.build_module_initializer(program).map(|function| {
            let index = functions.len() as u32;
            functions.push(function);
            index
        })
    });
    // The module's own top-level functions occupy the leading function indices,
    // in source order (see `collect_signatures`). Only those are candidates.
    let top_level_count = program.functions.len();
    let mut hir_program = hir::Program {
        functions,
        classes,
        main,
        module_init,
        closures: std::mem::take(&mut checker.closures),
        regex_matchers: std::mem::take(&mut checker.regex_matchers),
        codegen_sites: std::mem::take(&mut checker.codegen_sites),
    };
    // Phase 3b: synthesize static monomorphic specializations for the certified
    // candidates and route the agreeing intra-module call sites to them. This is
    // purely additive — the generic functions stay unchanged and present as the
    // always-safe fallback — and touches neither the `.zm` format nor the
    // verifier. The report is computed over the final HIR (specialization reuses
    // the exact same analysis that drives `zun prove`).
    if specialize {
        let report = crate::prove::prove_program_with(&hir_program, top_level_count, partial);
        checker.emit_specializations(program, &mut hir_program, &report, top_level_count);
    }
    hir_program
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
    /// Byte offset where the class declaration starts.
    span_start: u32,
    base: Option<u32>,
    base_name: Option<String>,
    own_fields: Vec<(String, Ty, bool, bool)>,
    own_initializers: Vec<Option<Box<ast::Expr>>>,
}

struct Checker<'a> {
    codegen_sites: Vec<(u32, u32)>,
    source: &'a str,
    diagnostics: &'a mut Vec<Diagnostic>,
    signatures: Vec<Signature>,
    by_name: HashMap<String, u32>,
    classes: Vec<ClassInfo>,
    classes_by_name: HashMap<String, u32>,
    /// Memoized full field layout per class (inherited-first slot order), built
    /// lazily once the class table is final. Avoids re-walking the base chain on
    /// every `full_fields`/`field_of` call (previously O(classes * depth)).
    full_fields_cache:
        std::cell::RefCell<HashMap<u32, std::rc::Rc<Vec<(String, Ty, bool, bool)>>>>,
    /// Memoized full initializer list per class, paired with `full_fields_cache`.
    full_initializers_cache:
        std::cell::RefCell<HashMap<u32, std::rc::Rc<Vec<Option<Box<ast::Expr>>>>>>,
    /// Per class: field name -> (first slot, type), for O(1) `field_of`.
    field_index_cache:
        std::cell::RefCell<HashMap<u32, std::rc::Rc<HashMap<String, (u16, Ty)>>>>,
    /// Per class index: method name -> global func index.
    methods_by_class: Vec<HashMap<String, u32>>,
    /// Imported intrinsic name -> (dispatch id, its native signature).
    imports: HashMap<String, (u32, &'static zun_natives::NativeFn)>,
    /// Namespace binding -> native module identity for static member calls.
    namespace_imports: HashMap<String, String>,
    /// `import { <ns> } from "zun:core"` binding -> `zun:core` namespace. Member
    /// calls on the binding resolve to the native `call.builtin` for that
    /// namespace (PLAN.md §3.4).
    core_namespaces: HashMap<String, &'static str>,
    /// Imported source-module function signatures by local name.
    external_functions: HashMap<String, ExternalFunction>,
    /// Export signatures indexed by module and name for namespace imports.
    external_namespace_functions: HashMap<(String, String), ExternalFunction>,
    /// Imported default type bindings over the frozen runtime floor.
    type_aliases: HashMap<String, Ty>,
    closures: Vec<hir::ClosureFunction>,
    /// Declarative realm globals (PLAN §3.1): base-type name -> binding. Seeded
    /// from `builtin/realm.zun`; consulted before the hardcoded base-type
    /// matches in annotation/`new`/bare-identifier resolution.
    realm_globals: HashMap<String, crate::realm::RealmBinding>,
    /// Static regexes (the `[GeneratedRegex]` analog) discovered at call sites
    /// (`/pat/.test(s)`, `new RegExp(const).test(s)`). Each is synthesized to a
    /// ZIL matcher by `zun_ilgen`; a [`hir::ExprKind::RegexMatch`] `matcher`
    /// indexes this vector.
    regex_matchers: Vec<hir::CompiledRegex>,
    /// Class computed-symbol-key methods discovered in `collect_signatures`:
    /// `(class index, well-known symbol id, global func index)`. The module
    /// initializer attaches each to the per-type symbol-method side table.
    class_symbol_methods: Vec<(u32, u64, u32)>,
    /// Class indices that define a `[Symbol.hasInstance]` method. Such classes
    /// force the runtime `instanceof` path (the method decides membership).
    classes_with_has_instance: HashSet<u32>,
    classes_with_iterator: HashSet<u32>,
    /// Contextual typing hint for the NEXT closure literal checked (TS-style):
    /// when a closure argument sits in a known function-typed parameter
    /// position, its expected parameter types are parked here so an
    /// *unannotated* closure parameter adopts the hinted type instead of the
    /// `F64` default. Consumed (taken) once at the top of `check_closure_expr`,
    /// so it never leaks to a nested or sibling closure. `None` keeps the
    /// `F64` default — numeric callbacks (`arr.map(x => x + 1)`) are unchanged.
    closure_param_hint: Option<Vec<Ty>>,
    /// Set for the NEXT closure literal: an object-literal method that uses
    /// `this`. Such a closure takes the receiver as a hidden first parameter
    /// (named `this`) and is marked by an `@this` name suffix, which the
    /// interpreter reads to pass the receiver on a dynamic member call.
    closure_takes_this: bool,
    /// Set while lowering the string-key arm of a `recv[key](...)` call whose
    /// key may be a Symbol, so the arm is not split again.
    symbol_call_guard: bool,
    /// Static methods whose body uses `this`/`super`: func index -> declaring
    /// class. They take the receiving class value as a hidden first parameter.
    static_this: HashMap<u32, u32>,
    /// Declaring class of the static method currently being checked (for `super`).
    cur_static_class: Option<u32>,
    static_init_class: Option<u32>,
    /// Name a closure being bound by `let name = <closure>` may call itself by.
    closure_self_name: Option<String>,
    /// Instance methods whose runtime decorator returns a function (method
    /// replacement): calls dispatch through the `class_named_methods` side
    /// table instead of the static vtable slot. Value: the declaring class.
    routed_methods: HashMap<u32, u32>,
    /// Classes (and subclasses of classes) with runtime-decorated instance
    /// members: `new` runs the instance-initializer and field-transform hooks.
    hooked_classes: HashSet<u32>,
    /// `enum` declarations: enum name -> (member name -> constant value). A
    /// `Enum.Member` field access folds to the member's number/string literal;
    /// the enum name resolves as a type through `type_aliases`.
    enums: HashMap<String, HashMap<String, types::EnumConst>>,
    /// Names of top-level `let`/`const` bindings (module globals). They are
    /// stored on the realm global singleton (`globalSet`/`globalGet`) so a
    /// function body can read them, matching JS module scope. All are dynamic
    /// (`any`).
    module_globals: HashSet<String>,
    /// Module globals that some function/method/arrow body references by name
    /// (so a top-level write must be mirrored to the realm store for that
    /// reader to observe it). A sound over-approximation; names outside this set
    /// are pure `<main>` locals whose per-write realm mirror is dead, which also
    /// keeps the top-level entry closer to JIT-eligible. Over-inclusion only ever
    /// keeps an unnecessary mirror — it never drops a needed one. See
    /// `docs/design/module-jit-osr.md` (slice 1).
    globals_read_in_functions: HashSet<String>,
    /// Classes whose non-literal static initializers run in the module body at
    /// their declaration position (ascending by source offset), so they can see
    /// the module bindings declared above them.
    pending_static_classes: VecDeque<u32>,
    /// Functions whose body materializes an `arguments` object: their callers
    /// report the real argument count (see `setCallArgc`).
    arguments_functions: HashSet<u32>,
    /// Module globals assigned inside a function or method body (a superset:
    /// the name may also be a local there). `<main>` reads such a global
    /// through the realm store so it observes those writes, and mirrors its
    /// own assignments back.
    fn_mutated_globals: HashSet<String>,
    /// Names of function-valued bindings that are reassigned (dynamic callee).
    reassigned_fn_names: HashSet<String>,
    /// `(function, "name" | "length")` pairs removed by `Reflect.deleteProperty`:
    /// later static reads of that metadata see the deleted value.
    deleted_fn_meta: HashSet<(u32, String)>,
    /// Temps bound by enclosing `OptionalChain` guards (innermost last).
    chain_roots: Vec<ast::Name>,
    /// Every generic type-parameter name declared anywhere in the program
    /// (`function f<T>`, `class C<U>`, `method m<K>`). Zun erases generics, so a
    /// type parameter has no distinct type: `resolve_annotation` treats a name
    /// in this set as `Ty::Any`, but only as the final fallback — after every
    /// real type (class, builtin, alias) has had its chance — so it never masks
    /// a concrete type, it only rescues an otherwise-unknown generic name.
    type_param_names: HashSet<String>,
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
    switch_depth: usize,
    block_labels: Vec<String>,
    mutated: HashSet<hir::LocalSlot>,
    mutated_names: HashSet<String>,
    /// Parameter names still in their temporal dead zone while a default
    /// expression is being checked (later parameters and the parameter itself).
    tdz_names: HashSet<String>,
    /// Number of leading local slots that are parameters of a top-level function
    /// (0 when unknown): an unboxed parameter is never captured by cell.
    param_slot_count: usize,
    boxed: HashSet<hir::LocalSlot>,
    captures: HashMap<String, hir::Capture>,
    this_class: Option<u32>,
    /// True while checking a class `constructor` body (where `super(...)` is legal).
    is_constructor: bool,
    /// True while checking the synthetic top-level `<main>` body, where a
    /// `let`/`const` declaration binds a module global (stored via `globalSet`)
    /// rather than a frame local.
    is_module_scope: bool,
    /// Number of enclosing `try` statements that have a `finally` clause.
    finally_depth: usize,
}

#[derive(Default)]
struct CaptureUse {
    slots: Vec<hir::LocalSlot>,
    slot_set: HashSet<hir::LocalSlot>,
    mutated: HashSet<hir::LocalSlot>,
    inherited: Vec<hir::Capture>,
    inherited_names: HashSet<String>,
}

impl Checker<'_> {
    /// Recognize a *static* regex used as a method-call receiver: a regex
    /// literal `/pat/flags` (CASE 1) or `new RegExp(pattern, flags?)` whose
    /// arguments const-fold to string literals (CASE 2). Returns the pattern and
    /// flag text when the pattern is statically known, else `None` (a dynamic
    /// `new RegExp(variable)` or any other receiver). The `const r = /pat/;`
    /// binding (CASE 3) is intentionally not handled here (deferred).
    fn static_regex(&self, receiver: &ast::Expr) -> Option<(String, String)> {
        match receiver {
            ast::Expr::RegexLiteral { pattern, flags, .. } => {
                Some((pattern.clone(), flags.clone()))
            }
            ast::Expr::New { class, args, .. }
                if self.text(class) == "RegExp" && (args.len() == 1 || args.len() == 2) =>
            {
                let pattern = match &args[0] {
                    ast::Expr::String { value, .. } => value.clone(),
                    _ => return None,
                };
                let flags = match args.get(1) {
                    None => String::new(),
                    Some(ast::Expr::String { value, .. }) => value.clone(),
                    Some(_) => return None,
                };
                Some((pattern, flags))
            }
            _ => None,
        }
    }

    /// Build an expression that materializes the opaque class descriptor for a
    /// realm binding's `token` (PLAN §3.1). Lowers to the `realmDescriptor`
    /// intrinsic over the token, boxed as an `Any`.
    fn realm_descriptor_expr(&self, token: u32, span: Span) -> hir::Expr {
        let (id, _) = zun_natives::resolve("collections", "realmDescriptor")
            .expect("realmDescriptor intrinsic must exist");
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![hir::Expr {
                    kind: hir::ExprKind::Number(token as f64),
                    ty: Ty::F64,
                    span,
                }],
            },
            ty: Ty::Any,
            span,
        }
    }

    fn rewrite_boxed_lets(&self, statements: &mut [hir::Stmt], boxed: &HashSet<hir::LocalSlot>) {
        boxed_locals::rewrite_boxed_lets(statements, boxed);
    }

    fn collect_mutated_names(&self, statements: &[ast::Stmt], names: &mut HashSet<String>) {
        capture_analysis::collect_mutated_names(self, statements, names);
    }
}

#[cfg(test)]
mod tests;

/// The full source span of a top-level statement (start..end), for the mirror
/// elision scan. Mirrors [`Checker::stmt_start`]'s variant coverage.
fn stmt_full_span(statement: &ast::Stmt) -> Option<Span> {
    use ast::Stmt::*;
    match statement {
        Let { span, .. }
        | Destructure { span, .. }
        | If { span, .. }
        | While { span, .. }
        | DoWhile { span, .. }
        | For { span, .. }
        | ForOf { span, .. }
        | ForIn { span, .. }
        | Switch { span, .. }
        | Labeled { span, .. }
        | Break { span, .. }
        | Continue { span, .. }
        | Return { span, .. }
        | Throw { span, .. }
        | Try { span, .. } => Some(*span),
        Expr(expr) => Some(expr.span()),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// Whether a top-level statement contains any function / arrow / object- or
/// class-method node anywhere (all are `Expr::Function`). Exhaustive matches
/// (no wildcard arm) make the compiler enforce that no AST shape is missed,
/// which is what keeps the mirror-elision sound.
fn stmt_has_function(statement: &ast::Stmt) -> bool {
    use ast::Stmt::*;
    let block = |b: &ast::Block| b.statements.iter().any(stmt_has_function);
    match statement {
        Let { init, .. } | Destructure { init, .. } => expr_has_function(init),
        Expr(expr) => expr_has_function(expr),
        If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            expr_has_function(cond)
                || block(then_block)
                || else_block.as_ref().is_some_and(block)
        }
        While { cond, body, .. } | DoWhile { body, cond, .. } => {
            expr_has_function(cond) || block(body)
        }
        For {
            init,
            extra_init,
            cond,
            update,
            body,
            ..
        } => {
            init.as_ref().is_some_and(|s| stmt_has_function(s))
                || extra_init.iter().any(stmt_has_function)
                || cond.as_ref().is_some_and(expr_has_function)
                || update.as_ref().is_some_and(expr_has_function)
                || block(body)
        }
        ForOf { iterable, body, .. } => expr_has_function(iterable) || block(body),
        ForIn { object, body, .. } => expr_has_function(object) || block(body),
        Switch {
            discriminant,
            cases,
            ..
        } => {
            expr_has_function(discriminant)
                || cases.iter().any(|case| {
                    case.test.as_ref().is_some_and(expr_has_function) || block(&case.body)
                })
        }
        Labeled { statement, .. } => stmt_has_function(statement),
        Return { value, .. } => value.as_ref().is_some_and(expr_has_function),
        Throw { value, .. } => expr_has_function(value),
        Try {
            body,
            catches,
            finally,
            ..
        } => {
            block(body)
                || catches.iter().any(|clause| block(&clause.body))
                || finally.as_ref().is_some_and(block)
        }
        Break { .. } | Continue { .. } => false,
    }
}

/// Whether an expression contains a function / arrow / method node anywhere.
fn expr_has_function(expr: &ast::Expr) -> bool {
    use ast::Expr::*;
    let any = |es: &[ast::Expr]| es.iter().any(expr_has_function);
    match expr {
        Function(_) => true,
        ObjectLiteral { properties, .. } => properties.iter().any(|member| match member {
            ast::ObjectMember::Property(property) => expr_has_function(&property.value),
            ast::ObjectMember::Spread { expr, .. } => expr_has_function(expr),
        }),
        ArrayLiteral { elements, .. } => elements.iter().any(|element| match element {
            ast::ArrayElement::Expr(e) | ast::ArrayElement::Spread(e) => expr_has_function(e),
            ast::ArrayElement::Hole => false,
        }),
        Template { parts, .. } => parts.iter().any(|part| match part {
            ast::TemplatePart::Expr(e) => expr_has_function(e),
            ast::TemplatePart::Text(_) => false,
        }),
        TaggedTemplate { tag, exprs, .. } => {
            expr_has_function(tag) || exprs.iter().any(|e| expr_has_function(e))
        }
        StaticMethodCall { args, .. } | Call { args, .. } | New { args, .. } => any(args),
        Assign { value, .. }
        | Await { operand: value, .. }
        | Yield { value, .. }
        | Unary { operand: value, .. }
        | Cast { expr: value, .. }
        | Spread { expr: value, .. } => expr_has_function(value),
        ImportCall { specifier, .. } => expr_has_function(specifier),
        Binary { lhs, rhs, .. } => expr_has_function(lhs) || expr_has_function(rhs),
        Conditional {
            condition,
            when_true,
            when_false,
            ..
        } => {
            expr_has_function(condition)
                || expr_has_function(when_true)
                || expr_has_function(when_false)
        }
        ComputedCall { callee, args, .. }
        | OptionalCall { callee, args, .. }
        | NewDynamic { callee, args, .. } => expr_has_function(callee) || any(args),
        MethodCall { receiver, args, .. } => expr_has_function(receiver) || any(args),
        OptionalMethodCall { object, args, .. } => expr_has_function(object) || any(args),
        OptionalField { object, .. }
        | OptionalIndex { object, .. }
        | Field { object, .. }
        | PrivateField { object, .. } => expr_has_function(object),
        OptionalChain { object, body, .. } | TempBind { value: object, body, .. } => {
            expr_has_function(object) || expr_has_function(body)
        }
        FieldAssign { object, value, .. } | PrivateFieldAssign { object, value, .. } => {
            expr_has_function(object) || expr_has_function(value)
        }
        Index { object, index, .. } => expr_has_function(object) || expr_has_function(index),
        IndexAssign {
            object,
            index,
            value,
            ..
        } => {
            expr_has_function(object) || expr_has_function(index) || expr_has_function(value)
        }
        Number { .. }
        | BigInt { .. }
        | String { .. }
        | RegexLiteral { .. }
        | Bool { .. }
        | Null { .. }
        | Ident(_)
        | StaticField { .. }
        | ChainRoot { .. } => false,
    }
}

/// Collect every maximal identifier word in `text` into `names`. Used as a
/// sound over-approximation of the names a function body references; string and
/// comment contents are swept in too, which only ever keeps an extra (harmless)
/// realm-store mirror.
fn collect_identifier_words(text: &str, names: &mut HashSet<String>) {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
    let mut start: Option<usize> = None;
    for (index, ch) in text.char_indices() {
        if is_ident(ch) {
            start.get_or_insert(index);
        } else if let Some(from) = start.take() {
            names.insert(text[from..index].to_string());
        }
    }
    if let Some(from) = start {
        names.insert(text[from..].to_string());
    }
}

/// Where a runtime decorator applies, for the typed-context lowering.
#[derive(Clone, Copy)]
struct DecoratorTarget {
    /// The decorated element's declaring class.
    class_idx: u32,
    /// Instance field slot of a field decorator, else `-1`.
    slot: i32,
    /// A returned function replaces the (routed) instance method.
    replaces_method: bool,
    /// A returned function is the field's initializer transform.
    transforms_field: bool,
}
