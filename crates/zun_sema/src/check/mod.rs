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

#[path = "../closures.rs"]
mod closures;
#[path = "../class_layout.rs"]
mod class_layout;
#[path = "../field_init_hoist.rs"]
mod field_init_hoist;
#[path = "../classes.rs"]
mod classes;
#[path = "../conditions.rs"]
mod conditions;
#[path = "../blocks.rs"]
mod blocks;
#[path = "../default_args.rs"]
mod default_args;
#[path = "../diagnostics.rs"]
mod diagnostics;
#[path = "../module_resolution.rs"]
mod module_resolution;
#[path = "../native_types.rs"]
mod native_types;
#[path = "../numeric_literals.rs"]
mod numeric_literals;
#[path = "../function_context.rs"]
mod function_context;
#[path = "../boxed_locals.rs"]
mod boxed_locals;
#[path = "../capture_analysis.rs"]
mod capture_analysis;
#[path = "../poison.rs"]
mod poison;
#[path = "../function_reflect.rs"]
mod function_reflect;
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
    check_impl(source, program, diagnostics, &[], false, true)
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
    check_impl(source, program, diagnostics, external_functions, true, true)
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
    check_impl(source, program, diagnostics, external_functions, true, allow_core)
}

fn check_impl(
    source: &str,
    program: &ast::Program,
    diagnostics: &mut Vec<Diagnostic>,
    external_functions: &[ExternalFunction],
    allow_external_modules: bool,
    allow_core: bool,
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
        closure_self_name: None,
        routed_methods: HashMap::new(),
        hooked_classes: HashSet::new(),
        type_param_names: HashSet::new(),
        enums: HashMap::new(),
        module_globals: HashSet::new(),
        pending_static_classes: Vec::new(),
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
            checker.module_globals.insert(checker.text(name).to_string());
        }
    }
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
            if let ast::Stmt::Let { name, init: ast::Expr::Function(_), .. } = statement {
                let text = checker.text(name).to_string();
                if top_assigned.contains(&text) {
                    assigned.insert(text);
                }
            }
        }
        checker.fn_mutated_globals =
            assigned.into_iter().filter(|name| checker.module_globals.contains(name)).collect();
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
    hir::Program {
        functions,
        classes,
        main,
        module_init,
        closures: checker.closures,
        regex_matchers: checker.regex_matchers,
        codegen_sites: checker.codegen_sites,
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
    /// Classes whose non-literal static initializers run in the module body at
    /// their declaration position (ascending by source offset), so they can see
    /// the module bindings declared above them.
    pending_static_classes: Vec<u32>,
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
    mutated: HashSet<hir::LocalSlot>,
    inherited: Vec<hir::Capture>,
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

