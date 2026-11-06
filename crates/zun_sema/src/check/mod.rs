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

mod expr;
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

/// The `value` argument passed to a runtime decorator, as a `null` of type
/// `Any`.
///
/// Ideally a method decorator would receive the method as a first-class
/// function value (a handle to its MethodDef RID) and a class decorator the
/// constructor. Neither can cross the `Any` boundary here: `Value::Function` is
/// deliberately excluded from `Any` (so closure environments stay visible to
/// precise GC-root handling, see `Value::box_any`), and there is no first-class
/// runtime class/constructor value in this typed-class model. Delivering a live
/// `value` is therefore deferred together with return-value replacement (both
/// need the dispatch-model change). The decorator still receives the full
/// `context`, which is the observable Stage-3 behavior.
fn decorator_null_value(span: Span) -> hir::Expr {
    hir::Expr {
        kind: hir::ExprKind::Null,
        ty: Ty::Any,
        span,
    }
}

/// One named property of a runtime decorator's `context` object. The value is
/// boxed to `Any`, as `DynamicObjectLiteral` stores require.
fn decorator_context_entry(
    name: &str,
    value: hir::ExprKind,
    value_ty: Ty,
    span: Span,
) -> hir::ObjectEntry {
    hir::ObjectEntry::Data {
        key: hir::ObjectKey::Named(name.to_string()),
        value: hir::Expr {
            kind: hir::ExprKind::BoxAny {
                value: Box::new(hir::Expr {
                    kind: value,
                    ty: value_ty,
                    span,
                }),
            },
            ty: Ty::Any,
            span,
        },
    }
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

    /// Build the module initializer (PLAN §3.1, doc 02 area 10): a synthetic
    /// static no-arg void function whose body is the declarative realm seed.
    ///
    /// For each base type registered in `builtin/realm.zun` it emits
    /// `globalSet("<Name>", realmDescriptor(<token>), true)`, so the realm
    /// global holds every base-type class descriptor before the entry point runs
    /// — without the program calling `global.set` itself. The compiler keeps
    /// this out of `main`, so `main`'s IL and JIT/AOT eligibility are unchanged.
    ///
    /// Returns `None` when there is nothing to run (empty realm registry and no
    /// runtime decorators). A future user-facing `@module_init` attribute would
    /// extend this same seam by appending user statements to the initializer
    /// body.
    ///
    /// The initializer also hosts **runtime TC39 decorator** invocations
    /// (PLAN §3.5 Stage 3): a `@decorator` whose callee resolves to an ordinary
    /// top-level function (not a compile-time macro) is called here, once, at
    /// class-definition time, with `(value, context)`. See
    /// [`Self::append_runtime_decorator_calls`] for the protocol and its
    /// documented deferrals.
    /// Lower the module's executable top-level statements into a synthetic
    /// entry function (`<main>`), a static no-arg void function whose body is
    /// those statements. Top-level `const`/`let` become its locals; it may call
    /// any hoisted top-level function or class, which were registered before any
    /// body was checked. This is how a JS/TS module body runs as the program.
    fn check_top_level(&mut self, statements: &[ast::Stmt]) -> hir::Function {
        let block = ast::Block { statements: statements.to_vec(), span: Span::new(0, 0) };
        let mut cx = FunctionCx {
            locals: Vec::new(),
            scopes: vec![Scope::new()],
            return_ty: Ty::Void,
            is_generator: false,
            loop_depth: 0,
            loop_labels: Vec::new(),
            switch_depth: 0,
            block_labels: Vec::new(),
            mutated: HashSet::new(),
            mutated_names: HashSet::new(),
            tdz_names: HashSet::new(),
            param_slot_count: 0,
            boxed: HashSet::new(),
            captures: HashMap::new(),
            this_class: None,
            is_constructor: false,
            is_module_scope: true,
            finally_depth: 0,
        };
        self.collect_mutated_names(&block.statements, &mut cx.mutated_names);
        // A class declared before the last top-level statement runs its static
        // initializers in the module body, at its textual position; the rest
        // (and prelude classes, which trail the user source) run up front.
        let last_start = block.statements.iter().filter_map(Self::stmt_start).max();
        let mut early = Vec::new();
        self.pending_static_classes.clear();
        for class_idx in 0..self.classes.len() {
            match last_start {
                Some(last) if self.classes[class_idx].span_start < last => {
                    self.pending_static_classes.push(class_idx as u32)
                }
                _ => early.push(class_idx as u32),
            }
        }
        let mut body = Vec::new();
        for class_idx in early {
            body.extend(self.class_static_initializers(&mut cx, class_idx));
        }
        body.extend(self.check_block(&mut cx, &block));
        self.rewrite_boxed_lets(&mut body, &cx.boxed);
        hir::Function {
            name: "<main>".to_string(),
            param_count: 0,
            rest_param: None,
            defaults: Vec::new(),
            locals: cx.locals,
            return_ty: Ty::Void,
            is_async: false,
            is_static: true,
            is_generator: false,
            async_plan: None,
            generator_plan: None,
            body,
        }
    }

    /// `(this as any).#x`: the cast only silences the type checker, so inside a
    /// class body a cast-wrapped `this` is the typed instance receiver again.
    fn peel_this_cast(&self, cx: &FunctionCx, expr: &ast::Expr) -> ast::Expr {
        let mut inner = expr;
        while let ast::Expr::Cast { expr: next, .. } = inner {
            inner = next;
        }
        match inner {
            ast::Expr::Ident(name) if cx.this_class.is_some() && self.text(name) == "this" => inner.clone(),
            _ => expr.clone(),
        }
    }

    /// A private name resolves lexically to the enclosing class, so a receiver
    /// the checker only knows as `any` (`other.#x` after a `v is Point` guard)
    /// is downcast to that class; a wrong-brand value then fails the cast with a
    /// TypeError at run time.
    fn assume_private_receiver(&mut self, cx: &FunctionCx, object: hir::Expr, name: &str) -> hir::Expr {
        if !matches!(object.ty, Ty::Any | Ty::DynamicObject) {
            return object;
        }
        let Some(class) = cx.this_class else { return object };
        if self.private_field_of(class, name).is_none() && !self.has_private_member(class, name) {
            return object;
        }
        let span = object.span;
        hir::Expr {
            kind: hir::ExprKind::Convert { value: Box::new(Self::box_to_any(object)) },
            ty: Ty::Instance(class),
            span,
        }
    }

    /// An expression that throws a `TypeError` when evaluated (calling
    /// `undefined`), used for writes the language rejects at run time.
    fn runtime_type_error(&mut self, cx: &mut FunctionCx, span: Span) -> hir::Expr {
        let callee = self.check_expr(
            cx,
            &ast::Expr::Unary {
                op: ast::UnaryOp::Void,
                operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                span,
            },
        );
        hir::Expr {
            kind: hir::ExprKind::IndirectCall { callee: Box::new(callee), args: Vec::new(), func: hir::DYNAMIC_FUNCTION },
            ty: Ty::Any,
            span,
        }
    }

    /// Evaluate non-literal `static` field initializers (`static n: number = 2 + 3`,
    /// `static #next = 1`) once, in class declaration order, before the module
    /// body. Literal initializers of a matching type are handled by the module
    /// initializer; this covers every other expression.
    fn class_static_initializers(&mut self, cx: &mut FunctionCx, class_idx: u32) -> Vec<hir::Stmt> {
        let span = Span::new(0, 0);
        let mut out = Vec::new();
        let class_idx = class_idx as usize;
        {
            let fields = self.full_fields(class_idx as u32);
            let initializers = self.full_initializers(class_idx as u32);
            let first_own = fields.len().saturating_sub(self.classes[class_idx].own_fields.len());
            for (slot, (_, field_ty, is_static, _)) in fields.iter().enumerate() {
                if !*is_static || slot < first_own {
                    continue;
                }
                let Some(Some(init)) = initializers.get(slot) else { continue };
                let init = init.as_ref().clone();
                let field_ty = *field_ty;
                let literal = matches!(
                    (&init, field_ty),
                    (ast::Expr::Number { .. }, Ty::F64)
                        | (ast::Expr::String { .. }, Ty::String)
                        | (ast::Expr::Bool { .. }, Ty::Boolean)
                );
                // `static x;` (an implicit `void 0`) leaves the slot unset.
                if literal || matches!(init, ast::Expr::Unary { op: ast::UnaryOp::Void, .. }) {
                    continue;
                }
                let mut value = self.check_expr(cx, &init);
                coerce_numeric_literal(&mut value, field_ty);
                if field_ty == Ty::Any {
                    value = Self::box_to_any(value);
                } else if !field_ty.is_error() && !value.ty.is_error() && !self.is_assignable(value.ty, field_ty) {
                    self.error(value.span, format!("cannot assign {:?} to static field of type {field_ty:?}", value.ty));
                    continue;
                }
                out.push(hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::StaticFieldSet {
                        class: class_idx as u32,
                        field_slot: slot as u16,
                        value: Box::new(value),
                    },
                    ty: field_ty,
                    span,
                }));
            }
        }
        out
    }

    /// Source offset where a top-level statement starts, when known.
    fn stmt_start(statement: &ast::Stmt) -> Option<u32> {
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
            | Try { span, .. } => Some(span.start),
            Expr(expr) => Some(expr.span().start),
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }

    /// Whether every value-returning `return` in `block` (outside nested functions)
    /// is `return this;`, and there is at least one. Such a method is fluent: its
    /// result is the receiver's own class.
    fn block_returns_only_this(&self, block: &ast::Block) -> bool {
        fn walk(checker: &Checker<'_>, block: &ast::Block, this_count: &mut usize, other: &mut bool) {
            for statement in &block.statements {
                stmt(checker, statement, this_count, other);
            }
        }
        fn stmt(checker: &Checker<'_>, statement: &ast::Stmt, this_count: &mut usize, other: &mut bool) {
            match statement {
                ast::Stmt::Return { value: Some(value), .. } => match value {
                    ast::Expr::Ident(name) if checker.text(name) == "this" => *this_count += 1,
                    _ => *other = true,
                },
                ast::Stmt::If { then_block, else_block, .. } => {
                    walk(checker, then_block, this_count, other);
                    if let Some(else_block) = else_block {
                        walk(checker, else_block, this_count, other);
                    }
                }
                ast::Stmt::While { body, .. }
                | ast::Stmt::DoWhile { body, .. }
                | ast::Stmt::For { body, .. }
                | ast::Stmt::ForOf { body, .. }
                | ast::Stmt::ForIn { body, .. } => walk(checker, body, this_count, other),
                ast::Stmt::Labeled { statement, .. } => stmt(checker, statement, this_count, other),
                ast::Stmt::Switch { cases, .. } => {
                    for case in cases {
                        walk(checker, &case.body, this_count, other);
                    }
                }
                ast::Stmt::Try { body, catches, finally, .. } => {
                    walk(checker, body, this_count, other);
                    for clause in catches {
                        walk(checker, &clause.body, this_count, other);
                    }
                    if let Some(finally) = finally {
                        walk(checker, finally, this_count, other);
                    }
                }
                _ => {}
            }
        }
        let mut this_count = 0;
        let mut other = false;
        walk(self, block, &mut this_count, &mut other);
        this_count > 0 && !other
    }

    /// An iterable spread source that is not statically an Array (a string, a Set,
    /// an `any` value) is materialized through the iteration protocol.
    fn spread_source_to_array(&mut self, source: hir::Expr) -> hir::Expr {
        if source.ty == Ty::Array || source.ty.is_error() {
            return source;
        }
        let span = source.span;
        match zun_natives::resolve("array", "arrayFrom") {
            Some((id, _)) => hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![Self::box_to_any(source)] },
                ty: Ty::Array,
                span,
            },
            None => source,
        }
    }

    /// `apply`/`Reflect.apply`/`Reflect.construct` argument lists: an Array passes
    /// through; any other object is read as an array-like (no iteration).
    pub(super) fn array_like_to_array(&mut self, source: hir::Expr) -> hir::Expr {
        if source.ty == Ty::Array || source.ty.is_error() {
            return source;
        }
        let span = source.span;
        match zun_natives::resolve("array", "arrayFromArrayLike") {
            Some((id, _)) => hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![Self::box_to_any(source)] },
                ty: Ty::Array,
                span,
            },
            None => source,
        }
    }

    fn build_module_initializer(&self, program: &ast::Program) -> Option<hir::Function> {
        let span = Span::new(0, 0);
        let mut body: Vec<hir::Stmt> = Vec::new();

        // 1) Declarative realm seed (base-type class descriptors).
        let registry = crate::realm::registry();
        if !registry.is_empty() {
            let (set_id, _) = zun_natives::resolve("collections", "globalSet")
                .expect("globalSet intrinsic must exist");
            // Namespace globals are plain objects; function-like globals reuse the
            // `Object` descriptor so `typeof globalThis[name]` reports the right kind.
            let object_token = registry.iter().find(|(n, _)| n.as_str() == "Object").map(|(_, b)| b.token);
            let extra_globals: Vec<(&str, bool)> = vec![
                ("Math", true), ("JSON", true), ("Reflect", true), ("Proxy", false), ("parseInt", false),
                ("parseFloat", false), ("isNaN", false), ("isFinite", false), ("encodeURIComponent", false),
                ("decodeURIComponent", false), ("encodeURI", false), ("decodeURI", false),
            ];
            for (name, is_object) in extra_globals {
                if registry.iter().any(|(n, _)| n.as_str() == name) && name != "Reflect" {
                    continue;
                }
                let value = if is_object {
                    hir::Expr {
                        kind: hir::ExprKind::BoxAny {
                            value: Box::new(hir::Expr {
                                kind: hir::ExprKind::DynamicObjectLiteral { entries: Vec::new() },
                                ty: Ty::DynamicObject,
                                span,
                            }),
                        },
                        ty: Ty::Any,
                        span,
                    }
                } else if let Some(token) = object_token {
                    self.realm_descriptor_expr(token, span)
                } else {
                    continue;
                };
                body.push(hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: set_id,
                        args: vec![
                            hir::Expr { kind: hir::ExprKind::String(name.to_string()), ty: Ty::String, span },
                            value,
                            hir::Expr { kind: hir::ExprKind::Boolean(true), ty: Ty::Boolean, span },
                        ],
                    },
                    ty: Ty::Void,
                    span,
                }));
            }
            for (name, binding) in registry {
                if name.as_str() == "Reflect" {
                    continue;
                }
                let descriptor = self.realm_descriptor_expr(binding.token, span);
                body.push(hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: set_id,
                        args: vec![
                            hir::Expr {
                                kind: hir::ExprKind::String(name),
                                ty: Ty::String,
                                span,
                            },
                            descriptor,
                            hir::Expr {
                                kind: hir::ExprKind::Boolean(true),
                                ty: Ty::Boolean,
                                span,
                            },
                        ],
                    },
                    ty: Ty::Void,
                    span,
                }));
            }
        }

        // 2) Runtime TC39 decorators (Stage 3), run at class-definition time.
        self.append_runtime_decorator_calls(program, &mut body, span);

        // 3) Class computed-symbol-key methods: attach each to the per-type
        //    symbol-method side table at class-definition time.
        self.append_class_symbol_methods(&mut body, span);

        // 4) Named `valueOf`/`toString` methods and literal field defaults, for
        //    ToPrimitive and runtime class construction.
        self.append_class_named_methods_and_defaults(&mut body, span);

        if body.is_empty() {
            return None;
        }
        Some(hir::Function {
            name: "<module-init>".to_string(),
            param_count: 0,
            rest_param: None,
            defaults: Vec::new(),
            // Slot 0 hosts the typed `DecoratorContext` of the decorator being
            // applied (see `typed_context_decorator`).
            locals: self
                .classes_by_name
                .get("DecoratorContext")
                .map(|&class| vec![Ty::Instance(class)])
                .unwrap_or_default(),
            return_ty: Ty::Void,
            is_async: false,
            is_static: true,
            is_generator: false,
            async_plan: None,
            generator_plan: None,
            body,
        })
    }

    /// The signature of a bare `@dec` decorator whose callee is a top-level
    /// `(any, DecoratorContext)` function: the typed-context decorator form.
    fn typed_decorator_signature(&self, decorator: &ast::Decorator) -> Option<&Signature> {
        if decorator.args.is_some() {
            return None;
        }
        let ctx_class = *self.classes_by_name.get("DecoratorContext")?;
        let span = decorator.callee.span();
        let callee = self.source.get(span.start as usize..span.end as usize)?;
        let signature = self.signatures.get(*self.by_name.get(callee)? as usize)?;
        (signature.params.len() == 2
            && signature.rest_param.is_none()
            && matches!(signature.params[0], Ty::Any)
            && signature.params[1] == Ty::Instance(ctx_class))
        .then_some(signature)
    }

    /// Decide, before any body is checked, which instance methods are routed
    /// through the `class_named_methods` side table (a typed-context decorator
    /// returns a function, so it may replace the method) and which classes need
    /// the `new` hooks (instance `addInitializer`, field transforms). Only
    /// decorated elements opt in: undecorated methods and classes keep static
    /// dispatch and plain `new`.
    fn collect_decorator_routing(&mut self, program: &ast::Program) {
        let mut routed: HashMap<u32, u32> = HashMap::new();
        let mut own_hooked: HashSet<u32> = HashSet::new();
        for (class_idx, class) in program.classes.iter().enumerate() {
            let class_idx = class_idx as u32;
            for method in &class.methods {
                if method.is_static {
                    continue;
                }
                let name = self.text(&method.name).to_string();
                let Some(&func) = self
                    .methods_by_class
                    .get(class_idx as usize)
                    .and_then(|m| m.get(&name))
                else {
                    continue;
                };
                let rest = self.signatures[func as usize].rest_param.is_some();
                for decorator in &method.decorators {
                    let Some(signature) = self.typed_decorator_signature(decorator) else {
                        continue;
                    };
                    own_hooked.insert(class_idx);
                    if !rest && matches!(signature.return_ty, Ty::Function(_)) {
                        routed.insert(func, class_idx);
                    }
                }
            }
            for field in &class.fields {
                if field.is_static || field.is_private {
                    continue;
                }
                if field
                    .decorators
                    .iter()
                    .any(|decorator| self.typed_decorator_signature(decorator).is_some())
                {
                    own_hooked.insert(class_idx);
                }
            }
        }
        let mut hooked = HashSet::new();
        for class in 0..self.classes.len() as u32 {
            let mut current = Some(class);
            while let Some(c) = current {
                if own_hooked.contains(&c) {
                    hooked.insert(class);
                    break;
                }
                current = self.classes.get(c as usize).and_then(|info| info.base);
            }
        }
        self.routed_methods = routed;
        self.hooked_classes = hooked;
    }

    /// Emit the runtime TC39 decorator (PLAN §3.5 Stage 3, the "runtime flavor")
    /// invocations into the module-initializer `body`.
    ///
    /// ## What runs
    ///
    /// For each decorated class and each decorated method whose decorator callee
    /// is a bare `@dec` resolving to an ordinary top-level function value, this
    /// appends a single call `dec(value, context)` that executes once, before
    /// the entry point, at class-definition time. `context` is a `DynamicObject`
    /// `{ kind, name, static }`:
    ///
    /// * method decorator — `kind: "method"`, `name: <methodName>`,
    ///   `static: <bool>`.
    /// * class decorator — `kind: "class"`, `name: <className>`,
    ///   `static: false`.
    ///
    /// `value` is `null` in both cases: see [`decorator_null_value`] for why a
    /// live method/class value cannot be delivered in the current model.
    ///
    /// ## Evaluation order (TC39)
    ///
    /// Member decorators run before the class decorator, and the decorators of a
    /// single element run bottom-to-top (nearest the declaration last), so each
    /// element's list is applied in reverse source order.
    ///
    /// ## Macro-vs-runtime disambiguation
    ///
    /// Compile-time macro decorators (e.g. `@autoToString`) are consumed earlier,
    /// in `zun_compiler`'s macro-expansion pass, and their callee names are not
    /// declared functions, so they never resolve here. A decorator whose callee
    /// resolves to a top-level function value takes this runtime path; anything
    /// else (a macro name, an unknown name, `@sealed`/`@trace`) resolves to no
    /// function and stays inert — exactly the Phase-1 behavior. `zun_sema` cannot
    /// depend on `zun_compiler`'s macro registry, so "resolves to a function" is
    /// the registry-free equivalent of the coordinator's rule.
    ///
    /// ## Deferred (documented)
    ///
    /// A typed-context instance-method decorator that returns a `Function`
    /// **replaces the method**: only such decorated methods dispatch through
    /// `class_named_methods` (see `routed_methods`); the replacement receives the
    /// instance as its leading argument. A returned function from a field
    /// decorator is its initializer transform. Still deferred: class
    /// return-value replacement (`new C` targets a compile-time TypeDef, there
    /// is no first-class runtime class value to rebind); static-method
    /// replacement; a live class `value`; accessor and private-element
    /// decorators; and replacement results of decorator factories. Decorator
    /// factories `@dec(args)` are
    /// supported (literal arguments): the factory call is evaluated in the
    /// initializer and its result invoked via the `applyDecorator` native.
    /// Emit `classSetSymbolFunction(classRid, symbol, fn)` for each class
    /// computed-symbol-key method, attaching the method (as a function value)
    /// to the per-type symbol-method side table at class-definition time. The
    /// class rid is `class index + 1` (one TypeDef per class, in order).
    fn append_class_symbol_methods(&self, body: &mut Vec<hir::Stmt>, span: Span) {
        if self.class_symbol_methods.is_empty() {
            return;
        }
        let Some((set_id, _)) = zun_natives::resolve("collections", "classSetSymbolFunction") else {
            return;
        };
        let Some((well_known_id, _)) = zun_natives::resolve("collections", "symbolWellKnown") else {
            return;
        };
        for &(class_idx, symbol_id, func_index) in &self.class_symbol_methods {
            let class_rid = hir::Expr {
                kind: hir::ExprKind::Number((class_idx + 1) as f64),
                ty: Ty::Int(IntKind::U32),
                span,
            };
            let symbol = hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: well_known_id,
                    args: vec![hir::Expr {
                        kind: hir::ExprKind::Number(symbol_id as f64),
                        ty: Ty::F64,
                        span,
                    }],
                },
                ty: Ty::Symbol,
                span,
            };
            let function = hir::Expr {
                kind: hir::ExprKind::FunctionRef { func: func_index },
                ty: Ty::Function(func_index),
                span,
            };
            body.push(hir::Stmt::Expr(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: set_id,
                    args: vec![class_rid, symbol, function],
                },
                ty: Ty::Void,
                span,
            }));
        }
    }

    /// Register, at class-definition time, (a) each class's zero-parameter
    /// instance `valueOf`/`toString` (own or inherited) via `classSetMethod`,
    /// so the interpreter floor can resolve them by name for ToPrimitive, and
    /// (b) each literal field-initializer default via `classSetFieldDefault`, so
    /// runtime construction (`Reflect.construct`, proxy `construct`) can fill
    /// absent trailing args. Non-literal initializers are not registered.
    fn append_class_named_methods_and_defaults(&self, body: &mut Vec<hir::Stmt>, span: Span) {
        let class_rid = |class_idx: usize| hir::Expr {
            kind: hir::ExprKind::Number((class_idx + 1) as f64),
            ty: Ty::Int(IntKind::U32),
            span,
        };
        if let Some((set_method, _)) = zun_natives::resolve("collections", "classSetMethod") {
            for class_idx in 0..self.classes.len() {
                for name in ["valueOf", "toString", "get_name"] {
                    let Some(func) = self.method_of(class_idx as u32, name) else { continue };
                    let sig = &self.signatures[func as usize];
                    if sig.is_static || sig.params.len() != 1 {
                        continue;
                    }
                    // A routed (decorator-replaceable) method is already
                    // registered, and its entry may have been replaced.
                    if self.routed_methods.contains_key(&func) {
                        continue;
                    }
                    body.push(hir::Stmt::Expr(hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id: set_method,
                            args: vec![
                                class_rid(class_idx),
                                hir::Expr {
                                    kind: hir::ExprKind::String(name.to_string()),
                                    ty: Ty::String,
                                    span,
                                },
                                hir::Expr {
                                    kind: hir::ExprKind::FunctionRef { func },
                                    ty: Ty::Function(func),
                                    span,
                                },
                            ],
                        },
                        ty: Ty::Void,
                        span,
                    }));
                }
            }
        }
        // Static fields with a literal initializer (`static NONE: number = 0`)
        // are stored once at module initialization.
        for class_idx in 0..self.classes.len() {
            let fields = self.full_fields(class_idx as u32);
            let initializers = self.full_initializers(class_idx as u32);
            for (slot, (_, field_ty, is_static, _)) in fields.iter().enumerate() {
                if !*is_static {
                    continue;
                }
                let Some(Some(init)) = initializers.get(slot) else { continue };
                let (kind, ty) = match (init.as_ref(), *field_ty) {
                    (ast::Expr::Number { value, .. }, Ty::F64) => {
                        (hir::ExprKind::Number(*value), Ty::F64)
                    }
                    (ast::Expr::String { value, .. }, Ty::String) => {
                        (hir::ExprKind::String(value.clone()), Ty::String)
                    }
                    (ast::Expr::Bool { value, .. }, Ty::Boolean) => {
                        (hir::ExprKind::Boolean(*value), Ty::Boolean)
                    }
                    _ => continue,
                };
                body.push(hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::StaticFieldSet {
                        class: class_idx as u32,
                        field_slot: slot as u16,
                        value: Box::new(hir::Expr { kind, ty, span }),
                    },
                    ty,
                    span,
                }));
            }
        }
        if let Some((set_default, _)) = zun_natives::resolve("collections", "classSetFieldDefault") {
            for class_idx in 0..self.classes.len() {
                let fields = self.full_fields(class_idx as u32);
                let initializers = self.full_initializers(class_idx as u32);
                for (slot, (_, field_ty, _, _)) in fields.iter().enumerate() {
                    let Some(Some(init)) = initializers.get(slot) else { continue };
                    let (kind, ty) = match (init.as_ref(), *field_ty) {
                        (ast::Expr::Number { value, .. }, Ty::F64 | Ty::Any) => {
                            (hir::ExprKind::Number(*value), Ty::F64)
                        }
                        (ast::Expr::String { value, .. }, Ty::String | Ty::Any) => {
                            (hir::ExprKind::String(value.clone()), Ty::String)
                        }
                        (ast::Expr::Bool { value, .. }, Ty::Boolean | Ty::Any) => {
                            (hir::ExprKind::Boolean(*value), Ty::Boolean)
                        }
                        _ => continue,
                    };
                    body.push(hir::Stmt::Expr(hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id: set_default,
                            args: vec![
                                class_rid(class_idx),
                                hir::Expr {
                                    kind: hir::ExprKind::Number(slot as f64),
                                    ty: Ty::Int(IntKind::U32),
                                    span,
                                },
                                hir::Expr {
                                    kind: hir::ExprKind::BoxAny {
                                        value: Box::new(hir::Expr { kind, ty, span }),
                                    },
                                    ty: Ty::Any,
                                    span,
                                },
                            ],
                        },
                        ty: Ty::Void,
                        span,
                    }));
                }
            }
        }
    }

    /// Register, ahead of every decorator, the original body of each routed
    /// (decorated, function-returning) instance method in `class_named_methods`,
    /// plus the undecorated overrides of that method in subclasses, so a routed
    /// call site finds the runtime class's own implementation.
    fn register_routed_methods(&self, program: &ast::Program, body: &mut Vec<hir::Stmt>, span: Span) {
        let Some((set_method, _)) = zun_natives::resolve("collections", "classSetMethod") else {
            return;
        };
        let mut routed: Vec<(u32, u32)> = self.routed_methods.iter().map(|(&f, &c)| (f, c)).collect();
        routed.sort_unstable();
        let register = |class: u32, name: &str, func: u32| {
            hir::Stmt::Expr(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: set_method,
                    args: vec![
                        hir::Expr {
                            kind: hir::ExprKind::Number((class + 1) as f64),
                            ty: Ty::Int(IntKind::U32),
                            span,
                        },
                        hir::Expr {
                            kind: hir::ExprKind::String(name.to_string()),
                            ty: Ty::String,
                            span,
                        },
                        hir::Expr {
                            kind: hir::ExprKind::FunctionRef { func },
                            ty: Ty::Function(func),
                            span,
                        },
                    ],
                },
                ty: Ty::Void,
                span,
            })
        };
        for (func, class) in routed {
            let Some(name) = self.method_name_of(class, func) else { continue };
            body.push(register(class, &name, func));
            // Undecorated overrides in subclasses.
            for (sub, sub_class) in program.classes.iter().enumerate() {
                let sub = sub as u32;
                if sub == class || !self.is_subclass(sub, class) {
                    continue;
                }
                if !sub_class.methods.iter().any(|m| self.text(&m.name) == name) {
                    continue;
                }
                let Some(&own) = self.methods_by_class.get(sub as usize).and_then(|m| m.get(&name)) else {
                    continue;
                };
                if !self.routed_methods.contains_key(&own) {
                    body.push(register(sub, &name, own));
                }
            }
        }
    }

    /// Lower a call to a routed (decorator-replaceable) instance method as an
    /// indirect call: `classBoundMethod(receiver, declaringClass, name)` looks the
    /// receiver's runtime-class entry up in `class_named_methods` and binds the
    /// receiver as `this`; the bound function value is then invoked with the
    /// remaining arguments. The dynamic call yields `Any`, converted back to the
    /// method's return type (a `void` method's result is left as `Any`).
    fn routed_method_call(
        &self,
        func: u32,
        owner: u32,
        call_args: &mut Vec<hir::Expr>,
        return_ty: Ty,
        span: Span,
    ) -> Option<hir::Expr> {
        if matches!(return_ty, Ty::Function(_)) {
            return None;
        }
        let (bound_id, _) = zun_natives::resolve("collections", "classBoundMethod")?;
        let name = self.method_name_of(owner, func)?;
        let receiver = call_args.remove(0);
        let boxed = hir::Expr {
            kind: hir::ExprKind::BoxAny { value: Box::new(receiver) },
            ty: Ty::Any,
            span,
        };
        let callee = hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id: bound_id,
                args: vec![
                    boxed,
                    hir::Expr {
                        kind: hir::ExprKind::Number((owner + 1) as f64),
                        ty: Ty::Int(IntKind::U32),
                        span,
                    },
                    hir::Expr { kind: hir::ExprKind::String(name), ty: Ty::String, span },
                ],
            },
            ty: Ty::Function(hir::DYNAMIC_FUNCTION),
            span,
        };
        let call = hir::Expr {
            kind: hir::ExprKind::IndirectCall {
                callee: Box::new(callee),
                args: std::mem::take(call_args),
                func: hir::DYNAMIC_FUNCTION,
            },
            ty: Ty::Any,
            span,
        };
        if matches!(return_ty, Ty::Void | Ty::Error) {
            return Some(call);
        }
        Some(hir::Expr {
            kind: hir::ExprKind::Convert { value: Box::new(call) },
            ty: return_ty,
            span,
        })
    }

    /// The source-level name of method `func` declared by `class`.
    fn method_name_of(&self, class: u32, func: u32) -> Option<String> {
        self.methods_by_class
            .get(class as usize)?
            .iter()
            .find(|(_, &index)| index == func)
            .map(|(name, _)| name.clone())
    }

    fn append_runtime_decorator_calls(
        &self,
        program: &ast::Program,
        body: &mut Vec<hir::Stmt>,
        span: Span,
    ) {
        self.register_routed_methods(program, body, span);
        for (class_idx, class) in program.classes.iter().enumerate() {
            let class_idx = class_idx as u32;
            let class_name = self.text(&class.name).to_string();
            let full_fields = self.full_fields(class_idx);
            // Field decorators run at class-definition time with
            // `kind: "field"`. Private (`#x`) fields stay deferred. A decorator
            // returning a function installs it as the field's initializer
            // transform, applied per instance by the `new` hook.
            for field in &class.fields {
                if field.is_private {
                    continue;
                }
                let field_name = self.text(&field.name).to_string();
                let slot = full_fields
                    .iter()
                    .position(|(name, _, _, _)| *name == field_name)
                    .map_or(-1, |slot| slot as i32);
                for decorator in field.decorators.iter().rev() {
                    let target = DecoratorTarget {
                        class_idx,
                        slot,
                        replaces_method: false,
                        transforms_field: !field.is_static && slot >= 0,
                    };
                    if let Some(stmt) = self.runtime_decorator_call(
                        decorator,
                        || decorator_null_value(span),
                        "field",
                        &field_name,
                        field.is_static,
                        target,
                        span,
                    ) {
                        body.push(stmt);
                    }
                }
            }
            // Member decorators first (TC39: before the class decorator).
            for method in &class.methods {
                let method_name = self.text(&method.name).to_string();
                let Some(&func_index) = self
                    .methods_by_class
                    .get(class_idx as usize)
                    .and_then(|m| m.get(&method_name))
                else {
                    continue;
                };
                let routed = self.routed_methods.contains_key(&func_index);
                // Bottom-to-top per element. A routed method hands its current
                // body to each decorator as a `functionBox`ed `value`.
                for decorator in method.decorators.iter().rev() {
                    let make_value = || {
                        if routed {
                            self.method_value_expr(class_idx, &method_name, span)
                        } else {
                            decorator_null_value(span)
                        }
                    };
                    let target = DecoratorTarget {
                        class_idx,
                        slot: -1,
                        replaces_method: routed,
                        transforms_field: false,
                    };
                    if let Some(stmt) = self.runtime_decorator_call(
                        decorator,
                        make_value,
                        "method",
                        &method_name,
                        method.is_static,
                        target,
                        span,
                    ) {
                        body.push(stmt);
                    }
                }
            }
            // Class decorators after members, bottom-to-top.
            for decorator in class.decorators.iter().rev() {
                let target = DecoratorTarget {
                    class_idx,
                    slot: -1,
                    replaces_method: false,
                    transforms_field: false,
                };
                if let Some(stmt) = self.runtime_decorator_call(
                    decorator,
                    || decorator_null_value(span),
                    "class",
                    &class_name,
                    false,
                    target,
                    span,
                ) {
                    body.push(stmt);
                }
            }
        }
    }

    /// `classMethodBox(classRid, name)`: the method's current body as a
    /// `functionBox` record in an `Any`, the `value` a method decorator receives.
    fn method_value_expr(&self, class_idx: u32, name: &str, span: Span) -> hir::Expr {
        let Some((id, _)) = zun_natives::resolve("collections", "classMethodBox") else {
            return decorator_null_value(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![
                    hir::Expr {
                        kind: hir::ExprKind::Number((class_idx + 1) as f64),
                        ty: Ty::Int(IntKind::U32),
                        span,
                    },
                    hir::Expr { kind: hir::ExprKind::String(name.to_string()), ty: Ty::String, span },
                ],
            },
            ty: Ty::Any,
            span,
        }
    }

    /// Build one statement for a runtime decorator: a direct `dec(value,
    /// context)` call for a bare `@dec` whose callee is a `(any, any)`
    /// top-level function, or, for a factory `@dec(args)`, `applyDecorator(
    /// dec(args), value, context)`. Returns `None` when the decorator is inert
    /// (callee is not a resolvable top-level function, or a factory with
    /// non-literal arguments or a non-function return).
    fn runtime_decorator_call(
        &self,
        decorator: &ast::Decorator,
        make_value: impl Fn() -> hir::Expr,
        kind: &str,
        name: &str,
        is_static: bool,
        target: DecoratorTarget,
        span: Span,
    ) -> Option<hir::Stmt> {
        let callee_span = decorator.callee.span();
        let callee = self
            .source
            .get(callee_span.start as usize..callee_span.end as usize)?;
        // Must resolve to a top-level function value (else inert / macro).
        let dec_index = *self.by_name.get(callee)?;
        let signature = self.signatures.get(dec_index as usize)?;

        // Typed context: `(value: any, context: DecoratorContext)`. Lowered to
        // `ctx = make(..); dec(null, ctx); finish(ctx)` so `addInitializer`
        // callbacks live in typed class fields (functions cannot cross `Any`).
        if decorator.args.is_none() {
            if let Some(stmt) =
                self.typed_context_decorator(dec_index, signature, make_value(), kind, name, is_static, target, span)
            {
                return Some(stmt);
            }
        }

        let context = hir::Expr {
            kind: hir::ExprKind::DynamicObjectLiteral {
                entries: vec![
                    decorator_context_entry(
                        "kind",
                        hir::ExprKind::String(kind.to_string()),
                        Ty::String,
                        span,
                    ),
                    decorator_context_entry(
                        "name",
                        hir::ExprKind::String(name.to_string()),
                        Ty::String,
                        span,
                    ),
                    decorator_context_entry(
                        "static",
                        hir::ExprKind::Boolean(is_static),
                        Ty::Boolean,
                        span,
                    ),
                ],
            },
            ty: Ty::DynamicObject,
            span,
        };
        let context_arg = hir::Expr {
            kind: hir::ExprKind::BoxAny {
                value: Box::new(context),
            },
            ty: Ty::Any,
            span,
        };

        match &decorator.args {
            None => {
                // Bare `@dec`: the callee is itself the decorator and must be
                // exactly `(value: any, context: any)`.
                if signature.params.len() != 2
                    || signature.rest_param.is_some()
                    || !matches!(signature.params[0], Ty::Any)
                    || !matches!(signature.params[1], Ty::Any)
                {
                    return None;
                }
                Some(hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::Call {
                        func: dec_index,
                        args: vec![make_value(), context_arg],
                    },
                    // The return value is discarded by the expression statement.
                    ty: signature.return_ty,
                    span,
                }))
            }
            Some(args) => {
                // Decorator factory `@dec(args)`: evaluate `dec(args)` to produce
                // the actual decorator function value, then invoke it with
                // `(value, context)` via `applyDecorator`. The factory must take
                // exactly `args` (literal) parameters and return a function value.
                if signature.rest_param.is_some() || signature.params.len() != args.len() {
                    return None;
                }
                if !matches!(signature.return_ty, Ty::Function(_) | Ty::Any) {
                    return None;
                }
                let mut lowered = Vec::with_capacity(args.len());
                for arg in args {
                    lowered.push(self.lower_decorator_literal_arg(arg, span)?);
                }
                let factory_call = hir::Expr {
                    kind: hir::ExprKind::Call {
                        func: dec_index,
                        args: lowered,
                    },
                    ty: signature.return_ty,
                    span,
                };
                let (apply_id, _) = zun_natives::resolve("collections", "applyDecorator")?;
                Some(hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: apply_id,
                        args: vec![factory_call, make_value(), context_arg],
                    },
                    ty: Ty::Void,
                    span,
                }))
            }
        }
    }

    /// Lowering for a decorator whose second parameter is the prelude's typed
    /// `DecoratorContext`. Uses module-initializer local slot 0.
    #[allow(clippy::too_many_arguments)]
    fn typed_context_decorator(
        &self,
        dec_index: u32,
        signature: &Signature,
        value: hir::Expr,
        kind: &str,
        name: &str,
        is_static: bool,
        target: DecoratorTarget,
        span: Span,
    ) -> Option<hir::Stmt> {
        let ctx_class = *self.classes_by_name.get("DecoratorContext")?;
        let ctx_ty = Ty::Instance(ctx_class);
        if signature.params.len() != 2
            || signature.rest_param.is_some()
            || !matches!(signature.params[0], Ty::Any)
            || signature.params[1] != ctx_ty
        {
            return None;
        }
        let make = self.method_of(ctx_class, "make")?;
        let finish = self.method_of(ctx_class, "finish")?;
        let expr = |kind: hir::ExprKind, ty: Ty| hir::Expr { kind, ty, span };
        let local = || expr(hir::ExprKind::Local(0), ctx_ty);
        let call = expr(
            hir::ExprKind::Call { func: dec_index, args: vec![value, local()] },
            signature.return_ty,
        );
        let class_rid = |ty: Ty| expr(hir::ExprKind::Number((target.class_idx + 1) as f64), ty);
        // A function returned by a method decorator replaces the method; one
        // returned by a field decorator becomes its initializer transform.
        let returns_function = matches!(signature.return_ty, Ty::Function(_));
        let result_stmt = if returns_function && target.replaces_method {
            let (id, _) = zun_natives::resolve("collections", "classReplaceMethod")?;
            hir::Stmt::Expr(expr(
                hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![
                        class_rid(Ty::Int(IntKind::U32)),
                        expr(hir::ExprKind::String(name.to_string()), Ty::String),
                        call,
                    ],
                },
                Ty::Void,
            ))
        } else if returns_function && target.transforms_field {
            let (id, _) = zun_natives::resolve("collections", "classSetFieldTransform")?;
            hir::Stmt::Expr(expr(
                hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![
                        class_rid(Ty::Int(IntKind::U32)),
                        expr(hir::ExprKind::Number(target.slot as f64), Ty::Int(IntKind::U32)),
                        call,
                    ],
                },
                Ty::Void,
            ))
        } else {
            hir::Stmt::Expr(call)
        };
        Some(hir::Stmt::Group(vec![
            hir::Stmt::Let {
                slot: 0,
                init: expr(
                    hir::ExprKind::StaticCall {
                        func: make,
                        args: vec![
                            expr(hir::ExprKind::String(kind.to_string()), Ty::String),
                            expr(hir::ExprKind::String(name.to_string()), Ty::String),
                            expr(hir::ExprKind::Boolean(is_static), Ty::Boolean),
                            class_rid(Ty::F64),
                            expr(hir::ExprKind::Number(target.slot as f64), Ty::F64),
                        ],
                    },
                    ctx_ty,
                ),
            },
            result_stmt,
            hir::Stmt::Expr(expr(
                hir::ExprKind::StaticCall { func: finish, args: vec![local()] },
                Ty::Void,
            )),
        ]))
    }

    /// Lower a decorator-factory argument. Only literals are supported (the
    /// module initializer has no local scope); anything else makes the whole
    /// decorator inert.
    fn lower_decorator_literal_arg(&self, arg: &ast::Expr, span: Span) -> Option<hir::Expr> {
        Some(match arg {
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
            _ => return None,
        })
    }

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
            if matches!(f.body.statements.first(), Some(ast::Stmt::Let { name, .. }) if self.text(name) == "arguments") {
                self.arguments_functions.insert(self.signatures.len() as u32);
            }
            let params: Vec<Ty> = f
                .params
                .iter()
                .map(|p| match &p.ty {
                    // An unannotated parameter is untyped (`any`), as in JS/TS
                    // without `--noImplicitAny`; types are erased anyway.
                    Some(t) => self.resolve_annotation(t),
                    None => Ty::Any,
                })
                .collect();
            let return_ty = match &f.return_type {
                Some(t) => self.resolve_annotation(t),
                // An unannotated return type is `any` when the body returns a
                // value, else `void` — so a value-returning JS function is not
                // forced to `void` (which would reject every `return x`).
                None => {
                    if f.is_async {
                        Ty::Promise
                    } else if block_returns_value(&f.body) {
                        Ty::Any
                    } else {
                        Ty::Void
                    }
                }
            };
            let optional_count = {
                let mut count = 0;
                for (index, p) in f.params.iter().enumerate().rev() {
                    if p.is_rest {
                        continue;
                    }
                    if p.default_value.is_some() || params.get(index) == Some(&Ty::Any) {
                        count += 1;
                    } else {
                        break;
                    }
                }
                count
            };
            self.signatures.push(Signature {
                name,
                params,
                return_ty,
                // Trailing defaulted parameters, and trailing dynamic (`any`) ones —
                // JS lets any argument be omitted, which reads as `undefined`.
                optional_count,
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
                    "WeakSet" => Ty::WeakSet,
                    "WeakRef" => Ty::WeakRef,
                    "RegExp" => Ty::RegExp,
                    "BigInt" => Ty::BigInt,
                    "FinalizationRegistry" => Ty::FinalizationRegistry,
                    _ => Ty::Instance(class_idx as u32),
                };
                // A static method that mentions `this`/`super` receives the
                // receiving class value as a hidden first parameter.
                let hidden_class_param = m.is_static
                    && !m.is_extern
                    && self
                        .source
                        .get(m.body.span.start as usize..m.body.span.end as usize)
                        .is_some_and(|text| {
                            text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
                                .any(|word| word == "this" || word == "super")
                        });
                let mut params = if m.is_static {
                    if hidden_class_param { vec![Ty::Any] } else { Vec::new() }
                } else {
                    vec![receiver_ty]
                };
                for p in &m.params {
                    params.push(match &p.ty {
                        Some(t) => self.resolve_annotation(t),
                        // An untyped parameter is dynamic (`any`), matching JS.
                        None => Ty::Any,
                    });
                }
                let fluent = m.return_type.is_none()
                    && m.accessor.is_none()
                    && !m.is_static
                    && !m.is_async
                    && !m.is_generator
                    && !program.classes.iter().any(|other| {
                        other.base.is_some_and(|base| self.text(&base) == self.text(&class.name))
                    })
                    && self.block_returns_only_this(&m.body);
                let return_ty = match &m.return_type {
                    Some(t) => self.resolve_annotation(t),
                    None => {
                        if fluent {
                            Ty::Instance(class_idx as u32)
                        } else if block_returns_value(&m.body) {
                            Ty::Any
                        } else {
                            Ty::Void
                        }
                    }
                };
                let func_index = self.signatures.len() as u32;
                if self.methods_by_class[class_idx]
                    .insert(name.clone(), func_index)
                    .is_some()
                {
                    self.error(m.name.span, format!("duplicate method `{name}`"));
                }
                let name = if hidden_class_param {
                    self.static_this.insert(func_index, class_idx as u32);
                    format!("{name}@this")
                } else {
                    name
                };
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

        // Computed-symbol-key methods follow all vtable methods, in (class,
        // method) order, so every existing func index is unchanged. Each is
        // modeled as a static function (no implicit `this`) and recorded for
        // the module-initializer side-table registration.
        for (class_idx, class) in program.classes.iter().enumerate() {
            for sm in &class.symbol_methods {
                // A non-static symbol method receives the instance as an implicit
                // leading `this` (flagged by the `@this` name suffix, which the
                // interpreter's `with_this_arg` keys on); a static one has none.
                let takes_this = !sm.function.is_static;
                let mut params = if takes_this {
                    vec![Ty::Instance(class_idx as u32)]
                } else {
                    Vec::new()
                };
                for p in &sm.function.params {
                    params.push(match &p.ty {
                        Some(t) => self.resolve_annotation(t),
                        // An untyped parameter is dynamic (`any`), matching JS.
                        None => Ty::Any,
                    });
                }
                let return_ty = match &sm.function.return_type {
                    Some(t) => self.resolve_annotation(t),
                    None => {
                        if block_returns_value(&sm.function.body) {
                            Ty::Any
                        } else {
                            Ty::Void
                        }
                    }
                };
                let func_index = self.signatures.len() as u32;
                self.signatures.push(Signature {
                    name: if takes_this {
                        format!("[symbol]@{class_idx}@this")
                    } else {
                        format!("[symbol]@{class_idx}")
                    },
                    params,
                    return_ty,
                    optional_count: sm
                        .function
                        .params
                        .iter()
                        .rev()
                        .take_while(|p| p.default_value.is_some())
                        .count(),
                    rest_param: sm
                        .function
                        .params
                        .iter()
                        .position(|parameter| parameter.is_rest)
                        .map(|index| index + usize::from(takes_this)),
                    is_generator: false,
                    is_static: !takes_this,
                });
                match self.well_known_symbol_of_key(&sm.key) {
                    Some(symbol) => {
                        self.class_symbol_methods.push((
                            class_idx as u32,
                            symbol.id().get(),
                            func_index,
                        ));
                        if symbol == zun_core::WellKnownSymbol::HasInstance {
                            self.classes_with_has_instance.insert(class_idx as u32);
                        }
                        if symbol == zun_core::WellKnownSymbol::Iterator {
                            self.classes_with_iterator.insert(class_idx as u32);
                        }
                    }
                    None => self.error(
                        sm.key.span(),
                        "a class computed member key must be a well-known Symbol \
                         (e.g. `Symbol.hasInstance`, `Symbol.iterator`)",
                    ),
                }
            }
        }
    }

    /// Recognize a computed class-member key that is a well-known Symbol
    /// (`Symbol.hasInstance`, `Symbol.iterator`, …).
    /// `re[Symbol.replace|match|matchAll|split|search](str, ...)` on a RegExp
    /// value (a literal, or any expression statically typed `RegExp`).
    fn is_regex_symbol_call(&mut self, cx: &mut FunctionCx, callee: &ast::Expr, args: &[ast::Expr]) -> bool {
        let (ast::Expr::Index { object, index, .. }, Some(first)) = (callee, args.first()) else { return false };
        if matches!(first, ast::Expr::Spread { .. }) {
            return false;
        }
        if !matches!(
            self.well_known_symbol_of_key(index),
            Some(zun_core::WellKnownSymbol::Split)
                | Some(zun_core::WellKnownSymbol::Replace)
                | Some(zun_core::WellKnownSymbol::Match)
                | Some(zun_core::WellKnownSymbol::Search)
                | Some(zun_core::WellKnownSymbol::MatchAll)
        ) {
            return false;
        }
        matches!(object.as_ref(), ast::Expr::RegexLiteral { .. }) || self.check_expr(cx, object).ty == Ty::RegExp
    }

    fn well_known_symbol_of_key(&self, key: &ast::Expr) -> Option<zun_core::WellKnownSymbol> {
        let (object, field) = match key {
            ast::Expr::Field { object, field, .. } => {
                let ast::Expr::Ident(name) = object.as_ref() else { return None };
                (self.text(name), self.text(field))
            }
            ast::Expr::StaticField { class, field, .. } => (self.text(class), self.text(field)),
            _ => return None,
        };
        if object != "Symbol" || self.by_name.contains_key("Symbol") {
            return None;
        }
        zun_core::WellKnownSymbol::from_member(field)
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
            } else if saw_default && !parameter.is_rest {
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
        if f.is_async && !f.is_generator {
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
        // An async body is checked (and lowered) as a coroutine returning `any`;
        // the Promise typing lives only on the signature that callers see.
        let return_ty = if f.is_async { Ty::Any } else { return_ty };

        let mut cx = FunctionCx {
            locals: Vec::new(),
            scopes: vec![Scope::new()],
            return_ty,
            is_generator: f.is_generator || f.is_async,
            loop_depth: 0,
            loop_labels: Vec::new(),
            switch_depth: 0,
            block_labels: Vec::new(),
            mutated: HashSet::new(),
            mutated_names: HashSet::new(),
            tdz_names: HashSet::new(),
            param_slot_count: 0,
            boxed: HashSet::new(),
            captures: HashMap::new(),
            this_class,
            is_constructor: this_class.is_some() && self.text(&f.name) == "constructor",
            is_module_scope: false,
            finally_depth: 0,
        };
        // For a method, slot 0 is the implicit `this`; the declared params
        // follow it. For a free function, the params occupy every slot.
        let mut param_tys_iter = param_tys.iter();
        self.cur_static_class = self.static_this.get(&(index as u32)).copied();
        if this_class.is_some() || self.cur_static_class.is_some() {
            let this_ty = *param_tys_iter.next().expect("method has a this param");
            cx.declare("this", this_ty);
        }
        let mut declared_params: Vec<(String, hir::LocalSlot)> = Vec::new();
        for (p, &ty) in f.params.iter().zip(param_tys_iter) {
            let pname = self.text(&p.name).to_string();
            let slot = cx.declare(&pname, ty);
            declared_params.push((pname, slot));
        }
        self.collect_mutated_names(&f.body.statements, &mut cx.mutated_names);
        // A closure created in a default expression captures the parameter scope.
        cx.param_slot_count = cx.locals.len();
        let mut default_names: HashSet<String> = HashSet::new();
        for parameter in &f.params {
            if let Some(default_value) = parameter.default_value.as_deref() {
                capture_analysis::collect_mutated_expr(self, default_value, &mut default_names);
            }
        }
        cx.mutated_names.extend(default_names.iter().cloned());
        for (pname, slot) in declared_params {
            // A body-level `var` of the same name is a separate binding (the
            // parameter scope keeps the argument for default-value closures), so
            // body closures never capture the parameter itself.
            let shadowed_by_body = f.params.iter().any(|p| p.default_value.is_some() || p.is_rest)
                && f.body.statements.iter().any(
                    |statement| matches!(statement, ast::Stmt::Let { name, .. } if self.text(name) == pname),
                );
            let needs_cell = if shadowed_by_body {
                default_names.contains(&pname) && default_names.contains(&format!("{pname}"))
            } else {
                cx.mutated_names.contains(&pname)
                    && cx.mutated_names.contains(&format!("{pname}"))
            };
            if needs_cell && !matches!(cx.locals[slot as usize], Ty::Function(_)) {
                cx.boxed.insert(slot);
            }
        }

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
            boxed_locals::box_parameters(&mut body, &cx.boxed, &cx.locals, param_tys.len());
            // A derived constructor that touches `this` in a top-level
            // statement before its `super(...)` call throws a ReferenceError.
            if cx.is_constructor
                && this_class.is_some_and(|c| self.classes[c as usize].base.is_some())
            {
                let mut this_before_super = false;
                for statement in &f.body.statements {
                    let ast::Stmt::Expr(expr) = statement else { continue };
                    let is_super_call = matches!(expr, ast::Expr::Call { callee, .. } if self.text(callee) == "super");
                    if is_super_call {
                        break;
                    }
                    let text = &self.source[expr.span().start as usize..expr.span().end as usize];
                    if text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).any(|word| word == "this") {
                        this_before_super = true;
                        break;
                    }
                }
                if this_before_super {
                    if let Some(&class_idx) = self.classes_by_name.get("ReferenceError") {
                        let message = ast::Expr::String {
                            value: "Must call super constructor in derived class before accessing 'this' or returning from derived constructor".to_string(),
                            span: f.name.span,
                        };
                        let thrown = self.check_error_new(
                            &mut cx,
                            class_idx,
                            "ReferenceError",
                            std::slice::from_ref(&message),
                            f.name.span,
                        );
                        body.insert(0, hir::Stmt::Throw(thrown));
                    }
                }
            }
            body
        };
        let defaults = f
            .params
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                parameter.default_value.as_deref().map(|value| {
                    cx.tdz_names = f.params[index..].iter().map(|p| self.text(&p.name).to_string()).collect();
                    let mut checked = self.check_expr(&mut cx, value);
                    cx.tdz_names.clear();
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

    /// A collision-free hidden local name (contains a space, which no source
    /// identifier can), unique per slot index within the function.
    fn fresh_temp_name(&self, cx: &FunctionCx) -> String {
        format!(" destr#{}", cx.locals.len())
    }

    /// Wrap a read with a destructuring default: `coalesceUndefined(read, d)`
    /// returns `d` when `read` is `undefined` (eager default evaluation).
    fn apply_default(
        &mut self,
        cx: &mut FunctionCx,
        read: hir::Expr,
        default: Option<&ast::Expr>,
        out: &mut Vec<hir::Stmt>,
    ) -> hir::Expr {
        let Some(default) = default else { return read };
        let span = read.span;
        let Some((id, _)) = zun_natives::resolve("array", "anyIsUndefined") else {
            self.error(span, "missing `anyIsUndefined` intrinsic");
            return read;
        };
        // The read happens once (into a temp); the default expression is
        // evaluated lazily, only when the read is `undefined`.
        let temp = cx.declare(&self.fresh_temp_name(cx), Ty::Any);
        out.push(hir::Stmt::Let { slot: temp, init: Self::box_to_any(read) });
        let default = self.check_expr(cx, default);
        let default = Self::box_to_any(default);
        let temp_read = || hir::Expr { kind: hir::ExprKind::Local(temp), ty: Ty::Any, span };
        hir::Expr {
            kind: hir::ExprKind::Conditional {
                condition: Box::new(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![temp_read()] },
                    ty: Ty::Boolean,
                    span,
                }),
                when_true: Box::new(default),
                when_false: Box::new(temp_read()),
            },
            ty: Ty::Any,
            span,
        }
    }

    /// Bind a declaration/for-of pattern from the value produced by `value`,
    /// appending the lowered binding statements to `out`. Array and object
    /// patterns stash `value` into a hidden temp and read each sub-binding
    /// from it; nested patterns recurse.
    fn bind_pattern(
        &mut self,
        cx: &mut FunctionCx,
        pattern: &ast::Pattern,
        value: hir::Expr,
        out: &mut Vec<hir::Stmt>,
    ) {
        let span = value.span;
        match pattern {
            ast::Pattern::Ident { name, ty } => {
                let declared = ty.as_ref().map(|t| self.resolve_annotation(t));
                let slot_ty = declared.unwrap_or(value.ty);
                let text = self.text(name).to_string();
                let slot = cx.declare(&text, slot_ty);
                out.push(hir::Stmt::Let { slot, init: value });
            }
            ast::Pattern::Array { elements, rest, .. } => {
                // A string destructures by code point.
                let value = if value.ty == Ty::String { self.spread_source_to_array(value) } else { value };
                if !matches!(value.ty, Ty::Array | Ty::Any | Ty::Error) {
                    self.error(span, "array destructuring requires an Array or any value");
                }
                // With defaults the iterator is pulled one element at a time so a
                // throwing default closes it before later elements are read.
                if value.ty == Ty::Any
                    && elements.iter().flatten().any(|element| element.default.is_some())
                {
                    if let (Some((open, _)), Some((step, _)), Some((rest_id, _)), Some((finish, _))) = (
                        zun_natives::resolve("array", "destructOpen"),
                        zun_natives::resolve("array", "destructStep"),
                        zun_natives::resolve("array", "destructRest"),
                        zun_natives::resolve("array", "destructFinish"),
                    ) {
                        let source = cx.declare(&self.fresh_temp_name(cx), Ty::Any);
                        out.push(hir::Stmt::Let {
                            slot: source,
                            init: hir::Expr {
                                kind: hir::ExprKind::CallBuiltin { id: open, args: vec![value] },
                                ty: Ty::Any,
                                span,
                            },
                        });
                        let local = || hir::Expr { kind: hir::ExprKind::Local(source), ty: Ty::Any, span };
                        let finish_call = |throwing: bool| hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id: finish,
                                args: vec![
                                    local(),
                                    hir::Expr { kind: hir::ExprKind::Boolean(throwing), ty: Ty::Boolean, span },
                                ],
                            },
                            ty: Ty::Boolean,
                            span,
                        };
                        let mut inner: Vec<hir::Stmt> = Vec::new();
                        for (index, element) in elements.iter().enumerate() {
                            let Some(element) = element else {
                                // A hole still consumes one value.
                                inner.push(hir::Stmt::Expr(hir::Expr {
                                    kind: hir::ExprKind::CallBuiltin {
                                        id: step,
                                        args: vec![
                                            local(),
                                            hir::Expr { kind: hir::ExprKind::Number(index as f64), ty: Ty::F64, span },
                                        ],
                                    },
                                    ty: Ty::Any,
                                    span,
                                }));
                                continue;
                            };
                            let read = hir::Expr {
                                kind: hir::ExprKind::CallBuiltin {
                                    id: step,
                                    args: vec![
                                        local(),
                                        hir::Expr { kind: hir::ExprKind::Number(index as f64), ty: Ty::F64, span },
                                    ],
                                },
                                ty: Ty::Any,
                                span,
                            };
                            let read = self.apply_default(cx, read, element.default.as_ref(), &mut inner);
                            self.bind_pattern(cx, &element.pattern, read, &mut inner);
                        }
                        if let Some(rest) = rest {
                            let read = hir::Expr {
                                kind: hir::ExprKind::CallBuiltin {
                                    id: rest_id,
                                    args: vec![
                                        local(),
                                        hir::Expr {
                                            kind: hir::ExprKind::Number(elements.len() as f64),
                                            ty: Ty::Int(IntKind::U64),
                                            span,
                                        },
                                    ],
                                },
                                ty: Ty::Array,
                                span,
                            };
                            self.bind_pattern(cx, rest, read, &mut inner);
                        }
                        let caught = cx.declare(&self.fresh_temp_name(cx), Ty::Any);
                        out.push(hir::Stmt::Try {
                            body: inner,
                            catches: vec![hir::CatchClause {
                                slot: caught,
                                ty: Ty::Any,
                                body: vec![
                                    hir::Stmt::Expr(finish_call(true)),
                                    hir::Stmt::Throw(hir::Expr {
                                        kind: hir::ExprKind::Local(caught),
                                        ty: Ty::Any,
                                        span,
                                    }),
                                ],
                            }],
                            finally_body: None,
                        });
                        out.push(hir::Stmt::Expr(finish_call(false)));
                        return;
                    }
                }
                // A dynamic source may hold a string or any iterable, not just
                // an Array: materialize it through the iteration protocol first.
                let value = if value.ty == Ty::Any {
                    // Without a rest element only `elements.len()` values are taken
                    // and an unfinished iterator is closed afterwards.
                    let limited = rest.is_none().then(|| zun_natives::resolve("array", "arrayDestructure")).flatten();
                    if let Some((id, _)) = limited {
                        hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id,
                                args: vec![
                                    value,
                                    hir::Expr {
                                        kind: hir::ExprKind::Number(elements.len() as f64),
                                        ty: Ty::Int(IntKind::U32),
                                        span,
                                    },
                                ],
                            },
                            ty: Ty::Array,
                            span,
                        }
                    } else {
                    match zun_natives::resolve("array", "arrayFrom") {
                        Some((id, _)) => hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: vec![value] },
                            ty: Ty::Array,
                            span,
                        },
                        None => value,
                    }
                    }
                } else {
                    value
                };
                let temp = cx.declare(&self.fresh_temp_name(cx), value.ty);
                let temp_ty = value.ty;
                out.push(hir::Stmt::Let { slot: temp, init: value });
                for (index, element) in elements.iter().enumerate() {
                    let Some(element) = element else { continue };
                    let read = self.array_element_read(temp, temp_ty, index, span);
                    let read = self.apply_default(cx, read, element.default.as_ref(), out);
                    self.bind_pattern(cx, &element.pattern, read, out);
                }
                if let Some(rest) = rest {
                    let read = self.array_rest_read(temp, temp_ty, elements.len(), span);
                    self.bind_pattern(cx, rest, read, out);
                }
            }
            ast::Pattern::Object { properties, rest, .. } => {
                if !matches!(value.ty, Ty::Any | Ty::DynamicObject | Ty::Instance(_) | Ty::Error) {
                    self.error(span, "object destructuring requires an object value");
                }
                let temp = cx.declare(&self.fresh_temp_name(cx), value.ty);
                let temp_ty = value.ty;
                out.push(hir::Stmt::Let { slot: temp, init: value });
                let mut excluded: Vec<String> = Vec::new();
                // Computed keys are evaluated exactly once (into a temp) and
                // then also excluded from the `...rest` object.
                let mut computed: Vec<(hir::LocalSlot, Ty)> = Vec::new();
                for property in properties {
                    let (read, static_key) = if let ast::PropertyKey::Computed(key_expr) = &property.key {
                        let key_value = self.check_expr(cx, key_expr);
                        let key_ty = key_value.ty;
                        let key_temp = cx.declare(&self.fresh_temp_name(cx), key_ty);
                        out.push(hir::Stmt::Let { slot: key_temp, init: key_value });
                        computed.push((key_temp, key_ty));
                        let object = hir::Expr { kind: hir::ExprKind::Local(temp), ty: temp_ty, span };
                        (
                            hir::Expr {
                                kind: hir::ExprKind::DynamicGetValue {
                                    object: Box::new(object),
                                    key: Box::new(hir::Expr {
                                        kind: hir::ExprKind::Local(key_temp),
                                        ty: key_ty,
                                        span,
                                    }),
                                },
                                ty: Ty::Any,
                                span,
                            },
                            None,
                        )
                    } else {
                        self.object_member_read(cx, temp, temp_ty, &property.key, span)
                    };
                    if let Some(key) = static_key {
                        excluded.push(key);
                    }
                    let read = self.apply_default(cx, read, property.default.as_ref(), out);
                    self.bind_pattern(cx, &property.value, read, out);
                }
                if let Some(rest) = rest {
                    let extra: Vec<hir::Expr> = computed
                        .iter()
                        .map(|&(slot, ty)| hir::Expr { kind: hir::ExprKind::Local(slot), ty, span })
                        .collect();
                    let read = self.object_rest_read(temp, temp_ty, &excluded, extra, span);
                    self.bind_pattern(cx, rest, read, out);
                }
            }
        }
    }

    /// Read array element `index` from the temp holding the source array.
    /// Uses `arrayAt`, which yields `undefined` for an out-of-range index
    /// (so trailing holes and defaults read `undefined` rather than trapping).
    fn array_element_read(&mut self, temp: hir::LocalSlot, temp_ty: Ty, index: usize, span: Span) -> hir::Expr {
        let Some((id, _)) = zun_natives::resolve("array", "arrayAt") else {
            self.error(span, "missing `arrayAt` intrinsic");
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![
                    hir::Expr { kind: hir::ExprKind::Local(temp), ty: temp_ty, span },
                    hir::Expr { kind: hir::ExprKind::Number(index as f64), ty: Ty::F64, span },
                ],
            },
            ty: Ty::Any,
            span,
        }
    }

    /// Collect array elements from `start` to the end into a fresh Array.
    fn array_rest_read(&mut self, temp: hir::LocalSlot, temp_ty: Ty, start: usize, span: Span) -> hir::Expr {
        let Some((id, _)) = zun_natives::resolve("array", "arraySlice") else {
            self.error(span, "missing `arraySlice` intrinsic");
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![
                    hir::Expr { kind: hir::ExprKind::Local(temp), ty: temp_ty, span },
                    hir::Expr {
                        kind: hir::ExprKind::Number(start as f64),
                        ty: Ty::Int(IntKind::U64),
                        span,
                    },
                ],
            },
            ty: Ty::Array,
            span,
        }
    }

    /// Read one property from the temp. Returns the read expression and, for a
    /// statically-known string key, that key (for object-rest exclusion).
    fn object_member_read(
        &mut self,
        cx: &mut FunctionCx,
        temp: hir::LocalSlot,
        temp_ty: Ty,
        key: &ast::PropertyKey,
        span: Span,
    ) -> (hir::Expr, Option<String>) {
        let object = hir::Expr { kind: hir::ExprKind::Local(temp), ty: temp_ty, span };
        let name = match key {
            ast::PropertyKey::Ident(name) => Some(self.text(name).to_string()),
            ast::PropertyKey::String { value, .. } => Some(value.clone()),
            ast::PropertyKey::Number { value, .. } => Some(value.to_string()),
            ast::PropertyKey::Computed(_) => None,
        };
        if let Some(name) = name {
            // A typed instance reads its declared field directly.
            if let Ty::Instance(class) = temp_ty {
                if let Some((field_slot, ty)) = self.field_of(class, &name) {
                    return (
                        hir::Expr {
                            kind: hir::ExprKind::FieldGet { object: Box::new(object), class, field_slot },
                            ty,
                            span,
                        },
                        Some(name),
                    );
                }
            }
            return (
                hir::Expr {
                    kind: hir::ExprKind::DynamicGet { object: Box::new(object), name: name.clone() },
                    ty: Ty::Any,
                    span,
                },
                Some(name),
            );
        }
        // Computed key: evaluate the key expression and read dynamically.
        let ast::PropertyKey::Computed(expr) = key else { unreachable!() };
        let key_expr = self.check_expr(cx, expr);
        (
            hir::Expr {
                kind: hir::ExprKind::DynamicGetValue { object: Box::new(object), key: Box::new(key_expr) },
                ty: Ty::Any,
                span,
            },
            None,
        )
    }

    /// Collect the source object's remaining own keys into a fresh object.
    fn object_rest_read(
        &mut self,
        temp: hir::LocalSlot,
        temp_ty: Ty,
        excluded: &[String],
        extra: Vec<hir::Expr>,
        span: Span,
    ) -> hir::Expr {
        let Some((id, _)) = zun_natives::resolve("array", "objectRest") else {
            self.error(span, "missing `objectRest` intrinsic");
            return self.poison(span);
        };
        let excluded_elements = excluded
            .iter()
            .map(|key| {
                Some(hir::Expr {
                    kind: hir::ExprKind::String(key.clone()),
                    ty: Ty::String,
                    span,
                })
            })
            .chain(extra.into_iter().map(Some))
            .collect();
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![
                    hir::Expr { kind: hir::ExprKind::Local(temp), ty: temp_ty, span },
                    hir::Expr {
                        kind: hir::ExprKind::ArrayLiteral { elements: excluded_elements },
                        ty: Ty::Array,
                        span,
                    },
                ],
            },
            ty: Ty::DynamicObject,
            span,
        }
    }

    fn check_stmt(&mut self, cx: &mut FunctionCx, stmt: &ast::Stmt) -> hir::Stmt {
        match stmt {
            ast::Stmt::Destructure { pattern, init, .. } => {
                let value = self.check_expr(cx, init);
                let mut out = Vec::new();
                self.bind_pattern(cx, pattern, value, &mut out);
                hir::Stmt::Group(out)
            }
            ast::Stmt::Let { name, ty, init, .. } => {
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
            ast::Stmt::Expr(e) => hir::Stmt::Expr(self.check_expr(cx, e)),
            ast::Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                let narrowing = self.type_guard_narrowing(cx, cond);
                // A typed shadow copy would swallow writes to the guarded variable
                // (`if (Array.isArray(c)) c = c[0]`), so assigned names stay untyped.
                let narrowing = narrowing.filter(|(name, _, _)| {
                    let mut assigned = HashSet::new();
                    self.collect_mutated_names(&then_block.statements, &mut assigned);
                    !assigned.contains(name)
                });
                let cond = self.check_condition(cx, cond);
                let then_body = match narrowing {
                    // Inside the guarded branch the untyped local is shadowed by
                    // a typed copy (`typeof x === "string"`, `Array.isArray(x)`).
                    Some((name, slot, narrowed)) => {
                        cx.scopes.push(Scope::new());
                        let read = hir::Expr {
                            kind: hir::ExprKind::Local(slot),
                            ty: Ty::Any,
                            span: then_block.span,
                        };
                        let init = hir::Expr {
                            kind: hir::ExprKind::Convert { value: Box::new(read) },
                            ty: narrowed,
                            span: then_block.span,
                        };
                        let shadow = cx.declare(&name, narrowed);
                        let mut body = vec![hir::Stmt::Let { slot: shadow, init }];
                        body.extend(self.check_block(cx, then_block));
                        cx.scopes.pop();
                        body
                    }
                    None => self.check_block(cx, then_block),
                };
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
                extra_init,
                cond,
                update,
                body,
                ..
            } => {
                // The init clause's `let` is scoped to the loop.
                cx.scopes.push(Scope::new());
                let init = init.as_ref().map(|stmt| {
                    let first = self.check_stmt(cx, stmt);
                    if extra_init.is_empty() {
                        Box::new(first)
                    } else {
                        let mut group = vec![first];
                        for extra in extra_init {
                            group.push(self.check_stmt(cx, extra));
                        }
                        Box::new(hir::Stmt::Group(group))
                    }
                });
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
                cx.scopes.pop();
                hir::Stmt::For {
                    init,
                    cond,
                    update,
                    body,
                }
            }
            ast::Stmt::ForOf {
                pattern,
                iterable,
                body,
                span: _,
            } => {
                let iterable_expr = self.check_expr(cx, iterable);
                let (new_name, done_name, next_name) = match iterable_expr.ty {
                    Ty::Array => ("arrayIteratorNew", "arrayIteratorDone", "arrayIteratorNext"),
                    Ty::Map => ("mapIteratorNew", "mapIteratorDone", "mapIteratorNext"),
                    Ty::Set => ("setIteratorNew", "setIteratorDone", "setIteratorNext"),
                    // A string iterates by code point; reuses the array
                    // iterator's `done`/`next` over a code-point array.
                    Ty::String => ("stringIteratorNew", "arrayIteratorDone", "arrayIteratorNext"),
                    Ty::Any => ("generatorIteratorNew", "generatorIteratorDone", "generatorIteratorNext"),
                    // A user object implementing `[Symbol.iterator]`: a dynamic
                    // object literal's symbol-function slot, or a typed class's
                    // computed-symbol method. Both route through the generic
                    // iteration protocol, which calls `[Symbol.iterator]()` and
                    // drives the returned iterator's `next()`.
                    Ty::DynamicObject => {
                        ("generatorIteratorNew", "generatorIteratorDone", "generatorIteratorNext")
                    }
                    Ty::Instance(idx) if self.class_has_iterator(idx) => {
                        ("generatorIteratorNew", "generatorIteratorDone", "generatorIteratorNext")
                    }
                    Ty::Error => ("arrayIteratorNew", "arrayIteratorDone", "arrayIteratorNext"),
                    // A typed array iterates its elements; it builds an element
                    // sequence and reuses the array iterator's `done`/`next`,
                    // as a string iterator does. `DataView` is not iterable.
                    Ty::BufferView(kind) => match crate::typed_arrays::iterator_new(kind) {
                        Some(new_name) => (new_name, "arrayIteratorDone", "arrayIteratorNext"),
                        None => {
                            self.error(
                                iterable_expr.span,
                                "a DataView is not iterable".to_string(),
                            );
                            ("arrayIteratorNew", "arrayIteratorDone", "arrayIteratorNext")
                        }
                    },
                    other => {
                        self.error(
                            iterable_expr.span,
                            format!("type {other:?} is not iterable"),
                        );
                        ("arrayIteratorNew", "arrayIteratorDone", "arrayIteratorNext")
                    }
                };
                self.lower_for_iteration(
                    cx,
                    iterable_expr,
                    new_name,
                    done_name,
                    next_name,
                    pattern,
                    body,
                )
            }
            ast::Stmt::ForIn { pattern, object, body, .. } => {
                // `for (k in obj)` iterates the object's enumerable keys: build
                // `objectKeys(obj)` (an Array) and run the array-iterator loop.
                let object_expr = self.check_expr(cx, object);
                let span = object_expr.span;
                let keys_arg = if object_expr.ty == Ty::Any {
                    object_expr
                } else {
                    hir::Expr {
                        span,
                        kind: hir::ExprKind::BoxAny { value: Box::new(object_expr) },
                        ty: Ty::Any,
                    }
                };
                let Some((keys_id, _)) = zun_natives::resolve("collections", "objectForInKeys") else {
                    self.error(span, "missing `objectKeys` intrinsic");
                    return hir::Stmt::Expr(self.poison(span));
                };
                let Some((has_id, _)) = zun_natives::resolve("collections", "forInHas") else {
                    self.error(span, "missing `forInHas` intrinsic");
                    return hir::Stmt::Expr(self.poison(span));
                };
                // The enumerated object is evaluated once; every key is
                // re-checked against it before its iteration runs (for-in is live:
                // a key deleted earlier in the loop is skipped).
                let subject = cx.declare(&self.fresh_temp_name(cx), Ty::Any);
                let subject_read = hir::Expr { kind: hir::ExprKind::Local(subject), ty: Ty::Any, span };
                let keys = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id: keys_id, args: vec![hir::Expr { kind: hir::ExprKind::Local(subject), ty: Ty::Any, span }] },
                    ty: Ty::Array,
                    span,
                };
                let lowered = self.lower_for_iteration(
                    cx,
                    keys,
                    "arrayIteratorNew",
                    "arrayIteratorDone",
                    "arrayIteratorNext",
                    pattern,
                    body,
                );
                match lowered {
                    hir::Stmt::ForOf { iterator, iterator_slot, done, next, binding, body } => {
                        let key_read = hir::Expr {
                            kind: hir::ExprKind::Local(binding),
                            ty: cx.locals[binding as usize],
                            span,
                        };
                        let key_arg = if key_read.ty == Ty::Any || key_read.ty == Ty::String {
                            key_read
                        } else {
                            Self::box_to_any(key_read)
                        };
                        let still_there = hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id: has_id, args: vec![subject_read, key_arg] },
                            ty: Ty::Boolean,
                            span,
                        };
                        hir::Stmt::Group(vec![
                            hir::Stmt::Let { slot: subject, init: keys_arg },
                            hir::Stmt::ForOf {
                                iterator,
                                iterator_slot,
                                done,
                                next,
                                binding,
                                body: vec![hir::Stmt::If {
                                    cond: still_there,
                                    then_body: body,
                                    else_body: Vec::new(),
                                }],
                            },
                        ])
                    }
                    other => other,
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
                        | ast::Stmt::ForIn { .. }
                );
                if labels_loop {
                    cx.loop_labels.push(label_text.clone());
                } else {
                    cx.block_labels.push(label_text.clone());
                }
                let body = self.check_stmt(cx, statement);
                if labels_loop {
                    cx.loop_labels.pop();
                } else {
                    cx.block_labels.pop();
                }
                hir::Stmt::Labeled {
                    label: label_text,
                    body: Box::new(body),
                }
            }
            ast::Stmt::Switch { discriminant, cases, span } => {
                self.check_switch(cx, discriminant, cases, *span)
            }
            ast::Stmt::Break { label, span } => {
                match label {
                    Some(label) => {
                        let name = self.text(label);
                        if !cx.loop_labels.iter().any(|c| c == name)
                            && !cx.block_labels.iter().any(|c| c == name)
                        {
                            self.error(*span, format!("unknown break label `{name}`"));
                        }
                    }
                    None => {
                        if cx.loop_depth == 0 && cx.switch_depth == 0 {
                            self.error(*span, "`break` is only valid inside a loop or switch");
                        }
                    }
                }
                hir::Stmt::Break(label.map(|name| self.text(&name).to_string()))
            }
            ast::Stmt::Continue { label, span } => {
                match label {
                    Some(label) => {
                        let name = self.text(label);
                        if !cx.loop_labels.iter().any(|c| c == name) {
                            self.error(*span, format!("unknown continue label `{name}`"));
                        }
                    }
                    None => {
                        if cx.loop_depth == 0 {
                            self.error(*span, "`continue` is only valid inside a loop");
                        }
                    }
                }
                hir::Stmt::Continue(label.map(|name| self.text(&name).to_string()))
            }
            ast::Stmt::Return { value, span } => {
                let value = value.as_ref().map(|v| {
                    let mut e = self.check_expr(cx, v);
                    coerce_numeric_literal(&mut e, cx.return_ty);
                    let e = Self::unbox_to(e, cx.return_ty);
                    // Implicit widening of an integer value to `number`.
                    if matches!(e.ty, Ty::Int(_)) && cx.return_ty == Ty::F64 {
                        let span = e.span;
                        hir::Expr {
                            kind: hir::ExprKind::Convert { value: Box::new(e) },
                            ty: Ty::F64,
                            span,
                        }
                    } else {
                        e
                    }
                });
                let value = if value.is_none() && cx.return_ty == Ty::Any {
                    Some(self.check_expr(
                        cx,
                        &ast::Expr::Unary {
                            op: ast::UnaryOp::Void,
                            operand: Box::new(ast::Expr::Number { value: 0.0, span: *span }),
                            span: *span,
                        },
                    ))
                } else {
                    value
                };
                match (&value, cx.return_ty) {
                    (None, Ty::Void) => {}
                    // A bare `return;` in a generator (or an `any`-returning
                    // function) yields `undefined`.
                    (None, Ty::Any) => {}
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
                // Inside a `try` with a `finally`, the return value is computed
                // first and held in a temporary while the cleanup runs.
                if cx.finally_depth > 0 {
                    if let Some(v) = value {
                        if !matches!(v.kind, hir::ExprKind::Number(_) | hir::ExprKind::String(_) | hir::ExprKind::Boolean(_) | hir::ExprKind::Null) && !v.ty.is_error() {
                            let span = v.span;
                            let ty = v.ty;
                            let (slot, _) = self.synthetic_local(cx, ty);
                            return hir::Stmt::Group(vec![
                                hir::Stmt::Let { slot, init: v },
                                hir::Stmt::Return(Some(hir::Expr {
                                    kind: hir::ExprKind::Local(slot),
                                    ty,
                                    span,
                                })),
                            ]);
                        }
                        return hir::Stmt::Return(Some(v));
                    }
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
                let has_finally = finally.is_some();
                if has_finally {
                    cx.finally_depth += 1;
                }
                let body = self.check_block(cx, body);
                let catches = catches
                    .iter()
                    .map(|catch| {
                        let ty = self.resolve_annotation(&catch.ty);
                        cx.scopes.push(Scope::new());
                        let slot = cx.declare(self.text(&catch.binding), ty);
                        let mut body = self.check_block(cx, &catch.body);
                        // A catch binding captured and mutated by a closure lives
                        // in a cell: the handler receives the exception in a raw
                        // slot and the body starts by moving it into the cell.
                        let slot = if cx.boxed.contains(&slot) {
                            let raw = cx.declare("__catch_exception", ty);
                            body.insert(
                                0,
                                hir::Stmt::Let {
                                    slot,
                                    init: hir::Expr {
                                        kind: hir::ExprKind::Local(raw),
                                        ty,
                                        span: catch.body.span,
                                    },
                                },
                            );
                            raw
                        } else {
                            slot
                        };
                        cx.scopes.pop();
                        hir::CatchClause { slot, ty, body }
                    })
                    .collect();
                if has_finally {
                    cx.finally_depth -= 1;
                }
                let finally_body = finally.as_ref().map(|block| {
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

    /// Lower an `Object.<method>(...)` reflective static to its managed
    /// builtin. Operates on both typed objects (declared fields) and dynamic
    /// objects (insertion-ordered dynamic keys); the heavy lifting lives in the
    /// interpreter floor. Descriptor-based statics are intentionally absent —
    /// Zun has no property-descriptor model yet (see NUCLEUS.md).
    /// Variadic `console.log/info/debug/error/warn`: format every argument with
    /// the prelude `ConsoleFormat.top`, join with single spaces (Node), and
    /// print once to stdout (or stderr for `error`/`warn`). Returns `None` for
    /// any other `console` member so it falls through to normal dispatch.
    fn check_console_variadic(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        let target = match method {
            "log" | "info" | "debug" => "log",
            "error" | "warn" => "err",
            "group" | "groupCollapsed" => "group",
            "groupEnd" => "groupEnd",
            "count" => "count",
            "countReset" => "countReset",
            "assert" => "assert",
            _ => return None,
        };
        let fmt_class = *self.classes_by_name.get("ConsoleFormat2")?;
        let func = self.method_of(fmt_class, target)?;
        let elements: Vec<Option<hir::Expr>> =
            args.iter().map(|arg| Some(self.check_expr(cx, arg))).collect();
        let array = hir::Expr {
            kind: hir::ExprKind::ArrayLiteral { elements },
            ty: Ty::Array,
            span,
        };
        Some(hir::Expr {
            kind: hir::ExprKind::StaticCall { func, args: vec![array] },
            ty: Ty::Void,
            span,
        })
    }

    fn check_object_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let intrinsic = |name: &str| {
            zun_natives::resolve("collections", name).map(|(id, _)| id)
        };
        let builtin = |checker: &mut Self, name: &str, checked: Vec<hir::Expr>, ty: Ty| {
            let Some(id) = intrinsic(name) else {
                checker.error(span, format!("missing Object intrinsic `{name}`"));
                return checker.poison(span);
            };
            hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: checked }, ty, span }
        };
        // `Object.getOwnPropertyDescriptor(Map, "groupBy")`: a well-known
        // built-in static method is a writable, non-enumerable, configurable
        // data property.
        if method == "getOwnPropertyDescriptor" && args.len() == 2 {
            if let (ast::Expr::Ident(global), ast::Expr::String { .. }) = (&args[0], &args[1]) {
                let global_name = self.text(global).to_string();
                let key = match &args[1] {
                    ast::Expr::String { value, .. } => value.clone(),
                    _ => String::new(),
                };
                if cx.lookup(&global_name).is_none()
                    && !self.classes_by_name.contains_key(global_name.as_str())
                    && builtin_function_length(&[global_name.as_str(), key.as_str()]).is_some()
                {
                    let flag = |name: &str, value: bool| {
                        decorator_context_entry(name, hir::ExprKind::Boolean(value), Ty::Boolean, span)
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::DynamicObjectLiteral {
                            entries: vec![
                                flag("writable", true),
                                flag("enumerable", false),
                                flag("configurable", true),
                            ],
                        },
                        ty: Ty::Any,
                        span,
                    };
                }
            }
        }
        // Expected argument count per static (`assign` is variadic: 1+).
        let fixed = match method {
            "create" if args.len() == 2 => Some(2usize),
            "keys" | "values" | "entries" | "freeze" | "isFrozen" | "create"
            | "getPrototypeOf" | "fromEntries" | "getOwnPropertyDescriptors"
            | "getOwnPropertyNames" | "getOwnPropertySymbols" | "seal" | "isSealed"
            | "preventExtensions" | "isExtensible" => Some(1usize),
            "setPrototypeOf" | "is" | "getOwnPropertyDescriptor"
            | "defineProperties" | "hasOwn" | "groupBy" => Some(2usize),
            "defineProperty" => Some(3usize),
            "assign" => None,
            _ => {
                self.error(
                    method_span,
                    format!("`Object.{method}` is not supported in this runtime"),
                );
                return self.poison(span);
            }
        };
        let mut checked: Vec<hir::Expr> =
            args.iter().map(|argument| self.check_expr(cx, argument)).collect();
        if let Some(expected) = fixed {
            if checked.len() != expected {
                self.error(
                    span,
                    format!("`Object.{method}` expects {expected} argument(s), found {}", checked.len()),
                );
                return self.poison(span);
            }
        } else if checked.is_empty() {
            self.error(span, "`Object.assign` expects at least one argument");
            return self.poison(span);
        }
        match method {
            "keys" | "values" | "entries" => {
                let name = match method {
                    "keys" => "objectKeys",
                    "values" => "objectValues",
                    _ => "objectEntries",
                };
                builtin(self, name, checked, Ty::Array)
            }
            "freeze" => {
                // Freezing a RegExp keeps its static type (the same object comes back).
                let ty = if checked[0].ty == Ty::RegExp { Ty::RegExp } else { Ty::Any };
                builtin(self, "objectFreeze", checked, ty)
            }
            "hasOwn" => builtin(self, "objectHasOwn", checked, Ty::Boolean),
            "isFrozen" => builtin(self, "objectIsFrozen", checked, Ty::Boolean),
            "create" if checked.len() == 2 => {
                // `Object.create(proto, props)`: create, then define the descriptors.
                let props = checked.pop().unwrap();
                let (slot, _) = self.synthetic_local(cx, Ty::DynamicObject);
                let local = |span| hir::Expr { kind: hir::ExprKind::Local(slot), ty: Ty::DynamicObject, span };
                let created = builtin(self, "objectCreate", checked, Ty::DynamicObject);
                let assign = hir::Expr {
                    kind: hir::ExprKind::Assign { slot, value: Box::new(created) },
                    ty: Ty::DynamicObject,
                    span,
                };
                let define = builtin(self, "objectDefineProperties", vec![local(span), props], Ty::Any);
                hir::Expr {
                    kind: hir::ExprKind::Sequence {
                        effect: Box::new(assign),
                        value: Box::new(hir::Expr {
                            kind: hir::ExprKind::Sequence { effect: Box::new(define), value: Box::new(local(span)) },
                            ty: Ty::DynamicObject,
                            span,
                        }),
                    },
                    ty: Ty::DynamicObject,
                    span,
                }
            }
            "create" => builtin(self, "objectCreate", checked, Ty::DynamicObject),
            "getPrototypeOf" => builtin(self, "objectGetPrototypeOf", checked, Ty::Any),
            "setPrototypeOf" => builtin(self, "objectSetPrototypeOf", checked, Ty::Any),
            "fromEntries" => builtin(self, "objectFromEntries", checked, Ty::DynamicObject),
            "groupBy" => builtin(self, "objectGroupBy", checked, Ty::DynamicObject),
            "is" => builtin(self, "objectIs", checked, Ty::Boolean),
            "defineProperty" => builtin(self, "objectDefineProperty", checked, Ty::Any),
            "defineProperties" => builtin(self, "objectDefineProperties", checked, Ty::Any),
            "getOwnPropertyDescriptor" => {
                builtin(self, "objectGetOwnPropertyDescriptor", checked, Ty::Any)
            }
            "getOwnPropertyDescriptors" => {
                builtin(self, "objectGetOwnPropertyDescriptors", checked, Ty::DynamicObject)
            }
            "getOwnPropertyNames" => builtin(self, "objectGetOwnPropertyNames", checked, Ty::Array),
            "getOwnPropertySymbols" => {
                builtin(self, "objectGetOwnPropertySymbols", checked, Ty::Array)
            }
            "seal" => builtin(self, "objectSeal", checked, Ty::Any),
            "isSealed" => builtin(self, "objectIsSealed", checked, Ty::Boolean),
            "preventExtensions" => builtin(self, "objectPreventExtensions", checked, Ty::Any),
            "isExtensible" => builtin(self, "objectIsExtensible", checked, Ty::Boolean),
            "assign" => {
                let Some(id) = intrinsic("objectAssign") else {
                    self.error(span, "missing Object intrinsic `objectAssign`");
                    return self.poison(span);
                };
                let mut sources = checked.split_off(1);
                let mut acc = checked.pop().unwrap();
                for source in sources.drain(..) {
                    acc = hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![acc, source] },
                        ty: Ty::DynamicObject,
                        span,
                    };
                }
                acc
            }
            _ => unreachable!(),
        }
    }

    /// Lower `new Function(p1, ..., body)` / `Function(...)` to a
    /// `functionConstruct([p1, ..., body])` intrinsic. The runtime compiles the
    /// strings through an injected `EvalHost` (absent under AOT, where it
    /// throws "code generation from strings disallowed"). The result is a
    /// dynamic function value, called through the ordinary indirect path.
    fn lower_function_construct(
        &mut self,
        cx: &mut FunctionCx,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let mut elements = Vec::with_capacity(args.len().max(1));
        for arg in args {
            let checked = self.check_expr(cx, arg);
            if !checked.ty.is_error() && checked.ty != Ty::String {
                self.error(checked.span, "the `Function` constructor expects string arguments");
                return self.poison(span);
            }
            elements.push(Some(checked));
        }
        if elements.is_empty() {
            // `new Function()` is a function with an empty body.
            elements.push(Some(hir::Expr {
                kind: hir::ExprKind::String(String::new()),
                ty: Ty::String,
                span,
            }));
        }
        let Some((id, _)) = zun_natives::resolve("collections", "functionConstruct") else {
            self.error(span, "missing `functionConstruct` intrinsic");
            return self.poison(span);
        };
        self.codegen_sites.push((span.start, span.end));
        let array = hir::Expr {
            kind: hir::ExprKind::ArrayLiteral { elements },
            ty: Ty::Array,
            span,
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: vec![array] },
            ty: Ty::Function(hir::DYNAMIC_FUNCTION),
            span,
        }
    }

    /// Build a `proxyGet(proxy, key)` native call routing a property read
    /// through the handler's `get` trap (absent trap → target default).
    fn proxy_get(&mut self, object: hir::Expr, key: hir::Expr, span: Span) -> hir::Expr {
        if !key.ty.is_error() && key.ty != Ty::String {
            self.error(key.span, "a proxy property key must be a string");
        }
        let Some((id, _)) = zun_natives::resolve("collections", "proxyGet") else {
            self.error(span, "missing `proxyGet` intrinsic");
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: vec![object, key] },
            ty: Ty::Any,
            span,
        }
    }

    /// Build a `proxySet(proxy, key, value)` native call routing a property
    /// write through the handler's `set` trap (absent trap → target default).
    /// The value is boxed to `Any`.
    fn proxy_set(
        &mut self,
        object: hir::Expr,
        key: hir::Expr,
        value: hir::Expr,
        span: Span,
    ) -> hir::Expr {
        if !key.ty.is_error() && key.ty != Ty::String {
            self.error(key.span, "a proxy property key must be a string");
        }
        let boxed = if value.ty == Ty::Any {
            value
        } else {
            hir::Expr {
                span: value.span,
                kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                ty: Ty::Any,
            }
        };
        let Some((id, _)) = zun_natives::resolve("collections", "proxySet") else {
            self.error(span, "missing `proxySet` intrinsic");
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: vec![object, key, boxed] },
            ty: Ty::Boolean,
            span,
        }
    }

    /// Lower a `Reflect.<method>(...)` static call. `Reflect` is a surface-only
    /// reflection namespace (realm global, no `new`); these are the default
    /// property operations the Proxy traps fall back to.
    ///
    /// Supported: `get(target, key)`, `set(target, key, value)`,
    /// `has(target, key)`, `deleteProperty(target, key)`, `ownKeys(target)`,
    /// the descriptor-dependent statics `getOwnPropertyDescriptor`,
    /// `defineProperty`, `getPrototypeOf`, `setPrototypeOf`, `isExtensible`,
    /// `preventExtensions` (NUCLEUS #6's descriptor model, proxy-aware), and
    /// `apply(target, thisArg, argsArray)`, and `construct` for CLASS targets
    /// (`Reflect.construct` / `new proxy(...)` via class descriptors; arbitrary
    /// function constructors are out of scope).
    fn check_reflect_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        // `Reflect.apply(target, thisArg, argsArray)`: invoke a callable (an
        // ordinary function, or a callable proxy routed through its `apply`
        // trap) with a spread arguments array. Handled before the object-target
        // path because the target is a function or proxy, not a plain object.
        if method == "apply" {
            let mut checked: Vec<hir::Expr> =
                args.iter().map(|argument| self.check_expr(cx, argument)).collect();
            if checked.len() != 3 {
                self.error(
                    span,
                    format!("`Reflect.apply` expects 3 argument(s), found {}", checked.len()),
                );
                return self.poison(span);
            }
            let array = checked.pop().unwrap();
            let mut array = self.array_like_to_array(array);
            let this_arg = checked.pop().unwrap();
            let target = checked.pop().unwrap();
            self.box_function_array_elements(&mut array);
            if array.ty != Ty::Array && !array.ty.is_error() {
                self.error(array.span, "`Reflect.apply` expects an Array of arguments");
                return self.poison(span);
            }
            if !matches!(target.ty, Ty::Function(_) | Ty::Proxy | Ty::Any)
                && !target.ty.is_error()
            {
                self.error(target.span, "`Reflect.apply` target must be callable");
            }
            let native_name = if target.ty == Ty::Proxy { "proxyApply" } else { "functionApply" };
            let Some((id, _)) = zun_natives::resolve("collections", native_name) else {
                self.error(span, format!("missing Reflect intrinsic `{native_name}`"));
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![target, this_arg, array] },
                ty: Ty::Any,
                span,
            };
        }
        // `Reflect.construct(target, argsArray, newTarget?)`: CLASS targets only
        // (a class descriptor value). An arbitrary-function constructor is out
        // of scope: Zun has no function.prototype / `this` construction model
        // (classes are frozen typed layouts). `newTarget` is approximated by the
        // target; the array supplies the positional field values.
        if method == "construct" {
            let mut checked: Vec<hir::Expr> =
                args.iter().map(|argument| self.check_expr(cx, argument)).collect();
            if checked.len() < 2 || checked.len() > 3 {
                self.error(
                    span,
                    format!("`Reflect.construct` expects 2 or 3 argument(s), found {}", checked.len()),
                );
                return self.poison(span);
            }
            checked.truncate(2);
            let array = checked.pop().unwrap();
            let array = self.array_like_to_array(array);
            let target = checked.pop().unwrap();
            if array.ty != Ty::Array && !array.ty.is_error() {
                self.error(array.span, "`Reflect.construct` expects an Array of arguments");
                return self.poison(span);
            }
            let native_name = match target.ty {
                Ty::Proxy => "proxyConstruct",
                Ty::Any => "reflectConstruct",
                t if t.is_error() => return self.poison(span),
                _ => {
                    self.error(target.span, "`Reflect.construct` target must be a class");
                    return self.poison(span);
                }
            };
            let Some((id, _)) = zun_natives::resolve("collections", native_name) else {
                self.error(span, format!("missing Reflect intrinsic `{native_name}`"));
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![target, array] },
                ty: Ty::Any,
                span,
            };
        }
        let expected = match method {
            "get" | "has" | "deleteProperty" | "getOwnPropertyDescriptor"
            | "setPrototypeOf" => 2usize,
            "set" | "defineProperty" => 3,
            "ownKeys" | "getPrototypeOf" | "isExtensible" | "preventExtensions" => 1,
            other => {
                self.error(
                    method_span,
                    format!("`Reflect.{other}` is not supported in this runtime"),
                );
                return self.poison(span);
            }
        };
        let mut checked: Vec<hir::Expr> =
            args.iter().map(|argument| self.check_expr(cx, argument)).collect();
        if method == "deleteProperty" && checked.len() == 2 {
            if let (Ty::Function(func), ast::Expr::String { value, .. }) = (checked[0].ty, &args[1]) {
                if func < hir::DYNAMIC_FUNCTION && (value == "name" || value == "length") {
                    self.deleted_fn_meta.insert((func, value.clone()));
                }
            }
        }
        // The optional `receiver` of `Reflect.get` / `Reflect.set` is accepted and
        // ignored: the operation is applied to the target directly.
        if (method == "get" && checked.len() == 3) || (method == "set" && checked.len() == 4) {
            checked.pop();
        }
        if checked.len() != expected {
            self.error(
                span,
                format!("`Reflect.{method}` expects {expected} argument(s), found {}", checked.len()),
            );
            return self.poison(span);
        }
        // The target must be a dynamic receiver (plain object or `any`); a Proxy
        // target is itself a dynamic receiver at runtime.
        // A managed collection target is handed to the natives as a dynamic receiver.
        if matches!(checked[0].ty, Ty::Array | Ty::Map | Ty::Set | Ty::Instance(_) | Ty::RegExp | Ty::Function(_)) {
            let target = checked.remove(0);
            checked.insert(0, Self::box_to_any(target));
        }
        let target_ok = matches!(checked[0].ty, Ty::DynamicObject | Ty::Any | Ty::Proxy)
            || checked[0].ty.is_error();
        if !target_ok {
            self.error(checked[0].span, "`Reflect` target must be an object");
        }
        let builtin = |checker: &mut Self, name: &str, checked: Vec<hir::Expr>, ty: Ty| {
            let Some((id, _)) = zun_natives::resolve("collections", name) else {
                checker.error(span, format!("missing Reflect intrinsic `{name}`"));
                return checker.poison(span);
            };
            hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: checked }, ty, span }
        };
        match method {
            "get" => {
                let key = checked.pop().unwrap();
                let object = checked.pop().unwrap();
                hir::Expr {
                    kind: hir::ExprKind::DynamicGetValue {
                        object: Box::new(object),
                        key: Box::new(key),
                    },
                    ty: Ty::Any,
                    span,
                }
            }
            "set" => {
                // `Reflect.set` returns a boolean success and respects
                // writability/extensibility (unlike `target[key] = value`),
                // so it routes to the dedicated `reflectSet` native.
                let value = checked.pop().unwrap();
                let key = checked.pop().unwrap();
                let object = checked.pop().unwrap();
                let Some((id, _)) = zun_natives::resolve("collections", "reflectSet") else {
                    self.error(span, "missing `reflectSet` intrinsic");
                    return self.poison(span);
                };
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![
                            Self::box_to_any(object),
                            Self::box_to_any(key),
                            Self::box_to_any(value),
                        ],
                    },
                    ty: Ty::Boolean,
                    span,
                }
            }
            "has" => builtin(self, "reflectHas", checked, Ty::Boolean),
            "deleteProperty" => builtin(self, "reflectDelete", checked, Ty::Boolean),            // `ownKeys` reports all own string keys (not just enumerable ones),
            // and consults a proxy's `ownKeys` trap via the proxy-aware native.
            "ownKeys" => builtin(self, "reflectOwnKeys", checked, Ty::Array),
            "getOwnPropertyDescriptor" => {
                builtin(self, "objectGetOwnPropertyDescriptor", checked, Ty::Any)
            }
            "defineProperty" => builtin(self, "reflectDefineProperty", checked, Ty::Boolean),
            "getPrototypeOf" => builtin(self, "objectGetPrototypeOf", checked, Ty::Any),
            "setPrototypeOf" => builtin(self, "objectSetPrototypeOf", checked, Ty::Any),
            "isExtensible" => builtin(self, "objectIsExtensible", checked, Ty::Boolean),
            "preventExtensions" => builtin(self, "objectPreventExtensions", checked, Ty::Any),
            _ => unreachable!(),
        }
    }

    /// Lower a `Buffer.<method>(...)` static call. `Buffer` is a nominal alias
    /// for the `Uint8Array` byte-view floor, so these allocate/operate over a
    /// `U8` view; they are recognized by receiver name (like `ArrayBuffer`'s
    /// static `isView`).
    /// Lower the second-generation typed-array instance surface: the bulk
    /// `set(source, offset?)` (overloaded against the indexed
    /// `set(index, value)`), the copying `slice`, the in-place `copyWithin`,
    /// the search helpers `indexOf`/`includes`/`join`, and the functional
    /// methods. All route to the kind-agnostic `typedArray*` natives; the
    /// element kind travels with the view at run time, so the only per-kind
    /// facts needed here are the element type a `find`/`get` yields and the
    /// view type a `slice`/`map`/`filter` returns.
    fn check_typed_array_method(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        kind: hir::BufferViewKind,
        mname: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let view_ty = Ty::BufferView(kind);
        let i64_ty = Ty::Int(IntKind::I64);
        let u64_ty = Ty::Int(IntKind::U64);
        // The element a read of this kind yields — the type `find` returns and
        // an indexed `set` value must match.
        let element_ty = crate::typed_arrays::method(kind, "get")
            .map(|(_, ret, _)| ret)
            .unwrap_or(Ty::F64);
        // `end`/length defaults: a value past any real length that `f64` holds
        // exactly, clamped down to the length by the relative-index logic.
        let length_sentinel = (1i64 << 53) as f64;
        let int_lit = |value: f64, ty: Ty, span: Span| hir::Expr {
            kind: hir::ExprKind::Number(value),
            ty,
            span,
        };
        let string_lit = |text: &str, span: Span| hir::Expr {
            kind: hir::ExprKind::String(text.to_string()),
            ty: Ty::String,
            span,
        };
        let box_any = |value: hir::Expr| {
            if value.ty == Ty::Any || value.ty.is_error() {
                value
            } else {
                hir::Expr {
                    span: value.span,
                    kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                    ty: Ty::Any,
                }
            }
        };
        let call = |checker: &mut Self, native: &str, call_args: Vec<hir::Expr>, ty: Ty| {
            let Some((id, _)) = zun_natives::resolve("buffer", native) else {
                checker.error(span, format!("missing typed-array intrinsic `{native}`"));
                return checker.poison(span);
            };
            hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: call_args }, ty, span }
        };
        // An optional integer argument with a default, coerced to `ty`.
        macro_rules! opt_int {
            ($pos:expr, $ty:expr, $default:expr) => {{
                match args.get($pos) {
                    Some(arg) => {
                        let mut value = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut value, $ty);
                        if !value.ty.is_int() && !value.ty.is_error() {
                            self.error(value.span, "typed-array index must be an integer");
                        }
                        value
                    }
                    None => int_lit($default, $ty, span),
                }
            }};
        }
        // A required callback argument.
        let check_callback = |checker: &mut Self, cx: &mut FunctionCx| -> Option<hir::Expr> {
            if args.len() != 1 {
                checker.error(span, format!("`{mname}` expects 1 argument (a callback), found {}", args.len()));
                return None;
            }
            let callback = checker.check_expr(cx, &args[0]);
            if !matches!(callback.ty, Ty::Function(_)) && !callback.ty.is_error() {
                checker.error(callback.span, "typed-array callback must be a function");
            }
            Some(callback)
        };
        match mname {
            "set" => {
                // Overload resolution: an array / typed-array first argument is
                // the bulk `set(source, offset?)`; two numeric arguments are
                // the indexed `set(index, value)`.
                if args.is_empty() || args.len() > 2 {
                    self.error(span, format!("`set` expects 1 or 2 arguments, found {}", args.len()));
                    return self.poison(span);
                }
                let first = self.check_expr(cx, &args[0]);
                let is_bulk = matches!(first.ty, Ty::Array | Ty::BufferView(_));
                if is_bulk {
                    let offset = opt_int!(1, u64_ty, 0.0);
                    call(self, "typedArraySetFrom", vec![receiver, first, offset], Ty::Boolean)
                } else {
                    // Indexed set: the generic per-kind `<native>Set`.
                    if args.len() != 2 {
                        self.error(span, "indexed `set(index, value)` expects 2 arguments".to_string());
                        return self.poison(span);
                    }
                    let Some((native, _ret, params)) = crate::typed_arrays::method(kind, "set") else {
                        self.error(span, "missing typed-array `set` intrinsic".to_string());
                        return self.poison(span);
                    };
                    let mut index = first;
                    coerce_numeric_literal(&mut index, u64_ty);
                    if !index.ty.is_int() && !index.ty.is_error() {
                        self.error(index.span, "typed-array index must be an integer");
                    }
                    let elem_param = params.get(1).copied().unwrap_or(element_ty);
                    let mut value = self.check_expr(cx, &args[1]);
                    coerce_numeric_literal(&mut value, elem_param);
                    if !value.ty.is_error() && !self.is_assignable(value.ty, elem_param) {
                        self.error(value.span, format!("typed-array element must be {elem_param:?}"));
                    }
                    call(self, native, vec![receiver, index, value], Ty::Boolean)
                }
            }
            "slice" => {
                if args.len() > 2 {
                    self.error(span, "`slice` expects at most 2 arguments".to_string());
                    return self.poison(span);
                }
                let start = opt_int!(0, i64_ty, 0.0);
                let end = opt_int!(1, i64_ty, length_sentinel);
                call(self, "typedArraySlice", vec![receiver, start, end], view_ty)
            }
            "copyWithin" => {
                if args.is_empty() || args.len() > 3 {
                    self.error(span, "`copyWithin` expects 1 to 3 arguments".to_string());
                    return self.poison(span);
                }
                let target = opt_int!(0, i64_ty, 0.0);
                let start = opt_int!(1, i64_ty, 0.0);
                let end = opt_int!(2, i64_ty, length_sentinel);
                call(self, "typedArrayCopyWithin", vec![receiver, target, start, end], view_ty)
            }
            "indexOf" | "includes" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(span, format!("`{mname}` expects 1 or 2 arguments"));
                    return self.poison(span);
                }
                let needle = box_any(self.check_expr(cx, &args[0]));
                let from = opt_int!(1, i64_ty, 0.0);
                if mname == "indexOf" {
                    call(self, "typedArrayIndexOf", vec![receiver, needle, from], i64_ty)
                } else {
                    call(self, "typedArrayIncludes", vec![receiver, needle, from], Ty::Boolean)
                }
            }
            "join" => {
                if args.len() > 1 {
                    self.error(span, "`join` expects at most 1 argument".to_string());
                    return self.poison(span);
                }
                let separator = match args.first() {
                    Some(arg) => {
                        let value = self.check_expr(cx, arg);
                        if value.ty != Ty::String && !value.ty.is_error() {
                            self.error(value.span, "`join` separator must be a string");
                        }
                        value
                    }
                    None => string_lit(",", span),
                };
                call(self, "typedArrayJoin", vec![receiver, separator], Ty::String)
            }
            "forEach" => match check_callback(self, cx) {
                Some(callback) => call(self, "typedArrayForEach", vec![receiver, callback], Ty::Void),
                None => self.poison(span),
            },
            "map" => match check_callback(self, cx) {
                Some(callback) => call(self, "typedArrayMap", vec![receiver, callback], view_ty),
                None => self.poison(span),
            },
            "filter" => match check_callback(self, cx) {
                Some(callback) => call(self, "typedArrayFilter", vec![receiver, callback], view_ty),
                None => self.poison(span),
            },
            "find" => match check_callback(self, cx) {
                Some(callback) => call(self, "typedArrayFind", vec![receiver, callback], element_ty),
                None => self.poison(span),
            },
            "findIndex" => match check_callback(self, cx) {
                Some(callback) => call(self, "typedArrayFindIndex", vec![receiver, callback], i64_ty),
                None => self.poison(span),
            },
            "some" => match check_callback(self, cx) {
                Some(callback) => call(self, "typedArraySome", vec![receiver, callback], Ty::Boolean),
                None => self.poison(span),
            },
            "every" => match check_callback(self, cx) {
                Some(callback) => call(self, "typedArrayEvery", vec![receiver, callback], Ty::Boolean),
                None => self.poison(span),
            },
            "reduce" | "reduceRight" => {
                // Both the initialized form `(cb, init)` and the no-initial
                // form `(cb)` (which seeds from the first/last element and
                // throws on an empty typed array, NUCLEUS #19).
                if args.is_empty() || args.len() > 2 {
                    self.error(span, format!("`{mname}` expects a callback and an optional initial value"));
                    return self.poison(span);
                }
                let callback = self.check_expr(cx, &args[0]);
                if !matches!(callback.ty, Ty::Function(_)) && !callback.ty.is_error() {
                    self.error(callback.span, format!("typed-array `{mname}` callback must be a function"));
                }
                let (native, call_args) = match (mname, args.len()) {
                    ("reduce", 2) => (
                        "typedArrayReduce",
                        vec![receiver, callback, box_any(self.check_expr(cx, &args[1]))],
                    ),
                    ("reduce", _) => ("typedArrayReduceNoInit", vec![receiver, callback]),
                    ("reduceRight", 2) => (
                        "typedArrayReduceRight",
                        vec![receiver, callback, box_any(self.check_expr(cx, &args[1]))],
                    ),
                    (_, _) => ("typedArrayReduceRightNoInit", vec![receiver, callback]),
                };
                call(self, native, call_args, Ty::Any)
            }
            _ => {
                self.error(span, format!("`{mname}` is not a typed-array method"));
                self.poison(span)
            }
        }
    }

    /// Lower `<Kind>Array.from(source, mapFn?)` and `<Kind>Array.of(...items)`.
    /// Both produce a view of `kind`; the kind crosses to the native as a
    /// table-order discriminant so one native serves all eleven kinds.
    /// A `DataView` element accessor (`getInt32`/`setFloat64`/…). The
    /// little-endian flag is optional and defaults to `false` (big-endian),
    /// per the spec; single-byte widths take no flag. Returns `None` for a
    /// method that is not one of these (so `byteLength`/`byteOffset`/`buffer`
    /// stay on the generic path).
    fn check_data_view_method(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        // (native, element type, takes an endianness flag).
        let (native, elem, endian): (&str, Ty, bool) = match mname {
            "getInt8" => ("dataViewGetInt8", Ty::Int(IntKind::I8), false),
            "getUint8" => ("dataViewGetUint8", Ty::Int(IntKind::U8), false),
            "getInt16" => ("dataViewGetInt16", Ty::Int(IntKind::I16), true),
            "getUint16" => ("dataViewGetU16", Ty::Int(IntKind::U16), true),
            "getInt32" => ("dataViewGetInt32", Ty::Int(IntKind::I32), true),
            "getUint32" => ("dataViewGetU32", Ty::Int(IntKind::U32), true),
            "getInt64" => ("dataViewGetInt64", Ty::Int(IntKind::I64), true),
            "getUint64" => ("dataViewGetUint64", Ty::Int(IntKind::U64), true),
            "getFloat32" => ("dataViewGetFloat32", Ty::F32, true),
            "getFloat64" => ("dataViewGetF64", Ty::F64, true),
            "setInt8" => ("dataViewSetInt8", Ty::Int(IntKind::I8), false),
            "setUint8" => ("dataViewSetUint8", Ty::Int(IntKind::U8), false),
            "setInt16" => ("dataViewSetInt16", Ty::Int(IntKind::I16), true),
            "setUint16" => ("dataViewSetU16", Ty::Int(IntKind::U16), true),
            "setInt32" => ("dataViewSetInt32", Ty::Int(IntKind::I32), true),
            "setUint32" => ("dataViewSetU32", Ty::Int(IntKind::U32), true),
            "setInt64" => ("dataViewSetInt64", Ty::Int(IntKind::I64), true),
            "setUint64" => ("dataViewSetUint64", Ty::Int(IntKind::U64), true),
            "setFloat32" => ("dataViewSetFloat32", Ty::F32, true),
            "setFloat64" => ("dataViewSetF64", Ty::F64, true),
            // Only called behind `data_view_accessor`, which lists exactly
            // these names.
            other => unreachable!("not a DataView accessor: {other}"),
        };
        let is_set = mname.starts_with("set");
        let u64_ty = Ty::Int(IntKind::U64);
        let bool_lit = |value: bool, span: Span| hir::Expr {
            kind: hir::ExprKind::Boolean(value),
            ty: Ty::Boolean,
            span,
        };
        // Expected positional arity, excluding the optional endianness flag:
        // byteOffset (+ value for a setter).
        let fixed = if is_set { 2 } else { 1 };
        let max = fixed + usize::from(endian);
        if args.len() < fixed || args.len() > max {
            self.error(
                span,
                format!("`{mname}` expects {fixed} or {max} argument(s), found {}", args.len()),
            );
            return self.poison(span);
        }
        let mut call_args = vec![receiver];
        let mut offset = self.check_expr(cx, &args[0]);
        coerce_numeric_literal(&mut offset, u64_ty);
        // JS ToIndex: a `number` offset is truncated to an unsigned index.
        if offset.ty.is_float() {
            let span = offset.span;
            offset = hir::Expr { kind: hir::ExprKind::Convert { value: Box::new(offset) }, ty: u64_ty, span };
        }
        if !offset.ty.is_int() && !offset.ty.is_error() {
            self.error(offset.span, "DataView byte offset must be an integer");
        }
        call_args.push(offset);
        if is_set {
            let mut value = self.check_expr(cx, &args[1]);
            coerce_numeric_literal(&mut value, elem);
            if !value.ty.is_error() && !self.is_assignable(value.ty, elem) {
                self.error(value.span, format!("DataView value must be {elem:?}"));
            }
            call_args.push(value);
        }
        if endian {
            let flag = match args.get(fixed) {
                Some(arg) => {
                    let value = self.check_expr(cx, arg);
                    if value.ty != Ty::Boolean && !value.ty.is_error() {
                        self.error(value.span, "DataView endianness flag must be a boolean");
                    }
                    value
                }
                None => bool_lit(false, span),
            };
            call_args.push(flag);
        }
        let ret = if is_set { Ty::Boolean } else { elem };
        let Some((id, _)) = zun_natives::resolve("buffer", native) else {
            self.error(span, format!("missing DataView intrinsic `{native}`"));
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: call_args },
            ty: ret,
            span,
        }
    }

    fn check_typed_array_static(
        &mut self,
        cx: &mut FunctionCx,
        kind: hir::BufferViewKind,
        method: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let view_ty = Ty::BufferView(kind);
        let Some(discriminant) = crate::typed_arrays::element_index(kind) else {
            self.error(span, "typed-array statics require an element kind".to_string());
            return self.poison(span);
        };
        let kind_lit = hir::Expr {
            kind: hir::ExprKind::Number(discriminant as f64),
            ty: Ty::Int(IntKind::U32),
            span,
        };
        let call = |checker: &mut Self, native: &str, call_args: Vec<hir::Expr>| {
            let Some((id, _)) = zun_natives::resolve("buffer", native) else {
                checker.error(span, format!("missing typed-array intrinsic `{native}`"));
                return checker.poison(span);
            };
            hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: call_args }, ty: view_ty, span }
        };
        match method {
            "of" => {
                // `<Kind>Array.of(...items)` is an array literal of the args,
                // then a copy into a view of the kind.
                let elements = args
                    .iter()
                    .map(|element| Some(self.check_expr(cx, element)))
                    .collect();
                let source = hir::Expr {
                    kind: hir::ExprKind::ArrayLiteral { elements },
                    ty: Ty::Array,
                    span,
                };
                call(self, "typedArrayOf", vec![source, kind_lit])
            }
            "from" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(span, format!("`from` expects 1 or 2 arguments, found {}", args.len()));
                    return self.poison(span);
                }
                let source = self.check_expr(cx, &args[0]);
                if !matches!(source.ty, Ty::Array | Ty::BufferView(_)) && !source.ty.is_error() {
                    self.error(source.span, "`from` source must be an array or typed array");
                }
                match args.get(1) {
                    Some(map_arg) => {
                        let map_fn = self.check_expr(cx, map_arg);
                        if !matches!(map_fn.ty, Ty::Function(_)) && !map_fn.ty.is_error() {
                            self.error(map_fn.span, "`from` map function must be a function");
                        }
                        call(self, "typedArrayFromMapped", vec![source, map_fn, kind_lit])
                    }
                    None => call(self, "typedArrayFrom", vec![source, kind_lit]),
                }
            }
            other => {
                self.error(span, format!("`{other}` is not a typed-array static"));
                self.poison(span)
            }
        }
    }

    fn check_buffer_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let u8_view = Ty::BufferView(hir::BufferViewKind::U8);
        let string_lit = |text: &str, span: Span| hir::Expr {
            kind: hir::ExprKind::String(text.to_string()),
            ty: Ty::String,
            span,
        };
        let int_lit = |value: u64, span: Span| hir::Expr {
            kind: hir::ExprKind::Number(value as f64),
            ty: Ty::Int(IntKind::U64),
            span,
        };
        let call = |checker: &mut Self, native: &str, call_args: Vec<hir::Expr>, ty: Ty| {
            let Some((id, _)) = zun_natives::resolve("nodeBuffer", native) else {
                checker.error(span, format!("missing Buffer intrinsic `{native}`"));
                return checker.poison(span);
            };
            hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: call_args }, ty, span }
        };
        match method {
            "from" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(span, "`Buffer.from` expects 1 or 2 arguments".to_string());
                    return self.poison(span);
                }
                let source = self.check_expr(cx, &args[0]);
                match source.ty {
                    Ty::String => {
                        let encoding = match args.get(1) {
                            Some(arg) => {
                                let value = self.check_expr(cx, arg);
                                if value.ty != Ty::String && !value.ty.is_error() {
                                    self.error(value.span, "Buffer encoding must be a string");
                                }
                                value
                            }
                            None => string_lit("utf8", span),
                        };
                        call(self, "nodeBufferFromString", vec![source, encoding], u8_view)
                    }
                    Ty::Array => call(self, "nodeBufferFromArray", vec![source], u8_view),
                    Ty::Buffer => {
                        call(self, "nodeBufferFromArrayBuffer", vec![source], u8_view)
                    }
                    other if !other.is_error() => {
                        self.error(
                            source.span,
                            format!("`Buffer.from` expects a string, array or ArrayBuffer, found {other:?}"),
                        );
                        self.poison(span)
                    }
                    _ => self.poison(span),
                }
            }
            "alloc" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(span, "`Buffer.alloc` expects 1 or 2 arguments".to_string());
                    return self.poison(span);
                }
                let mut size = self.check_expr(cx, &args[0]);
                coerce_numeric_literal(&mut size, Ty::Int(IntKind::U64));
                if !size.ty.is_int() && !size.ty.is_error() {
                    self.error(size.span, "Buffer size must be an integer");
                }
                let fill = match args.get(1) {
                    Some(arg) => {
                        let mut value = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut value, Ty::Int(IntKind::U8));
                        if !value.ty.is_int() && !value.ty.is_error() {
                            self.error(value.span, "Buffer fill byte must be an integer");
                        }
                        value
                    }
                    None => int_lit(0, span),
                };
                call(self, "nodeBufferAlloc", vec![size, fill], u8_view)
            }
            "allocUnsafe" => {
                if args.len() != 1 {
                    self.error(span, "`Buffer.allocUnsafe` expects 1 argument".to_string());
                    return self.poison(span);
                }
                let mut size = self.check_expr(cx, &args[0]);
                coerce_numeric_literal(&mut size, Ty::Int(IntKind::U64));
                if !size.ty.is_int() && !size.ty.is_error() {
                    self.error(size.span, "Buffer size must be an integer");
                }
                call(self, "nodeBufferAllocUnsafe", vec![size], u8_view)
            }
            "concat" => {
                if args.len() != 1 {
                    self.error(span, "`Buffer.concat` expects 1 argument (a list)".to_string());
                    return self.poison(span);
                }
                let list = self.check_expr(cx, &args[0]);
                if list.ty != Ty::Array && !list.ty.is_error() {
                    self.error(list.span, "`Buffer.concat` expects an array of buffers");
                }
                call(self, "nodeBufferConcat", vec![list], u8_view)
            }
            "isBuffer" => {
                if args.len() != 1 {
                    self.error(span, "`Buffer.isBuffer` expects 1 argument".to_string());
                    return self.poison(span);
                }
                let mut arg = self.check_expr(cx, &args[0]);
                if arg.ty != Ty::Any && !arg.ty.is_error() {
                    arg = hir::Expr {
                        span: arg.span,
                        kind: hir::ExprKind::BoxAny { value: Box::new(arg) },
                        ty: Ty::Any,
                    };
                }
                call(self, "nodeBufferIsBuffer", vec![arg], Ty::Boolean)
            }
            "byteLength" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(span, "`Buffer.byteLength` expects 1 or 2 arguments".to_string());
                    return self.poison(span);
                }
                let string = self.check_expr(cx, &args[0]);
                if string.ty != Ty::String && !string.ty.is_error() {
                    self.error(string.span, "`Buffer.byteLength` expects a string");
                }
                let encoding = match args.get(1) {
                    Some(arg) => self.check_expr(cx, arg),
                    None => string_lit("utf8", span),
                };
                call(self, "nodeBufferByteLength", vec![string, encoding], Ty::Int(IntKind::U64))
            }
            other => {
                self.error(method_span, format!("`Buffer.{other}` is not supported in this runtime"));
                self.poison(span)
            }
        }
    }

    /// Lower a `Buffer` instance method on a `U8` byte view. Handles the
    /// optional-argument surface (defaults applied here); fixed readers/writers
    /// are checked against their `nodeBuffer` native signature. A length-valued
    /// default (`toString`/`slice` end) is the sentinel `u64::MAX`, clamped to
    /// the real length by the interpreter floor.
    fn check_buffer_method(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let u8_view = Ty::BufferView(hir::BufferViewKind::U8);
        let u64_ty = Ty::Int(IntKind::U64);
        let string_lit = |text: &str, span: Span| hir::Expr {
            kind: hir::ExprKind::String(text.to_string()),
            ty: Ty::String,
            span,
        };
        let int_lit = |value: u64, span: Span| hir::Expr {
            kind: hir::ExprKind::Number(value as f64),
            ty: u64_ty,
            span,
        };
        let call = |checker: &mut Self, native: &str, call_args: Vec<hir::Expr>, ty: Ty| {
            let Some((id, _)) = zun_natives::resolve("nodeBuffer", native) else {
                checker.error(span, format!("missing Buffer intrinsic `{native}`"));
                return checker.poison(span);
            };
            hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: call_args }, ty, span }
        };
        // Check an optional integer argument, applying `default` when absent.
        macro_rules! opt_int {
            ($pos:expr, $default:expr) => {{
                match args.get($pos) {
                    Some(arg) => {
                        let mut value = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut value, u64_ty);
                        if !value.ty.is_int() && !value.ty.is_error() {
                            self.error(value.span, "Buffer offset/index must be an integer");
                        }
                        value
                    }
                    None => int_lit($default, span),
                }
            }};
        }
        match mname {
            "toString" => {
                if args.len() > 3 {
                    self.error(span, "`toString` expects at most 3 arguments".to_string());
                    return self.poison(span);
                }
                let encoding = match args.first() {
                    Some(arg) => {
                        let value = self.check_expr(cx, arg);
                        if value.ty != Ty::String && !value.ty.is_error() {
                            self.error(value.span, "Buffer encoding must be a string");
                        }
                        value
                    }
                    None => string_lit("utf8", span),
                };
                let start = opt_int!(1, 0);
                let end = opt_int!(2, u64::MAX);
                call(self, "nodeBufferToString", vec![receiver, encoding, start, end], Ty::String)
            }
            "write" => {
                if args.is_empty() || args.len() > 3 {
                    self.error(span, "`write` expects 1 to 3 arguments".to_string());
                    return self.poison(span);
                }
                let string = self.check_expr(cx, &args[0]);
                if string.ty != Ty::String && !string.ty.is_error() {
                    self.error(string.span, "`write` expects a string");
                }
                let offset = opt_int!(1, 0);
                let encoding = match args.get(2) {
                    Some(arg) => self.check_expr(cx, arg),
                    None => string_lit("utf8", span),
                };
                call(self, "nodeBufferWrite", vec![receiver, string, offset, encoding], u64_ty)
            }
            "slice" => {
                if args.len() > 2 {
                    self.error(span, "`slice` expects at most 2 arguments".to_string());
                    return self.poison(span);
                }
                // Signed indices: a negative counts from the end. The `end`
                // default is a large positive sentinel the floor clamps to the
                // real length (a negative sentinel would be read as `len-1`).
                let i64_ty = Ty::Int(IntKind::I64);
                let opt_signed = |checker: &mut Self, cx: &mut FunctionCx, pos: usize, default: f64| {
                    match args.get(pos) {
                        Some(arg) => {
                            let mut value = checker.check_expr(cx, arg);
                            coerce_numeric_literal(&mut value, i64_ty);
                            if !value.ty.is_int() && !value.ty.is_error() {
                                checker.error(value.span, "Buffer offset/index must be an integer");
                            }
                            value
                        }
                        None => hir::Expr {
                            kind: hir::ExprKind::Number(default),
                            ty: i64_ty,
                            span,
                        },
                    }
                };
                let start = opt_signed(self, cx, 0, 0.0);
                let end = opt_signed(self, cx, 1, (1i64 << 53) as f64);
                call(self, "nodeBufferSlice", vec![receiver, start, end], u8_view)
            }
            "fill" => {
                if args.len() != 1 {
                    self.error(span, "`fill` expects 1 argument (a byte value)".to_string());
                    return self.poison(span);
                }
                let mut value = self.check_expr(cx, &args[0]);
                coerce_numeric_literal(&mut value, Ty::Int(IntKind::U8));
                if !value.ty.is_int() && !value.ty.is_error() {
                    self.error(value.span, "Buffer fill byte must be an integer");
                }
                call(self, "nodeBufferFill", vec![receiver, value], Ty::Boolean)
            }
            "equals" => {
                if args.len() != 1 {
                    self.error(span, "`equals` expects 1 argument".to_string());
                    return self.poison(span);
                }
                let other = self.check_expr(cx, &args[0]);
                if other.ty != u8_view && !other.ty.is_error() {
                    self.error(other.span, "`equals` expects another Buffer");
                }
                call(self, "nodeBufferEquals", vec![receiver, other], Ty::Boolean)
            }
            "copy" => {
                if args.is_empty() || args.len() > 4 {
                    self.error(span, "`copy` expects 1 to 4 arguments".to_string());
                    return self.poison(span);
                }
                let target = self.check_expr(cx, &args[0]);
                if target.ty != u8_view && !target.ty.is_error() {
                    self.error(target.span, "`copy` target must be a Buffer");
                }
                // Signed indices so a negative counts from the end (consistent
                // with `slice`). The `sourceEnd` default is a large positive
                // sentinel the floor clamps to the real length (NUCLEUS #21).
                let i64_ty = Ty::Int(IntKind::I64);
                let opt_signed = |checker: &mut Self, cx: &mut FunctionCx, pos: usize, default: f64| {
                    match args.get(pos) {
                        Some(arg) => {
                            let mut value = checker.check_expr(cx, arg);
                            coerce_numeric_literal(&mut value, i64_ty);
                            if !value.ty.is_int() && !value.ty.is_error() {
                                checker.error(value.span, "Buffer offset/index must be an integer");
                            }
                            value
                        }
                        None => hir::Expr {
                            kind: hir::ExprKind::Number(default),
                            ty: i64_ty,
                            span,
                        },
                    }
                };
                let target_start = opt_signed(self, cx, 1, 0.0);
                let source_start = opt_signed(self, cx, 2, 0.0);
                let source_end = opt_signed(self, cx, 3, (1i64 << 53) as f64);
                call(
                    self,
                    "nodeBufferCopy",
                    vec![receiver, target, target_start, source_start, source_end],
                    u64_ty,
                )
            }
            "swap16" | "swap32" | "swap64" => {
                if !args.is_empty() {
                    self.error(span, format!("`{mname}` expects no arguments"));
                    return self.poison(span);
                }
                let native = match mname {
                    "swap16" => "nodeBufferSwap16",
                    "swap32" => "nodeBufferSwap32",
                    _ => "nodeBufferSwap64",
                };
                call(self, native, vec![receiver], u8_view)
            }
            "indexOf" | "includes" | "lastIndexOf" => {
                if args.is_empty() || args.len() > 3 {
                    self.error(span, format!("`{mname}` expects 1 to 3 arguments"));
                    return self.poison(span);
                }
                // The needle (number / string / Buffer) is boxed to `Any`; the
                // optional byteOffset is signed; the optional encoding is a
                // string. `lastIndexOf` defaults its offset to the length (a
                // large sentinel the floor clamps down).
                let needle_raw = self.check_expr(cx, &args[0]);
                let needle = if needle_raw.ty == Ty::Any || needle_raw.ty.is_error() {
                    needle_raw
                } else {
                    hir::Expr {
                        span: needle_raw.span,
                        kind: hir::ExprKind::BoxAny { value: Box::new(needle_raw) },
                        ty: Ty::Any,
                    }
                };
                let i64_ty = Ty::Int(IntKind::I64);
                let default_offset = if mname == "lastIndexOf" { (1i64 << 53) as f64 } else { 0.0 };
                let byte_offset = match args.get(1) {
                    Some(arg) => {
                        let mut value = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut value, i64_ty);
                        if !value.ty.is_int() && !value.ty.is_error() {
                            self.error(value.span, "Buffer byteOffset must be an integer");
                        }
                        value
                    }
                    None => hir::Expr {
                        kind: hir::ExprKind::Number(default_offset),
                        ty: i64_ty,
                        span,
                    },
                };
                let encoding = match args.get(2) {
                    Some(arg) => self.check_expr(cx, arg),
                    None => string_lit("utf8", span),
                };
                let (native, ret) = match mname {
                    "indexOf" => ("nodeBufferIndexOf", Ty::Int(IntKind::I64)),
                    "lastIndexOf" => ("nodeBufferLastIndexOf", Ty::Int(IntKind::I64)),
                    _ => ("nodeBufferIncludes", Ty::Boolean),
                };
                call(self, native, vec![receiver, needle, byte_offset, encoding], ret)
            }
            // Variable-width integer readers `readUIntLE(offset, byteLength)`.
            "readUIntLE" | "readUIntBE" | "readIntLE" | "readIntBE" => {
                let native = buffer_accessor_native(mname);
                let Some((id, info)) = zun_natives::resolve("nodeBuffer", &native) else {
                    self.error(span, format!("`{mname}` is not a supported Buffer reader"));
                    return self.poison(span);
                };
                if args.len() != 2 {
                    self.error(span, format!("`{mname}` expects an offset and a byteLength"));
                    return self.poison(span);
                }
                let offset = opt_int!(0, 0);
                let byte_length = opt_int!(1, 1);
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, offset, byte_length] },
                    ty: native_ty(info.ret),
                    span,
                }
            }
            // Variable-width integer writers `writeUIntLE(value, offset, byteLength)`.
            "writeUIntLE" | "writeUIntBE" | "writeIntLE" | "writeIntBE" => {
                let native = buffer_accessor_native(mname);
                let Some((id, info)) = zun_natives::resolve("nodeBuffer", &native) else {
                    self.error(span, format!("`{mname}` is not a supported Buffer writer"));
                    return self.poison(span);
                };
                if args.len() != 3 {
                    self.error(span, format!("`{mname}` expects a value, offset and byteLength"));
                    return self.poison(span);
                }
                // Node order `(value, offset, byteLength)`; native takes
                // `(view, offset, value, byteLength)`.
                let mut value = self.check_expr(cx, &args[0]);
                let value_ty = native_ty(info.params[2]);
                coerce_numeric_literal(&mut value, value_ty);
                if !value.ty.is_error() && !self.is_assignable(value.ty, value_ty) {
                    self.error(value.span, format!("argument type {:?} does not match {value_ty:?}", value.ty));
                }
                let offset = opt_int!(1, 0);
                let byte_length = opt_int!(2, 1);
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![receiver, offset, value, byte_length],
                    },
                    ty: Ty::Int(IntKind::U64),
                    span,
                }
            }
            read if read.starts_with("read") => {
                let native = buffer_accessor_native(read);
                let Some((id, info)) = zun_natives::resolve("nodeBuffer", &native) else {
                    self.error(span, format!("`{read}` is not a supported Buffer reader"));
                    return self.poison(span);
                };
                if args.len() > 1 {
                    self.error(span, format!("`{read}` expects at most 1 argument"));
                    return self.poison(span);
                }
                let offset = opt_int!(0, 0);
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, offset] },
                    ty: native_ty(info.ret),
                    span,
                }
            }
            write if write.starts_with("write") => {
                let native = buffer_accessor_native(write);
                let Some((id, info)) = zun_natives::resolve("nodeBuffer", &native) else {
                    self.error(span, format!("`{write}` is not a supported Buffer writer"));
                    return self.poison(span);
                };
                if args.is_empty() || args.len() > 2 {
                    self.error(span, format!("`{write}` expects 1 or 2 arguments"));
                    return self.poison(span);
                }
                // Node order is `write<Kind>(value, offset?)`; the native takes
                // `(view, offset, value)`.
                let mut value = self.check_expr(cx, &args[0]);
                let value_ty = native_ty(info.params[2]);
                coerce_numeric_literal(&mut value, value_ty);
                if !value.ty.is_error() && !self.is_assignable(value.ty, value_ty) {
                    self.error(value.span, format!("argument type {:?} does not match {value_ty:?}", value.ty));
                }
                let offset = opt_int!(1, 0);
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, offset, value] },
                    ty: Ty::Int(IntKind::U64),
                    span,
                }
            }
            other => {
                self.error(span, format!("`{other}` is not a supported Buffer method"));
                self.poison(span)
            }
        }
    }

    /// Lower a `Number.<method>(...)` static call. The strict predicates accept
    /// any argument type (JS does not coerce — a non-number is simply `false`);
    /// `parseFloat`/`parseInt` take a string and an optional radix.
    /// `Math.<fn>(...)` — route a JS Math call to the `math` native group's
    /// f64 helper of the matching name (`floor` → `floorNumber`, `pow` →
    /// `powNumber`, …). Arity and argument coercion follow the native's declared
    /// signature; the result type is the native's return type.
    /// Shared lowering for `for-of`/`for-in`: given the iterable (or key array)
    /// and the iterator-protocol intrinsic names, drive the iterator and bind
    /// each value to the loop pattern.
    fn lower_for_iteration(
        &mut self,
        cx: &mut FunctionCx,
        iterable_expr: hir::Expr,
        new_name: &str,
        done_name: &str,
        next_name: &str,
        pattern: &ast::Pattern,
        body: &ast::Block,
    ) -> hir::Stmt {
        let span = iterable_expr.span;
        let Some((new_id, _)) = zun_natives::resolve("buffer", new_name) else {
            self.error(span, "missing iterator constructor intrinsic");
            return hir::Stmt::Expr(self.poison(span));
        };
        let Some((done_id, _)) = zun_natives::resolve("buffer", done_name) else {
            self.error(span, "missing iterator completion intrinsic");
            return hir::Stmt::Expr(self.poison(span));
        };
        let Some((next_id, _)) = zun_natives::resolve("buffer", next_name) else {
            self.error(span, "missing iterator next intrinsic");
            return hir::Stmt::Expr(self.poison(span));
        };
        let iterator = hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id: new_id, args: vec![iterable_expr] },
            ty: Ty::Any,
            span,
        };
        let iterator_slot = cx.declare("__for_of_iterator", Ty::Any);
        let done = hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id: done_id,
                args: vec![hir::Expr { kind: hir::ExprKind::Local(iterator_slot), ty: Ty::Any, span }],
            },
            ty: Ty::Boolean,
            span,
        };
        let next = hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id: next_id,
                args: vec![hir::Expr { kind: hir::ExprKind::Local(iterator_slot), ty: Ty::Any, span }],
            },
            ty: Ty::Any,
            span,
        };
        cx.scopes.push(Scope::new());
        let (binding, prelude) = match pattern {
            ast::Pattern::Ident { name, ty } => {
                let declared = ty.as_ref().map(|t| self.resolve_annotation(t)).unwrap_or(Ty::Any);
                let name_text = self.text(name).to_string();
                let slot = cx.declare(&name_text, declared);
                // A loop variable that a closure captures and that is assigned
                // lives in a per-iteration cell from the start of the body.
                if cx.mutated_names.contains(&name_text)
                    && cx.mutated_names.contains(&format!("{name_text}"))
                    && !matches!(declared, Ty::Function(_))
                {
                    cx.boxed.insert(slot);
                }
                (slot, Vec::new())
            }
            _ => {
                let element = cx.declare(&self.fresh_temp_name(cx), Ty::Any);
                let mut prelude = Vec::new();
                let element_expr = hir::Expr { kind: hir::ExprKind::Local(element), ty: Ty::Any, span };
                self.bind_pattern(cx, pattern, element_expr, &mut prelude);
                (element, prelude)
            }
        };
        cx.loop_depth += 1;
        let mut body = self.check_block(cx, body);
        cx.loop_depth -= 1;
        if !prelude.is_empty() {
            let mut combined = prelude;
            combined.append(&mut body);
            body = combined;
        }
        cx.scopes.pop();
        hir::Stmt::ForOf { iterator, iterator_slot, done, next, binding, body }
    }

    /// `f(a, ...xs, b)` on a function value: collect the arguments into one
    /// array literal (spreads flattened) and invoke through `functionCallSpread`.
    fn check_spread_call(
        &mut self,
        cx: &mut FunctionCx,
        callee: &ast::Expr,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        let callee_hir = self.check_expr(cx, callee);
        if !matches!(callee_hir.ty, Ty::Function(_) | Ty::Any) {
            return None;
        }
        let elements = args
            .iter()
            .map(|arg| match arg {
                ast::Expr::Spread { expr, .. } => ast::ArrayElement::Spread((**expr).clone()),
                other => ast::ArrayElement::Expr(other.clone()),
            })
            .collect();
        let literal = ast::Expr::ArrayLiteral { elements, span };
        let mut array = self.check_expr(cx, &literal);
        self.box_function_array_elements(&mut array);
        let (id, _) = zun_natives::resolve("collections", "functionCallSpread")?;
        Some(hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: vec![callee_hir, array] },
            ty: Ty::Any,
            span,
        })
    }

    /// `{ slot = bound; test == null ? undefined : call }`: the shared shape of
    /// optional calls on dynamic receivers (`bound` evaluated exactly once).
    fn optional_guarded(
        &mut self,
        cx: &mut FunctionCx,
        slot: hir::LocalSlot,
        bound: hir::Expr,
        test: ast::Expr,
        call: ast::Expr,
        span: Span,
    ) -> hir::Expr {
        self.optional_guarded_with(cx, slot, bound, test, call, None, span)
    }

    /// Like [`Self::optional_guarded`], with an explicit expression for the
    /// nullish case (default: `undefined`).
    #[allow(clippy::too_many_arguments)]
    fn optional_guarded_with(
        &mut self,
        cx: &mut FunctionCx,
        slot: hir::LocalSlot,
        bound: hir::Expr,
        test: ast::Expr,
        call: ast::Expr,
        nullish: Option<ast::Expr>,
        span: Span,
    ) -> hir::Expr {
        if let Some(probe) = nullish {
            // Nullish member: only a managed receiver (Map, class instance, Date ...)
            // may still resolve the method through the `?`-prefixed dynamic call;
            // anything else short-circuits to `undefined` without evaluating the
            // arguments.
            let Some((probe_id, _)) = zun_natives::resolve("collections", "optionalProbeOk") else {
                return self.poison(span);
            };
            let tmp_name = match &test {
                ast::Expr::Assign { value, .. } => match value.as_ref() {
                    ast::Expr::Field { object, .. } => object.as_ref().clone(),
                    _ => return self.poison(span),
                },
                _ => return self.poison(span),
            };
            let receiver_hir = self.check_expr(cx, &tmp_name);
            let ok = hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id: probe_id, args: vec![Self::box_to_any(receiver_hir)] },
                ty: Ty::Boolean,
                span,
            };
            let probe_hir = self.check_expr(cx, &probe);
            let undefined_hir = self.check_expr(
                cx,
                &ast::Expr::Unary {
                    op: ast::UnaryOp::Void,
                    operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                    span,
                },
            );
            let nullish_hir = hir::Expr {
                kind: hir::ExprKind::Conditional {
                    condition: Box::new(ok),
                    when_true: Box::new(Self::box_to_any(probe_hir)),
                    when_false: Box::new(Self::box_to_any(undefined_hir)),
                },
                ty: Ty::Any,
                span,
            };
            let cond_ast = ast::Expr::Binary {
                op: ast::BinaryOp::LooseEq,
                lhs: Box::new(test),
                rhs: Box::new(ast::Expr::Null { span }),
                span,
            };
            let condition = self.check_condition(cx, &cond_ast);
            let call_hir = self.check_expr(cx, &call);
            let value = hir::Expr {
                kind: hir::ExprKind::Conditional {
                    condition: Box::new(condition),
                    when_true: Box::new(nullish_hir),
                    when_false: Box::new(Self::box_to_any(call_hir)),
                },
                ty: Ty::Any,
                span,
            };
            return hir::Expr {
                kind: hir::ExprKind::Sequence {
                    effect: Box::new(hir::Expr {
                        kind: hir::ExprKind::Assign { slot, value: Box::new(bound) },
                        ty: Ty::Any,
                        span,
                    }),
                    value: Box::new(value),
                },
                ty: Ty::Any,
                span,
            };
        }
        let cond = ast::Expr::Conditional {
            condition: Box::new(ast::Expr::Binary {
                op: ast::BinaryOp::LooseEq,
                lhs: Box::new(test),
                rhs: Box::new(ast::Expr::Null { span }),
                span,
            }),
            when_true: Box::new(ast::Expr::Unary {
                op: ast::UnaryOp::Void,
                operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                span,
            }),
            when_false: Box::new(call),
            span,
        };
        let value = self.check_expr(cx, &cond);
        let ty = value.ty;
        hir::Expr {
            kind: hir::ExprKind::Sequence {
                effect: Box::new(hir::Expr {
                    kind: hir::ExprKind::Assign { slot, value: Box::new(bound) },
                    ty: Ty::Any,
                    span,
                }),
                value: Box::new(value),
            },
            ty,
            span,
        }
    }

    /// Lower a `switch` with fall-through: the discriminant is evaluated once
    /// into a temp, the first matching case index is computed lazily with a
    /// nested conditional (default index otherwise), and each clause body runs
    /// when `index <= clause`. `break` exits the enclosing synthetic label.
    fn check_switch(
        &mut self,
        cx: &mut FunctionCx,
        discriminant: &ast::Expr,
        cases: &[ast::SwitchCase],
        span: Span,
    ) -> hir::Stmt {
        let disc = self.check_expr(cx, discriminant);
        let disc = Self::box_to_any(disc);
        cx.scopes.push(Scope::new());
        let disc_slot = cx.declare(&self.fresh_temp_name(cx), Ty::Any);
        let idx_slot = cx.declare(&self.fresh_temp_name(cx), Ty::F64);
        let num = |value: f64| hir::Expr { kind: hir::ExprKind::Number(value), ty: Ty::F64, span };
        let none_index = cases.len() as f64;
        let default_index = cases
            .iter()
            .position(|c| c.test.is_none())
            .map_or(none_index, |p| p as f64);
        let mut index_expr = num(default_index);
        let mut tests = Vec::new();
        for (position, case) in cases.iter().enumerate() {
            if let Some(test) = &case.test {
                let test = self.check_expr(cx, test);
                tests.push((position, Self::box_to_any(test)));
            }
        }
        for (position, test) in tests.into_iter().rev() {
            let cond = hir::Expr {
                kind: hir::ExprKind::Binary {
                    op: ast::BinaryOp::Eq,
                    lhs: Box::new(hir::Expr { kind: hir::ExprKind::Local(disc_slot), ty: Ty::Any, span }),
                    rhs: Box::new(test),
                },
                ty: Ty::Boolean,
                span,
            };
            index_expr = hir::Expr {
                kind: hir::ExprKind::Conditional {
                    condition: Box::new(cond),
                    when_true: Box::new(num(position as f64)),
                    when_false: Box::new(index_expr),
                },
                ty: Ty::F64,
                span,
            };
        }
        let mut out = vec![
            hir::Stmt::Let { slot: disc_slot, init: disc },
            hir::Stmt::Let { slot: idx_slot, init: index_expr },
        ];
        cx.switch_depth += 1;
        for (position, case) in cases.iter().enumerate() {
            if case.body.statements.is_empty() {
                continue;
            }
            // All case bodies share the switch's one lexical scope, so a `let`
            // declared in one case is visible to the cases after it.
            let body: Vec<hir::Stmt> =
                case.body.statements.iter().map(|statement| self.check_stmt(cx, statement)).collect();
            let cond = hir::Expr {
                kind: hir::ExprKind::Binary {
                    op: ast::BinaryOp::Le,
                    lhs: Box::new(hir::Expr { kind: hir::ExprKind::Local(idx_slot), ty: Ty::F64, span }),
                    rhs: Box::new(num(position as f64)),
                },
                ty: Ty::Boolean,
                span,
            };
            out.push(hir::Stmt::If { cond, then_body: body, else_body: Vec::new() });
        }
        cx.switch_depth -= 1;
        cx.scopes.pop();
        hir::Stmt::Labeled {
            label: "\0switch".to_string(),
            body: Box::new(hir::Stmt::Group(out)),
        }
    }

    /// Coerce a computed array index to an integer. An integer passes through;
    /// a `number`/`any` index is converted (JS `ToInteger` on the index) rather
    /// than rejected, so `arr[someNumber]` works.
    fn coerce_array_index(&mut self, index: hir::Expr) -> hir::Expr {
        if index.ty.is_int() || index.ty.is_error() {
            return index;
        }
        if index.ty == Ty::F64 || index.ty == Ty::Any {
            let span = index.span;
            return hir::Expr {
                kind: hir::ExprKind::Convert { value: Box::new(index) },
                ty: Ty::Int(IntKind::U64),
                span,
            };
        }
        self.error(index.span, "array index must be an integer");
        self.poison(index.span)
    }

    /// JS ToNumber of an arbitrary operand as an `f64` expression. `number`
    /// operands pass through; everything else is boxed into `Any` and run
    /// through the `numberCoerce` native (string parsing, object->primitive).
    fn to_number_expr(&mut self, value: hir::Expr) -> hir::Expr {
        self.to_number_expr_with(value, "numberCoerce")
    }

    /// ToNumber that throws on BigInt/Symbol (`Math.*` argument coercion).
    fn to_number_expr_strict(&mut self, value: hir::Expr) -> hir::Expr {
        self.to_number_expr_with(value, "numberCoerceStrict")
    }

    fn to_number_expr_with(&mut self, value: hir::Expr, native: &str) -> hir::Expr {
        if value.ty == Ty::F64 || value.ty.is_error() {
            return value;
        }
        let span = value.span;
        let Some((id, _)) = zun_natives::resolve("collections", native) else {
            self.error(span, "missing `numberCoerce` intrinsic");
            return self.poison(span);
        };
        let arg = if value.ty == Ty::Any {
            value
        } else {
            hir::Expr { span, kind: hir::ExprKind::BoxAny { value: Box::new(value) }, ty: Ty::Any }
        };
        hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] }, ty: Ty::F64, span }
    }

    /// Coerce a value to an `f64` argument: integers widen, everything else
    /// goes through JS ToNumber.
    /// ToString of a `parseInt`/`parseFloat` operand: a string passes through,
    /// anything else goes through the `String(x)` coercion helper.
    /// `split` limit operand: an integer passes through; anything else goes
    /// through ToUint32 (`undefined` meaning unbounded) at run time.
    fn split_limit_expr(&mut self, value: hir::Expr) -> hir::Expr {
        if value.ty.is_int() || value.ty.is_error() {
            return value;
        }
        let span = value.span;
        let Some((id, _)) = zun_natives::resolve("collections", "splitLimit") else {
            return value;
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: vec![Self::box_to_any(value)] },
            ty: Ty::Int(IntKind::I64),
            span,
        }
    }

    fn to_string_for_parse(&mut self, value: hir::Expr) -> hir::Expr {
        if value.ty == Ty::String || value.ty.is_error() {
            return value;
        }
        let span = value.span;
        let Some((id, _)) = zun_natives::resolve("collections", "stringCoerceStrict") else {
            return value;
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: vec![Self::box_to_any(value)] },
            ty: Ty::String,
            span,
        }
    }

    /// `new Array(...)` / `Array(...)` whose argument list is not a single
    /// statically-integer length (several arguments, a spread, or a non-integer
    /// single value): lowered to the prelude `__zunArrayCtor(argsArray)`.
    fn array_ctor_dynamic(&mut self, cx: &mut FunctionCx, args: &[ast::Expr], span: Span) -> Option<hir::Expr> {
        let func = *self.by_name.get("__zunArrayCtor")?;
        let single_plain = args.len() == 1 && !matches!(args[0], ast::Expr::Spread { .. });
        if single_plain {
            let mut probe = self.check_expr(cx, &args[0]);
            coerce_numeric_literal(&mut probe, Ty::INT);
            if probe.ty.is_int() || probe.ty.is_error() {
                return None;
            }
        }
        let elements = args
            .iter()
            .map(|arg| match arg {
                ast::Expr::Spread { expr, .. } => ast::ArrayElement::Spread((**expr).clone()),
                other => ast::ArrayElement::Expr(other.clone()),
            })
            .collect();
        let list = self.check_expr(cx, &ast::Expr::ArrayLiteral { elements, span });
        Some(hir::Expr { kind: hir::ExprKind::Call { func, args: vec![list] }, ty: Ty::Array, span })
    }

    /// Whether an argument is a `function (this: T, ...)` expression that
    /// declares the hidden receiver parameter.
    fn ast_function_takes_this(&self, expr: &ast::Expr) -> bool {
        let ast::Expr::Function(function) = expr else { return false };
        if function.is_arrow {
            return false;
        }
        let text = &self.source[function.span.start as usize..function.span.end as usize];
        text.find('(').is_some_and(|open| text[open + 1..].trim_start().starts_with("this"))
    }

    fn to_number_for_arg(&mut self, value: hir::Expr) -> hir::Expr {
        if value.ty.is_int() {
            let span = value.span;
            return hir::Expr {
                kind: hir::ExprKind::Convert { value: Box::new(value) },
                ty: Ty::F64,
                span,
            };
        }
        self.to_number_expr(value)
    }

    /// `Math.<fn>.apply(null, array)` and `Math.<fn>(...array)`: evaluate the
    /// array at run time through the `mathApply` intrinsic.
    fn math_apply_expr(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        array: &ast::Expr,
        span: Span,
    ) -> hir::Expr {
        let Some((id, _)) = zun_natives::resolve("collections", "mathApply") else {
            self.error(span, "missing `mathApply` intrinsic");
            return self.poison(span);
        };
        let value = self.check_expr(cx, array);
        let value = if value.ty == Ty::Any || value.ty.is_error() {
            value
        } else {
            hir::Expr { span: value.span, kind: hir::ExprKind::BoxAny { value: Box::new(value) }, ty: Ty::Any }
        };
        let name = hir::Expr { kind: hir::ExprKind::String(method.to_string()), ty: Ty::String, span };
        hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: vec![name, value] }, ty: Ty::F64, span }
    }

    fn check_math_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        if let [ast::Expr::Spread { expr, .. }] = args {
            return self.math_apply_expr(cx, method, expr, span);
        }
        // `Math.min`/`max`/`hypot` are variadic: fold the 2-arg native left to
        // right (`f(a,b,c)` → `f(f(a,b),c)`). 0 args give the identity.
        if matches!(method, "min" | "max" | "hypot") {
            let native_name = match method {
                "min" => "minNumber",
                "max" => "maxNumber",
                _ => "hypotNumber",
            };
            let Some((id, _)) = zun_natives::resolve("math", native_name) else {
                self.error(span, format!("missing Math intrinsic `{native_name}`"));
                return self.poison(span);
            };
            let identity = match method {
                "min" => f64::INFINITY,
                "max" => f64::NEG_INFINITY,
                _ => 0.0,
            };
            // Start from the identity and fold every argument through the 2-arg
            // native: `f(identity, a) = a` for min/max, `hypot(0, a) = |a|`, so
            // 0/1/N arguments all get the correct value.
            let mut acc = hir::Expr { kind: hir::ExprKind::Number(identity), ty: Ty::F64, span };
            for arg in args {
                let mut value = self.check_expr(cx, arg);
                coerce_numeric_literal(&mut value, Ty::F64);
                if !value.ty.is_int() && value.ty != Ty::F64 {
                    value = self.to_number_expr_strict(value);
                }
                acc = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![acc, value] },
                    ty: Ty::F64,
                    span,
                };
            }
            return acc;
        }
        let native_name = match method {
            "floor" => "floorNumber",
            "ceil" => "ceilNumber",
            "round" => "roundNumber",
            "trunc" => "truncNumber",
            "sign" => "signNumber",
            "abs" => "abs",
            "sqrt" => "sqrtNumber",
            "cbrt" => "cbrtNumber",
            "sin" => "sinNumber",
            "cos" => "cosNumber",
            "tan" => "tanNumber",
            "exp" => "expNumber",
            "log" => "logNumber",
            "log2" => "log2Number",
            "log10" => "log10Number",
            "expm1" => "expm1Number",
            "log1p" => "log1pNumber",
            "sinh" => "sinhNumber",
            "cosh" => "coshNumber",
            "tanh" => "tanhNumber",
            "asinh" => "asinhNumber",
            "acosh" => "acoshNumber",
            "atanh" => "atanhNumber",
            "pow" => "powNumber",
            "atan2" => "atan2Number",
            "hypot" => "hypotNumber",
            "imul" => "imulNumber",
            "clz32" => "clz32Number",
            "asin" => "asinNumber",
            "acos" => "acosNumber",
            "atan" => "atanNumber",
            "fround" => "froundNumber",
            "min" => "minNumber",
            "max" => "maxNumber",
            _ => {
                self.error(method_span, format!("`Math.{method}` is not supported yet"));
                return self.poison(span);
            }
        };
        let Some((id, native)) = zun_natives::resolve("math", native_name) else {
            self.error(span, format!("missing Math intrinsic `{native_name}`"));
            return self.poison(span);
        };
        // JS semantics: missing arguments are `undefined` (-> NaN), extras ignored.
        let missing = native.params.len().saturating_sub(args.len());
        let args = &args[..args.len().min(native.params.len())];
        if false {
            self.error(
                span,
                format!(
                    "`Math.{method}` expects {} argument(s), found {}",
                    native.params.len(),
                    args.len()
                ),
            );
            return self.poison(span);
        }
        let mut checked = Vec::with_capacity(args.len());
        for (arg, expected) in args.iter().zip(native.params.iter().map(|ty| native_ty(*ty))) {
            let mut value = self.check_expr(cx, arg);
            coerce_numeric_literal(&mut value, expected);
            if expected == Ty::F64 && !value.ty.is_int() && value.ty != Ty::F64 {
                value = self.to_number_expr_strict(value);
            } else if expected == Ty::F64 && value.ty.is_int() {
                value = self.to_number_for_arg(value);
            }
            if !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                self.error(
                    value.span,
                    format!("`Math.{method}` argument type {:?} does not match {expected:?}", value.ty),
                );
            }
            checked.push(value);
        }
        for _ in 0..missing {
            checked.push(hir::Expr { kind: hir::ExprKind::Number(f64::NAN), ty: Ty::F64, span });
        }
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: checked },
            ty: native_ty(native.ret),
            span,
        }
    }

    fn check_number_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let resolve = |checker: &mut Self, native: &str| {
            match zun_natives::resolve("number", native) {
                Some((id, _)) => Some(id),
                None => {
                    checker.error(span, format!("missing Number intrinsic `{native}`"));
                    None
                }
            }
        };
        match method {
            // Strict predicates: exactly one argument, of any type, no coercion.
            "isNaN" | "isFinite" | "isInteger" | "isSafeInteger" => {
                if args.len() != 1 {
                    self.error(span, format!("`Number.{method}` expects 1 argument, found {}", args.len()));
                    return self.poison(span);
                }
                let native = match method {
                    "isNaN" => "isNaNStrict",
                    "isFinite" => "isFiniteStrict",
                    "isInteger" => "isIntegerStrict",
                    _ => "isSafeIntegerStrict",
                };
                let Some(id) = resolve(self, native) else { return self.poison(span) };
                let arg = self.check_expr(cx, &args[0]);
                hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] }, ty: Ty::Boolean, span }
            }
            "parseFloat" => {
                if args.len() != 1 {
                    self.error(span, format!("`Number.parseFloat` expects 1 argument, found {}", args.len()));
                    return self.poison(span);
                }
                let Some(id) = resolve(self, "parseFloatPrefix") else { return self.poison(span) };
                let arg = self.check_expr(cx, &args[0]);
                let arg = self.to_string_for_parse(arg);
                hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] }, ty: Ty::F64, span }
            }
            "parseInt" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(span, format!("`Number.parseInt` expects 1 or 2 arguments, found {}", args.len()));
                    return self.poison(span);
                }
                let Some(id) = resolve(self, "parseIntRadix") else { return self.poison(span) };
                let value = self.check_expr(cx, &args[0]);
                let value = self.to_string_for_parse(value);
                let radix = match args.get(1) {
                    Some(arg) => {
                        let mut radix = self.check_expr(cx, arg);
                        // A dynamic radix goes through ToNumber first.
                        if radix.ty == Ty::Any {
                            if let Some((nid, _)) = zun_natives::resolve("collections", "numberCoerce") {
                                let rspan = radix.span;
                                radix = hir::Expr {
                                    kind: hir::ExprKind::CallBuiltin { id: nid, args: vec![radix] },
                                    ty: Ty::F64,
                                    span: rspan,
                                };
                            }
                        }
                        coerce_numeric_literal(&mut radix, Ty::Int(IntKind::I32));
                        // JS ToInt32 on the radix: a `number`/other int radix is converted.
                        if radix.ty == Ty::F64 {
                            // Reduce modulo 2^32 first so the saturating convert
                            // sees NaN for +-Infinity (ToInt32 gives 0).
                            let rspan = radix.span;
                            let modulus = hir::Expr {
                                kind: hir::ExprKind::Number(4294967296.0),
                                ty: Ty::F64,
                                span: rspan,
                            };
                            radix = hir::Expr {
                                kind: hir::ExprKind::Binary {
                                    op: ast::BinaryOp::Rem,
                                    lhs: Box::new(radix),
                                    rhs: Box::new(modulus),
                                },
                                ty: Ty::F64,
                                span: rspan,
                            };
                        }
                        if radix.ty.is_float() || (radix.ty.is_int() && radix.ty != Ty::Int(IntKind::I32)) {
                            let rspan = radix.span;
                            radix = hir::Expr {
                                kind: hir::ExprKind::Convert { value: Box::new(radix) },
                                ty: Ty::Int(IntKind::I32),
                                span: rspan,
                            };
                        }
                        if !radix.ty.is_error() && !self.is_assignable(radix.ty, Ty::Int(IntKind::I32)) {
                            self.error(radix.span, "`Number.parseInt` radix must be an integer");
                        }
                        radix
                    }
                    // Absent radix → 0 (auto-detect) at the native boundary.
                    None => hir::Expr { kind: hir::ExprKind::Number(0.0), ty: Ty::Int(IntKind::I32), span },
                };
                hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: vec![value, radix] }, ty: Ty::F64, span }
            }
            other => {
                self.error(method_span, format!("`Number.{other}` is not supported in this runtime"));
                self.poison(span)
            }
        }
    }

    /// `str.match/replace/search/split(arg)` where `arg` is a dynamic value
    /// (`any` / dynamic object): delegate at RUNTIME to the argument's
    /// `[Symbol.match/replace/search/split]` method (kind 0/1/2/3) via the
    /// `stringSymbolDispatch` intrinsic. The result is `any` because the
    /// user method's return type is unknown. `extra` is the replacement
    /// (replace) or limit (split); other kinds pass a placeholder.
    fn string_symbol_dispatch(
        &mut self,
        span: Span,
        kind: f64,
        receiver: hir::Expr,
        first: hir::Expr,
        extra: Option<hir::Expr>,
    ) -> hir::Expr {
        let Some((id, _)) = zun_natives::resolve("collections", "stringSymbolDispatch") else {
            self.error(span, "missing collection intrinsic `stringSymbolDispatch`");
            return self.poison(span);
        };
        let boxed = |e: hir::Expr| {
            if e.ty == Ty::Any {
                e
            } else {
                hir::Expr { span: e.span, kind: hir::ExprKind::BoxAny { value: Box::new(e) }, ty: Ty::Any }
            }
        };
        let extra = match extra {
            Some(e) => boxed(e),
            None => boxed(hir::Expr { kind: hir::ExprKind::Number(-1.0), ty: Ty::F64, span }),
        };
        let kind = hir::Expr { kind: hir::ExprKind::Number(kind), ty: Ty::Int(IntKind::I64), span };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, first, extra, kind] },
            ty: Ty::Any,
            span,
        }
    }

    /// Lower a variadic `String.fromCharCode(...)` / `String.fromCodePoint(...)`
    /// static. Each code is turned into its one-code string by a single-code
    /// native and the pieces are folded together with the string `concat`
    /// native, so the compiler needs no variadic native ABI. Zero arguments
    /// yield `""`.
    fn check_string_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let single_native = match method {
            "fromCharCode" => "fromCharCode1",
            "fromCodePoint" => "fromCodePoint1",
            other => {
                self.error(method_span, format!("`String.{other}` is not supported in this runtime"));
                return self.poison(span);
            }
        };
        let Some((single_id, _)) = zun_natives::resolve("string", single_native) else {
            self.error(span, format!("missing String intrinsic `{single_native}`"));
            return self.poison(span);
        };
        let Some((concat_id, _)) = zun_natives::resolve("string", "concat") else {
            self.error(span, "missing String intrinsic `concat`");
            return self.poison(span);
        };
        let mut acc = hir::Expr {
            kind: hir::ExprKind::String(String::new()),
            ty: Ty::String,
            span,
        };
        for arg in args {
            let mut code = self.check_expr(cx, arg);
            coerce_numeric_literal(&mut code, Ty::Int(IntKind::U32));
            if code.ty == Ty::Any || code.ty == Ty::String || code.ty == Ty::DynamicObject {
                // A dynamic code goes through ToNumber first.
                code = self.to_number_expr(code);
            }
            if code.ty.is_float() || (code.ty.is_int() && !self.is_assignable(code.ty, Ty::Int(IntKind::U32))) {
                let cspan = code.span;
                code = hir::Expr {
                    kind: hir::ExprKind::Convert { value: Box::new(code) },
                    ty: Ty::Int(IntKind::U32),
                    span: cspan,
                };
            }
            if !code.ty.is_error() && !self.is_assignable(code.ty, Ty::Int(IntKind::U32)) {
                self.error(
                    code.span,
                    format!("`String.{method}` expects numeric code arguments"),
                );
            }
            let piece = hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id: single_id, args: vec![code] },
                ty: Ty::String,
                span,
            };
            acc = hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id: concat_id, args: vec![acc, piece] },
                ty: Ty::String,
                span,
            };
        }
        acc
    }

    /// Lower an `Array.<method>(...)` static call. `isArray(x)` tests the value;
    /// `of(...)` builds an array from its positional arguments (lowered to an
    /// array literal); `from(src, mapFn?)` materializes an array from a managed
    /// array or string (other iterables deferred), optionally mapping each item.
    fn check_array_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let box_any = |value: hir::Expr| {
            if value.ty == Ty::Any || value.ty.is_error() {
                value
            } else {
                let span = value.span;
                hir::Expr {
                    kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                    ty: Ty::Any,
                    span,
                }
            }
        };
        match method {
            "isArray" => {
                if args.len() != 1 {
                    self.error(
                        span,
                        format!("`Array.isArray` expects 1 argument, found {}", args.len()),
                    );
                    return self.poison(span);
                }
                let arg = box_any(self.check_expr(cx, &args[0]));
                let Some((id, _)) = zun_natives::resolve("array", "arrayIsArray") else {
                    self.error(span, "missing Array intrinsic `arrayIsArray`");
                    return self.poison(span);
                };
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                    ty: Ty::Boolean,
                    span,
                }
            }
            "of" => {
                // `Array.of(...items)` is exactly an array literal of the args.
                let elements =
                    args.iter().map(|element| Some(self.check_expr(cx, element))).collect();
                hir::Expr {
                    kind: hir::ExprKind::ArrayLiteral { elements },
                    ty: Ty::Array,
                    span,
                }
            }
            "from" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(
                        span,
                        format!("`Array.from` expects 1 or 2 argument(s), found {}", args.len()),
                    );
                    return self.poison(span);
                }
                let source = box_any(self.check_expr(cx, &args[0]));
                if args.len() == 2 {
                    let map_fn = self.check_expr(cx, &args[1]);
                    if !map_fn.ty.is_error()
                        && !self.is_assignable(map_fn.ty, Ty::Function(hir::DYNAMIC_FUNCTION))
                    {
                        self.error(map_fn.span, "`Array.from` map argument must be a function");
                    }
                    let Some((id, _)) = zun_natives::resolve("array", "arrayFromMapped") else {
                        self.error(span, "missing Array intrinsic `arrayFromMapped`");
                        return self.poison(span);
                    };
                    hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![source, map_fn] },
                        ty: Ty::Array,
                        span,
                    }
                } else {
                    let Some((id, _)) = zun_natives::resolve("array", "arrayFrom") else {
                        self.error(span, "missing Array intrinsic `arrayFrom`");
                        return self.poison(span);
                    };
                    hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![source] },
                        ty: Ty::Array,
                        span,
                    }
                }
            }
            other => {
                self.error(
                    method_span,
                    format!("`Array.{other}` is not supported in this runtime"),
                );
                self.poison(span)
            }
        }
    }

    /// Lower a `Promise.<method>(...)` static call. `resolve`/`reject` take one
    /// value; the combinators (`all`/`allSettled`/`race`/`any`) take one
    /// iterable, which this floor requires to be a managed array.
    fn check_promise_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        // `Promise.withResolvers()` takes no arguments and returns a dynamic
        // object `{ promise, resolve, reject }`. `resolve`/`reject` live in the
        // object's function slot (function values do not box into `Any`), so
        // they are called as dynamic methods (`r.resolve(v)`); the result is
        // typed `any` so those calls route through the dynamic-call path.
        if method == "withResolvers" {
            if !args.is_empty() {
                self.error(span, format!("`Promise.withResolvers` expects no arguments, found {}", args.len()));
                return self.poison(span);
            }
            let Some((id, _)) = zun_natives::resolve("promise", "promiseWithResolvers") else {
                self.error(span, "missing Promise intrinsic `promiseWithResolvers`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![] },
                ty: Ty::Any,
                span,
            };
        }
        let (native, arg_ty): (&str, Ty) = match method {
            "resolve" => ("promiseResolveStatic", Ty::Any),
            "reject" => ("promiseRejectStatic", Ty::Any),
            "all" => ("promiseAll", Ty::Array),
            "allSettled" => ("promiseAllSettled", Ty::Array),
            "race" => ("promiseRace", Ty::Array),
            "any" => ("promiseAny", Ty::Array),
            other => {
                self.error(
                    method_span,
                    format!("`Promise.{other}` is not supported in this runtime"),
                );
                return self.poison(span);
            }
        };
        // `Promise.resolve()` / `Promise.reject()` settle with `undefined`.
        let undefined_arg = ast::Expr::Unary {
            op: ast::UnaryOp::Void,
            operand: Box::new(ast::Expr::Number { value: 0.0, span }),
            span,
        };
        let arg_expr = match args.len() {
            0 if arg_ty == Ty::Any => &undefined_arg,
            1 => &args[0],
            _ => {
                self.error(span, format!("`Promise.{method}` expects 1 argument, found {}", args.len()));
                return self.poison(span);
            }
        };
        let mut arg = self.check_expr(cx, arg_expr);
        if arg_ty == Ty::Array {
            if !arg.ty.is_error() && !self.is_assignable(arg.ty, Ty::Array) {
                self.error(arg.span, format!("`Promise.{method}` expects an array of promises"));
            }
        } else {
            // `resolve`/`reject` accept any value; box a non-`any` operand.
            if arg.ty != Ty::Any {
                arg = hir::Expr {
                    span: arg.span,
                    kind: hir::ExprKind::BoxAny { value: Box::new(arg) },
                    ty: Ty::Any,
                };
            }
        }
        let Some((id, _)) = zun_natives::resolve("promise", native) else {
            self.error(span, format!("missing Promise intrinsic `{native}`"));
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
            ty: Ty::Promise,
            span,
        }
    }

    /// Classes with runtime-decorated instance members run the
    /// instance-initializer / field-transform hooks on the new object.
    fn wrap_hooked_new(&mut self, class_idx: u32, new: hir::Expr, span: Span) -> hir::Expr {
        if self.hooked_classes.contains(&class_idx) {
            if let Some((init_id, _)) = zun_natives::resolve("collections", "classInstanceInit") {
                let boxed = hir::Expr {
                    kind: hir::ExprKind::BoxAny { value: Box::new(new) },
                    ty: Ty::Any,
                    span,
                };
                let hooked = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id: init_id, args: vec![boxed] },
                    ty: Ty::Any,
                    span,
                };
                return hir::Expr {
                    kind: hir::ExprKind::Convert { value: Box::new(hooked) },
                    ty: Ty::Instance(class_idx),
                    span,
                };
            }
        }
        new
    }

    /// Lower `super.method(args)` (`method = Some(name)`) and `super(args)`
    /// (`method = None`, constructor chaining). Both become a *direct*
    /// (non-virtual) call to the base class's implementation with `this` as the
    /// receiver, so an override in the current class is bypassed.
    ///
    /// The base method is found by walking the `base` chain from the enclosing
    /// class's base. `super(...)` is only legal inside a `constructor`; when no
    /// ancestor declares a `constructor`, `super()` with no arguments is a
    /// no-op (the instance already holds its field defaults) and `super(args)`
    /// is an error.
    fn check_super_call(
        &mut self,
        cx: &mut FunctionCx,
        method: Option<&ast::Name>,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        if cx.this_class.is_none() {
            if let (Some(class), Some(method)) = (self.cur_static_class, method) {
                return self.check_static_super_call(cx, class, method, args, span);
            }
        }
        let Some(this_class) = cx.this_class else {
            self.error(span, "`super` is only valid inside a class method");
            return self.poison(span);
        };
        let Some(base) = self.classes[this_class as usize].base else {
            self.error(span, "`super` used in a class that has no base class (no `extends`)");
            return self.poison(span);
        };
        if method.is_none() && !cx.is_constructor {
            self.error(span, "`super(...)` is only valid inside a constructor");
            return self.poison(span);
        }
        let Some(this_slot) = cx.lookup("this") else {
            self.error(span, "`super` is not supported inside a closure");
            return self.poison(span);
        };
        let this_expr = hir::Expr {
            ty: cx.locals[this_slot as usize],
            kind: hir::ExprKind::Local(this_slot),
            span,
        };
        let (mname, mspan) = match method {
            Some(m) => (self.text(m).to_string(), m.span),
            None => ("constructor".to_string(), span),
        };
        let Some(func) = self.method_of(base, &mname) else {
            if method.is_none() {
                if !args.is_empty() {
                    self.error(span, "base class has no constructor; `super(...)` takes no arguments");
                    return self.poison(span);
                }
                return hir::Expr { kind: hir::ExprKind::Null, ty: Ty::Void, span };
            }
            self.error(mspan, format!("no method `{mname}` on the base class"));
            return self.poison(span);
        };
        if self.signatures[func as usize].is_static {
            self.error(mspan, format!("static method `{mname}` must be called through its class"));
            return self.poison(span);
        }
        if self.signatures[func as usize].rest_param.is_some()
            || args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
        {
            self.error(span, "`super` calls do not support rest/spread parameters yet");
            return self.poison(span);
        }
        let (param_tys, return_ty, optional_count, signature_name) = {
            let sig = &self.signatures[func as usize];
            (sig.params.clone(), sig.return_ty, sig.optional_count, sig.name.clone())
        };
        let mut call_args = vec![this_expr];
        for a in args {
            call_args.push(self.check_expr(cx, a));
        }
        let max = param_tys.len();
        let min = max.saturating_sub(optional_count);
        if call_args.len() < min || call_args.len() > max {
            self.error(
                span,
                format!(
                    "`{mname}` expects {} to {} argument(s), found {}",
                    min - 1,
                    max - 1,
                    call_args.len() - 1
                ),
            );
            return self.poison(span);
        }
        let sig = Signature {
            name: signature_name,
            params: param_tys.clone(),
            return_ty,
            optional_count,
            rest_param: None,
            is_generator: self.signatures[func as usize].is_generator,
            is_static: false,
        };
        self.pad_default_args(&mut call_args, &sig, span);
        for (arg, &expected) in call_args.iter_mut().zip(&param_tys).skip(1) {
            coerce_numeric_literal(arg, expected);
            if !expected.is_error() && !arg.ty.is_error() && !self.is_assignable(arg.ty, expected) {
                self.error(
                    arg.span,
                    format!("argument type {:?} does not match {expected:?}", arg.ty),
                );
            }
        }
        hir::Expr { kind: hir::ExprKind::StaticCall { func, args: call_args }, ty: return_ty, span }
    }

    /// `super.m(args)` inside a static method of `class`: a direct call of the
    /// base class's static `m`, with the current `this` (the receiving class
    /// value) forwarded as its hidden first parameter.
    fn check_static_super_call(
        &mut self,
        cx: &mut FunctionCx,
        class: u32,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let Some(base) = self.classes[class as usize].base else {
            self.error(span, "`super` used in a class that has no base class (no `extends`)");
            return self.poison(span);
        };
        let mname = self.text(method).to_string();
        let Some(func) = self.method_of(base, &mname) else {
            self.error(method.span, format!("no static method `{mname}` on the base class"));
            return self.poison(span);
        };
        if !self.signatures[func as usize].is_static {
            self.error(method.span, format!("`{mname}` is not a static method of the base class"));
            return self.poison(span);
        }
        let hidden_class = self.static_this.contains_key(&func);
        let (param_tys, return_ty, optional_count, signature_name) = {
            let sig = &self.signatures[func as usize];
            (sig.params[hidden_class as usize..].to_vec(), sig.return_ty, sig.optional_count, sig.name.clone())
        };
        let mut call_args: Vec<hir::Expr> = args.iter().map(|a| self.check_expr(cx, a)).collect();
        if call_args.len() > param_tys.len()
            || call_args.len() < param_tys.len().saturating_sub(optional_count)
        {
            self.error(span, format!("`{mname}` expects {} argument(s), found {}", param_tys.len(), call_args.len()));
            return self.poison(span);
        }
        let sig = Signature {
            name: signature_name,
            params: param_tys.clone(),
            return_ty,
            optional_count,
            rest_param: None,
            is_generator: false,
            is_static: true,
        };
        self.pad_default_args(&mut call_args, &sig, span);
        for (arg, &expected) in call_args.iter_mut().zip(&param_tys) {
            coerce_numeric_literal(arg, expected);
        }
        if hidden_class {
            let Some(this_slot) = cx.lookup("this") else {
                self.error(span, "`super` is not supported inside this closure");
                return self.poison(span);
            };
            call_args.insert(
                0,
                hir::Expr { ty: cx.locals[this_slot as usize], kind: hir::ExprKind::Local(this_slot), span },
            );
        }
        hir::Expr { kind: hir::ExprKind::StaticCall { func, args: call_args }, ty: return_ty, span }
    }

    /// Lower `new C(args)` for a class that declares `constructor(...)`.
    /// Fields are allocated at their default initializer (or the typed zero);
    /// the constructor then runs with `this` = the instance. Arguments are
    /// checked against the constructor's parameters (arity, optional
    /// parameters, types), not the field list.
    fn check_new_with_constructor(
        &mut self,
        cx: &mut FunctionCx,
        class_idx: u32,
        name: &str,
        ctor: u32,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let fields = self.full_fields(class_idx);
        let initializers = self.full_initializers(class_idx);
        let mut defaults: Vec<hir::Expr> = Vec::with_capacity(fields.len());
        for (slot, (_, field_ty, _, _)) in fields.iter().enumerate() {
            let field_ty = *field_ty;
            // A typed field declared without an initializer (`id: number;`) carries the
            // implicit `void 0`; it takes the type's zero value instead.
            let initializer = initializers.get(slot).and_then(|init| init.as_ref()).filter(|init| {
                field_ty == Ty::Any
                    || !matches!(init.as_ref(), ast::Expr::Unary { op: ast::UnaryOp::Void, .. })
            });
            let mut value = match initializer {
                Some(init) => {
                    let init = init.clone();
                    self.check_expr(cx, &init)
                }
                None => match field_ty {
                    Ty::Int(_) | Ty::Float(_) => {
                        hir::Expr { kind: hir::ExprKind::Number(0.0), ty: Ty::F64, span }
                    }
                    Ty::Boolean => hir::Expr { kind: hir::ExprKind::Boolean(false), ty: Ty::Boolean, span },
                    Ty::String => {
                        hir::Expr { kind: hir::ExprKind::String(String::new()), ty: Ty::String, span }
                    }
                    // `undefined`: a missing key on an empty dynamic object.
                    Ty::Any => hir::Expr {
                        kind: hir::ExprKind::DynamicGet {
                            object: Box::new(hir::Expr {
                                kind: hir::ExprKind::DynamicObjectLiteral { entries: Vec::new() },
                                ty: Ty::DynamicObject,
                                span,
                            }),
                            name: "undefined".to_string(),
                        },
                        ty: Ty::Any,
                        span,
                    },
                    other => hir::Expr { kind: hir::ExprKind::Null, ty: other, span },
                },
            };
            coerce_numeric_literal(&mut value, field_ty);
            defaults.push(value);
        }

        let (param_tys, optional_count, rest_param, sig_name) = {
            let sig = &self.signatures[ctor as usize];
            (sig.params.clone(), sig.optional_count, sig.rest_param, sig.name.clone())
        };
        if let Some(rest_slot) = rest_param {
            // The arguments from the rest position on are gathered (spreads
            // flattened) into one array passed as the last constructor argument.
            let fixed = rest_slot - 1;
            if args.len() < fixed.saturating_sub(optional_count)
                || args[..args.len().min(fixed)].iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
            {
                self.error(span, format!("`{name}` constructor expects at least {fixed} fixed argument(s)"));
                return self.poison(span);
            }
            let mut call_args: Vec<hir::Expr> = Vec::new();
            for arg in args.iter().take(fixed) {
                call_args.push(self.check_expr(cx, arg));
            }
            while call_args.len() < fixed {
                let expected = param_tys[call_args.len() + 1];
                call_args.push(default_args::missing_arg(expected, span));
            }
            let elements = args
                .iter()
                .skip(fixed)
                .map(|arg| match arg {
                    ast::Expr::Spread { expr, .. } => ast::ArrayElement::Spread((**expr).clone()),
                    other => ast::ArrayElement::Expr(other.clone()),
                })
                .collect();
            let mut rest_array = self.check_expr(cx, &ast::Expr::ArrayLiteral { elements, span });
            self.box_function_array_elements(&mut rest_array);
            call_args.push(rest_array);
            for (arg, &expected) in call_args.iter_mut().zip(&param_tys[1..]).take(fixed) {
                coerce_numeric_literal(arg, expected);
                if !expected.is_error() && !arg.ty.is_error() && !self.is_assignable(arg.ty, expected) {
                    self.error(arg.span, format!("argument type {:?} does not match {expected:?}", arg.ty));
                }
            }
            return hir::Expr {
                kind: hir::ExprKind::NewWithConstructor { class: class_idx, defaults, func: ctor, args: call_args },
                ty: Ty::Instance(class_idx),
                span,
            };
        }
        // param_tys[0] is `this`.
        let max = param_tys.len() - 1;
        // Extra arguments beyond the declared parameters are evaluated for
        // their effects and then ignored, as in JS.
        let (args, extra_args) = if args.len() > max
            && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
        {
            args.split_at(max)
        } else {
            (args, &args[0..0])
        };
        let min = max.saturating_sub(optional_count);
        // A missing `any` parameter reads as `undefined` (JS passes fewer arguments freely).
        let padded: Vec<ast::Expr>;
        let args = if args.len() < min
            && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
            && param_tys[1 + args.len()..=min].iter().all(|ty| *ty == Ty::Any)
        {
            let mut extended = args.to_vec();
            while extended.len() < min {
                extended.push(ast::Expr::Unary {
                    op: ast::UnaryOp::Void,
                    operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                    span,
                });
            }
            padded = extended;
            &padded[..]
        } else {
            args
        };
        let mut call_args: Vec<hir::Expr> = args.iter().map(|a| self.check_expr(cx, a)).collect();
        if call_args.len() < min || call_args.len() > max {
            self.error(
                span,
                format!(
                    "`{name}` constructor expects {min} to {max} argument(s), found {}",
                    call_args.len()
                ),
            );
            return self.poison(span);
        }
        let sig = Signature {
            name: sig_name,
            params: param_tys[1..].to_vec(),
            return_ty: Ty::Void,
            optional_count,
            rest_param: None,
            is_generator: false,
            is_static: false,
        };
        self.pad_default_args(&mut call_args, &sig, span);
        for (arg, &expected) in call_args.iter_mut().zip(&param_tys[1..]) {
            coerce_numeric_literal(arg, expected);
            if !expected.is_error() && !arg.ty.is_error() && !self.is_assignable(arg.ty, expected) {
                self.error(
                    arg.span,
                    format!("argument type {:?} does not match {expected:?}", arg.ty),
                );
            }
        }
        let mut result = hir::Expr {
            kind: hir::ExprKind::NewWithConstructor { class: class_idx, defaults, func: ctor, args: call_args },
            ty: Ty::Instance(class_idx),
            span,
        };
        for extra in extra_args {
            let effect = self.check_expr(cx, extra);
            let ty = result.ty;
            result = hir::Expr {
                kind: hir::ExprKind::Sequence { effect: Box::new(effect), value: Box::new(result) },
                ty,
                span,
            };
        }
        result
    }

    /// Lower `new Error(message, { cause })` (and its subclasses) to a `New`
    /// node, bridging the ECMAScript `(message, options)` constructor shape onto
    /// Zun's one-argument-per-field object model. The arguments are rebuilt in
    /// slot order: `message` from arg0 (empty string when omitted), `cause` from
    /// the options object's `.cause` (`undefined` when no options object), and
    /// every other field (e.g. `stackFrames`) from its declared initializer.
    ///
    /// The `cause` field is `any`, carried by the frozen-layout `any`-field
    /// side table; absent and present are distinguished by `undefined` vs. the
    /// read value. Construction allocates and reads a dynamic object, so it runs
    /// on the Tier-0 interpreter (the `any` field keeps the method off the JIT
    /// allowlist), matching the throwing/dynamic nature of Error values.
    fn check_error_new(
        &mut self,
        cx: &mut FunctionCx,
        class_idx: u32,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        // Arguments beyond the message and the options object are ignored.
        let args = &args[..args.len().min(2)];
        // `message`: arg0, defaulting to the empty string (JS `new Error()`).
        let mut message = match args.first() {
            Some(arg) => self.check_expr(cx, arg),
            None => hir::Expr { kind: hir::ExprKind::String(String::new()), ty: Ty::String, span },
        };
        coerce_numeric_literal(&mut message, Ty::String);
        if !message.ty.is_error() && !self.is_assignable(message.ty, Ty::String) {
            self.error(message.span, format!("`{name}` message must be a string"));
        }
        // The options object: arg1, or an empty dynamic object so the `.cause`
        // read yields `undefined`.
        let options = match args.get(1) {
            Some(arg) => self.check_expr(cx, arg),
            None => hir::Expr {
                kind: hir::ExprKind::DynamicObjectLiteral { entries: Vec::new() },
                ty: Ty::DynamicObject,
                span,
            },
        };
        if !matches!(options.ty, Ty::DynamicObject | Ty::Any) && !options.ty.is_error() {
            self.error(options.span, format!("`{name}` options argument must be an object"));
        }
        // `cause` = `options.cause` (a missing key reads back as `undefined`).
        let cause = hir::Expr {
            kind: hir::ExprKind::DynamicGet { object: Box::new(options), name: "cause".to_string() },
            ty: Ty::Any,
            span,
        };
        // Assemble the constructor arguments in field-slot order.
        let fields = self.full_fields(class_idx);
        let initializers = self.full_initializers(class_idx);
        let mut message = Some(message);
        let mut cause = Some(cause);
        let mut new_args: Vec<hir::Expr> = Vec::with_capacity(fields.len());
        for (slot, (fname, _, _, _)) in fields.iter().enumerate() {
            let arg = match fname.as_str() {
                "message" => message.take().unwrap_or_else(|| self.poison(span)),
                "cause" => cause.take().unwrap_or_else(|| self.poison(span)),
                _ => match initializers.get(slot).and_then(|init| init.as_ref()) {
                    Some(init) => {
                        let init = init.clone();
                        self.check_expr(cx, &init)
                    }
                    None => {
                        self.error(span, format!("`{name}` field `{fname}` has no initializer"));
                        self.poison(span)
                    }
                },
            };
            new_args.push(arg);
        }
        hir::Expr {
            kind: hir::ExprKind::New { class: class_idx, args: new_args },
            ty: Ty::Instance(class_idx),
            span,
        }
    }

    /// `new ArrayBuffer(length)` or `new ArrayBuffer(length, { maxByteLength })`.
    /// Without the options bag the buffer is fixed-length (`bufferNew`); with a
    /// `maxByteLength` it is resizable (`bufferResizableNew`, ES2024).
    fn check_array_buffer_new(
        &mut self,
        cx: &mut FunctionCx,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        if args.is_empty() || args.len() > 2 {
            self.error(span, "`ArrayBuffer` expects a length and an optional options object");
            return self.poison(span);
        }
        let mut length = self.check_expr(cx, &args[0]);
        coerce_numeric_literal(&mut length, Ty::INT);
        if !length.ty.is_int() {
            self.error(length.span, "ArrayBuffer length must be an integer");
        }
        // The optional second argument is `{ maxByteLength: <expr> }`, which
        // marks the buffer resizable. Only `maxByteLength` is read; the bag is
        // otherwise an object literal like any other.
        let max = if let Some(options) = args.get(1) {
            match options {
                ast::Expr::ObjectLiteral { properties, .. } => {
                    let found = properties.iter().find_map(|member| match member {
                        ast::ObjectMember::Property(property)
                            if property.kind == ast::PropertyKind::Init =>
                        {
                            let matches = match &property.key {
                                ast::PropertyKey::Ident(name) => self.text(name) == "maxByteLength",
                                ast::PropertyKey::String { value, .. } => value == "maxByteLength",
                                _ => false,
                            };
                            matches.then_some(property)
                        }
                        _ => None,
                    });
                    match found {
                        Some(property) => {
                            let mut value = self.check_expr(cx, &property.value);
                            coerce_numeric_literal(&mut value, Ty::INT);
                            if !value.ty.is_int() {
                                self.error(value.span, "`maxByteLength` must be an integer");
                            }
                            Some(value)
                        }
                        None => {
                            self.error(
                                options.span(),
                                "`ArrayBuffer` options object must have a `maxByteLength`",
                            );
                            return self.poison(span);
                        }
                    }
                }
                other => {
                    self.error(other.span(), "`ArrayBuffer` second argument must be an options object");
                    return self.poison(span);
                }
            }
        } else {
            None
        };
        let (native, call_args) = match max {
            Some(max) => ("bufferResizableNew", vec![length, max]),
            None => ("bufferNew", vec![length]),
        };
        let Some((id, _)) = zun_natives::resolve("buffer", native) else {
            self.error(span, "missing ArrayBuffer intrinsic");
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: call_args },
            ty: Ty::Buffer,
            span,
        }
    }

    /// Resolve a `collections`-module native, reporting a missing intrinsic.
    fn resolve_collection(&mut self, span: Span, name: &str) -> Option<u32> {
        match zun_natives::resolve("collections", name) {
            Some((id, _)) => Some(id),
            None => {
                self.error(span, format!("missing collections intrinsic `{name}`"));
                None
            }
        }
    }

    /// Lower `str.replace(search, fn)` / `str.replaceAll(search, fn)`.
    fn replace_with_function(
        &mut self,
        span: Span,
        search: hir::Expr,
        receiver: hir::Expr,
        callback: hir::Expr,
        all: bool,
    ) -> hir::Expr {
        let Some((id, _)) = zun_natives::resolve("collections", "replaceFn") else {
            self.error(span, "missing collection intrinsic `replaceFn`");
            return self.poison(span);
        };
        let flag = hir::Expr { kind: hir::ExprKind::Boolean(all), ty: Ty::Boolean, span };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: vec![search, receiver, callback, flag] },
            ty: Ty::String,
            span,
        }
    }

    /// Recognise a simple type guard over an untyped (`any`) local and return
    /// the local's name, slot and narrowed type: `typeof x === "string" |
    /// "number" | "boolean"` and `Array.isArray(x)`.
    fn type_guard_narrowing(
        &self,
        cx: &FunctionCx,
        cond: &ast::Expr,
    ) -> Option<(String, hir::LocalSlot, Ty)> {
        let (ident, narrowed) = match cond {
            ast::Expr::Binary { op: ast::BinaryOp::Eq, lhs, rhs, .. } => {
                let (typeof_side, literal_side) = match (&**lhs, &**rhs) {
                    (ast::Expr::Unary { .. }, ast::Expr::String { .. }) => (&**lhs, &**rhs),
                    (ast::Expr::String { .. }, ast::Expr::Unary { .. }) => (&**rhs, &**lhs),
                    _ => return None,
                };
                let ast::Expr::Unary { op: ast::UnaryOp::Typeof, operand, .. } = typeof_side else {
                    return None;
                };
                let ast::Expr::Ident(name) = &**operand else { return None };
                let ast::Expr::String { value, .. } = literal_side else { return None };
                let narrowed = match value.as_str() {
                    "string" => Ty::String,
                    "number" => Ty::F64,
                    "boolean" => Ty::Boolean,
                    _ => return None,
                };
                (*name, narrowed)
            }
            ast::Expr::StaticMethodCall { class, method, args, .. }
                if self.text(class) == "Array" && self.text(method) == "isArray" && args.len() == 1 =>
            {
                let ast::Expr::Ident(name) = &args[0] else { return None };
                (*name, Ty::Array)
            }
            ast::Expr::MethodCall { receiver, method, args, .. }
                if matches!(&**receiver, ast::Expr::Ident(class) if self.text(class) == "Array")
                    && self.text(method) == "isArray"
                    && args.len() == 1 =>
            {
                let ast::Expr::Ident(name) = &args[0] else { return None };
                (*name, Ty::Array)
            }
            _ => return None,
        };
        let name = self.text(&ident).to_string();
        let slot = cx.lookup(&name)?;
        if cx.locals[slot as usize] != Ty::Any || cx.boxed.contains(&slot) {
            return None;
        }
        Some((name, slot, narrowed))
    }

    /// Box a checked value to `Any` unless it already is (or is poisoned).
    /// Unbox a dynamic value flowing into a typed primitive position so the
    /// destination holds the primitive itself (not an `Any` carrier).
    fn unbox_to(value: hir::Expr, target: Ty) -> hir::Expr {
        if value.ty == Ty::Any && matches!(target, Ty::F64 | Ty::Boolean | Ty::String) {
            let span = value.span;
            return hir::Expr {
                kind: hir::ExprKind::Convert { value: Box::new(value) },
                ty: target,
                span,
            };
        }
        value
    }

    fn box_to_any(value: hir::Expr) -> hir::Expr {
        if value.ty == Ty::Any || value.ty.is_error() {
            value
        } else {
            let span = value.span;
            hir::Expr {
                kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                ty: Ty::Any,
                span,
            }
        }
    }

    /// Whether a read of the module-scope local `name` (in slot `slot`) must go
    /// through the realm store because a function body assigns that global.
    fn reads_through_global(&self, cx: &FunctionCx, name: &str, slot: hir::LocalSlot) -> bool {
        cx.is_module_scope
            && self.fn_mutated_globals.contains(name)
            && cx.scopes.get(1).and_then(|s| s.get(name).copied()) == Some(slot)
            && (matches!(
                cx.locals[slot as usize],
                Ty::Any | Ty::F64 | Ty::Boolean | Ty::String
            ) || (cx.locals[slot as usize] == Ty::Function(hir::DYNAMIC_FUNCTION)
                && self.reassigned_fn_names.contains(name)))
    }

    /// A `Ty::String` HIR literal.
    fn string_literal(text: String, span: Span) -> hir::Expr {
        hir::Expr { kind: hir::ExprKind::String(text), ty: Ty::String, span }
    }

    /// `globalSet(name, box(value), false)` — store a module global.
    fn global_set_expr(&self, name: &str, value: hir::Expr, span: Span) -> hir::Expr {
        let (id, _) = zun_natives::resolve("collections", "globalSet")
            .expect("globalSet intrinsic must exist");
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![
                    Self::string_literal(name.to_string(), span),
                    Self::box_to_any(value),
                    hir::Expr { kind: hir::ExprKind::Boolean(false), ty: Ty::Boolean, span },
                ],
            },
            ty: Ty::Any,
            span,
        }
    }

    /// `globalGet(name)` — read a module global (dynamic).
    fn global_get_expr(&self, name: &str, span: Span) -> hir::Expr {
        let (id, _) = zun_natives::resolve("collections", "globalGet")
            .expect("globalGet intrinsic must exist");
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![Self::string_literal(name.to_string(), span)],
            },
            ty: Ty::Any,
            span,
        }
    }

    /// Lower an object literal's members (data properties, method shorthand,
    /// computed-symbol methods, getters/setters, and spread) into
    /// [`hir::ObjectEntry`] entries evaluated in source order.
    fn lower_object_members(
        &mut self,
        cx: &mut FunctionCx,
        members: &[ast::ObjectMember],
        span: Span,
    ) -> Vec<hir::ObjectEntry> {
        let mut entries = Vec::with_capacity(members.len());
        for member in members {
            match member {
                ast::ObjectMember::Spread { expr, .. } => {
                    // `{ ...o }` → objectAssign(obj, o): copy the source's own
                    // enumerable string keys (DynamicObject) or declared
                    // instance fields (typed object) into the new object.
                    let source = Self::box_to_any(self.check_expr(cx, expr));
                    let Some(builtin) = self.resolve_collection(span, "objectAssign") else {
                        continue;
                    };
                    entries.push(hir::ObjectEntry::Install { builtin, args: vec![source] });
                }
                ast::ObjectMember::Property(property) => {
                    self.lower_object_property(cx, property, span, &mut entries);
                }
            }
        }
        entries
    }

    /// A non-arrow `function` expression that mentions `this` and is stored as a
    /// property behaves like a method: arm `closure_takes_this` so it receives
    /// the receiver as its hidden first parameter.
    fn mark_this_closure(&mut self, value: &ast::Expr) {
        if let ast::Expr::Function(function) = value {
            if !function.is_arrow {
                let body = &self.source[function.span.start as usize..function.span.end as usize];
                self.closure_takes_this = body.contains("this");
            }
        }
    }

    fn lower_object_property(
        &mut self,
        cx: &mut FunctionCx,
        property: &ast::ObjectProperty,
        span: Span,
        entries: &mut Vec<hir::ObjectEntry>,
    ) {
        // A statically-known string key, or `None` for a computed key.
        let named_key = match &property.key {
            ast::PropertyKey::Ident(name) => Some(self.text(name).to_string()),
            ast::PropertyKey::String { value, .. } => Some(value.clone()),
            ast::PropertyKey::Number { value, .. } => Some(value.to_string()),
            ast::PropertyKey::Computed(_) => None,
        };
        match property.kind {
            ast::PropertyKind::Init => {
                // A `function` expression that mentions `this` and is stored as
                // a data property behaves like a method: it receives the object
                // as its hidden first parameter.
                if named_key.is_some() {
                    if let ast::Expr::Function(function) = &property.value {
                        if !function.is_arrow {
                            let body = &self.source[function.span.start as usize..function.span.end as usize];
                            self.closure_takes_this = body.contains("this");
                        }
                    }
                }
                let checked = self.check_expr(cx, &property.value);
                self.closure_takes_this = false;
                // A function-valued data property (`{ f: () => 1 }`) lives in the
                // object's function slot, exactly like method shorthand, so
                // `obj.f()` and the `valueOf`/`toString` coercion hooks find it.
                if let (Ty::Function(_), Some(name)) = (checked.ty, named_key.clone()) {
                    if let Some(builtin) = self.resolve_collection(span, "dynamicSetFunction") {
                        entries.push(hir::ObjectEntry::Install {
                            builtin,
                            args: vec![Self::string_literal(name, span), checked],
                        });
                        return;
                    }
                }
                // `{ [Symbol.iterator]: function () {} }`: a function value under a
                // computed key lives in the function slot, like method shorthand.
                if let (Ty::Function(_), ast::PropertyKey::Computed(key_expr)) =
                    (checked.ty, &property.key)
                {
                    let key = self.check_expr(cx, key_expr);
                    let slot_builtin = if key.ty == Ty::Symbol || key.ty == Ty::Any {
                        self.resolve_collection(span, "dynamicSetSymbolFunction")
                            .map(|builtin| (builtin, Self::box_to_any(key)))
                    } else if key.ty == Ty::String {
                        self.resolve_collection(span, "dynamicSetFunction")
                            .map(|builtin| (builtin, key))
                    } else {
                        None
                    };
                    if let Some((builtin, key)) = slot_builtin {
                        entries.push(hir::ObjectEntry::Install { builtin, args: vec![key, checked] });
                        return;
                    }
                    self.error(span, "a computed method key must be a string or a Symbol");
                    return;
                }
                let value = Self::box_to_any(checked);
                let key = match &property.key {
                    ast::PropertyKey::Computed(key) => {
                        hir::ObjectKey::Computed(self.check_expr(cx, key))
                    }
                    _ => hir::ObjectKey::Named(named_key.expect("named key")),
                };
                // A data property that replaces an accessor installed earlier in the
                // same literal, or a string computed key (which never sets the
                // prototype, even for `["__proto__"]`), is a definition: route it
                // through `objectDefineProperty` instead of an assignment.
                let define = self.resolve_collection(span, "objectDefineProperty");
                let replaces_accessor = match (&key, define) {
                    (hir::ObjectKey::Named(name), Some(define_id)) => entries.iter().any(|entry| {
                        matches!(entry, hir::ObjectEntry::Install { builtin, args }
                            if *builtin == define_id
                                && matches!(args.first(), Some(hir::Expr { kind: hir::ExprKind::String(existing), .. }) if existing == name))
                    }),
                    _ => false,
                };
                let computed_string = matches!(&key, hir::ObjectKey::Computed(k) if k.ty == Ty::String);
                if let (true, Some(define_id)) = (replaces_accessor || computed_string, define) {
                    let key_expr = match key {
                        hir::ObjectKey::Named(name) => Self::string_literal(name, span),
                        hir::ObjectKey::Computed(k) => k,
                    };
                    {
                        let descriptor = hir::Expr {
                            kind: hir::ExprKind::DynamicObjectLiteral {
                                entries: vec![
                                    hir::ObjectEntry::Data {
                                        key: hir::ObjectKey::Named("value".to_string()),
                                        value,
                                    },
                                    decorator_context_entry("writable", hir::ExprKind::Boolean(true), Ty::Boolean, span),
                                    decorator_context_entry("enumerable", hir::ExprKind::Boolean(true), Ty::Boolean, span),
                                    decorator_context_entry("configurable", hir::ExprKind::Boolean(true), Ty::Boolean, span),
                                ],
                            },
                            ty: Ty::DynamicObject,
                            span,
                        };
                        entries.push(hir::ObjectEntry::Install { builtin: define_id, args: vec![key_expr, descriptor] });
                        return;
                    }
                }
                entries.push(hir::ObjectEntry::Data { key, value });
            }
            ast::PropertyKind::Method => {
                // A method that mentions `this` receives the object as a hidden
                // first parameter (see `closure_takes_this`).
                if let ast::Expr::Function(function) = &property.value {
                    let body = &self.source[function.span.start as usize..function.span.end as usize];
                    self.closure_takes_this = body.contains("this");
                }
                let func = self.check_expr(cx, &property.value);
                self.closure_takes_this = false;
                match &property.key {
                    ast::PropertyKey::Computed(key) => {
                        let key = self.check_expr(cx, key);
                        // A `Symbol` computed key routes to the symbol-function
                        // slot (the enabler for user `[Symbol.iterator]` /
                        // `[Symbol.hasInstance]` methods); a string key uses the
                        // ordinary string function slot.
                        if key.ty == Ty::Symbol || key.ty == Ty::Any {
                            if let Some(builtin) =
                                self.resolve_collection(span, "dynamicSetSymbolFunction")
                            {
                                entries.push(hir::ObjectEntry::Install {
                                    builtin,
                                    args: vec![Self::box_to_any(key), func],
                                });
                            }
                        } else if key.ty == Ty::String {
                            if let Some(builtin) =
                                self.resolve_collection(span, "dynamicSetFunction")
                            {
                                entries.push(hir::ObjectEntry::Install {
                                    builtin,
                                    args: vec![key, func],
                                });
                            }
                        } else {
                            self.error(
                                span,
                                "a computed method key must be a string or a Symbol",
                            );
                        }
                    }
                    _ => {
                        let name = named_key.expect("named key");
                        if let Some(builtin) = self.resolve_collection(span, "dynamicSetFunction") {
                            entries.push(hir::ObjectEntry::Install {
                                builtin,
                                args: vec![Self::string_literal(name, span), func],
                            });
                        }
                    }
                }
            }
            ast::PropertyKind::Get | ast::PropertyKind::Set => {
                let Some(name) = named_key else {
                    self.error(
                        span,
                        "computed getter/setter keys in object literals are not supported yet",
                    );
                    return;
                };
                if let ast::Expr::Function(function) = &property.value {
                    let body = &self.source[function.span.start as usize..function.span.end as usize];
                    self.closure_takes_this = body.contains("this");
                }
                let func = self.check_expr(cx, &property.value);
                self.closure_takes_this = false;
                let accessor_slot = if property.kind == ast::PropertyKind::Get { "get" } else { "set" };
                // Build the accessor descriptor `{ get|set: fn }` as a dynamic
                // object (the function lives in its string function slot), then
                // install it with objectDefineProperty — the same model
                // `Object.defineProperty` uses, so the named-key accessor is
                // invoked on `obj.k` read/write.
                let Some(set_fn) = self.resolve_collection(span, "dynamicSetFunction") else {
                    return;
                };
                let descriptor = hir::Expr {
                    kind: hir::ExprKind::DynamicObjectLiteral {
                        entries: vec![
                            hir::ObjectEntry::Install {
                                builtin: set_fn,
                                args: vec![
                                    Self::string_literal(accessor_slot.to_string(), span),
                                    func,
                                ],
                            },
                            // Literal accessors are enumerable and configurable.
                            decorator_context_entry(
                                "enumerable",
                                hir::ExprKind::Boolean(true),
                                Ty::Boolean,
                                span,
                            ),
                            decorator_context_entry(
                                "configurable",
                                hir::ExprKind::Boolean(true),
                                Ty::Boolean,
                                span,
                            ),
                        ],
                    },
                    ty: Ty::DynamicObject,
                    span,
                };
                let Some(define) = self.resolve_collection(span, "objectDefineProperty") else {
                    return;
                };
                entries.push(hir::ObjectEntry::Install {
                    builtin: define,
                    args: vec![Self::string_literal(name, span), descriptor],
                });
            }
        }
    }

    /// Lower an array literal, choosing the fixed-length store path when there
    /// is no spread, and the growable builder path when a spread is present.
    fn lower_array_literal(
        &mut self,
        cx: &mut FunctionCx,
        elements: &[ast::ArrayElement],
        span: Span,
    ) -> hir::Expr {
        let has_spread = elements
            .iter()
            .any(|element| matches!(element, ast::ArrayElement::Spread(_)));
        if !has_spread {
            // Fixed-length path: allocate N slots and store each present
            // element at its index; a hole leaves the index as `undefined`.
            let elements = elements
                .iter()
                .map(|element| match element {
                    ast::ArrayElement::Expr(expr) => Some(self.check_expr(cx, expr)),
                    ast::ArrayElement::Hole => None,
                    ast::ArrayElement::Spread(_) => unreachable!("spread handled above"),
                })
                .collect();
            return hir::Expr {
                kind: hir::ExprKind::ArrayLiteral { elements },
                ty: Ty::Array,
                span,
            };
        }
        let mut parts = Vec::with_capacity(elements.len());
        for element in elements {
            match element {
                ast::ArrayElement::Expr(expr) => {
                    parts.push(hir::ArrayPart::Element(self.check_expr(cx, expr)));
                }
                ast::ArrayElement::Hole => parts.push(hir::ArrayPart::Hole),
                ast::ArrayElement::Spread(expr) => {
                    let source = Self::box_to_any(self.check_expr(cx, expr));
                    let Some((builtin, _)) = zun_natives::resolve("array", "arraySpreadAppend")
                    else {
                        self.error(span, "missing Array intrinsic `arraySpreadAppend`");
                        continue;
                    };
                    parts.push(hir::ArrayPart::Spread { builtin, source });
                }
            }
        }
        hir::Expr {
            kind: hir::ExprKind::ArraySpreadLiteral { parts },
            ty: Ty::Array,
            span,
        }
    }

    /// Lower `key in object` to the dynamic `has` mechanism. A Proxy receiver
    /// routes through its `has` trap (`proxyHas`); a plain object or `any`
    /// receiver uses `reflectHas`. The key must be a string (symbol keys are
    /// deferred).
    fn check_in_operator(
        &mut self,
        cx: &mut FunctionCx,
        key: &ast::Expr,
        object: &ast::Expr,
        span: Span,
    ) -> hir::Expr {
        // `"name" in Map.prototype` for well-known built-in prototypes: a
        // prototype method is present, a static-only method is absent.
        if let (ast::Expr::String { value, .. }, ast::Expr::Field { object: proto_owner, field: proto_field, .. }) =
            (key, object)
        {
            if self.text(proto_field) == "prototype" {
                if let ast::Expr::Ident(global) = proto_owner.as_ref() {
                    let global_name = self.text(global).to_string();
                    if cx.lookup(&global_name).is_none()
                        && !self.classes_by_name.contains_key(global_name.as_str())
                    {
                        let on_proto =
                            builtin_function_length(&[global_name.as_str(), "prototype", value.as_str()]).is_some();
                        let on_static =
                            builtin_function_length(&[global_name.as_str(), value.as_str()]).is_some();
                        if on_proto || on_static {
                            return hir::Expr { kind: hir::ExprKind::Boolean(on_proto), ty: Ty::Boolean, span };
                        }
                    }
                }
            }
        }
        let key = self.check_expr(cx, key);
        let object = self.check_expr(cx, object);
        // A non-string key (e.g. an array index) is converted with ToString,
        // matching ECMAScript property-key coercion.
        let dynamic_receiver = matches!(object.ty, Ty::DynamicObject | Ty::Any);
        if key.ty == Ty::Symbol && dynamic_receiver {
            // A Symbol key is looked up by identity, never stringified.
            if let Some((id, _)) = zun_natives::resolve("collections", "symbolHas") {
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![Self::box_to_any(object), Self::box_to_any(key)] },
                    ty: Ty::Boolean,
                    span,
                };
            }
        }
        let key = if key.ty != Ty::String && !key.ty.is_error() {
            self.to_string_for_parse(key)
        } else {
            key
        };
        if object.ty == Ty::String {
            // A string reaching `in` came from `Object(str)`: index / length keys.
            if let Some((id, _)) = zun_natives::resolve("string", "stringHasProperty") {
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![object, key] },
                    ty: Ty::Boolean,
                    span,
                };
            }
        }
        let native = match object.ty {
            Ty::Array => "arrayHas",
            Ty::Proxy => "proxyHas",
            Ty::DynamicObject | Ty::Any => "reflectHas",
            Ty::Map | Ty::Set | Ty::WeakMap | Ty::WeakSet => {
                let Some((id, _)) = zun_natives::resolve("collections", "collectionHas") else {
                    self.error(span, "missing `collectionHas` intrinsic");
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![object, key] },
                    ty: Ty::Boolean,
                    span,
                };
            }
            Ty::Error => return self.poison(span),
            other => {
                self.error(
                    object.span,
                    format!("`in` requires an object receiver, found {other:?}"),
                );
                return self.poison(span);
            }
        };
        let Some((id, _)) = zun_natives::resolve("collections", native) else {
            self.error(span, format!("missing `{native}` intrinsic"));
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: vec![object, key] },
            ty: Ty::Boolean,
            span,
        }
    }

    /// Lower `delete obj.k` / `delete obj[k]` to the dynamic delete mechanism
    /// (`reflectDelete`, or a Proxy's `deleteProperty` trap via `proxyDelete`).
    /// A non-member operand is a semantic error. The configurable-property
    /// descriptor model is deferred, so a successful delete always yields
    /// `true`.
    fn check_delete_operator(
        &mut self,
        cx: &mut FunctionCx,
        operand: &ast::Expr,
        span: Span,
    ) -> hir::Expr {
        // `delete (arr as any)[k]`: an `as any` cast around a plain identifier
        // does not change which object is deleted from.
        if let ast::Expr::Index { object, index, span: index_span } = operand {
            if let ast::Expr::Cast { expr: inner, ty, .. } = object.as_ref() {
                if self.text(&ty.name) == "any" && matches!(inner.as_ref(), ast::Expr::Ident(_)) {
                    let peeled = ast::Expr::Index { object: inner.clone(), index: index.clone(), span: *index_span };
                    return self.check_delete_operator(cx, &peeled, span);
                }
            }
        }
        // `delete a?.b` / `delete a?.[k]` / `delete a?.b.c`: a nullish guard
        // short-circuits to `true`; otherwise delete through the chain root.
        {
            let guarded: Option<(&ast::Expr, ast::Expr)> = match operand {
                ast::Expr::OptionalField { object, field, .. } => Some((
                    object.as_ref(),
                    ast::Expr::Field { object: Box::new(ast::Expr::ChainRoot { depth: 0, span }), field: *field, span },
                )),
                ast::Expr::OptionalIndex { object, index, .. } => Some((
                    object.as_ref(),
                    ast::Expr::Index { object: Box::new(ast::Expr::ChainRoot { depth: 0, span }), index: index.clone(), span },
                )),
                ast::Expr::OptionalChain { object, body, .. } => Some((object.as_ref(), body.as_ref().clone())),
                _ => None,
            };
            if let Some((guard_ast, body)) = guarded {
                let guard = self.check_expr(cx, guard_ast);
                let (slot, tmp) = self.synthetic_local(cx, if matches!(guard.ty, Ty::Any | Ty::DynamicObject) { Ty::Any } else { guard.ty });
                let tmp_expr = ast::Expr::Ident(tmp);
                self.chain_roots.push(tmp);
                let delete = ast::Expr::Unary { op: ast::UnaryOp::Delete, operand: Box::new(body), span };
                let cond = if matches!(guard.ty, Ty::Any | Ty::DynamicObject) {
                    ast::Expr::Conditional {
                        condition: Box::new(ast::Expr::Binary {
                            op: ast::BinaryOp::LooseEq,
                            lhs: Box::new(tmp_expr),
                            rhs: Box::new(ast::Expr::Null { span }),
                            span,
                        }),
                        when_true: Box::new(ast::Expr::Bool { value: true, span }),
                        when_false: Box::new(delete),
                        span,
                    }
                } else {
                    delete
                };
                let value = self.check_expr(cx, &cond);
                self.chain_roots.pop();
                let ty = value.ty;
                let bound = if matches!(guard.ty, Ty::Any | Ty::DynamicObject) { Self::box_to_any(guard) } else { guard };
                return hir::Expr {
                    kind: hir::ExprKind::Sequence {
                        effect: Box::new(hir::Expr {
                            kind: hir::ExprKind::Assign { slot, value: Box::new(bound) },
                            ty: Ty::Any,
                            span,
                        }),
                        value: Box::new(value),
                    },
                    ty,
                    span,
                };
            }
        }
        // `delete arr[i]` on a typed array leaves a hole.
        if let ast::Expr::Index { object, index, .. } = operand {
            if matches!(object.as_ref(), ast::Expr::Ident(_)) {
                let probe = self.check_expr(cx, object);
                if probe.ty == Ty::Array {
                    let mut key = self.check_expr(cx, index);
                    if !key.ty.is_error() && matches!(key.ty, Ty::Any | Ty::F64) || key.ty.is_int() {
                        key = Self::box_to_any(key);
                    }
                    if let Some((id, _)) = zun_natives::resolve("collections", "arrayDeleteIndex") {
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: vec![probe, key] },
                            ty: Ty::Boolean,
                            span,
                        };
                    }
                }
            }
        }
        let (object_ast, key) = match operand {
            ast::Expr::Field { object, field, .. } => {
                let key = hir::Expr {
                    kind: hir::ExprKind::String(self.text(field).to_string()),
                    ty: Ty::String,
                    span: field.span,
                };
                (object.as_ref(), key)
            }
            ast::Expr::Index { object, index, .. } => {
                let key = self.check_expr(cx, index);
                let key = if matches!(key.ty, Ty::Any | Ty::F64) || key.ty.is_int() { self.to_string_for_parse(key) } else { key };
                // A Symbol key rides the dynamic lane (boxed) to `deleteStrict`.
                let key = if key.ty == Ty::Symbol { Self::box_to_any(key) } else { key };
                if !key.ty.is_error() && key.ty != Ty::String && key.ty != Ty::Any {
                    self.error(key.span, "`delete obj[k]` requires a string key");
                }
                (object.as_ref(), key)
            }
            _ => {
                // `delete <non-reference>` evaluates to `true`.
                return hir::Expr {
                    kind: hir::ExprKind::Boolean(true),
                    ty: Ty::Boolean,
                    span,
                };
            }
        };
        let object = self.check_expr(cx, object_ast);
        let native = match object.ty {
            Ty::Proxy => "proxyDelete",
            // Modules are strict: a refused delete throws a TypeError.
            Ty::DynamicObject | Ty::Any => "deleteStrict",
            Ty::Error => return self.poison(span),
            other => {
                self.error(
                    object.span,
                    format!("`delete` requires an object receiver, found {other:?}"),
                );
                return self.poison(span);
            }
        };
        let Some((id, _)) = zun_natives::resolve("collections", native) else {
            self.error(span, format!("missing `{native}` intrinsic"));
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: vec![object, key] },
            ty: Ty::Boolean,
            span,
        }
    }

    /// Lower `typeof x`. A statically-typed operand folds to a constant JS type
    /// string (every non-`any` value has a known type in Zun, so the fold is
    /// exact). An `any` operand has no static type, so it is lowered to the
    /// `typeofAny` intrinsic, which inspects the runtime tag/heap value.
    fn check_typeof_operator(
        &mut self,
        cx: &mut FunctionCx,
        operand: &ast::Expr,
        span: Span,
    ) -> hir::Expr {
        // `typeof Map.groupBy` / `typeof Math.max`: well-known built-in functions.
        {
            let mut parts: Vec<String> = Vec::new();
            let mut cursor = operand;
            let mut ok = true;
            loop {
                match cursor {
                    ast::Expr::Field { object: inner, field: f, .. } => {
                        parts.push(self.text(f).to_string());
                        cursor = inner.as_ref();
                    }
                    ast::Expr::Ident(root) => {
                        parts.push(self.text(root).to_string());
                        break;
                    }
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && parts.len() >= 2 {
                parts.reverse();
                let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
                let known = builtin_function_length(&refs).is_some()
                    || (refs.len() == 2 && refs[0] == "Math" && math_function_length(refs[1]).is_some());
                if known
                    && !self.classes_by_name.contains_key(refs[0])
                    && cx.lookup(refs[0]).is_none()
                {
                    return hir::Expr {
                        kind: hir::ExprKind::String("function".to_string()),
                        ty: Ty::String,
                        span,
                    };
                }
            }
        }
        let mut operand = self.check_expr(cx, operand);
        // `typeof undeclared` is "undefined", never a ReferenceError.
        if let hir::ExprKind::CallBuiltin { id, args } = &mut operand.kind {
            if let (Some((read, _)), Some((get, _))) = (
                zun_natives::resolve("collections", "globalRead"),
                zun_natives::resolve("collections", "globalGet"),
            ) {
                if *id == read {
                    *id = get;
                    let _ = args;
                }
            }
        }
        let type_string = match operand.ty {
            Ty::Error => return self.poison(span),
            Ty::Int(_) | Ty::Float(_) => "number",
            Ty::String => "string",
            Ty::Boolean => "boolean",
            Ty::BigInt => "bigint",
            Ty::Symbol => "symbol",
            Ty::Function(_) => "function",
            Ty::Void => "undefined",
            // An `any` operand carries no static type, so inspect its runtime
            // tag with the `typeofAny` intrinsic instead of folding.
            Ty::Any => {
                let Some((id, _)) = zun_natives::resolve_name("typeofAny") else {
                    self.error(span, "missing `typeofAny` intrinsic".to_string());
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![operand] },
                    ty: Ty::String,
                    span,
                };
            }
            // Arrays, maps, sets, proxies, instances, buffers, null → "object".
            _ => "object",
        };
        let folded = hir::Expr {
            kind: hir::ExprKind::String(type_string.to_string()),
            ty: Ty::String,
            span,
        };
        // The operand is still evaluated for its side effects (`typeof arr.push(1)`).
        if matches!(
            operand.kind,
            hir::ExprKind::Local(_)
                | hir::ExprKind::Number(_)
                | hir::ExprKind::String(_)
                | hir::ExprKind::Boolean(_)
                | hir::ExprKind::Null
        ) {
            return folded;
        }
        hir::Expr {
            kind: hir::ExprKind::Sequence { effect: Box::new(operand), value: Box::new(folded) },
            ty: Ty::String,
            span,
        }
    }

    /// Lower `x instanceof C`. A statically-known instance that is provably in
    /// `C`'s hierarchy folds to constant `true`, and a value whose static type
    /// can never be an instance folds to `false`. Otherwise — an `any`/dynamic
    /// operand, or a base-typed operand that may hold a `C` subclass at runtime
    /// — it lowers to the `instanceofClass` intrinsic, which walks the operand's
    /// actual class hierarchy against `C`'s metadata rid.
    ///
    /// Scope of the runtime walk: it matches typed class instances (including
    /// ones upcast through `any`/a base type) by their runtime `type_rid`. A
    /// plain dynamic object built with `Object.create` has no class identity, so
    /// it answers `false` (never a false positive); matching such objects
    /// against a class `prototype` stays deferred (needs a class↔prototype-object
    /// model that does not exist yet). `Symbol.hasInstance` customization IS wired
    /// when the right operand is a dynamic object carrying the method: such an
    /// operand is not a class name, so it falls through to the `instanceofDynamic`
    /// intrinsic below. Only `[Symbol.hasInstance]` on a typed class stays deferred
    /// (needs class computed-symbol-key members).
    /// Whether a tagged-template tag is the built-in `String.raw`.
    fn tag_is_string_raw(&self, tag: &ast::Expr) -> bool {
        let (object, field) = match tag {
            ast::Expr::Field { object, field, .. } => {
                let ast::Expr::Ident(name) = object.as_ref() else { return false };
                (self.text(name), self.text(field))
            }
            ast::Expr::StaticField { class, field, .. } => {
                (self.text(class), self.text(field))
            }
            _ => return false,
        };
        object == "String" && field == "raw" && !self.by_name.contains_key("String")
    }

    /// Lower a tagged template literal. Only `String.raw` is supported as the
    /// tag: it stringifies each substitution and joins it between the **raw**
    /// (un-escaped) literal chunks, so `` String.raw`a\n${1}b` `` is
    /// `"a\\n1b"`. User-defined tags need a strings-array-with-`.raw` plus a
    /// variadic dynamic dispatch and stay deferred.
    fn check_tagged_template(
        &mut self,
        cx: &mut FunctionCx,
        tag: &ast::Expr,
        raw: &[String],
        exprs: &[Box<ast::Expr>],
        span: Span,
    ) -> hir::Expr {
        if !self.tag_is_string_raw(tag) {
            self.error(
                tag.span(),
                "only `String.raw` is supported as a template tag",
            );
            return self.poison(span);
        }
        let string_lit = |text: &str| hir::Expr {
            kind: hir::ExprKind::String(text.to_string()),
            ty: Ty::String,
            span,
        };
        // raw.len() == exprs.len() + 1 (the parser guarantees alignment).
        let mut result = string_lit(raw.first().map(String::as_str).unwrap_or(""));
        for (index, expression) in exprs.iter().enumerate() {
            let value = self.check_expr(cx, expression);
            let next = if value.ty == Ty::F64 {
                match zun_natives::resolve("number", "toString") {
                    Some((id, _)) => hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![value] },
                        ty: Ty::String,
                        span,
                    },
                    None => {
                        self.error(span, "number template conversion is unavailable");
                        self.poison(span)
                    }
                }
            } else {
                if value.ty != Ty::String && !value.ty.is_error() {
                    self.error(
                        value.span,
                        "String.raw substitution must have type string or number",
                    );
                }
                value
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
            let chunk = raw.get(index + 1).map(String::as_str).unwrap_or("");
            result = hir::Expr {
                kind: hir::ExprKind::Binary {
                    op: ast::BinaryOp::Add,
                    lhs: Box::new(result),
                    rhs: Box::new(string_lit(chunk)),
                },
                ty: Ty::String,
                span,
            };
        }
        result
    }

    fn check_instanceof_operator(
        &mut self,
        cx: &mut FunctionCx,
        value: &ast::Expr,
        class: &ast::Expr,
        span: Span,
    ) -> hir::Expr {
        let value = self.check_expr(cx, value);
        let ast::Expr::Ident(class_name) = class else {
            self.error(class.span(), "right operand of `instanceof` must be a class name");
            return self.poison(span);
        };
        let name = self.text(class_name).to_string();
        if !self.classes_by_name.contains_key(&name)
            && matches!(
                name.as_str(),
                "Array" | "Map" | "Set" | "Promise" | "RegExp" | "Object" | "Function"
            )
            && !cx.scopes.iter().any(|s| s.contains_key(name.as_str()))
        {
            if value.ty.is_error() {
                return self.poison(span);
            }
            let Some((id, _)) = zun_natives::resolve("typeinfo", "instanceofBuiltin") else {
                self.error(span, "missing `instanceofBuiltin` intrinsic".to_string());
                return self.poison(span);
            };
            let value = Self::box_to_any(value);
            let ctor = hir::Expr { kind: hir::ExprKind::String(name), ty: Ty::String, span };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![value, ctor] },
                ty: Ty::Boolean,
                span,
            };
        }
        let Some(&target) = self.classes_by_name.get(&name) else {
            // Not a class name: the right operand may be a dynamic object
            // carrying a `[Symbol.hasInstance]` method. If the identifier
            // resolves to an `any`/dynamic value, dispatch through that method
            // at runtime (`C[Symbol.hasInstance](value)`); otherwise it is a
            // genuine unknown-class reference.
            let rhs = self.check_expr(cx, class);
            if matches!(rhs.ty, Ty::Any | Ty::DynamicObject) {
                let Some((id, _)) = zun_natives::resolve("typeinfo", "instanceofDynamic") else {
                    self.error(span, "missing `instanceofDynamic` intrinsic".to_string());
                    return self.poison(span);
                };
                // The operand reaches the native as `any` (the method's
                // parameter is `any`), so box a non-dynamic left operand.
                let value = if value.ty == Ty::Any {
                    value
                } else {
                    hir::Expr {
                        span: value.span,
                        kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                        ty: Ty::Any,
                    }
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![value, rhs] },
                    ty: Ty::Boolean,
                    span,
                };
            }
            if !rhs.ty.is_error() {
                self.error(class_name.span, format!("unknown class `{name}`"));
            }
            return self.poison(span);
        };
        // Decide between a constant fold and a runtime class-hierarchy walk.
        // `None` means "check at runtime": an upcast base-typed operand that may
        // hold a `target` subclass, or an `any`/dynamic operand with no static
        // class. (The old code folded these to an unsound constant `false`.)
        let folded = if self.classes_with_has_instance.contains(&target) {
            // A custom `[Symbol.hasInstance]` always decides membership, so the
            // answer can never be constant-folded — force the runtime path,
            // which consults the per-type symbol-method side table.
            None
        } else {
            match value.ty {
                Ty::Error => return self.poison(span),
                // A statically-known instance provably in `target`'s hierarchy.
                Ty::Instance(c) if self.class_is_a(c, target) => Some(true),
                Ty::Instance(_) | Ty::Any | Ty::DynamicObject => None,
                // A primitive value is never an instance of a class.
                _ => Some(false),
            }
        };
        let Some(result) = folded else {
            // Runtime check against `target`'s metadata TypeDef rid. The rid is
            // `target + 1`: `emit_types` in `zun_ilgen` emits one TypeDef per
            // class in order, so class index `i` owns rid `i + 1` (asserted
            // there).
            let Some((id, _)) = zun_natives::resolve_name("instanceofClass") else {
                self.error(span, "missing `instanceofClass` intrinsic".to_string());
                return self.poison(span);
            };
            let class_rid = hir::Expr {
                kind: hir::ExprKind::Number((target + 1) as f64),
                ty: Ty::Int(IntKind::U32),
                span,
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin { id, args: vec![value, class_rid] },
                ty: Ty::Boolean,
                span,
            };
        };
        hir::Expr {
            kind: hir::ExprKind::Boolean(result),
            ty: Ty::Boolean,
            span,
        }
    }

    /// Whether the class or one of its bases declares `[Symbol.iterator]`.
    fn class_has_iterator(&self, class_index: u32) -> bool {
        let mut current = Some(class_index);
        while let Some(c) = current {
            if self.classes_with_iterator.contains(&c) {
                return true;
            }
            current = self.classes.get(c as usize).and_then(|info| info.base);
        }
        false
    }

    /// Whether class `class_index` is `target` or a (transitive) subclass of it.
    fn class_is_a(&self, mut class_index: u32, target: u32) -> bool {
        loop {
            if class_index == target {
                return true;
            }
            match self.classes.get(class_index as usize).and_then(|c| c.base) {
                Some(base) => class_index = base,
                None => return false,
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
                // `a ?? b` yields `b`'s type when `a` is dynamic; any other
                // operand pairing coerces through the dynamic path (JS).
                if lhs.ty == Ty::Any {
                    rhs.ty
                } else {
                    Ty::Any
                }
            }
            And | Or => {
                // Boolean operands keep a boolean result; otherwise the
                // operator follows JS truthiness over arbitrary values and the
                // result is dynamic.
                if lhs.ty == Ty::Boolean && rhs.ty == Ty::Boolean {
                    Ty::Boolean
                } else {
                    Ty::Any
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
                } else if lhs.ty == Ty::Any || rhs.ty == Ty::Any {
                    // A dynamic operand arithmetically coerces at run time (JS).
                    Ty::Any
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
            // `**` on integers → `pow.i32`; `number ** number` is intercepted
            // and routed to `Math.pow` before reaching here, so a float operand
            // here is the unsupported `float32` case.
            Pow => {
                if lhs.ty.is_int() && same_numeric(lhs.ty, rhs.ty) {
                    lhs.ty
                } else {
                    self.error(
                        span,
                        format!(
                            "exponentiation requires matching int or number operands, found {:?} and {:?}",
                            lhs.ty, rhs.ty
                        ),
                    );
                    Ty::Error
                }
            }
            // Bitwise and shift operators. On integer operands they use the
            // native width/signedness (`shr` is arithmetic for signed, logical
            // for unsigned). On `number` (f64) they follow JS: coerce both
            // operands to int32 (uint32 for the `>>>` left operand), operate,
            // and widen the int result back to a `number` — all lowered in
            // ilgen, so the result type here is the operand's own numeric type.
            BitAnd | BitOr | BitXor | Shl | Shr | UShr => {
                let both_int = lhs.ty.is_int() && same_numeric(lhs.ty, rhs.ty);
                let both_number = lhs.ty == Ty::F64 && rhs.ty == Ty::F64;
                if both_int || both_number {
                    lhs.ty
                } else if lhs.ty == Ty::Any || rhs.ty == Ty::Any {
                    // A dynamic operand is coerced to int32/uint32 at run time (JS).
                    Ty::Any
                } else {
                    self.error(
                        span,
                        format!(
                            "bitwise/shift operators require matching int or number operands, found {:?} and {:?}",
                            lhs.ty, rhs.ty
                        ),
                    );
                    Ty::Error
                }
            }
            Lt | Le | Gt | Ge => {
                // A dynamic (`any`) operand compares at run time (JS relational
                // coercion); otherwise both sides must be the same number type.
                if same_numeric(lhs.ty, rhs.ty)
                    || lhs.ty == Ty::Any
                    || rhs.ty == Ty::Any
                    || (lhs.ty == Ty::String && rhs.ty == Ty::String)
                    // Mixed primitives compare numerically at run time (JS).
                    || (matches!(lhs.ty, Ty::String | Ty::BigInt | Ty::Boolean | Ty::Void) && rhs.ty == Ty::F64)
                    || (lhs.ty == Ty::F64 && matches!(rhs.ty, Ty::String | Ty::BigInt | Ty::Boolean | Ty::Void))
                {
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
            Eq | Ne | LooseEq | LooseNe => {
                // Any two function values are comparable by reference identity,
                // even with different signatures (distinct `Function(_)` types):
                // different functions are simply unequal. A dynamic (`any`)
                // operand makes the comparison run-time (JS allows comparing any
                // two values), so it is accepted against anything.
                let both_functions =
                    matches!(lhs.ty, Ty::Function(_)) && matches!(rhs.ty, Ty::Function(_));
                if lhs.ty == rhs.ty
                    || both_functions
                    || lhs.ty == Ty::Any
                    || rhs.ty == Ty::Any
                {
                    Ty::Boolean
                } else {
                    self.error(
                        span,
                        format!("cannot compare {:?} with {:?}", lhs.ty, rhs.ty),
                    );
                    Ty::Error
                }
            }
            // `in`/`instanceof` are intercepted and lowered before reaching
            // here; this arm only satisfies exhaustiveness.
            In | Instanceof => Ty::Boolean,
        }
    }

}

/// The `nodeBuffer` native name for a Buffer reader/writer method, e.g.
/// `readUInt16LE` -> `nodeBufferReadUInt16LE`.
/// Whether `method` is a `DataView` element accessor handled by
/// [`Checker::check_data_view_method`] (the only methods whose endianness flag
/// is optional). Keeps the interceptor's gate and its match in lockstep.
fn data_view_accessor(method: &str) -> bool {
    matches!(
        method,
        "getInt8"
            | "getUint8"
            | "getInt16"
            | "getUint16"
            | "getInt32"
            | "getUint32"
            | "getInt64"
            | "getUint64"
            | "getFloat32"
            | "getFloat64"
            | "setInt8"
            | "setUint8"
            | "setInt16"
            | "setUint16"
            | "setInt32"
            | "setUint32"
            | "setInt64"
            | "setUint64"
            | "setFloat32"
            | "setFloat64"
    )
}

fn buffer_accessor_native(method: &str) -> String {
    let mut chars = method.chars();
    let first = chars.next().map(|c| c.to_ascii_uppercase()).unwrap_or_default();
    format!("nodeBuffer{first}{}", chars.as_str())
}

/// Whether a function body has a `return <value>` (so an unannotated return
/// type infers to `any` rather than `void`). Recurses into control-flow blocks
/// but not into expressions, so a nested closure's returns are excluded.
fn block_returns_value(block: &ast::Block) -> bool {
    block.statements.iter().any(stmt_returns_value)
}

fn stmt_returns_value(stmt: &ast::Stmt) -> bool {
    match stmt {
        ast::Stmt::Return { value: Some(_), .. } => true,
        ast::Stmt::If { then_block, else_block, .. } => {
            block_returns_value(then_block)
                || else_block.as_ref().is_some_and(block_returns_value)
        }
        ast::Stmt::While { body, .. }
        | ast::Stmt::DoWhile { body, .. }
        | ast::Stmt::For { body, .. }
        | ast::Stmt::ForOf { body, .. }
        | ast::Stmt::ForIn { body, .. } => block_returns_value(body),
        ast::Stmt::Labeled { statement, .. } => stmt_returns_value(statement),
        ast::Stmt::Switch { cases, .. } => cases.iter().any(|c| block_returns_value(&c.body)),
        ast::Stmt::Try { body, catches, finally, .. } => {
            block_returns_value(body)
                || catches.iter().any(|clause| block_returns_value(&clause.body))
                || finally.as_ref().is_some_and(block_returns_value)
        }
        _ => false,
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
        Let { init, .. } | Destructure { init, .. } => count_yields_in_expr(init),
        Expr(expr) => count_yields_in_expr(expr),
        If { cond, then_block, else_block, .. } => count_yields_in_expr(cond) + count_yields_in_block(then_block) + else_block.as_ref().map_or(0, count_yields_in_block),
        While { cond, body, .. } => count_yields_in_expr(cond) + count_yields_in_block(body),
        DoWhile { body, cond, .. } => count_yields_in_block(body) + count_yields_in_expr(cond),
        For { init, cond, update, body, .. } => init.as_deref().map_or(0, count_yields_in_stmt) + cond.as_ref().map_or(0, count_yields_in_expr) + update.as_ref().map_or(0, count_yields_in_expr) + count_yields_in_block(body),
        ForOf { iterable, body, .. } => count_yields_in_expr(iterable) + count_yields_in_block(body),
        ForIn { object, body, .. } => count_yields_in_expr(object) + count_yields_in_block(body),
        Switch { discriminant, cases, .. } => count_yields_in_expr(discriminant) + cases.iter().map(|c| c.test.as_ref().map_or(0, count_yields_in_expr) + count_yields_in_block(&c.body)).sum::<usize>(),
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
        ImportCall { specifier, .. } => count_yields_in_expr(specifier),
        Assign { value, .. } | Await { operand: value, .. } | Unary { operand: value, .. } | Spread { expr: value, .. } | Cast { expr: value, .. } => count_yields_in_expr(value),
        Binary { lhs, rhs, .. } => count_yields_in_expr(lhs) + count_yields_in_expr(rhs),
        Conditional { condition, when_true, when_false, .. } => count_yields_in_expr(condition) + count_yields_in_expr(when_true) + count_yields_in_expr(when_false),
        Call { args, .. } | New { args, .. } => args.iter().map(count_yields_in_expr).sum(),
        ComputedCall { callee, args, .. } | NewDynamic { callee, args, .. } => count_yields_in_expr(callee) + args.iter().map(count_yields_in_expr).sum::<usize>(),
        MethodCall { receiver, args, .. } | OptionalMethodCall { object: receiver, args, .. } => count_yields_in_expr(receiver) + args.iter().map(count_yields_in_expr).sum::<usize>(),
        OptionalField { object, .. } | Field { object, .. } | PrivateField { object, .. } => count_yields_in_expr(object),
        OptionalIndex { object, index, .. } => count_yields_in_expr(object) + count_yields_in_expr(index),
        ChainRoot { .. } => 0,
        TempBind { value, body, .. } => count_yields_in_expr(value) + count_yields_in_expr(body),
        OptionalChain { object, body, .. } => count_yields_in_expr(object) + count_yields_in_expr(body),
        OptionalCall { callee, args, .. } => count_yields_in_expr(callee) + args.iter().map(count_yields_in_expr).sum::<usize>(),
        FieldAssign { object, value, .. } | PrivateFieldAssign { object, value, .. } => count_yields_in_expr(object) + count_yields_in_expr(value),
        Index { object, index, .. } => count_yields_in_expr(object) + count_yields_in_expr(index),
        IndexAssign { object, index, value, .. } => count_yields_in_expr(object) + count_yields_in_expr(index) + count_yields_in_expr(value),
        Template { parts, .. } => parts.iter().map(|p| match p { ast::TemplatePart::Text(_) => 0, ast::TemplatePart::Expr(e) => count_yields_in_expr(e) }).sum(),
        TaggedTemplate { tag, exprs, .. } => count_yields_in_expr(tag) + exprs.iter().map(|e| count_yields_in_expr(e)).sum::<usize>(),
        Function(_) | Number { .. } | BigInt { .. } | String { .. } | RegexLiteral { .. } | Bool { .. } | Null { .. } | Ident(_) => 0,
        StaticMethodCall { args, .. } => args.iter().map(count_yields_in_expr).sum(),
        StaticField { .. } => 0,
        ObjectLiteral { properties, .. } => properties
            .iter()
            .map(|member| match member {
                ast::ObjectMember::Property(property) => {
                    let key = match &property.key {
                        ast::PropertyKey::Computed(key) => count_yields_in_expr(key),
                        _ => 0,
                    };
                    key + count_yields_in_expr(&property.value)
                }
                ast::ObjectMember::Spread { expr, .. } => count_yields_in_expr(expr),
            })
            .sum(),
        ArrayLiteral { elements, .. } => elements
            .iter()
            .map(|element| match element {
                ast::ArrayElement::Expr(expr) | ast::ArrayElement::Spread(expr) => {
                    count_yields_in_expr(expr)
                }
                ast::ArrayElement::Hole => 0,
            })
            .sum(),
    }
}

fn count_awaits_in_stmt(stmt: &ast::Stmt) -> usize {
    use ast::Stmt::*;
    match stmt {
        Let { init, .. } | Destructure { init, .. } => count_awaits_in_expr(init),
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
        ForIn { object, body, .. } => count_awaits_in_expr(object) + count_awaits_in_block(body),
        Switch { discriminant, cases, .. } => count_awaits_in_expr(discriminant) + cases.iter().map(|c| c.test.as_ref().map_or(0, count_awaits_in_expr) + count_awaits_in_block(&c.body)).sum::<usize>(),
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
        Function(_) | Number { .. } | BigInt { .. } | String { .. } | RegexLiteral { .. } | Bool { .. } | Null { .. } | Ident(_) => 0,
        Template { parts, .. } => parts
            .iter()
            .map(|part| match part {
                ast::TemplatePart::Text(_) => 0,
                ast::TemplatePart::Expr(expr) => count_awaits_in_expr(expr),
            })
            .sum(),
        TaggedTemplate { tag, exprs, .. } => count_awaits_in_expr(tag) + exprs.iter().map(|e| count_awaits_in_expr(e)).sum::<usize>(),
        ImportCall { specifier, .. } => count_awaits_in_expr(specifier),
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
        ComputedCall { callee, args, .. } | NewDynamic { callee, args, .. } => {
            count_awaits_in_expr(callee) + args.iter().map(count_awaits_in_expr).sum::<usize>()
        }
        MethodCall { receiver, args, .. } => {
            count_awaits_in_expr(receiver) + args.iter().map(count_awaits_in_expr).sum::<usize>()
        }
        StaticMethodCall { args, .. } => args.iter().map(count_awaits_in_expr).sum(),
        StaticField { .. } => 0,
        OptionalField { object, .. } => count_awaits_in_expr(object),
        OptionalIndex { object, index, .. } => count_awaits_in_expr(object) + count_awaits_in_expr(index),
        ChainRoot { .. } => 0,
        TempBind { value, body, .. } => count_awaits_in_expr(value) + count_awaits_in_expr(body),
        OptionalChain { object, body, .. } => count_awaits_in_expr(object) + count_awaits_in_expr(body),
        OptionalCall { callee, args, .. } => count_awaits_in_expr(callee) + args.iter().map(count_awaits_in_expr).sum::<usize>(),
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
        ObjectLiteral { properties, .. } => properties
            .iter()
            .map(|member| match member {
                ast::ObjectMember::Property(property) => {
                    let key = match &property.key {
                        ast::PropertyKey::Computed(key) => count_awaits_in_expr(key),
                        _ => 0,
                    };
                    key + count_awaits_in_expr(&property.value)
                }
                ast::ObjectMember::Spread { expr, .. } => count_awaits_in_expr(expr),
            })
            .sum(),
        ArrayLiteral { elements, .. } => elements
            .iter()
            .map(|element| match element {
                ast::ArrayElement::Expr(expr) | ast::ArrayElement::Spread(expr) => {
                    count_awaits_in_expr(expr)
                }
                ast::ArrayElement::Hole => 0,
            })
            .sum(),
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

/// `Function.length` of the `Date.prototype` methods (spec arities).
fn date_function_length(name: &str) -> Option<f64> {
    Some(match name {
        "setUTCFullYear" | "setFullYear" => 3.0,
        "setUTCMonth" | "setMonth" | "setUTCSeconds" | "setSeconds" => 2.0,
        "setUTCHours" | "setHours" => 4.0,
        "setUTCMinutes" | "setMinutes" => 3.0,
        "setUTCDate" | "setDate" | "setUTCMilliseconds" | "setMilliseconds" | "setTime" => 1.0,
        "getTime" | "valueOf" | "toISOString" | "toString" | "toUTCString" | "getUTCFullYear"
        | "getUTCMonth" | "getUTCDate" | "getUTCDay" | "getUTCHours" | "getUTCMinutes"
        | "getUTCSeconds" | "getUTCMilliseconds" | "getFullYear" | "getMonth" | "getDate"
        | "getDay" | "getHours" | "getMinutes" | "getSeconds" | "getMilliseconds"
        | "getTimezoneOffset" | "toDateString" | "toTimeString" => 0.0,
        "toJSON" => 1.0,
        _ => return None,
    })
}

/// `Function.length` of the `Math` methods (spec arities).
fn math_function_length(name: &str) -> Option<f64> {
    Some(match name {
        "max" | "min" | "hypot" | "pow" | "atan2" | "imul" => 2.0,
        "abs" | "acos" | "acosh" | "asin" | "asinh" | "atan" | "atanh" | "cbrt" | "ceil"
        | "clz32" | "cos" | "cosh" | "exp" | "expm1" | "floor" | "fround" | "log" | "log10"
        | "log1p" | "log2" | "round" | "sign" | "sin" | "sinh" | "sqrt" | "tan" | "tanh"
        | "trunc" => 1.0,
        "random" => 0.0,
        _ => return None,
    })
}

/// Spec `length` of well-known built-in functions addressed by a dotted path
/// such as `["Map", "groupBy"]` or `["Promise", "prototype", "then"]`.
fn builtin_function_length(path: &[&str]) -> Option<f64> {
    Some(match path {
        ["Map", "groupBy"] | ["Object", "groupBy"] => 2.0,
        ["Promise", "all" | "race" | "allSettled" | "any" | "resolve" | "reject"] => 1.0,
        ["Promise", "withResolvers"] => 0.0,
        ["Promise", "prototype", "then"] => 2.0,
        ["Promise", "prototype", "catch" | "finally"] => 1.0,
        ["Map" | "Set" | "WeakMap" | "WeakSet", "prototype", "get" | "has" | "delete" | "add"] => 1.0,
        ["Map" | "WeakMap", "prototype", "set"] => 2.0,
        ["Map" | "Set", "prototype", "clear"] => 0.0,
        ["Map" | "Set", "prototype", "forEach"] => 1.0,
        ["Object", "keys" | "values" | "entries" | "freeze" | "isFrozen" | "getPrototypeOf" | "fromEntries"] => 1.0,
        ["Object", "assign" | "defineProperty" ] => if path[1] == "assign" { 2.0 } else { 3.0 },
        ["Object", "getOwnPropertyDescriptor" | "hasOwn" | "setPrototypeOf"] => 2.0,
        ["String", "prototype", "repeat" | "at" | "charAt" | "charCodeAt" | "codePointAt" | "normalize" | "indexOf" | "lastIndexOf" | "includes" | "startsWith" | "endsWith" | "concat" | "localeCompare"] => {
            if path[2] == "normalize" { 0.0 } else { 1.0 }
        }
        ["String", "prototype", "padStart" | "padEnd" | "split" | "slice" | "substring" | "replace" | "replaceAll"] => 2.0,
        ["String", "prototype", "trim" | "trimStart" | "trimEnd" | "toUpperCase" | "toLowerCase" | "toString" | "valueOf"] => 0.0,
        ["Array", "prototype", "push" | "unshift" | "concat" | "at" | "every" | "fill" | "filter" | "find" | "findIndex" | "findLast" | "findLastIndex" | "flatMap" | "forEach" | "includes" | "indexOf" | "join" | "lastIndexOf" | "map" | "reduce" | "reduceRight" | "some" | "sort" | "toSorted"] => 1.0,
        ["Array", "prototype", "copyWithin" | "slice" | "splice" | "toSpliced" | "with"] => 2.0,
        ["Array", "prototype", "pop" | "shift" | "entries" | "keys" | "values" | "flat" | "reverse" | "toLocaleString" | "toReversed" | "toString"] => 0.0,
        ["Array", "isArray" | "of"] => 1.0,
        ["Array", "from"] => 1.0,
        ["Number", "isInteger" | "isNaN" | "isFinite" | "isSafeInteger" | "parseFloat"] => 1.0,
        ["Number", "parseInt"] => 2.0,
        _ => return None,
    })
}
