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
#[path = "function_reflect.rs"]
mod function_reflect;
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
        routed_methods: HashMap::new(),
        hooked_classes: HashSet::new(),
        type_param_names: HashSet::new(),
        enums: HashMap::new(),
        module_globals: HashSet::new(),
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
    for class in &program.classes {
        for sm in &class.symbol_methods {
            functions.push(checker.check_function(next, &sm.function, None));
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
    mutated: HashSet<hir::LocalSlot>,
    mutated_names: HashSet<String>,
    boxed: HashSet<hir::LocalSlot>,
    captures: HashMap<String, hir::Capture>,
    this_class: Option<u32>,
    /// True while checking a class `constructor` body (where `super(...)` is legal).
    is_constructor: bool,
    /// True while checking the synthetic top-level `<main>` body, where a
    /// `let`/`const` declaration binds a module global (stored via `globalSet`)
    /// rather than a frame local.
    is_module_scope: bool,
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
            mutated: HashSet::new(),
            mutated_names: HashSet::new(),
            boxed: HashSet::new(),
            captures: HashMap::new(),
            this_class: None,
            is_constructor: false,
            is_module_scope: true,
        };
        self.collect_mutated_names(&block.statements, &mut cx.mutated_names);
        let mut body = self.check_block(&mut cx, &block);
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

    fn build_module_initializer(&self, program: &ast::Program) -> Option<hir::Function> {
        let span = Span::new(0, 0);
        let mut body: Vec<hir::Stmt> = Vec::new();

        // 1) Declarative realm seed (base-type class descriptors).
        let registry = crate::realm::registry();
        if !registry.is_empty() {
            let (set_id, _) = zun_natives::resolve("collections", "globalSet")
                .expect("globalSet intrinsic must exist");
            for (name, binding) in registry {
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
                for name in ["valueOf", "toString"] {
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
        if let Some((set_default, _)) = zun_natives::resolve("collections", "classSetFieldDefault") {
            for class_idx in 0..self.classes.len() {
                let fields = self.full_fields(class_idx as u32);
                let initializers = self.full_initializers(class_idx as u32);
                for (slot, (_, field_ty, _, _)) in fields.iter().enumerate() {
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
            let params = f
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
                    if block_returns_value(&f.body) {
                        Ty::Any
                    } else {
                        Ty::Void
                    }
                }
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
                    "WeakSet" => Ty::WeakSet,
                    "WeakRef" => Ty::WeakRef,
                    "RegExp" => Ty::RegExp,
                    "BigInt" => Ty::BigInt,
                    "FinalizationRegistry" => Ty::FinalizationRegistry,
                    _ => Ty::Instance(class_idx as u32),
                };
                let mut params = if m.is_static { Vec::new() } else { vec![receiver_ty] };
                for p in &m.params {
                    params.push(match &p.ty {
                        Some(t) => self.resolve_annotation(t),
                        // An untyped parameter is dynamic (`any`), matching JS.
                        None => Ty::Any,
                    });
                }
                let return_ty = match &m.return_type {
                    Some(t) => self.resolve_annotation(t),
                    None => {
                        if block_returns_value(&m.body) {
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
                let mut params = Vec::new();
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
                    name: format!("[symbol]@{class_idx}"),
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
                        .position(|parameter| parameter.is_rest),
                    is_generator: false,
                    is_static: true,
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
            is_constructor: this_class.is_some() && self.text(&f.name) == "constructor",
            is_module_scope: false,
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
    ) -> hir::Expr {
        let Some(default) = default else { return read };
        let span = read.span;
        let default = self.check_expr(cx, default);
        let default = Self::box_to_any(default);
        let Some((id, _)) = zun_natives::resolve("array", "coalesceUndefined") else {
            self.error(span, "missing `coalesceUndefined` intrinsic");
            return read;
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: vec![Self::box_to_any(read), default] },
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
                if !matches!(value.ty, Ty::Array | Ty::Any | Ty::Error) {
                    self.error(span, "array destructuring requires an Array or any value");
                }
                let temp = cx.declare(&self.fresh_temp_name(cx), value.ty);
                let temp_ty = value.ty;
                out.push(hir::Stmt::Let { slot: temp, init: value });
                for (index, element) in elements.iter().enumerate() {
                    let Some(element) = element else { continue };
                    let read = self.array_element_read(temp, temp_ty, index, span);
                    let read = self.apply_default(cx, read, element.default.as_ref());
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
                for property in properties {
                    let (read, static_key) = self.object_member_read(cx, temp, temp_ty, &property.key, span);
                    if let Some(key) = static_key {
                        excluded.push(key);
                    }
                    let read = self.apply_default(cx, read, property.default.as_ref());
                    self.bind_pattern(cx, &property.value, read, out);
                }
                if let Some(rest) = rest {
                    let read = self.object_rest_read(temp, temp_ty, &excluded, span);
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
    fn object_rest_read(&mut self, temp: hir::LocalSlot, temp_ty: Ty, excluded: &[String], span: Span) -> hir::Expr {
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
                        init = Self::unbox_to(init, declared);
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
                    Ty::Instance(idx) if self.classes_with_iterator.contains(&idx) => {
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
                let Some((keys_id, _)) = zun_natives::resolve("collections", "objectKeys") else {
                    self.error(span, "missing `objectKeys` intrinsic");
                    return hir::Stmt::Expr(self.poison(span));
                };
                let keys = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id: keys_id, args: vec![keys_arg] },
                    ty: Ty::Array,
                    span,
                };
                self.lower_for_iteration(
                    cx,
                    keys,
                    "arrayIteratorNew",
                    "arrayIteratorDone",
                    "arrayIteratorNext",
                    pattern,
                    body,
                )
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
                    Self::unbox_to(e, cx.return_ty)
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
        let to_stderr = match method {
            "log" | "info" | "debug" => false,
            "error" | "warn" => true,
            _ => return None,
        };
        let fmt_class = *self.classes_by_name.get("ConsoleFormat")?;
        let top = self.method_of(fmt_class, "top")?;
        let print_name = if to_stderr { "eprintln" } else { "println" };
        let (print_id, _) = zun_natives::resolve("io", print_name)?;
        // Build the space-joined string of every formatted argument.
        let mut joined: Option<hir::Expr> = None;
        for arg in args {
            let value = self.check_expr(cx, arg);
            let formatted = hir::Expr {
                kind: hir::ExprKind::StaticCall {
                    func: top,
                    args: vec![Self::box_to_any(value)],
                },
                ty: Ty::String,
                span,
            };
            joined = Some(match joined {
                None => formatted,
                Some(acc) => {
                    let with_space = hir::Expr {
                        kind: hir::ExprKind::Binary {
                            op: ast::BinaryOp::Add,
                            lhs: Box::new(acc),
                            rhs: Box::new(Self::string_literal(" ".to_string(), span)),
                        },
                        ty: Ty::String,
                        span,
                    };
                    hir::Expr {
                        kind: hir::ExprKind::Binary {
                            op: ast::BinaryOp::Add,
                            lhs: Box::new(with_space),
                            rhs: Box::new(formatted),
                        },
                        ty: Ty::String,
                        span,
                    }
                }
            });
        }
        let text = joined.unwrap_or_else(|| Self::string_literal(String::new(), span));
        Some(hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id: print_id, args: vec![text] },
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
        // Expected argument count per static (`assign` is variadic: 1+).
        let fixed = match method {
            "keys" | "values" | "entries" | "freeze" | "isFrozen" | "create"
            | "getPrototypeOf" | "fromEntries" | "getOwnPropertyDescriptors"
            | "getOwnPropertyNames" | "getOwnPropertySymbols" | "seal" | "isSealed"
            | "preventExtensions" | "isExtensible" => Some(1usize),
            "setPrototypeOf" | "is" | "getOwnPropertyDescriptor"
            | "defineProperties" => Some(2usize),
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
            "freeze" => builtin(self, "objectFreeze", checked, Ty::Any),
            "isFrozen" => builtin(self, "objectIsFrozen", checked, Ty::Boolean),
            "create" => builtin(self, "objectCreate", checked, Ty::DynamicObject),
            "getPrototypeOf" => builtin(self, "objectGetPrototypeOf", checked, Ty::Any),
            "setPrototypeOf" => builtin(self, "objectSetPrototypeOf", checked, Ty::Any),
            "fromEntries" => builtin(self, "objectFromEntries", checked, Ty::DynamicObject),
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
            let mut array = checked.pop().unwrap();
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
        if checked.len() != expected {
            self.error(
                span,
                format!("`Reflect.{method}` expects {expected} argument(s), found {}", checked.len()),
            );
            return self.poison(span);
        }
        // The target must be a dynamic receiver (plain object or `any`); a Proxy
        // target is itself a dynamic receiver at runtime.
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
                let value = checked.pop().unwrap();
                let key = checked.pop().unwrap();
                let object = checked.pop().unwrap();
                let boxed = if value.ty == Ty::Any {
                    value
                } else {
                    hir::Expr {
                        span: value.span,
                        kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                        ty: Ty::Any,
                    }
                };
                hir::Expr {
                    kind: hir::ExprKind::DynamicSetValue {
                        object: Box::new(object),
                        key: Box::new(key),
                        value: Box::new(boxed),
                    },
                    ty: Ty::Any,
                    span,
                }
            }
            "has" => builtin(self, "reflectHas", checked, Ty::Boolean),
            "deleteProperty" => builtin(self, "reflectDelete", checked, Ty::Boolean),
            // `ownKeys` reports all own string keys (not just enumerable ones),
            // and consults a proxy's `ownKeys` trap via the proxy-aware native.
            "ownKeys" => builtin(self, "objectGetOwnPropertyNames", checked, Ty::Array),
            "getOwnPropertyDescriptor" => {
                builtin(self, "objectGetOwnPropertyDescriptor", checked, Ty::Any)
            }
            "defineProperty" => builtin(self, "objectDefineProperty", checked, Ty::Any),
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
                (cx.declare(self.text(name), declared), Vec::new())
            }
            _ => {
                let element = cx.declare(&self.fresh_temp_name(cx), Ty::Any);
                let mut prelude = Vec::new();
                let element_expr = hir::Expr { kind: hir::ExprKind::Local(element), ty: Ty::Any, span };
                self.bind_pattern(cx, pattern, element_expr, &mut prelude);
                (element, prelude)
            }
        };
        let mut body = self.check_block(cx, body);
        if !prelude.is_empty() {
            let mut combined = prelude;
            combined.append(&mut body);
            body = combined;
        }
        cx.scopes.pop();
        hir::Stmt::ForOf { iterator, iterator_slot, done, next, binding, body }
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
        if value.ty == Ty::F64 || value.ty.is_error() {
            return value;
        }
        let span = value.span;
        let Some((id, _)) = zun_natives::resolve("collections", "numberCoerce") else {
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

    fn check_math_static(
        &mut self,
        cx: &mut FunctionCx,
        method: &str,
        method_span: Span,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
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
                    value = self.to_number_expr(value);
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
        if native.params.len() != args.len() {
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
                value = self.to_number_expr(value);
            }
            if !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                self.error(
                    value.span,
                    format!("`Math.{method}` argument type {:?} does not match {expected:?}", value.ty),
                );
            }
            checked.push(value);
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
                if !arg.ty.is_error() && !self.is_assignable(arg.ty, Ty::String) {
                    self.error(arg.span, "`Number.parseFloat` expects a string argument");
                }
                hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] }, ty: Ty::F64, span }
            }
            "parseInt" => {
                if args.is_empty() || args.len() > 2 {
                    self.error(span, format!("`Number.parseInt` expects 1 or 2 arguments, found {}", args.len()));
                    return self.poison(span);
                }
                let Some(id) = resolve(self, "parseIntRadix") else { return self.poison(span) };
                let value = self.check_expr(cx, &args[0]);
                if !value.ty.is_error() && !self.is_assignable(value.ty, Ty::String) {
                    self.error(value.span, "`Number.parseInt` expects a string argument");
                }
                let radix = match args.get(1) {
                    Some(arg) => {
                        let mut radix = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut radix, Ty::Int(IntKind::I32));
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
        if args.len() != 1 {
            self.error(span, format!("`Promise.{method}` expects 1 argument, found {}", args.len()));
            return self.poison(span);
        }
        let mut arg = self.check_expr(cx, &args[0]);
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
            let mut value = match initializers.get(slot).and_then(|init| init.as_ref()) {
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
        if rest_param.is_some() {
            self.error(span, format!("`{name}` constructor rest parameters are not supported yet"));
            return self.poison(span);
        }
        // param_tys[0] is `this`.
        let mut call_args: Vec<hir::Expr> = args.iter().map(|a| self.check_expr(cx, a)).collect();
        let max = param_tys.len() - 1;
        let min = max.saturating_sub(optional_count);
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
        hir::Expr {
            kind: hir::ExprKind::NewWithConstructor { class: class_idx, defaults, func: ctor, args: call_args },
            ty: Ty::Instance(class_idx),
            span,
        }
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
        if args.len() > 2 {
            self.error(
                span,
                format!("`{name}` expects at most a message and an options object, found {}", args.len()),
            );
            return self.poison(span);
        }
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
                let checked = self.check_expr(cx, &property.value);
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
                let value = Self::box_to_any(checked);
                let key = match &property.key {
                    ast::PropertyKey::Computed(key) => {
                        hir::ObjectKey::Computed(self.check_expr(cx, key))
                    }
                    _ => hir::ObjectKey::Named(named_key.expect("named key")),
                };
                entries.push(hir::ObjectEntry::Data { key, value });
            }
            ast::PropertyKind::Method => {
                let func = self.check_expr(cx, &property.value);
                match &property.key {
                    ast::PropertyKey::Computed(key) => {
                        let key = self.check_expr(cx, key);
                        // A `Symbol` computed key routes to the symbol-function
                        // slot (the enabler for user `[Symbol.iterator]` /
                        // `[Symbol.hasInstance]` methods); a string key uses the
                        // ordinary string function slot.
                        if key.ty == Ty::Symbol {
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
                let func = self.check_expr(cx, &property.value);
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
                        entries: vec![hir::ObjectEntry::Install {
                            builtin: set_fn,
                            args: vec![Self::string_literal(accessor_slot.to_string(), span), func],
                        }],
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
            ast::Expr::ImportCall { specifier, .. } => {
                let mut checked = self.check_expr(cx, specifier);
                if !checked.ty.is_error() && checked.ty != Ty::String {
                    self.error(checked.span, "import() specifier must be a string");
                    checked = self.poison(span);
                }
                // v1 types every dynamic import as `Promise<any>`; the resolved
                // namespace is an opt-in DynamicObject accessed dynamically.
                hir::Expr {
                    kind: hir::ExprKind::ImportDynamic {
                        specifier: Box::new(checked),
                    },
                    ty: Ty::Promise,
                    span,
                }
            }
            ast::Expr::ObjectLiteral { properties, .. } => {
                let entries = self.lower_object_members(cx, properties, span);
                hir::Expr {
                    kind: hir::ExprKind::DynamicObjectLiteral { entries },
                    ty: Ty::DynamicObject,
                    span,
                }
            }
            ast::Expr::ArrayLiteral { elements, .. } => self.lower_array_literal(cx, elements, span),
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
                let Some((id, native)) = zun_natives::resolve("collections", "bigIntNew") else {
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
            ast::Expr::RegexLiteral { pattern, flags, .. } => {
                // Same lowering as `new RegExp(pattern, flags)`.
                let Some((id, _)) = zun_natives::resolve("collections", "regexpNew") else {
                    self.error(span, "missing collection intrinsic `regexpNew`");
                    return self.poison(span);
                };
                let string = |value: &str| hir::Expr {
                    kind: hir::ExprKind::String(value.to_owned()),
                    ty: Ty::String,
                    span,
                };
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![string(pattern), string(flags)],
                    },
                    ty: Ty::RegExp,
                    span,
                }
            }
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
                                if let Some((id, _)) = zun_natives::resolve("number", "toString") {
                                    hir::Expr {
                                        kind: hir::ExprKind::CallBuiltin { id, args: vec![value] },
                                        ty: Ty::String,
                                        span,
                                    }
                                } else {
                                    self.error(span, "number template conversion is unavailable");
                                    self.poison(span)
                                }
                            } else if !matches!(value.ty, Ty::String | Ty::Error | Ty::Void)
                                && zun_natives::resolve("collections", "stringCoerce").is_some()
                            {
                                let (id, _) = zun_natives::resolve("collections", "stringCoerce")
                                    .expect("checked above");
                                let arg = if value.ty == Ty::Any {
                                    value
                                } else {
                                    hir::Expr {
                                        span: value.span,
                                        kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                                        ty: Ty::Any,
                                    }
                                };
                                hir::Expr {
                                    kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                                    ty: Ty::String,
                                    span,
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
            ast::Expr::TaggedTemplate { tag, cooked: _, raw, exprs, .. } => {
                self.check_tagged_template(cx, tag, raw, exprs, span)
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
                        // A declarative realm global read as a value yields its
                        // opaque class descriptor (PLAN §3.1).
                        if let Some(token) = self.realm_globals.get(&text).map(|b| b.token) {
                            return self.realm_descriptor_expr(token, span);
                        }
                        // A user class read as a value yields a class descriptor
                        // (`USER_CLASS_FLAG | TypeDef rid`) usable by
                        // `Reflect.construct` and `new Proxy(C, handler)`.
                        if let Some(&class_idx) = self.classes_by_name.get(&text) {
                            let token = zun_natives::USER_CLASS_FLAG | (class_idx + 1);
                            return self.realm_descriptor_expr(token, span);
                        }
                        // A module global (top-level `let`/`const`) read from any
                        // scope resolves through `globalGet` (dynamic).
                        if self.module_globals.contains(&text) {
                            return self.global_get_expr(&text, span);
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
                        // A dynamic value stored into a typed primitive slot is
                        // unboxed so the slot holds the primitive itself.
                        value = Self::unbox_to(value, slot_ty);
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
                        // Assignment to a module global (top-level `let`/`const`)
                        // stores through `globalSet`.
                        if self.module_globals.contains(&text) {
                            return self.global_set_expr(&text, value, span);
                        }
                        self.error(target.span, format!("unknown variable `{text}`"));
                        self.poison(span)
                    }
                }
            }
            ast::Expr::Binary { op, lhs, rhs, .. } => {
                if let ast::BinaryOp::In = op {
                    return self.check_in_operator(cx, lhs, rhs, span);
                }
                if let ast::BinaryOp::Instanceof = op {
                    return self.check_instanceof_operator(cx, lhs, rhs, span);
                }
                let mut lhs = self.check_expr(cx, lhs);
                let mut rhs = self.check_expr(cx, rhs);
                // Let an integral number literal take on an Int sibling's type
                // so `x + 1` type-checks when `x: int` (and vice versa).
                unify_numeric_literal(&mut lhs, &mut rhs);
                if lhs.ty == Ty::BigInt && rhs.ty == Ty::BigInt {
                    // `>>>` has no BigInt form in ECMAScript: it is a TypeError.
                    if let ast::BinaryOp::UShr = op {
                        self.error(span, "BigInts have no unsigned right shift (`>>>`)");
                        return self.poison(span);
                    }
                    let intrinsic = match op {
                        ast::BinaryOp::Add => Some("bigIntAdd"),
                        ast::BinaryOp::Sub => Some("bigIntSub"),
                        ast::BinaryOp::Mul => Some("bigIntMul"),
                        ast::BinaryOp::Div => Some("bigIntDiv"),
                        ast::BinaryOp::Rem => Some("bigIntRem"),
                        ast::BinaryOp::Pow => Some("bigIntPow"),
                        ast::BinaryOp::BitAnd => Some("bigIntAnd"),
                        ast::BinaryOp::BitOr => Some("bigIntOr"),
                        ast::BinaryOp::BitXor => Some("bigIntXor"),
                        ast::BinaryOp::Shl => Some("bigIntShl"),
                        ast::BinaryOp::Shr => Some("bigIntShr"),
                        ast::BinaryOp::Eq => Some("bigIntEq"),
                        ast::BinaryOp::Ne => Some("bigIntNe"),
                        ast::BinaryOp::Lt => Some("bigIntLt"),
                        ast::BinaryOp::Le => Some("bigIntLe"),
                        ast::BinaryOp::Gt => Some("bigIntGt"),
                        ast::BinaryOp::Ge => Some("bigIntGe"),
                        _ => None,
                    };
                    if let Some(name) = intrinsic {
                        if let Some((id, native)) = zun_natives::resolve("collections", name) {
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
                // `number ** number` routes to the `Math.pow` intrinsic (JS
                // exponent semantics, NaN/Infinity edges included). Integer `**`
                // stays an IL opcode (`pow.i32`), lowered below via binary_result.
                if let ast::BinaryOp::Pow = op {
                    // `number ** number`, and any `**` with a dynamic operand,
                    // route to `Math.pow` (the dynamic side coerces to f64 at
                    // run time). Pure-int `**` stays the `pow.i32` opcode below.
                    let numberish = |t: Ty| t == Ty::F64 || t == Ty::Any;
                    if numberish(lhs.ty) && numberish(rhs.ty) {
                        if let Some((id, native)) = zun_natives::resolve("math", "powNumber") {
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
                }
                // `+` with an object/any operand: ECMAScript ToPrimitive(default)
                // on both sides, then string concat or numeric add, at run time.
                if let ast::BinaryOp::Add = op {
                    let objectish =
                        |t: Ty| matches!(t, Ty::Any | Ty::DynamicObject | Ty::Instance(_));
                    let operable = |t: Ty| {
                        objectish(t)
                            || t == Ty::String
                            || t == Ty::F64
                            || t == Ty::Boolean
                            || t == Ty::BigInt
                            || t == Ty::Array
                            || matches!(t, Ty::Int(_))
                    };
                    let numeric_ty = |t: Ty| t.is_int() || t.is_float();
                    // Route through the runtime `addAny` (ECMAScript
                    // ToPrimitive + string concat / numeric add) whenever an
                    // operand is object/any, or a string meets a non-string
                    // primitive (`"x" + 1`, `"x" + true`). Same-type numeric and
                    // string+string keep their direct lowerings below.
                    let involves_string = lhs.ty == Ty::String || rhs.ty == Ty::String;
                    if operable(lhs.ty)
                        && operable(rhs.ty)
                        && (objectish(lhs.ty)
                            || objectish(rhs.ty)
                            || (involves_string && lhs.ty != rhs.ty)
                            || (lhs.ty == Ty::Boolean && rhs.ty == Ty::Boolean)
                            || (lhs.ty != rhs.ty
                                && !(numeric_ty(lhs.ty) && numeric_ty(rhs.ty))
                                && lhs.ty != Ty::BigInt
                                && rhs.ty != Ty::BigInt))
                    {
                        if let Some((id, _)) = zun_natives::resolve("collections", "addAny") {
                            let to_any = |e: hir::Expr| {
                                if e.ty == Ty::Any {
                                    e
                                } else {
                                    hir::Expr {
                                        span: e.span,
                                        kind: hir::ExprKind::BoxAny { value: Box::new(e) },
                                        ty: Ty::Any,
                                    }
                                }
                            };
                            return hir::Expr {
                                kind: hir::ExprKind::CallBuiltin {
                                    id,
                                    args: vec![to_any(lhs), to_any(rhs)],
                                },
                                ty: Ty::Any,
                                span,
                            };
                        }
                    }
                }
                // Dynamic numeric operators (`-`, `*`, `/`, `%`) apply
                // ECMAScript ToNumber to both operands when a non-numeric or
                // dynamic operand is involved (`"5" * 2`, `+x`, `any - 1`), then
                // run the ordinary f64 op. Same-type numeric operands keep their
                // direct lowering below.
                if matches!(
                    *op,
                    ast::BinaryOp::Sub
                        | ast::BinaryOp::Mul
                        | ast::BinaryOp::Div
                        | ast::BinaryOp::Rem
                        | ast::BinaryOp::BitAnd
                        | ast::BinaryOp::BitOr
                        | ast::BinaryOp::BitXor
                        | ast::BinaryOp::Shl
                        | ast::BinaryOp::Shr
                        | ast::BinaryOp::UShr
                ) {
                    let numericish = |t: Ty| {
                        matches!(t, Ty::Int(_)) || matches!(t, Ty::Float(_)) || t == Ty::F64
                    };
                    let needs_coerce = !lhs.ty.is_error()
                        && !rhs.ty.is_error()
                        && !(numericish(lhs.ty) && numericish(rhs.ty))
                        && lhs.ty != Ty::BigInt
                        && rhs.ty != Ty::BigInt;
                    if needs_coerce {
                        if let Some((id, _)) = zun_natives::resolve("collections", "numberCoerce") {
                            let coerce = |e: hir::Expr| hir::Expr {
                                span: e.span,
                                kind: hir::ExprKind::CallBuiltin {
                                    id,
                                    args: vec![Self::box_to_any(e)],
                                },
                                ty: Ty::F64,
                            };
                            let lhs = coerce(lhs);
                            let rhs = coerce(rhs);
                            return hir::Expr {
                                kind: hir::ExprKind::Binary {
                                    op: *op,
                                    lhs: Box::new(lhs),
                                    rhs: Box::new(rhs),
                                },
                                ty: Ty::F64,
                                span,
                            };
                        }
                    }
                }
                let ty = self.binary_result(*op, &lhs, &rhs, span);
                // A logical/nullish operator whose result is dynamic keeps the
                // deciding operand's value (value-preserving short-circuit), so
                // both operands must be boxed to a uniform `any` carrier.
                let (lhs, rhs) = if ty == Ty::Any
                    && matches!(*op, ast::BinaryOp::And | ast::BinaryOp::Or | ast::BinaryOp::Nullish)
                {
                    (Self::box_to_any(lhs), Self::box_to_any(rhs))
                } else {
                    (lhs, rhs)
                };
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
                let mut when_true = self.check_expr(cx, when_true);
                let mut when_false = self.check_expr(cx, when_false);
                let ty = if when_true.ty == when_false.ty {
                    when_true.ty
                } else if matches!(when_true.ty, Ty::Function(_))
                    && matches!(when_false.ty, Ty::Function(_))
                {
                    // Two function-typed branches (e.g. `cond ? inc : dec`). Each
                    // function is a distinct nominal `Ty::Function(index)`. A
                    // *concrete* function type lowers to a monomorphic
                    // `call.indirect` whose token pins the callee to that one
                    // method (the interpreter rejects a value whose `method_rid`
                    // differs — see `CallIndirect` in zun_interp), so a join to
                    // either branch's index would be unsound: the other branch's
                    // value would fail the pin at run time. The sound join is the
                    // dynamic any-arity function type, which dispatches purely on
                    // the runtime value (token rid 0) and yields `any`.
                    Ty::Function(hir::DYNAMIC_FUNCTION)
                } else {
                    // Branches of differing types form a union, erased to a
                    // dynamic value. Box both sides so the stack value is a
                    // proper `any` carrier regardless of which branch runs.
                    when_true = Self::box_to_any(when_true);
                    when_false = Self::box_to_any(when_false);
                    Ty::Any
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
                match op {
                    ast::UnaryOp::Typeof => return self.check_typeof_operator(cx, operand, span),
                    ast::UnaryOp::Delete => return self.check_delete_operator(cx, operand, span),
                    ast::UnaryOp::Neg
                    | ast::UnaryOp::Not
                    | ast::UnaryOp::BitNot
                    | ast::UnaryOp::Void => {}
                }
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
                if operand.ty == Ty::BigInt
                    && matches!(op, ast::UnaryOp::Neg | ast::UnaryOp::BitNot)
                {
                    let native_name = match op {
                        ast::UnaryOp::Neg => "bigIntNeg",
                        ast::UnaryOp::BitNot => "bigIntNot",
                        _ => unreachable!(),
                    };
                    if let Some((id, native)) = zun_natives::resolve("collections", native_name) {
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
                // `-x` / `~x` on a non-numeric operand: JS ToNumber first.
                let operand = if matches!(op, ast::UnaryOp::Neg | ast::UnaryOp::BitNot)
                    && !operand.ty.is_int()
                    && !operand.ty.is_float()
                    && operand.ty != Ty::BigInt
                {
                    self.to_number_expr(operand)
                } else {
                    operand
                };
                let ty = match op {
                    // Negation keeps the operand's numeric type; `!` needs bool.
                    ast::UnaryOp::Neg if operand.ty.is_int() || operand.ty.is_float() => operand.ty,
                    ast::UnaryOp::Neg => {
                        self.error(operand.span, format!("cannot negate {:?}", operand.ty));
                        Ty::Error
                    }
                    ast::UnaryOp::Not => self.expect_ty(&operand, Ty::Boolean),
                    // `~` keeps the operand's numeric type (int, or `number` via
                    // the int32 coercion lowered in ilgen).
                    ast::UnaryOp::BitNot if operand.ty.is_int() || operand.ty == Ty::F64 => {
                        operand.ty
                    }
                    ast::UnaryOp::BitNot => {
                        self.error(
                            operand.span,
                            format!("cannot apply `~` to {:?}", operand.ty),
                        );
                        Ty::Error
                    }
                    // `void x` evaluates its operand and yields `undefined`.
                    ast::UnaryOp::Void => Ty::Any,
                    ast::UnaryOp::Typeof | ast::UnaryOp::Delete => {
                        unreachable!("handled before operand type-checking")
                    }
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
                if name == "super" {
                    return self.check_super_call(cx, None, args, span);
                }
                // Global `parseInt`/`parseFloat` are the same as their `Number.*`
                // statics; route to that implementation when no user binding
                // shadows the name.
                if matches!(name.as_str(), "parseInt" | "parseFloat")
                    && !self.by_name.contains_key(&name)
                {
                    return self.check_number_static(cx, &name, callee.span, args, span);
                }
                // Global `isNaN`/`isFinite` coerce with ToNumber first (unlike
                // the strict `Number.*` predicates).
                if matches!(name.as_str(), "isNaN" | "isFinite")
                    && !self.by_name.contains_key(&name)
                    && cx.lookup(&name).is_none()
                {
                    if args.len() != 1 {
                        self.error(span, format!("`{name}` expects 1 argument, found {}", args.len()));
                        return self.poison(span);
                    }
                    let Some((id, _)) = zun_natives::resolve("number", &name) else {
                        self.error(span, format!("missing Number intrinsic `{name}`"));
                        return self.poison(span);
                    };
                    let mut arg = self.check_expr(cx, &args[0]);
                    coerce_numeric_literal(&mut arg, Ty::F64);
                    let arg = self.to_number_for_arg(arg);
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                        ty: Ty::Boolean,
                        span,
                    };
                }
                // `Function(p1, ..., body)` behaves exactly like `new Function(...)`.
                if name == "Function"
                    && !self.by_name.contains_key(&name)
                    && !self.classes_by_name.contains_key(&name)
                {
                    return self.lower_function_construct(cx, args, span);
                }
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

                // `BigInt(x)` coercion when called as a plain function (JS
                // BigInt is not constructed with `new`). A string argument
                // parses (decimal or `0x`/`0o`/`0b`); a number must be
                // integer-valued (non-integers are a runtime RangeError).
                if name == "BigInt" && !self.by_name.contains_key(&name) {
                    if args.len() != 1 {
                        self.error(span, "`BigInt` expects one argument");
                        return self.poison(span);
                    }
                    let arg = args.into_iter().next().expect("one argument");
                    let native_name = if arg.ty.is_int() || arg.ty.is_float() {
                        "bigIntFromNumber"
                    } else if arg.ty == Ty::String {
                        "bigIntNew"
                    } else if arg.ty.is_error() {
                        return self.poison(span);
                    } else {
                        self.error(arg.span, "`BigInt` expects a string or number argument");
                        return self.poison(span);
                    };
                    let Some((id, _)) = zun_natives::resolve("collections", native_name) else {
                        self.error(span, format!("missing collection intrinsic `{native_name}`"));
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                        ty: Ty::BigInt,
                        span,
                    };
                }

                // `Boolean(x)` coercion when called as a plain function:
                // returns the JS truthiness of `x` as a `boolean` primitive
                // (not a wrapper object). The `truthy` native mirrors the
                // interpreter's `is_truthy`, so coercion matches `if`/`&&`.
                if name == "Boolean" && !self.by_name.contains_key(&name) {
                    if args.len() != 1 {
                        self.error(span, "`Boolean` expects one argument");
                        return self.poison(span);
                    }
                    let arg = args.into_iter().next().expect("one argument");
                    if arg.ty.is_error() {
                        return self.poison(span);
                    }
                    // A value already typed `boolean` is its own truthiness.
                    if arg.ty == Ty::Boolean {
                        return arg;
                    }
                    let Some((id, _)) = zun_natives::resolve("boolean", "booleanCoerce") else {
                        self.error(span, "missing Boolean intrinsic `booleanCoerce`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                        ty: Ty::Boolean,
                        span,
                    };
                }

                // `String(x)` coercion when called as a plain function (JS
                // `String` is a coercion, not a constructor, called bare): the
                // ECMAScript ToString of `x` as a `string` primitive. A value
                // already typed `string` is returned unchanged; every other
                // operand is boxed into `Any` and run through the `stringCoerce`
                // native, which performs object→primitive coercion
                // (`[Symbol.toPrimitive]` / `[Symbol.toStringTag]`) at run time.
                if name == "String" && !self.by_name.contains_key(&name) {
                    if args.len() != 1 {
                        self.error(span, "`String` expects one argument");
                        return self.poison(span);
                    }
                    let arg = args.into_iter().next().expect("one argument");
                    if arg.ty.is_error() {
                        return self.poison(span);
                    }
                    // A value already typed `string` is its own ToString.
                    if arg.ty == Ty::String {
                        return arg;
                    }
                    let Some((id, _)) = zun_natives::resolve("collections", "stringCoerce") else {
                        self.error(span, "missing `stringCoerce` intrinsic");
                        return self.poison(span);
                    };
                    // The native reads its operand as `any`, so box a non-`any`
                    // operand (primitive or object) into an `Any` carrier.
                    let arg = if arg.ty == Ty::Any {
                        arg
                    } else {
                        hir::Expr {
                            span: arg.span,
                            kind: hir::ExprKind::BoxAny { value: Box::new(arg) },
                            ty: Ty::Any,
                        }
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                        ty: Ty::String,
                        span,
                    };
                }

                // `Number(x)` coercion when called as a plain function (JS
                // `Number` as a coercion, not a constructor): the ECMAScript
                // ToNumber of `x` as a `number` primitive. A value already typed
                // numeric is returned unchanged; every other operand is boxed
                // into `Any` and run through the `numberCoerce` native, which
                // performs string parsing and object→primitive coercion
                // (`[Symbol.toPrimitive]("number")`) at run time.
                if name == "Number" && !self.by_name.contains_key(&name) {
                    if args.len() != 1 {
                        self.error(span, "`Number` expects one argument");
                        return self.poison(span);
                    }
                    let arg = args.into_iter().next().expect("one argument");
                    if arg.ty.is_error() {
                        return self.poison(span);
                    }
                    // A value already typed `number`/`float` is its own ToNumber.
                    if arg.ty == Ty::F64 {
                        return arg;
                    }
                    let Some((id, _)) = zun_natives::resolve("collections", "numberCoerce") else {
                        self.error(span, "missing `numberCoerce` intrinsic");
                        return self.poison(span);
                    };
                    // The native reads its operand as `any`, so box a non-`any`
                    // operand (primitive or object) into an `Any` carrier.
                    let arg = if arg.ty == Ty::Any {
                        arg
                    } else {
                        hir::Expr {
                            span: arg.span,
                            kind: hir::ExprKind::BoxAny { value: Box::new(arg) },
                            ty: Ty::Any,
                        }
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                        ty: Ty::F64,
                        span,
                    };
                }

                if let Some(&func) = self.by_name.get(&name) {
                    if self.signatures[func as usize].is_generator {
                        let Some((native_id, _)) = zun_natives::resolve("buffer", "generatorIteratorNewArgs") else {
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

                // A local shadows a capture of the same name. Inside a closure a captured
                // callable is read from the environment instead of a local slot.
                let callee_source: Option<(hir::ExprKind, Ty)> = match cx.lookup(&name) {
                    Some(slot) => Some((hir::ExprKind::Local(slot), cx.locals[slot as usize])),
                    None => cx.captures.get(&name).map(|capture| {
                        (
                            hir::ExprKind::CaptureLoad {
                                environment_slot: capture.environment_slot,
                                ty: capture.ty,
                                mode: capture.mode,
                            },
                            capture.ty,
                        )
                    }),
                };
                if let Some((callee_proto, function_ty)) = callee_source {
                    let callee_kind = || match &callee_proto {
                        hir::ExprKind::CaptureLoad { environment_slot, ty, mode } => {
                            hir::ExprKind::CaptureLoad {
                                environment_slot: *environment_slot,
                                ty: *ty,
                                mode: *mode,
                            }
                        }
                        hir::ExprKind::Local(slot) => hir::ExprKind::Local(*slot),
                        _ => unreachable!(),
                    };

                    // Calling an `any`-typed local performs a dynamic dispatch:
                    // the value must be callable at run time (e.g. a function
                    // read from an object's function slot via destructuring, as
                    // `const { resolve } = Promise.withResolvers()` produces).
                    // Lowered like a generic `Function` callback (single arg).
                    if function_ty == Ty::Any {
                        // A dynamic callee may be invoked with any argument count
                        // (JS); the counted indirect-call form carries the arity.
                        let callee_expr = hir::Expr {
                            kind: callee_kind(),
                            ty: function_ty,
                            span,
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::IndirectCall {
                                callee: Box::new(callee_expr),
                                args,
                                func: hir::DYNAMIC_FUNCTION,
                            },
                            ty: Ty::Any,
                            span,
                        };
                    }
                    // Calling a proxy value: `proxy(...args)` routes through the
                    // handler's `apply` trap (NUCLEUS #10). The arguments are
                    // packed into an array (each boxed to `Any` by `StoreArray`)
                    // and dispatched to `proxyApply` with an `undefined` `this`.
                    if function_ty == Ty::Proxy {
                        let callee_expr = hir::Expr {
                            kind: callee_kind(),
                            ty: function_ty,
                            span,
                        };
                        let args_array = hir::Expr {
                            kind: hir::ExprKind::ArrayLiteral {
                                elements: args.into_iter().map(Some).collect(),
                            },
                            ty: Ty::Array,
                            span,
                        };
                        let this_arg = hir::Expr { kind: hir::ExprKind::Null, ty: Ty::Any, span };
                        let Some((id, _)) = zun_natives::resolve("collections", "proxyApply") else {
                            self.error(span, "missing `proxyApply` intrinsic");
                            return self.poison(span);
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id,
                                args: vec![callee_expr, this_arg, args_array],
                            },
                            ty: Ty::Any,
                            span,
                        };
                    }
                    if let Ty::Function(func) = function_ty {
                        let return_ty = if func == hir::DYNAMIC_FUNCTION {
                            // Any arity: a bound function (`f.bind(..)`) forwards N call
                            // arguments (lowered to `call.indirect.n` when N != 1).
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
                                            kind: callee_kind(),
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
                            kind: callee_kind(),
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

                // The `timers` macrotask surface (ASYNC slice 4): bare global
                // functions on the VM's unified event loop. Resolved here like
                // `print`, but only when the name is not a user symbol (a local
                // or top-level function of the same name shadows it, handled
                // above). Each lowers to the interpreter's timer floor.
                if matches!(
                    name.as_str(),
                    "setTimeout"
                        | "setInterval"
                        | "clearTimeout"
                        | "clearInterval"
                        | "queueMicrotask"
                        | "setImmediate"
                ) && !self.by_name.contains_key(&name)
                    && cx.lookup(&name).is_none()
                {
                    let Some((id, _)) = zun_natives::resolve("timers", &name) else {
                        self.error(span, format!("missing timer intrinsic `{name}`"));
                        return self.poison(span);
                    };
                    let number = |span| hir::Expr {
                        kind: hir::ExprKind::Number(0.0),
                        ty: Ty::F64,
                        span,
                    };
                    let is_callback = |arg: &hir::Expr| {
                        matches!(arg.ty, Ty::Function(_)) || arg.ty.is_error()
                    };
                    let (built_args, ret) = match name.as_str() {
                        "setTimeout" | "setInterval" => {
                            if args.is_empty() || args.len() > 2 {
                                self.error(span, format!("`{name}` expects a callback and an optional delay"));
                                return self.poison(span);
                            }
                            let mut it = args.into_iter();
                            let cb = it.next().expect("callback");
                            if !is_callback(&cb) {
                                self.error(cb.span, format!("`{name}` expects a function callback"));
                            }
                            let mut ms = it.next().unwrap_or_else(|| number(span));
                            // Accept any numeric delay; a non-number is an error.
                            if !ms.ty.is_int() && !ms.ty.is_float() && !ms.ty.is_error() {
                                self.error(ms.span, format!("`{name}` delay must be a number"));
                            }
                            coerce_numeric_literal(&mut ms, Ty::F64);
                            (vec![cb, ms], Ty::F64)
                        }
                        "setImmediate" | "queueMicrotask" => {
                            if args.len() != 1 {
                                self.error(span, format!("`{name}` expects a single callback"));
                                return self.poison(span);
                            }
                            let cb = args.into_iter().next().expect("callback");
                            if !is_callback(&cb) {
                                self.error(cb.span, format!("`{name}` expects a function callback"));
                            }
                            let ret = if name == "setImmediate" { Ty::F64 } else { Ty::Void };
                            (vec![cb], ret)
                        }
                        // clearTimeout / clearInterval
                        _ => {
                            if args.len() != 1 {
                                self.error(span, format!("`{name}` expects a timer handle"));
                                return self.poison(span);
                            }
                            let mut handle = args.into_iter().next().expect("handle");
                            if !handle.ty.is_int() && !handle.ty.is_float() && !handle.ty.is_error() {
                                self.error(handle.span, format!("`{name}` expects a numeric handle"));
                            }
                            coerce_numeric_literal(&mut handle, Ty::F64);
                            (vec![handle], Ty::Void)
                        }
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: built_args },
                        ty: ret,
                        span,
                    };
                }

                if matches!(
                    name.as_str(),
                    "decoratorAddInstanceInitializer" | "decoratorAccessGet" | "decoratorAccessSet"
                ) && !self.by_name.contains_key(&name)
                {
                    // Prelude plumbing for `DecoratorContext` / `DecoratorAccess`
                    // (builtin/decorators.zun): checked against the native signature.
                    let Some((id, native)) = zun_natives::resolve("collections", &name) else {
                        return self.poison(span);
                    };
                    if args.len() != native.params.len() {
                        self.error(span, format!("`{name}` has an invalid argument list"));
                        return self.poison(span);
                    }
                    for (arg, &param) in args.iter_mut().zip(native.params) {
                        let expected = native_ty(param);
                        coerce_numeric_literal(arg, expected);
                        if !arg.ty.is_error()
                            && !(matches!(expected, Ty::Function(_)) && matches!(arg.ty, Ty::Function(_)))
                            && !self.is_assignable(arg.ty, expected)
                        {
                            self.error(arg.span, format!("`{name}` argument does not match {expected:?}"));
                        }
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args },
                        ty: native_ty(native.ret),
                        span,
                    };
                }

                if name == "decoratorAddInitializer" || name == "decoratorRunInitializers" {
                    // Prelude plumbing for the typed `DecoratorContext`
                    // (builtin/decorators.zun).
                    let expected = usize::from(name == "decoratorAddInitializer");
                    if args.len() != expected
                        || (expected == 1 && !matches!(args[0].ty, Ty::Function(_)))
                    {
                        self.error(span, format!("`{name}` has an invalid argument list"));
                        return self.poison(span);
                    }
                    let Some((id, native)) = zun_natives::resolve("collections", &name) else {
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args },
                        ty: native_ty(native.ret),
                        span,
                    };
                }

                if name == "dynamicSetFunction" {
                    if args.len() != 3 {
                        self.error(span, "dynamicSetFunction expects an object, name, and function");
                        return self.poison(span);
                    }
                    let Some((id, native)) = zun_natives::resolve("collections", &name) else {
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
                }

                if name == "dynamicSetSymbolFunction" {
                    if args.len() != 3 {
                        self.error(span, "dynamicSetSymbolFunction expects an object, symbol, and function");
                        return self.poison(span);
                    }
                    let Some((id, native)) = zun_natives::resolve("collections", &name) else {
                        return self.poison(span);
                    };
                    if args[0].ty != Ty::DynamicObject
                        || args[1].ty != Ty::Symbol
                        || !matches!(args[2].ty, Ty::Function(_))
                    {
                        self.error(span, "dynamicSetSymbolFunction expects (dynamic, symbol, function)");
                        return self.poison(span);
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args },
                        ty: native_ty(native.ret),
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

                // `Symbol(description?)` mints a fresh, unique symbol. The
                // description is optional; when present it must be a string.
                if name == "Symbol" && !self.by_name.contains_key("Symbol") {
                    if args.len() > 1 {
                        self.error(span, "`Symbol` takes an optional description argument");
                        return self.poison(span);
                    }
                    let Some((id, _)) = zun_natives::resolve("collections", "symbolNew") else {
                        self.error(span, "missing `symbolNew` intrinsic");
                        return self.poison(span);
                    };
                    let description = match args.pop() {
                        Some(mut value) => {
                            if value.ty != Ty::String && value.ty != Ty::Any && !value.ty.is_error() {
                                // Box non-string descriptions to `undefined` at
                                // runtime by passing them through as `Any`.
                                value = hir::Expr {
                                    span: value.span,
                                    kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                                    ty: Ty::Any,
                                };
                            }
                            value
                        }
                        None => hir::Expr { kind: hir::ExprKind::Null, ty: Ty::Any, span },
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![description] },
                        ty: Ty::Symbol,
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
                    if class_name == "super" {
                        return self.check_super_call(cx, Some(method), args, span);
                    }
                    if class_name == "Object" && !self.classes_by_name.contains_key("Object") {
                        return self.check_object_static(
                            cx,
                            &self.text(method).to_string(),
                            method.span,
                            args,
                            span,
                        );
                    }
                    // `console.log/info/debug/error/warn(...)` are variadic in JS
                    // (space-separated, each argument Node-formatted). The
                    // prelude `console` method takes a single value, so intercept
                    // here to format every argument through `ConsoleFormat.top`,
                    // join with spaces, and print once.
                    if class_name == "console" {
                        if let Some(result) =
                            self.check_console_variadic(cx, &self.text(method).to_string(), args, span)
                        {
                            return result;
                        }
                    }
                    // `Reflect.*` reflection statics (`get set has deleteProperty
                    // ownKeys`). Surface-only namespace, recognized by receiver
                    // name; a program may still declare its own `Reflect` class.
                    if class_name == "Reflect" && !self.classes_by_name.contains_key("Reflect") {
                        return self.check_reflect_static(
                            cx,
                            &self.text(method).to_string(),
                            method.span,
                            args,
                            span,
                        );
                    }
                    // `Buffer.*` statics (`from alloc allocUnsafe concat
                    // isBuffer byteLength`). Recognized by receiver name; a
                    // program may still declare its own `Buffer` class.
                    if class_name == "Buffer" && !self.classes_by_name.contains_key("Buffer") {
                        return self.check_buffer_static(
                            cx,
                            &self.text(method).to_string(),
                            method.span,
                            args,
                            span,
                        );
                    }
                    // `Number.*` statics (`isNaN isFinite isInteger isSafeInteger
                    // parseFloat parseInt`). Unlike `Object`, the `Number` class
                    // exists (its instance helpers), so recognition keys on the
                    // receiver name alone — a value never names `Number`.
                    if class_name == "Number" {
                        return self.check_number_static(
                            cx,
                            &self.text(method).to_string(),
                            method.span,
                            args,
                            span,
                        );
                    }
                    // `Math.<fn>(...)` static math, routed to the `math` native
                    // group's f64 helpers. Recognized by receiver name (a value
                    // never names `Math`); a program may still declare its own.
                    if class_name == "Math" && !self.classes_by_name.contains_key("Math") {
                        return self.check_math_static(
                            cx,
                            &self.text(method).to_string(),
                            method.span,
                            args,
                            span,
                        );
                    }
                    // `String.fromCharCode(...)` / `String.fromCodePoint(...)`.
                    // Recognized by receiver name only for the known statics;
                    // anything else on `String` falls through to class dispatch.
                    if class_name == "String"
                        && matches!(self.text(method), "fromCharCode" | "fromCodePoint")
                    {
                        return self.check_string_static(
                            cx,
                            &self.text(method).to_string(),
                            method.span,
                            args,
                            span,
                        );
                    }
                    // `Array.isArray(x)` / `Array.of(...)` / `Array.from(src,
                    // mapFn?)`. Recognized by receiver name for the known statics
                    // (the `Array` class exists for its instance surface, like
                    // `Number`); anything else on `Array` falls through.
                    if class_name == "Array"
                        && matches!(self.text(method), "isArray" | "of" | "from")
                    {
                        return self.check_array_static(
                            cx,
                            &self.text(method).to_string(),
                            method.span,
                            args,
                            span,
                        );
                    }
                    // `Symbol.for(key)` / `Symbol.keyFor(symbol)` — the global
                    // cross-realm registry.
                    if class_name == "Symbol" && !self.classes_by_name.contains_key("Symbol") {
                        let member = self.text(method).to_string();
                        let (native, arg_ty, ret_ty) = match member.as_str() {
                            "for" => ("symbolFor", Ty::String, Ty::Symbol),
                            "keyFor" => ("symbolKeyFor", Ty::Symbol, Ty::Any),
                            other => {
                                self.error(method.span, format!("`Symbol.{other}` is not a function"));
                                return self.poison(span);
                            }
                        };
                        if args.len() != 1 {
                            self.error(span, format!("`Symbol.{member}` expects one argument"));
                            return self.poison(span);
                        }
                        let Some((id, _)) = zun_natives::resolve("collections", native) else {
                            self.error(span, format!("missing `{native}` intrinsic"));
                            return self.poison(span);
                        };
                        let mut arg = self.check_expr(cx, &args[0]);
                        if !arg.ty.is_error() && !self.is_assignable(arg.ty, arg_ty) {
                            self.error(arg.span, format!("`Symbol.{member}` expects a {arg_ty:?} argument"));
                        }
                        coerce_numeric_literal(&mut arg, arg_ty);
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                            ty: ret_ty,
                            span,
                        };
                    }
                    // `BigInt.asIntN(bits, value)` / `BigInt.asUintN(bits,
                    // value)`. The `BigInt` class exists (instance helpers), so
                    // recognition keys on the receiver name and method pair; any
                    // other member falls through to class dispatch.
                    if class_name == "BigInt"
                        && matches!(self.text(method), "asIntN" | "asUintN")
                    {
                        let member = self.text(method).to_string();
                        let native = if member == "asIntN" {
                            "bigIntAsIntN"
                        } else {
                            "bigIntAsUintN"
                        };
                        if args.len() != 2 {
                            self.error(span, format!("`BigInt.{member}` expects two arguments"));
                            return self.poison(span);
                        }
                        let Some((id, _)) = zun_natives::resolve("collections", native) else {
                            self.error(span, format!("missing `{native}` intrinsic"));
                            return self.poison(span);
                        };
                        let mut bits = self.check_expr(cx, &args[0]);
                        coerce_numeric_literal(&mut bits, Ty::Int(IntKind::I64));
                        if !bits.ty.is_error() && !bits.ty.is_int() {
                            self.error(bits.span, format!("`BigInt.{member}` bit width must be an integer"));
                        }
                        let value = self.check_expr(cx, &args[1]);
                        if !value.ty.is_error() && value.ty != Ty::BigInt {
                            self.error(value.span, format!("`BigInt.{member}` expects a BigInt value"));
                        }
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: vec![bits, value] },
                            ty: Ty::BigInt,
                            span,
                        };
                    }
                    // `Promise.*` statics (`resolve reject all allSettled race
                    // any`). Recognized by receiver name when no user class
                    // shadows `Promise`; a Promise *value* is dispatched by type.
                    if class_name == "Promise" && !self.classes_by_name.contains_key("Promise") {
                        return self.check_promise_static(
                            cx,
                            &self.text(method).to_string(),
                            method.span,
                            args,
                            span,
                        );
                    }
                    // `ArrayBuffer.isView(x)` — true for a DataView or typed
                    // array, false for anything else. Accepts any value; a
                    // non-`any` operand is boxed, so a plain value answers
                    // `false` rather than failing to type-check.
                    if class_name == "ArrayBuffer"
                        && self.text(method) == "isView"
                        && !self.classes_by_name.contains_key("ArrayBuffer")
                    {
                        if args.len() != 1 {
                            self.error(span, "`ArrayBuffer.isView` expects one argument".to_string());
                            return self.poison(span);
                        }
                        let Some((id, _)) = zun_natives::resolve("buffer", "isView") else {
                            self.error(span, "missing `isView` intrinsic".to_string());
                            return self.poison(span);
                        };
                        let mut arg = self.check_expr(cx, &args[0]);
                        if arg.ty != Ty::Any && !arg.ty.is_error() {
                            arg = hir::Expr {
                                span: arg.span,
                                kind: hir::ExprKind::BoxAny { value: Box::new(arg) },
                                ty: Ty::Any,
                            };
                        }
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                            ty: Ty::Boolean,
                            span,
                        };
                    }
                    // `Int32Array.from(src, mapFn?)` / `Int32Array.of(...items)`
                    // and the other ten kinds. Recognized by the constructor
                    // name when no user class shadows it; the target element
                    // kind travels to the kind-agnostic native as a table-order
                    // discriminant argument.
                    if let Some(view_kind) = crate::typed_arrays::view_kind(&class_name) {
                        if matches!(self.text(method), "from" | "of")
                            && !self.classes_by_name.contains_key(&class_name)
                        {
                            return self.check_typed_array_static(
                                cx,
                                view_kind,
                                &self.text(method).to_string(),
                                args,
                                span,
                            );
                        }
                    }
                    if let Some(&namespace) = self.core_namespaces.get(&class_name) {
                        let Some((id, native)) = zun_natives::resolve_core(namespace, self.text(method)) else {
                            self.error(method.span, format!("`zun:core` namespace `{}` has no member `{}`", namespace, self.text(method)));
                            return self.poison(span);
                        };
                        if native.params.len() != args.len() {
                            self.error(span, format!("`{}` expects {} argument(s), found {}", self.text(method), native.params.len(), args.len()));
                            return self.poison(span);
                        }
                        let mut checked = Vec::with_capacity(args.len());
                        for (arg, expected) in args.iter().zip(native.params.iter().map(|ty| native_ty(*ty))) {
                            let mut value = self.check_expr(cx, arg);
                            coerce_numeric_literal(&mut value, expected);
                            if !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                                self.error(value.span, format!("argument type {:?} does not match {expected:?}", value.ty));
                            }
                            checked.push(value);
                        }
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: checked },
                            ty: native_ty(native.ret),
                            span,
                        };
                    }
                    if let Some(module) = self.namespace_imports.get(&class_name).cloned() {
                        if zun_natives::module(&module).is_none() {
                            // Source-module namespaces are handled below.
                        } else {
                        let Some((id, native)) = zun_natives::resolve(&module, self.text(method)) else {
                            self.error(method.span, format!("`{}` has no export named `{}`", module, self.text(method)));
                            return self.poison(span);
                        };
                        if native.params.len() != args.len() {
                            self.error(span, format!("`{}` expects {} argument(s), found {}", self.text(method), native.params.len(), args.len()));
                            return self.poison(span);
                        }
                        let mut checked = Vec::with_capacity(args.len());
                        for (arg, expected) in args.iter().zip(native.params.iter().map(|ty| native_ty(*ty))) {
                            let mut value = self.check_expr(cx, arg);
                            coerce_numeric_literal(&mut value, expected);
                            if !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                                self.error(value.span, format!("argument type {:?} does not match {expected:?}", value.ty));
                            }
                            checked.push(value);
                        }
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: checked },
                            ty: native_ty(native.ret),
                            span,
                        };
                        }
                    }
                    if let Some(module) = self.namespace_imports.get(&class_name).cloned() {
                        if let Some(external) = self
                            .external_namespace_functions
                            .get(&(module.clone(), self.text(method).to_string()))
                            .cloned()
                        {
                            if external.params.len() != args.len() {
                                self.error(span, format!("`{}` expects {} argument(s), found {}", self.text(method), external.params.len(), args.len()));
                                return self.poison(span);
                            }
                            let mut checked = Vec::with_capacity(args.len());
                            for (arg, &expected) in args.iter().zip(&external.params) {
                                let mut value = self.check_expr(cx, arg);
                                coerce_numeric_literal(&mut value, expected);
                                if !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                                    self.error(value.span, format!("argument type {:?} does not match {expected:?}", value.ty));
                                }
                                checked.push(value);
                            }
                            return hir::Expr {
                                kind: hir::ExprKind::ExternalCall {
                                    module: external.module,
                                    name: external.name,
                                    args: checked,
                                },
                                ty: external.return_ty,
                                span,
                            };
                        }
                    }
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
                                    let mut value = self.check_expr(cx, arg);
                                    // The only argument-checking site that
                                    // skipped this, so a bare literal failed
                                    // here while the identical argument was
                                    // accepted through every other call shape.
                                    coerce_numeric_literal(&mut value, expected);
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
                // Static-regex fast path (the `[GeneratedRegex]` analog): a
                // regex literal or `new RegExp(const, const)` used directly by
                // `.test` / `.isMatch`. The pattern is known at compile time, so
                // it is recorded for `zun_ilgen` to synthesize a ZIL matcher
                // (which tiers up to native like any loop). `zun_ilgen` falls
                // back to the runtime engine for patterns outside the compilable
                // subset, so this never changes observable behavior. `exec` and
                // the string `match`/`search`/etc. paths are left untouched.
                {
                    let mname_peek = self.text(method).to_string();
                    if (mname_peek == "test" || mname_peek == "isMatch") && args.len() == 1 {
                        if let Some((pattern, flags)) = self.static_regex(receiver.as_ref()) {
                            let mut subject = self.check_expr(cx, &args[0]);
                            coerce_numeric_literal(&mut subject, Ty::String);
                            if !subject.ty.is_error() && !self.is_assignable(subject.ty, Ty::String)
                            {
                                self.error(
                                    subject.span,
                                    format!(
                                        "`{mname_peek}` expects a string argument, found {:?}",
                                        subject.ty
                                    ),
                                );
                            }
                            let matcher = self.regex_matchers.len() as u32;
                            self.regex_matchers.push(hir::CompiledRegex { pattern, flags });
                            return hir::Expr {
                                kind: hir::ExprKind::RegexMatch { matcher, args: vec![subject] },
                                ty: Ty::Boolean,
                                span,
                            };
                        }
                    }
                }
                let receiver = self.check_expr(cx, receiver);
                let mname = self.text(method).to_string();
                // `Object.prototype` instance reflectors over typed, dynamic, or
                // `any` receivers. Checked before the generic `any` member-call
                // path so they are not swallowed by dynamic dispatch.
                if (mname == "hasOwnProperty"
                    || mname == "isPrototypeOf"
                    || mname == "propertyIsEnumerable")
                    && args.len() == 1
                    && matches!(
                        receiver.ty,
                        Ty::DynamicObject | Ty::Any | Ty::Instance(_)
                    )
                {
                    let argument = self.check_expr(cx, &args[0]);
                    let (name, ty) = if mname == "hasOwnProperty" {
                        ("objectHasOwn", Ty::Boolean)
                    } else if mname == "propertyIsEnumerable" {
                        ("objectPropertyIsEnumerable", Ty::Boolean)
                    } else {
                        ("objectIsPrototypeOf", Ty::Boolean)
                    };
                    let Some((id, _)) = zun_natives::resolve("collections", name) else {
                        self.error(span, format!("missing Object intrinsic `{name}`"));
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, argument] },
                        ty,
                        span,
                    };
                }
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
                if receiver.ty == Ty::Array && mname == "splice" {
                    // `splice(start, deleteCount, ...items)`. The inserted items
                    // are gathered into a managed array (an array literal), so a
                    // single fixed-arity native carries the variadic inserts.
                    if args.len() < 2 {
                        self.error(
                            span,
                            format!(
                                "`Array.splice` expects at least 2 argument(s), found {}",
                                args.len()
                            ),
                        );
                        return self.poison(span);
                    }
                    let mut start = self.check_expr(cx, &args[0]);
                    coerce_numeric_literal(&mut start, Ty::Int(IntKind::U64));
                    if !start.ty.is_error() && !self.is_assignable(start.ty, Ty::Int(IntKind::U64)) {
                        self.error(start.span, "`Array.splice` start must be an integer");
                    }
                    let mut delete_count = self.check_expr(cx, &args[1]);
                    coerce_numeric_literal(&mut delete_count, Ty::Int(IntKind::U64));
                    if !delete_count.ty.is_error()
                        && !self.is_assignable(delete_count.ty, Ty::Int(IntKind::U64))
                    {
                        self.error(delete_count.span, "`Array.splice` deleteCount must be an integer");
                    }
                    let items: Vec<Option<hir::Expr>> =
                        args[2..].iter().map(|item| Some(self.check_expr(cx, item))).collect();
                    let items = hir::Expr {
                        kind: hir::ExprKind::ArrayLiteral { elements: items },
                        ty: Ty::Array,
                        span,
                    };
                    let Some((id, _)) = zun_natives::resolve("array", "arraySpliceInsert") else {
                        self.error(span, "missing Array intrinsic `arraySpliceInsert`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![receiver, start, delete_count, items],
                        },
                        ty: Ty::Array,
                        span,
                    };
                }
                if receiver.ty == Ty::Array && mname == "toReversed" {
                    if !args.is_empty() {
                        self.error(
                            span,
                            format!("`Array.toReversed` expects 0 arguments, found {}", args.len()),
                        );
                        return self.poison(span);
                    }
                    let Some((id, _)) = zun_natives::resolve("array", "arrayToReversed") else {
                        self.error(span, "missing Array intrinsic `arrayToReversed`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver] },
                        ty: Ty::Array,
                        span,
                    };
                }
                if receiver.ty == Ty::Array && mname == "toSorted" {
                    // `toSorted(cmp?)`: a sorted copy, optionally by comparator
                    // (default ascending string order). The receiver is untouched.
                    if args.len() > 1 {
                        self.error(
                            span,
                            format!("`Array.toSorted` accepts at most 1 argument, found {}", args.len()),
                        );
                        return self.poison(span);
                    }
                    let (native_name, call_args) = match args.first() {
                        Some(arg) => {
                            let value = self.check_expr(cx, arg);
                            if !value.ty.is_error()
                                && !self.is_assignable(value.ty, Ty::Function(hir::DYNAMIC_FUNCTION))
                            {
                                self.error(value.span, "`Array.toSorted` comparator must be a function");
                            }
                            ("arrayToSortedBy", vec![receiver, value])
                        }
                        None => ("arrayToSorted", vec![receiver]),
                    };
                    let Some((id, _)) = zun_natives::resolve("array", native_name) else {
                        self.error(span, format!("missing Array intrinsic `{native_name}`"));
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                        ty: Ty::Array,
                        span,
                    };
                }
                if receiver.ty == Ty::Array && mname == "with" {
                    // `with(index, value)`: a copy with `index` replaced; the
                    // index is a (possibly negative) integer, the value any.
                    if args.len() != 2 {
                        self.error(
                            span,
                            format!("`Array.with` expects 2 arguments, found {}", args.len()),
                        );
                        return self.poison(span);
                    }
                    let mut index = self.check_expr(cx, &args[0]);
                    coerce_numeric_literal(&mut index, Ty::Int(IntKind::I64));
                    if !index.ty.is_error() && !self.is_assignable(index.ty, Ty::Int(IntKind::I64)) {
                        self.error(index.span, "`Array.with` index must be an integer");
                    }
                    let value = self.check_expr(cx, &args[1]);
                    let value = if value.ty == Ty::Any || value.ty.is_error() {
                        value
                    } else {
                        let vspan = value.span;
                        hir::Expr {
                            kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                            ty: Ty::Any,
                            span: vspan,
                        }
                    };
                    let Some((id, _)) = zun_natives::resolve("array", "arrayWith") else {
                        self.error(span, "missing Array intrinsic `arrayWith`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, index, value] },
                        ty: Ty::Array,
                        span,
                    };
                }
                if receiver.ty == Ty::Array && mname == "toSpliced" {
                    // `toSpliced(start, deleteCount, ...items)`: a copy with the
                    // splice applied, mirroring `splice`'s index resolution but
                    // leaving the receiver untouched.
                    if args.len() < 2 {
                        self.error(
                            span,
                            format!(
                                "`Array.toSpliced` expects at least 2 argument(s), found {}",
                                args.len()
                            ),
                        );
                        return self.poison(span);
                    }
                    let mut start = self.check_expr(cx, &args[0]);
                    coerce_numeric_literal(&mut start, Ty::Int(IntKind::U64));
                    if !start.ty.is_error() && !self.is_assignable(start.ty, Ty::Int(IntKind::U64)) {
                        self.error(start.span, "`Array.toSpliced` start must be an integer");
                    }
                    let mut delete_count = self.check_expr(cx, &args[1]);
                    coerce_numeric_literal(&mut delete_count, Ty::Int(IntKind::U64));
                    if !delete_count.ty.is_error()
                        && !self.is_assignable(delete_count.ty, Ty::Int(IntKind::U64))
                    {
                        self.error(delete_count.span, "`Array.toSpliced` deleteCount must be an integer");
                    }
                    let items: Vec<Option<hir::Expr>> =
                        args[2..].iter().map(|item| Some(self.check_expr(cx, item))).collect();
                    let items = hir::Expr {
                        kind: hir::ExprKind::ArrayLiteral { elements: items },
                        ty: Ty::Array,
                        span,
                    };
                    let Some((id, _)) = zun_natives::resolve("array", "arrayToSpliced") else {
                        self.error(span, "missing Array intrinsic `arrayToSpliced`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![receiver, start, delete_count, items],
                        },
                        ty: Ty::Array,
                        span,
                    };
                }
                if receiver.ty == Ty::Array && mname == "concat" {
                    // `concat(...items)`: the variadic items are gathered into
                    // a managed Array; the native spreads or appends each.
                    let items: Vec<Option<hir::Expr>> =
                        args.iter().map(|item| Some(self.check_expr(cx, item))).collect();
                    let items = hir::Expr {
                        kind: hir::ExprKind::ArrayLiteral { elements: items },
                        ty: Ty::Array,
                        span,
                    };
                    let Some((id, _)) = zun_natives::resolve("array", "arrayConcat") else {
                        self.error(span, "missing Array intrinsic `arrayConcat`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, items] },
                        ty: Ty::Array,
                        span,
                    };
                }
                if receiver.ty == Ty::Array && mname == "flat" {
                    // `flat(depth=1)` — the depth defaults to 1 when omitted.
                    if args.len() > 1 {
                        self.error(
                            span,
                            format!("`Array.flat` accepts at most 1 argument, found {}", args.len()),
                        );
                        return self.poison(span);
                    }
                    let depth = match args.first() {
                        Some(arg) => {
                            let mut value = self.check_expr(cx, arg);
                            coerce_numeric_literal(&mut value, Ty::F64);
                            if !value.ty.is_error() && !self.is_assignable(value.ty, Ty::F64) {
                                self.error(value.span, "`Array.flat` depth must be a number");
                            }
                            value
                        }
                        None => hir::Expr {
                            kind: hir::ExprKind::Number(1.0),
                            ty: Ty::F64,
                            span,
                        },
                    };
                    let Some((id, _)) = zun_natives::resolve("array", "arrayFlat") else {
                        self.error(span, "missing Array intrinsic `arrayFlat`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, depth] },
                        ty: Ty::Array,
                        span,
                    };
                }
                if receiver.ty == Ty::Array
                    && matches!(mname.as_str(), "sort" | "reduce" | "reduceRight")
                {
                    // Variable-arity standard Array methods. `sort` takes an
                    // optional comparator (default is ascending string order);
                    // `reduce`/`reduceRight` require an initial accumulator on
                    // this floor (the no-initial form is deferred). The callback
                    // is a dynamic function value invoked by the native with the
                    // ECMAScript argument shape.
                    let (native_name, return_ty, expected): (&str, Ty, Vec<Ty>) =
                        match (mname.as_str(), args.len()) {
                            ("sort", 0) => ("arraySortDefault", Ty::Array, vec![]),
                            ("sort", 1) => (
                                "arraySortBy",
                                Ty::Array,
                                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
                            ),
                            ("reduce", 2) => (
                                "arrayReduce",
                                Ty::Any,
                                vec![Ty::Function(hir::DYNAMIC_FUNCTION), Ty::Any],
                            ),
                            ("reduceRight", 2) => (
                                "arrayReduceRight",
                                Ty::Any,
                                vec![Ty::Function(hir::DYNAMIC_FUNCTION), Ty::Any],
                            ),
                            // No-initial-value form: the first/last element seeds
                            // the accumulator; an empty array throws a TypeError.
                            ("reduce", 1) => (
                                "arrayReduceNoInit",
                                Ty::Any,
                                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
                            ),
                            ("reduceRight", 1) => (
                                "arrayReduceRightNoInit",
                                Ty::Any,
                                vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
                            ),
                            (other, found) => {
                                self.error(
                                    span,
                                    format!(
                                        "`Array.{other}` does not accept {found} argument(s)"
                                    ),
                                );
                                return self.poison(span);
                            }
                        };
                    let mut call_args = vec![receiver];
                    for (arg, expected_ty) in args.iter().zip(expected) {
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
                    let Some((id, _)) = zun_natives::resolve("collections", native_name)
                        .or_else(|| zun_natives::resolve("array", native_name))
                    else {
                        self.error(span, format!("missing collection intrinsic `{native_name}`"));
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                        ty: return_ty,
                        span,
                    };
                }
                if receiver.ty == Ty::Promise
                    && matches!(mname.as_str(), "then" | "catch" | "finally")
                {
                    // `then`/`catch`/`finally` take optional function callbacks
                    // and return a NEW Promise settled from the callback's result
                    // (adopting a returned Promise). Absent callback slots lower
                    // to `null`, which the floor reads as "no handler". The
                    // callback re-enters the interpreter, so this stays Tier 0.
                    let (native_name, max_callbacks) = match mname.as_str() {
                        "then" => ("promiseThen", 2usize),
                        "catch" => ("promiseCatch", 1usize),
                        _ => ("promiseFinally", 1usize),
                    };
                    if args.len() > max_callbacks {
                        self.error(
                            span,
                            format!(
                                "`Promise.{mname}` accepts at most {max_callbacks} argument(s), found {}",
                                args.len()
                            ),
                        );
                        return self.poison(span);
                    }
                    let mut call_args = vec![receiver];
                    for index in 0..max_callbacks {
                        match args.get(index) {
                            Some(arg) => {
                                let value = self.check_expr(cx, arg);
                                if !value.ty.is_error()
                                    && !self.is_assignable(
                                        value.ty,
                                        Ty::Function(hir::DYNAMIC_FUNCTION),
                                    )
                                {
                                    self.error(
                                        value.span,
                                        format!("`Promise.{mname}` expects a function callback"),
                                    );
                                }
                                call_args.push(value);
                            }
                            None => call_args.push(hir::Expr {
                                kind: hir::ExprKind::Null,
                                ty: Ty::Any,
                                span,
                            }),
                        }
                    }
                    let Some((id, _)) = zun_natives::resolve("promise", native_name) else {
                        self.error(span, format!("missing Promise intrinsic `{native_name}`"));
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                        ty: Ty::Promise,
                        span,
                    };
                }
                if matches!(receiver.ty, Ty::Any | Ty::DynamicObject) && args.len() <= 4 {
                    // Dynamic member call on an `any`/dynamic receiver (e.g. a
                    // namespace from `import()`, or an object-literal method in
                    // the function slot): dispatched by name at run time.
                    let native_name = [
                        "dynamicCall0",
                        "dynamicCall1",
                        "dynamicCall2",
                        "dynamicCall3",
                        "dynamicCall4",
                    ][args.len()];
                    let Some((id, _)) = zun_natives::resolve("collections", native_name) else {
                        self.error(span, format!("missing collection intrinsic `{native_name}`"));
                        return self.poison(span);
                    };
                    let mut call_args = vec![
                        receiver,
                        hir::Expr { kind: hir::ExprKind::String(mname.clone()), ty: Ty::String, span: method.span },
                    ];
                    for arg in args {
                        call_args.push(self.check_expr(cx, arg));
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                        ty: Ty::Any,
                        span,
                    };
                }
                // `String.prototype.concat(...strs)` — variadic. Folded into a
                // chain of the two-argument string `concat` native; zero
                // arguments return the receiver (as `"x".concat() === "x"`).
                if receiver.ty == Ty::String && mname == "concat" {
                    let Some((id, _)) = zun_natives::resolve("string", "concat") else {
                        self.error(span, "missing String intrinsic `concat`");
                        return self.poison(span);
                    };
                    let mut acc = receiver;
                    for arg in args {
                        let value = self.check_expr(cx, arg);
                        if !value.ty.is_error() && !self.is_assignable(value.ty, Ty::String) {
                            self.error(value.span, "`concat` expects string arguments");
                        }
                        acc = hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: vec![acc, value] },
                            ty: Ty::String,
                            span,
                        };
                    }
                    return acc;
                }
                // `String.prototype.slice(start?, end?)` — negative indices
                // count from the end (distinct from `substring`). An absent
                // argument defaults to the whole range; `end` is passed as a
                // value past any length so it clamps to the length natively.
                if receiver.ty == Ty::String && mname == "slice" {
                    if args.len() > 2 {
                        self.error(span, "`slice` expects at most 2 arguments");
                        return self.poison(span);
                    }
                    let Some((id, _)) = zun_natives::resolve("string", "slice") else {
                        self.error(span, "missing String intrinsic `slice`");
                        return self.poison(span);
                    };
                    let mut bound = |checker: &mut Self, arg: Option<&ast::Expr>, default: f64| {
                        match arg {
                            Some(arg) => {
                                let mut value = checker.check_expr(cx, arg);
                                coerce_numeric_literal(&mut value, Ty::F64);
                                if !value.ty.is_error() && !checker.is_assignable(value.ty, Ty::F64) {
                                    checker.error(value.span, "`slice` index must be a number");
                                }
                                value
                            }
                            None => hir::Expr { kind: hir::ExprKind::Number(default), ty: Ty::F64, span },
                        }
                    };
                    let start = bound(self, args.first(), 0.0);
                    // 2^53: past any representable string length, so it clamps to
                    // the length and means "to the end".
                    let end = bound(self, args.get(1), 9_007_199_254_740_992.0);
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, start, end] },
                        ty: Ty::String,
                        span,
                    };
                }
                // `String.prototype.split(separator, limit?)` — dispatch by the
                // separator's type: a RegExp stays on the regex engine, a string
                // uses the string-separator native (with an optional limit).
                if receiver.ty == Ty::String && mname == "split" {
                    if args.is_empty() || args.len() > 2 {
                        self.error(span, "`split` expects 1 or 2 arguments");
                        return self.poison(span);
                    }
                    let first = self.check_expr(cx, &args[0]);
                    if matches!(first.ty, Ty::Any | Ty::DynamicObject) {
                        let limit = args.get(1).map(|arg| self.check_expr(cx, arg));
                        return self.string_symbol_dispatch(span, 3.0, receiver, first, limit);
                    }
                    if first.ty == Ty::RegExp {
                        if args.len() != 1 {
                            self.error(span, "`split` with a RegExp does not accept a limit");
                            return self.poison(span);
                        }
                        let Some((id, _)) = zun_natives::resolve("collections", "regexpSplit") else {
                            self.error(span, "missing collection intrinsic `regexpSplit`");
                            return self.poison(span);
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: vec![first, receiver] },
                            ty: Ty::Array,
                            span,
                        };
                    }
                    if !first.ty.is_error() && !self.is_assignable(first.ty, Ty::String) {
                        self.error(first.span, "`split` expects a string or RegExp separator");
                        return self.poison(span);
                    }
                    let limit = match args.get(1) {
                        Some(arg) => {
                            let mut value = self.check_expr(cx, arg);
                            coerce_numeric_literal(&mut value, Ty::Int(IntKind::I64));
                            if !value.ty.is_error() && !self.is_assignable(value.ty, Ty::Int(IntKind::I64)) {
                                self.error(value.span, "`split` limit must be an integer");
                            }
                            value
                        }
                        // Absent limit → negative sentinel meaning unbounded.
                        None => hir::Expr {
                            kind: hir::ExprKind::Number(-1.0),
                            ty: Ty::Int(IntKind::I64),
                            span,
                        },
                    };
                    let Some((id, _)) = zun_natives::resolve("collections", "stringSplit") else {
                        self.error(span, "missing collection intrinsic `stringSplit`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, first, limit] },
                        ty: Ty::Array,
                        span,
                    };
                }
                // `String.prototype.replace(search, replacement)` — dispatch by
                // the search's type: a RegExp uses the regex engine, a string
                // replaces the first occurrence via the string native (so it no
                // longer needs the `String` class in scope).
                if receiver.ty == Ty::String && mname == "replace" && args.len() == 2 {
                    let first = self.check_expr(cx, &args[0]);
                    let replacement = self.check_expr(cx, &args[1]);
                    if matches!(first.ty, Ty::Any | Ty::DynamicObject) {
                        return self.string_symbol_dispatch(span, 1.0, receiver, first, Some(replacement));
                    }
                    if replacement.ty != Ty::String && !replacement.ty.is_error() {
                        self.error(
                            replacement.span,
                            "`replace` replacement must be a string (replace-with-function is not supported)",
                        );
                        return self.poison(span);
                    }
                    if first.ty == Ty::RegExp {
                        let Some((id, _)) = zun_natives::resolve("collections", "regexpReplace") else {
                            self.error(span, "missing collection intrinsic `regexpReplace`");
                            return self.poison(span);
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: vec![first, receiver, replacement] },
                            ty: Ty::String,
                            span,
                        };
                    }
                    if !first.ty.is_error() && !self.is_assignable(first.ty, Ty::String) {
                        self.error(first.span, "`replace` expects a string or RegExp search");
                        return self.poison(span);
                    }
                    let Some((id, _)) = zun_natives::resolve("string", "replace") else {
                        self.error(span, "missing String intrinsic `replace`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, first, replacement] },
                        ty: Ty::String,
                        span,
                    };
                }
                // String methods taking a RegExp: match/search.
                if receiver.ty == Ty::String
                    && matches!(mname.as_str(), "match" | "search")
                {
                    let expected_args = 1;
                    if args.len() == expected_args {
                        let first = self.check_expr(cx, &args[0]);
                        if matches!(first.ty, Ty::Any | Ty::DynamicObject) {
                            let kind = if mname == "match" { 0.0 } else { 2.0 };
                            return self.string_symbol_dispatch(span, kind, receiver, first, None);
                        }
                        {
                            if first.ty != Ty::RegExp && !first.ty.is_error() {
                                self.error(
                                    first.span,
                                    format!("`{mname}` expects a RegExp argument"),
                                );
                                return self.poison(span);
                            }
                            let (native_name, return_ty) = match mname.as_str() {
                                "match" => ("regexpExec", Ty::Array),
                                _ => ("regexpSearch", Ty::Int(IntKind::I32)),
                            };
                            let call_args = vec![first, receiver];
                            let Some((id, _)) =
                                zun_natives::resolve("collections", native_name)
                            else {
                                self.error(
                                    span,
                                    format!("missing collection intrinsic `{native_name}`"),
                                );
                                return self.poison(span);
                            };
                            return hir::Expr {
                                kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                                ty: return_ty,
                                span,
                            };
                        }
                    }
                }
                // `length`/`byteLength`/`get`/`set` on a typed-array view come
                // from the master table in `zun_core`, so every kind the
                // language accepts has the same reachable method surface.
                // `FinalizationRegistry.register(target, heldValue, unregisterToken?)`
                // and `unregister(unregisterToken)`. Hand-lowered rather than
                // routed through the fixed-arity `managed_method` table because
                // `register`'s unregister token is optional: when omitted, a
                // `null` sentinel is synthesized so the registration carries no
                // token and can never be unregistered (per the ECMAScript spec).
                if receiver.ty == Ty::FinalizationRegistry
                    && (mname == "register" || mname == "unregister")
                {
                    let null_sentinel = |span| hir::Expr {
                        kind: hir::ExprKind::Null,
                        ty: Ty::Any,
                        span,
                    };
                    if mname == "register" {
                        if args.len() != 2 && args.len() != 3 {
                            self.error(
                                span,
                                format!("`register` expects 2 or 3 argument(s), found {}", args.len()),
                            );
                            return self.poison(span);
                        }
                        let target = self.check_expr(cx, &args[0]);
                        let held = self.check_expr(cx, &args[1]);
                        if held.ty.is_error() {
                            return self.poison(span);
                        }
                        // The held value is arbitrary: it is boxed into `Any`,
                        // GC-traced while the registration is pending, and
                        // delivered to the cleanup callback on collection.
                        let held = if held.ty == Ty::Any {
                            held
                        } else {
                            hir::Expr {
                                span: held.span,
                                kind: hir::ExprKind::BoxAny { value: Box::new(held) },
                                ty: Ty::Any,
                            }
                        };
                        let token = match args.get(2) {
                            Some(arg) => self.check_expr(cx, arg),
                            None => null_sentinel(span),
                        };
                        let Some((id, _)) =
                            zun_natives::resolve("collections", "finalizationRegister")
                        else {
                            self.error(span, "missing intrinsic `finalizationRegister`");
                            return self.poison(span);
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id,
                                args: vec![receiver, target, held, token],
                            },
                            ty: Ty::Void,
                            span,
                        };
                    }
                    // `unregister(unregisterToken)`.
                    if args.len() != 1 {
                        self.error(
                            span,
                            format!("`unregister` expects 1 argument, found {}", args.len()),
                        );
                        return self.poison(span);
                    }
                    let token = self.check_expr(cx, &args[0]);
                    let Some((id, _)) =
                        zun_natives::resolve("collections", "finalizationUnregister")
                    else {
                        self.error(span, "missing intrinsic `finalizationUnregister`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![receiver, token],
                        },
                        ty: Ty::Boolean,
                        span,
                    };
                }
                // Node `Buffer` instance surface. A Buffer is a `U8` byte view,
                // so these methods live on that kind; they take optional
                // arguments (defaults applied here) and route to the
                // `nodeBuffer` natives, which operate over the shared byte
                // floor. Placed before the generated typed-array dispatch so
                // `slice`/`subarray`/`fill` get the Buffer (optional-arg)
                // semantics rather than the typed-array ones.
                // A Buffer is a `U8` view, so it shares its type with a plain
                // `Uint8Array`. Only intercept the Buffer-unique method shapes;
                // `fill`/`subarray`/`get`/`set`/`length`/`byteLength` keep their
                // typed-array semantics (generic dispatch below). `fill(value)`
                // with a single argument is the one Node form added here (the
                // 3-arg typed-array `fill` still falls through).
                if receiver.ty == Ty::BufferView(hir::BufferViewKind::U8) {
                    let intercept = match mname.as_str() {
                        "toString" | "write" | "slice" | "equals" | "copy" => true,
                        // Node Buffer search accepts number / string / Buffer
                        // needles (richer than the numeric typed-array form).
                        "indexOf" | "includes" | "lastIndexOf" => true,
                        "swap16" | "swap32" | "swap64" => true,
                        "fill" => args.len() == 1,
                        other => other.starts_with("read") || other.starts_with("write"),
                    };
                    if intercept {
                        return self.check_buffer_method(cx, receiver, &mname, args, span);
                    }
                }
                // The second-generation typed-array surface (bulk `set`,
                // `slice`/`copyWithin`, the functional methods). These are
                // kind-agnostic natives that need optional-argument handling
                // and `set` arity/type overloading, so they are lowered by a
                // dedicated routine rather than the generic method table. Not
                // for `DataView`, and `slice` on a `U8` view is already taken
                // by the Node Buffer intercept above (Buffer semantics).
                if let Ty::BufferView(kind) = receiver.ty {
                    if kind != hir::BufferViewKind::DataView
                        && matches!(
                            mname.as_str(),
                            "set"
                                | "slice"
                                | "copyWithin"
                                | "indexOf"
                                | "includes"
                                | "join"
                                | "forEach"
                                | "map"
                                | "filter"
                                | "reduce"
                                | "reduceRight"
                                | "find"
                                | "findIndex"
                                | "some"
                                | "every"
                        )
                    {
                        return self.check_typed_array_method(
                            cx,
                            receiver,
                            kind,
                            mname.as_str(),
                            args,
                            span,
                        );
                    }
                }
                // `DataView` element accessors, with an **optional** little-
                // endian flag that defaults to `false` (big-endian) as the
                // spec requires. Previously the flag was mandatory, so
                // `view.getUint32(0)` failed to type-check; the single-byte
                // accessors never take one.
                if receiver.ty == Ty::BufferView(hir::BufferViewKind::DataView)
                    && data_view_accessor(mname.as_str())
                {
                    return self.check_data_view_method(cx, receiver, mname.as_str(), args, span);
                }
                // `ArrayBuffer.prototype.transfer(newLength?)` and
                // `transferToFixedLength(newLength?)` (NUCLEUS #17). Hand-lowered
                // rather than routed through the fixed-arity `managed_method`
                // table because `newLength` is optional: when omitted, a `-1`
                // sentinel is synthesized meaning "keep the source length".
                // Both detach the source and yield a fresh `Buffer`.
                if receiver.ty == Ty::Buffer
                    && (mname == "transfer" || mname == "transferToFixedLength")
                {
                    if args.len() > 1 {
                        self.error(
                            span,
                            format!("`{mname}` expects 0 or 1 argument(s), found {}", args.len()),
                        );
                        return self.poison(span);
                    }
                    let new_length = match args.first() {
                        Some(arg) => {
                            let mut length = self.check_expr(cx, arg);
                            coerce_numeric_literal(&mut length, Ty::Int(IntKind::I64));
                            if !length.ty.is_error() && !length.ty.is_int() {
                                self.error(length.span, "transfer length must be an integer");
                            }
                            length
                        }
                        None => hir::Expr {
                            kind: hir::ExprKind::Number(-1.0),
                            ty: Ty::Int(IntKind::I64),
                            span,
                        },
                    };
                    let native = if mname == "transfer" {
                        "bufferTransfer"
                    } else {
                        "bufferTransferFixed"
                    };
                    let Some((id, _)) = zun_natives::resolve("buffer", native) else {
                        self.error(span, format!("missing buffer intrinsic `{native}`"));
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, new_length] },
                        ty: Ty::Buffer,
                        span,
                    };
                }
                // `String.prototype.normalize(form?)` (NUCLEUS #1). Hand-lowered
                // because `form` is optional: the ECMAScript default `"NFC"` is
                // synthesized when it is omitted. The ICU4X-backed
                // `stringNormalize` native validates the form at run time.
                if receiver.ty == Ty::String && mname == "normalize" {
                    if args.len() > 1 {
                        self.error(
                            span,
                            format!("`normalize` expects 0 or 1 argument(s), found {}", args.len()),
                        );
                        return self.poison(span);
                    }
                    let form = match args.first() {
                        Some(arg) => {
                            let checked = self.check_expr(cx, arg);
                            if !checked.ty.is_error() && checked.ty != Ty::String {
                                self.error(checked.span, "normalize form must be a string");
                            }
                            checked
                        }
                        None => hir::Expr {
                            kind: hir::ExprKind::String("NFC".to_string()),
                            ty: Ty::String,
                            span,
                        },
                    };
                    let Some((id, _)) = zun_natives::resolve("string", "stringNormalize") else {
                        self.error(span, "missing string intrinsic `stringNormalize`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, form] },
                        ty: Ty::String,
                        span,
                    };
                }
                let typed_array_method = match receiver.ty {
                    Ty::BufferView(kind) => crate::typed_arrays::method(kind, mname.as_str()),
                    _ => None,
                };
                let managed_method = typed_array_method.or_else(|| match (receiver.ty, mname.as_str()) {
                    // Standard `Array.prototype` methods routed to natives. The
                    // callback methods invoke the dynamic function value with the
                    // ECMAScript `(value, index, array)` signature (extra
                    // arguments dropped to the callback's declared arity).
                    (Ty::Array, "forEach") => {
                        Some(("arrayForEach", Ty::Void, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
                    }
                    (Ty::Array, "map") => {
                        Some(("arrayMap", Ty::Array, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
                    }
                    (Ty::Array, "filter") => {
                        Some(("arrayFilter", Ty::Array, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
                    }
                    (Ty::Array, "find") => {
                        Some(("arrayFind", Ty::Any, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
                    }
                    (Ty::Array, "findIndex") => Some((
                        "arrayFindIndex",
                        Ty::Int(IntKind::I32),
                        vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
                    )),
                    (Ty::Array, "some") => {
                        Some(("arraySome", Ty::Boolean, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
                    }
                    (Ty::Array, "every") => {
                        Some(("arrayEvery", Ty::Boolean, vec![Ty::Function(hir::DYNAMIC_FUNCTION)]))
                    }
                    (Ty::Array, "join") => {
                        Some(("arrayJoin", Ty::String, vec![Ty::String]))
                    }
                    (Ty::Array, "shift") => Some(("arrayShift", Ty::Any, vec![])),
                    (Ty::Array, "unshift") => {
                        Some(("arrayUnshift", Ty::Int(IntKind::U64), vec![Ty::Any]))
                    }
                    // `at(index)` with relative/undefined semantics (native).
                    (Ty::Array, "at") => Some(("arrayAt", Ty::Any, vec![Ty::F64])),
                    (Ty::Array, "findLast") => Some((
                        "arrayFindLast",
                        Ty::Any,
                        vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
                    )),
                    (Ty::Array, "findLastIndex") => Some((
                        "arrayFindLastIndex",
                        Ty::Int(IntKind::I32),
                        vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
                    )),
                    (Ty::Array, "flatMap") => Some((
                        "arrayFlatMap",
                        Ty::Array,
                        vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
                    )),
                    (Ty::Array, "group") => Some((
                        "arrayGroup",
                        Ty::DynamicObject,
                        vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
                    )),
                    (Ty::Array, "groupToMap") => Some((
                        "arrayGroupToMap",
                        Ty::Map,
                        vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
                    )),
                    (Ty::Array, "keys") => Some(("arrayKeys", Ty::Any, vec![])),
                    (Ty::Array, "values") => Some(("arrayValues", Ty::Any, vec![])),
                    (Ty::Array, "entries") => Some(("arrayEntries", Ty::Any, vec![])),
                    (Ty::Map, "set") => Some(("mapSet", Ty::Void, vec![Ty::Any, Ty::Any])),
                    (Ty::Map, "get") => Some(("mapGet", Ty::Any, vec![Ty::Any])),
                    (Ty::Map, "has") => Some(("mapHas", Ty::Boolean, vec![Ty::Any])),
                    (Ty::Map, "size") => Some(("mapSize", Ty::Int(IntKind::U64), vec![])),
                    (Ty::Map, "delete") => Some(("mapDelete", Ty::Boolean, vec![Ty::Any])),
                    (Ty::Map, "clear") => Some(("mapClear", Ty::Void, vec![])),
                    (Ty::Map, "values") => Some(("mapValues", Ty::Any, vec![])),
                    (Ty::Map, "keys") => Some(("mapKeys", Ty::Any, vec![])),
                    (Ty::Map, "entries") => Some(("mapEntries", Ty::Any, vec![])),
                    (Ty::Map, "forEach") => Some((
                        "mapForEach",
                        Ty::Void,
                        vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
                    )),
                    (Ty::Set, "add") => Some(("setAdd", Ty::Boolean, vec![Ty::Any])),
                    (Ty::Set, "has") => Some(("setHas", Ty::Boolean, vec![Ty::Any])),
                    (Ty::Set, "size") => Some(("setSize", Ty::Int(IntKind::U64), vec![])),
                    (Ty::Set, "delete") => Some(("setDelete", Ty::Boolean, vec![Ty::Any])),
                    (Ty::Set, "clear") => Some(("setClear", Ty::Void, vec![])),
                    (Ty::Set, "values") => Some(("setValues", Ty::Any, vec![])),
                    (Ty::Set, "keys") => Some(("setValues", Ty::Any, vec![])),
                    (Ty::Set, "entries") => Some(("setEntries", Ty::Any, vec![])),
                    (Ty::Set, "forEach") => Some((
                        "setForEach",
                        Ty::Void,
                        vec![Ty::Function(hir::DYNAMIC_FUNCTION)],
                    )),
                    (Ty::WeakMap, "set") => {
                        Some(("weakMapSet", Ty::Boolean, vec![Ty::Any, Ty::Any]))
                    }
                    (Ty::WeakMap, "get") => Some(("weakMapGet", Ty::Any, vec![Ty::Any])),
                    (Ty::WeakMap, "has") => Some(("weakMapHas", Ty::Boolean, vec![Ty::Any])),
                    (Ty::WeakMap, "delete") => Some(("weakMapDelete", Ty::Boolean, vec![Ty::Any])),
                    (Ty::WeakSet, "add") => Some(("weakSetAdd", Ty::Boolean, vec![Ty::Any])),
                    (Ty::WeakSet, "has") => Some(("weakSetHas", Ty::Boolean, vec![Ty::Any])),
                    (Ty::WeakSet, "delete") => Some(("weakSetDelete", Ty::Boolean, vec![Ty::Any])),
                    (Ty::WeakRef, "deref") => Some(("weakRefDeref", Ty::Any, vec![])),
                    (Ty::RegExp, "isMatch" | "test") => Some(("regexpIsMatch", Ty::Boolean, vec![Ty::String])),
                    (Ty::RegExp, "exec") => Some(("regexpExec", Ty::Array, vec![Ty::String])),
                    (Ty::RegExp, "source") => Some(("regexpSource", Ty::String, vec![])),
                    (Ty::BigInt, "add") => Some(("bigIntAdd", Ty::BigInt, vec![Ty::BigInt])),
                    (Ty::BigInt, "sub") => Some(("bigIntSub", Ty::BigInt, vec![Ty::BigInt])),
                    (Ty::BigInt, "mul") => Some(("bigIntMul", Ty::BigInt, vec![Ty::BigInt])),
                    (Ty::BigInt, "div") => Some(("bigIntDiv", Ty::BigInt, vec![Ty::BigInt])),
                    (Ty::BigInt, "rem") => Some(("bigIntRem", Ty::BigInt, vec![Ty::BigInt])),
                    (Ty::BigInt, "pow") => Some(("bigIntPow", Ty::BigInt, vec![Ty::BigInt])),
                    (Ty::BigInt, "neg") => Some(("bigIntNeg", Ty::BigInt, vec![])),
                    (Ty::BigInt, "and") => Some(("bigIntAnd", Ty::BigInt, vec![Ty::BigInt])),
                    (Ty::BigInt, "or") => Some(("bigIntOr", Ty::BigInt, vec![Ty::BigInt])),
                    (Ty::BigInt, "xor") => Some(("bigIntXor", Ty::BigInt, vec![Ty::BigInt])),
                    (Ty::BigInt, "not") => Some(("bigIntNot", Ty::BigInt, vec![])),
                    (Ty::BigInt, "shl") => Some(("bigIntShl", Ty::BigInt, vec![Ty::BigInt])),
                    (Ty::BigInt, "shr") => Some(("bigIntShr", Ty::BigInt, vec![Ty::BigInt])),
                    (Ty::BigInt, "compare") => Some(("bigIntCompare", Ty::Int(IntKind::I32), vec![Ty::BigInt])),
                    (Ty::BigInt, "toString") => Some(("bigIntToString", Ty::String, vec![])),
                    (Ty::Promise, "state") => Some(("promiseState", Ty::Int(IntKind::U8), vec![])),
                    (Ty::Promise, "result") => Some(("promiseResult", Ty::Any, vec![])),
                    (Ty::Promise, "isReady") => Some(("promiseIsReady", Ty::Boolean, vec![])),
                    (Ty::Promise, "resolve") => Some(("promiseResolve", Ty::Boolean, vec![Ty::Any])),
                    (Ty::Promise, "reject") => Some(("promiseReject", Ty::Boolean, vec![Ty::String])),
                    (Ty::String, "toInt") => Some(("toInt", Ty::Int(IntKind::I32), vec![])),
                    (Ty::String, "toFloat") => Some(("toFloat", Ty::F64, vec![])),
                    (Ty::String, "toBoolean") => Some(("toBoolean", Ty::Boolean, vec![])),
                    (Ty::String, "reverse") => Some(("reverse", Ty::String, vec![])),
                    (Ty::String, "length") => Some(("length", Ty::Int(IntKind::U64), vec![])),
                    (Ty::String, "toUpperCase") => Some(("toUpperCase", Ty::String, vec![])),
                    (Ty::String, "toLowerCase") => Some(("toLowerCase", Ty::String, vec![])),
                    (Ty::String, "trim") => Some(("trim", Ty::String, vec![])),
                    (Ty::String, "trimStart") => Some(("trimStart", Ty::String, vec![])),
                    (Ty::String, "trimEnd") => Some(("trimEnd", Ty::String, vec![])),
                    (Ty::String, "charCodeAt") => {
                        Some(("charCodeAt", Ty::Int(IntKind::U32), vec![Ty::Int(IntKind::U64)]))
                    }
                    (Ty::String, "indexOf") => {
                        Some(("indexOf", Ty::Int(IntKind::I32), vec![Ty::String]))
                    }
                    (Ty::String, "lastIndexOf") => {
                        Some(("lastIndexOf", Ty::Int(IntKind::I32), vec![Ty::String]))
                    }
                    (Ty::String, "substring") => Some((
                        "substring",
                        Ty::String,
                        vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U64)],
                    )),
                    (Ty::String, "repeat") => {
                        Some(("repeat", Ty::String, vec![Ty::Int(IntKind::U64)]))
                    }
                    (Ty::String, "startsWith") => {
                        Some(("startsWith", Ty::Boolean, vec![Ty::String]))
                    }
                    (Ty::String, "endsWith") => Some(("endsWith", Ty::Boolean, vec![Ty::String])),
                    // ECMA-402 locale surface (NUCLEUS #1), ICU4X-backed.
                    // `localeCompare` returns the ECMAScript -1/0/1; v1 uses
                    // root collation and the root (`und`) locale for casing.
                    (Ty::String, "includes") => Some(("includes", Ty::Boolean, vec![Ty::String])),
                    // Index is a `number` so a negative value (from the end) can
                    // cross the boundary; out of range yields `undefined`.
                    (Ty::String, "at") => Some(("at", Ty::String, vec![Ty::F64])),
                    (Ty::String, "codePointAt") => {
                        Some(("codePointAt", Ty::Int(IntKind::U32), vec![Ty::Int(IntKind::U64)]))
                    }
                    (Ty::String, "replaceAll") => {
                        Some(("replaceAll", Ty::String, vec![Ty::String, Ty::String]))
                    }
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
                    // The remaining DataView element widths. JS names
                    // 64-bit access `getBigInt64`/`getBigUint64` and returns a
                    // BigInt; Zun has native 64-bit integers, so these return
                    // `i64`/`u64` and are named without the Big.
                    (Ty::BufferView(hir::BufferViewKind::DataView), "getInt8") => Some((
                        "dataViewGetInt8",
                        Ty::Int(IntKind::I8),
                        vec![Ty::Int(IntKind::U64)],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "setInt8") => Some((
                        "dataViewSetInt8",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::I8)],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "getUint8") => Some((
                        "dataViewGetUint8",
                        Ty::Int(IntKind::U8),
                        vec![Ty::Int(IntKind::U64)],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "setUint8") => Some((
                        "dataViewSetUint8",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U8)],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "getInt16") => Some((
                        "dataViewGetInt16",
                        Ty::Int(IntKind::I16),
                        vec![Ty::Int(IntKind::U64), Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "setInt16") => Some((
                        "dataViewSetInt16",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::I16), Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "getInt32") => Some((
                        "dataViewGetInt32",
                        Ty::Int(IntKind::I32),
                        vec![Ty::Int(IntKind::U64), Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "setInt32") => Some((
                        "dataViewSetInt32",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::I32), Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "getFloat32") => Some((
                        "dataViewGetFloat32",
                        Ty::F32,
                        vec![Ty::Int(IntKind::U64), Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "setFloat32") => Some((
                        "dataViewSetFloat32",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64), Ty::F32, Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "getInt64") => Some((
                        "dataViewGetInt64",
                        Ty::Int(IntKind::I64),
                        vec![Ty::Int(IntKind::U64), Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "setInt64") => Some((
                        "dataViewSetInt64",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::I64), Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "getUint64") => Some((
                        "dataViewGetUint64",
                        Ty::Int(IntKind::U64),
                        vec![Ty::Int(IntKind::U64), Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "setUint64") => Some((
                        "dataViewSetUint64",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U64), Ty::Boolean],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "byteLength") => Some((
                        "dataViewByteLength",
                        Ty::Int(IntKind::U64),
                        vec![],
                    )),
                    // `byteOffset`/`buffer`: the view's position in its
                    // backing buffer and a reference to that `ArrayBuffer`.
                    (Ty::BufferView(hir::BufferViewKind::DataView), "byteOffset") => Some((
                        "dataViewByteOffset",
                        Ty::Int(IntKind::U64),
                        vec![],
                    )),
                    (Ty::BufferView(hir::BufferViewKind::DataView), "buffer") => Some((
                        "dataViewBuffer",
                        Ty::Buffer,
                        vec![],
                    )),
                    // `ArrayBuffer`, which is what `Ty::Buffer` denotes. These
                    // are methods rather than the properties JS has, matching
                    // `String.length()` and `Map.size()` here; the divergence
                    // is recorded in NUCLEUS rather than special-cased.
                    (Ty::Buffer, "byteLength") => {
                        Some(("bufferLength", Ty::Int(IntKind::U64), vec![]))
                    }
                    (Ty::Buffer, "slice") => Some((
                        "bufferSlice",
                        Ty::Buffer,
                        vec![Ty::Int(IntKind::U64), Ty::Int(IntKind::U64)],
                    )),
                    // ES2024 resizable ArrayBuffer. As with `byteLength`, these
                    // JS properties are methods here (the divergence is in
                    // NUCLEUS). `resize` returns a boolean rather than JS's
                    // `undefined`; a rejected resize is a runtime error.
                    (Ty::Buffer, "maxByteLength") => {
                        Some(("bufferMaxByteLength", Ty::Int(IntKind::U64), vec![]))
                    }
                    (Ty::Buffer, "resizable") => {
                        Some(("bufferResizable", Ty::Boolean, vec![]))
                    }
                    (Ty::Buffer, "detached") => {
                        Some(("bufferDetached", Ty::Boolean, vec![]))
                    }
                    (Ty::Buffer, "resize") => Some((
                        "bufferResize",
                        Ty::Boolean,
                        vec![Ty::Int(IntKind::U64)],
                    )),
                    _ => None,
                });
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
                        "string"
                    } else if native_name.starts_with("dataView")
                        || native_name.starts_with("buffer")
                        // The typed-array natives, by the prefixes the master
                        // table gives them. Matching the method suffix
                        // instead (`ArrayGet`, `ArraySet`) missed
                        // `u32ArrayLength` and friends, which then resolved
                        // against `zun:collections` and were reported
                        // missing; a hand-written list of prefixes missed
                        // every kind added after it was written.
                        || crate::typed_arrays::NATIVE_PREFIXES
                            .iter()
                            .any(|prefix| native_name.starts_with(prefix))
                    {
                        "buffer"
                    } else if native_name.starts_with("promise") {
                        "promise"
                    } else if matches!(
                        native_name,
                        "arrayAt"
                            | "arrayFindLast"
                            | "arrayFindLastIndex"
                            | "arrayFlat"
                            | "arrayFlatMap"
                            | "arrayGroup"
                            | "arrayGroupToMap"
                            | "arrayKeys"
                            | "arrayValues"
                            | "arrayEntries"
                            | "arraySpliceInsert"
                            | "arrayReduceNoInit"
                            | "arrayReduceRightNoInit"
                            | "arrayIsArray"
                            | "arrayFrom"
                            | "arrayFromMapped"
                    ) {
                        "array"
                    } else {
                        "collections"
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
                if let Ty::Function(func) = receiver.ty {
                    return self.lower_function_reflective_call(
                        cx, receiver, &mname, func, args, span, method.span,
                    );
                }
                // JS-conformant `toFixed` / `toExponential` / `toPrecision` /
                // `toString(radix)`: the raw argument is passed as `any` and
                // coerced, range-checked (managed `RangeError`) and formatted
                // exactly on the interpreter floor.
                if receiver.ty == Ty::F64
                    && matches!(
                        mname.as_str(),
                        "toFixed" | "toExponential" | "toPrecision" | "toString"
                    )
                    && args.len() <= 1
                {
                    let native_name = match mname.as_str() {
                        "toFixed" => "numberToFixedJs",
                        "toExponential" => "numberToExponentialJs",
                        "toPrecision" => "numberToPrecisionJs",
                        _ => "numberToStringRadixJs",
                    };
                    let Some((id, _)) = zun_natives::resolve("number", native_name) else {
                        self.error(span, format!("missing Number intrinsic `{native_name}`"));
                        return self.poison(span);
                    };
                    let arg = match args.first() {
                        Some(arg) => {
                            let value = self.check_expr(cx, arg);
                            Self::box_to_any(value)
                        }
                        None => hir::Expr {
                            kind: hir::ExprKind::Unary {
                                op: ast::UnaryOp::Void,
                                operand: Box::new(hir::Expr {
                                    kind: hir::ExprKind::Number(0.0),
                                    ty: Ty::F64,
                                    span,
                                }),
                            },
                            ty: Ty::Any,
                            span,
                        },
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, arg] },
                        ty: Ty::String,
                        span,
                    };
                }
                // `number.toString(radix?)` — base-`radix` (2..=36) rendering,
                // default 10. Intercepted before class dispatch so it supersedes
                // the no-arg extern `toString` for the radix form and the
                // no-arg form alike.
                if receiver.ty == Ty::F64 && mname == "toString" && args.len() <= 1 {
                    let Some((id, _)) = zun_natives::resolve("number", "toStringRadix") else {
                        self.error(span, "missing Number intrinsic `toStringRadix`");
                        return self.poison(span);
                    };
                    let radix = match args.first() {
                        Some(arg) => {
                            let mut radix = self.check_expr(cx, arg);
                            coerce_numeric_literal(&mut radix, Ty::Int(IntKind::I32));
                            if !radix.ty.is_error()
                                && !self.is_assignable(radix.ty, Ty::Int(IntKind::I32))
                            {
                                self.error(radix.span, "`toString` radix must be an integer");
                            }
                            radix
                        }
                        None => hir::Expr {
                            kind: hir::ExprKind::Number(10.0),
                            ty: Ty::Int(IntKind::I32),
                            span,
                        },
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, radix] },
                        ty: Ty::String,
                        span,
                    };
                }
                // `number.toExponential(fractionDigits?)` /
                // `number.toPrecision(precision?)` — instance formatting,
                // intercepted before class dispatch like `toString(radix?)`.
                // The optional count is lowered to an out-of-range sentinel for
                // the native: `-1` ("absent") for `toExponential`, `0` for
                // `toPrecision`.
                if receiver.ty == Ty::F64
                    && matches!(mname.as_str(), "toExponential" | "toPrecision")
                    && args.len() <= 1
                {
                    let native = if mname == "toExponential" {
                        "toExponential"
                    } else {
                        "toPrecision"
                    };
                    let sentinel = if mname == "toExponential" { -1.0 } else { 0.0 };
                    let Some((id, _)) = zun_natives::resolve("number", native) else {
                        self.error(span, format!("missing Number intrinsic `{native}`"));
                        return self.poison(span);
                    };
                    let count = match args.first() {
                        Some(arg) => {
                            let mut count = self.check_expr(cx, arg);
                            coerce_numeric_literal(&mut count, Ty::Int(IntKind::I32));
                            if !count.ty.is_error()
                                && !self.is_assignable(count.ty, Ty::Int(IntKind::I32))
                            {
                                self.error(
                                    count.span,
                                    format!("`{mname}` argument must be an integer"),
                                );
                            }
                            count
                        }
                        None => hir::Expr {
                            kind: hir::ExprKind::Number(sentinel),
                            ty: Ty::Int(IntKind::I32),
                            span,
                        },
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, count] },
                        ty: Ty::String,
                        span,
                    };
                }
                // `number.toFixed(digits?)` — fixed-point formatting, digits
                // default 0 (range 0..=100 enforced by the native). Hand-lowered
                // before class dispatch like `toExponential`/`toPrecision`.
                if receiver.ty == Ty::F64 && mname == "toFixed" && args.len() <= 1 {
                    let Some((id, _)) = zun_natives::resolve("number", "toFixed") else {
                        self.error(span, "missing Number intrinsic `toFixed`");
                        return self.poison(span);
                    };
                    let digits = match args.first() {
                        Some(arg) => {
                            let mut digits = self.check_expr(cx, arg);
                            coerce_numeric_literal(&mut digits, Ty::Int(IntKind::I32));
                            if !digits.ty.is_error()
                                && !self.is_assignable(digits.ty, Ty::Int(IntKind::I32))
                            {
                                self.error(digits.span, "`toFixed` argument must be an integer");
                            }
                            digits
                        }
                        None => hir::Expr {
                            kind: hir::ExprKind::Number(0.0),
                            ty: Ty::Int(IntKind::I32),
                            span,
                        },
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver, digits] },
                        ty: Ty::String,
                        span,
                    };
                }
                // Optional trailing BCP-47 `locale` (ECMA-402) for the root-locale
                // `toLocale*` family. Hand-lowered because `managed_method` is
                // fixed-arity: an omitted locale is the sentinel `""` (root).
                if (receiver.ty == Ty::String
                    && matches!(mname.as_str(), "localeCompare" | "toLocaleLowerCase" | "toLocaleUpperCase"))
                    || (receiver.ty == Ty::F64 && mname == "toLocaleString")
                {
                    let (module, native, leading) = match (receiver.ty, mname.as_str()) {
                        (Ty::String, "localeCompare") => ("string", "localeCompare", 1usize),
                        (Ty::String, "toLocaleLowerCase") => ("string", "toLocaleLowerCase", 0),
                        (Ty::String, _) => ("string", "toLocaleUpperCase", 0),
                        _ => ("number", "intlNumberFormat", 0),
                    };
                    if args.len() < leading || args.len() > leading + 1 {
                        self.error(
                            span,
                            format!("`{mname}` expects {leading} or {} argument(s), found {}", leading + 1, args.len()),
                        );
                        return self.poison(span);
                    }
                    let mut call_args = Vec::new();
                    if leading == 1 {
                        let that = self.check_expr(cx, &args[0]);
                        if !that.ty.is_error() && that.ty != Ty::String {
                            self.error(that.span, "`localeCompare` argument must be a string");
                        }
                        call_args.push(that);
                    }
                    let locale = match args.get(leading) {
                        Some(arg) => {
                            let checked = self.check_expr(cx, arg);
                            if !checked.ty.is_error() && checked.ty != Ty::String {
                                self.error(checked.span, format!("`{mname}` locale must be a string"));
                            }
                            checked
                        }
                        None => hir::Expr {
                            kind: hir::ExprKind::String(String::new()),
                            ty: Ty::String,
                            span,
                        },
                    };
                    let Some((id, _)) = zun_natives::resolve(module, native) else {
                        self.error(span, format!("missing intrinsic `{native}`"));
                        return self.poison(span);
                    };
                    // `intlNumberFormat(locale, n)` takes the locale first; the
                    // string natives take `(receiver, [that,] locale)`.
                    let mut all = Vec::new();
                    if receiver.ty == Ty::F64 {
                        all.push(locale);
                        all.push(receiver);
                    } else {
                        all.push(receiver);
                        all.extend(call_args);
                        all.push(locale);
                    }
                    let ty = if native == "localeCompare" { Ty::Int(IntKind::I32) } else { Ty::String };
                    return hir::Expr { kind: hir::ExprKind::CallBuiltin { id, args: all }, ty, span };
                }
                // `number.valueOf()` / `string.valueOf()` — identity, returning
                // the primitive itself. No native needed: the checked receiver
                // expression already is the value.
                if matches!(receiver.ty, Ty::F64 | Ty::String | Ty::BigInt)
                    && mname == "valueOf"
                    && args.is_empty()
                {
                    return receiver;
                }
                // `String.prototype.matchAll(regexp)` — all matches of a global
                // regexp as an array of match arrays (each like `exec`). The `g`
                // flag is required (checked natively); the array-of-arrays shape
                // is the same pragmatic choice as `keys()/values()/entries()`
                // (no distinct iterator object yet).
                if receiver.ty == Ty::String && mname == "matchAll" && args.len() == 1 {
                    let first = self.check_expr(cx, &args[0]);
                    if first.ty != Ty::RegExp && !first.ty.is_error() {
                        self.error(first.span, "`matchAll` expects a RegExp argument");
                        return self.poison(span);
                    }
                    let Some((id, _)) = zun_natives::resolve("collections", "stringMatchAll") else {
                        self.error(span, "missing collection intrinsic `stringMatchAll`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![first, receiver] },
                        ty: Ty::Array,
                        span,
                    };
                }
                // `boolean.toString()` → `"true"` / `"false"`. Intercepted
                // before class dispatch, like `number.toString(radix?)`, since
                // the primitive `boolean` has no backing class.
                if receiver.ty == Ty::Boolean && mname == "toString" && args.is_empty() {
                    let Some((id, _)) = zun_natives::resolve("boolean", "booleanToString") else {
                        self.error(span, "missing Boolean intrinsic `booleanToString`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver] },
                        ty: Ty::String,
                        span,
                    };
                }
                // String/Number/Array are primitives with no concatenated class
                // (see PRELUDE_CLASSES): resolve a method not already handled by
                // the tables above directly against its native group by name —
                // covering every native method without re-listing each.
                let primitive_group = match receiver.ty {
                    Ty::String => Some(("string", "String")),
                    Ty::F64 => Some(("number", "Number")),
                    Ty::Array => Some(("array", "Array")),
                    _ => None,
                };
                if receiver.ty == Ty::Array
                    && matches!(
                        mname.as_str(),
                        "slice" | "indexOf" | "lastIndexOf" | "includes" | "reverse"
                    )
                    && !self.classes_by_name.contains_key("Array")
                {
                    if mname == "reverse" {
                        let Some((id, _)) = zun_natives::resolve("array", "arrayReverse") else {
                            self.error(span, "missing array intrinsic `arrayReverse`");
                            return self.poison(span);
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: vec![receiver] },
                            ty: Ty::Array,
                            span,
                        };
                    }
                    let native_name = match mname.as_str() {
                        "slice" => "arraySliceRange",
                        "indexOf" => "arrayIndexOf",
                        "lastIndexOf" => "arrayLastIndexOf",
                        _ => "arrayIncludes",
                    };
                    let Some((id, native)) = zun_natives::resolve("array", native_name) else {
                        self.error(span, format!("missing array intrinsic `{native_name}`"));
                        return self.poison(span);
                    };
                    let is_slice = mname == "slice";
                    let max_args = if is_slice { 2 } else { 2 };
                    let min_args = if is_slice { 0 } else { 1 };
                    if args.len() < min_args || args.len() > max_args {
                        self.error(span, format!("`{mname}` called with a wrong number of arguments"));
                        return self.poison(span);
                    }
                    let mut call_args = vec![receiver];
                    let mut rest = args.iter();
                    if !is_slice {
                        let needle = self.check_expr(cx, rest.next().expect("needle"));
                        call_args.push(Self::box_to_any(needle));
                    }
                    let defaults = if is_slice {
                        [0.0, f64::INFINITY]
                    } else if mname == "lastIndexOf" {
                        [f64::INFINITY, 0.0]
                    } else {
                        [0.0, 0.0]
                    };
                    let wanted = native.params.len() - call_args.len();
                    for position in 0..wanted {
                        let value = match rest.next() {
                            Some(arg) => {
                                let mut value = self.check_expr(cx, arg);
                                coerce_numeric_literal(&mut value, Ty::F64);
                                self.to_number_for_arg(value)
                            }
                            None => hir::Expr {
                                kind: hir::ExprKind::Number(defaults[position]),
                                ty: Ty::F64,
                                span,
                            },
                        };
                        call_args.push(value);
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                        ty: native_ty(native.ret),
                        span,
                    };
                }
                if let Some((group, class_name)) = primitive_group {
                    if !self.classes_by_name.contains_key(class_name) {
                        // `string`/`number` natives are bare (`toUpperCase`);
                        // `array` natives carry an `array` prefix (`arraySlice`).
                        let native_name = if group == "array" {
                            let mut chars = mname.chars();
                            match chars.next() {
                                Some(first) => format!("array{}{}", first.to_ascii_uppercase(), chars.as_str()),
                                None => mname.clone(),
                            }
                        } else {
                            mname.clone()
                        };
                        if let Some((id, native)) = zun_natives::resolve(group, &native_name) {
                            if native.params.len() == args.len() + 1 {
                                let mut call_args = vec![receiver];
                                for (arg, expected) in args
                                    .iter()
                                    .zip(native.params[1..].iter().map(|t| native_ty(*t)))
                                {
                                    let mut value = self.check_expr(cx, arg);
                                    coerce_numeric_literal(&mut value, expected);
                                    call_args.push(value);
                                }
                                return hir::Expr {
                                    kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                                    ty: native_ty(native.ret),
                                    span,
                                };
                            }
                        }
                        self.error(
                            method.span,
                            format!("no method `{mname}` on {class_name}"),
                        );
                        return self.poison(span);
                    }
                }
                let class = match receiver.ty {
                    Ty::String => self.classes_by_name["String"],
                    Ty::F64 => self.classes_by_name["Number"],
                    Ty::Array => self.classes_by_name["Array"],
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
                if rest_index.is_none() {
                    if let Some(&owner) = self.routed_methods.get(&func) {
                        if let Some(routed) =
                            self.routed_method_call(func, owner, &mut call_args, return_ty, span)
                        {
                            return routed;
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
                if let Some(&namespace) = self.core_namespaces.get(&class_name) {
                    let Some((id, native)) = zun_natives::resolve_core(namespace, self.text(method)) else {
                        self.error(method.span, format!("`zun:core` namespace `{}` has no member `{}`", namespace, self.text(method)));
                        return self.poison(span);
                    };
                    if native.params.len() != args.len() {
                        self.error(span, format!("`{}` expects {} argument(s), found {}", self.text(method), native.params.len(), args.len()));
                        return self.poison(span);
                    }
                    let mut checked = Vec::with_capacity(args.len());
                    for (arg, expected) in args.iter().zip(native.params.iter().map(|ty| native_ty(*ty))) {
                        let mut value = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut value, expected);
                        if !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                            self.error(value.span, format!("argument type {:?} does not match {expected:?}", value.ty));
                        }
                        checked.push(value);
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: checked },
                        ty: native_ty(native.ret),
                        span,
                    };
                }
                if let Some(module) = self.namespace_imports.get(&class_name).cloned() {
                    let Some((id, native)) = zun_natives::resolve(&module, self.text(method)) else {
                        self.error(method.span, format!("`{}` has no export named `{}`", module, self.text(method)));
                        return self.poison(span);
                    };
                    if native.params.len() != args.len() {
                        self.error(span, format!("`{}` expects {} argument(s), found {}", self.text(method), native.params.len(), args.len()));
                        return self.poison(span);
                    }
                    let mut checked = Vec::with_capacity(args.len());
                    for (arg, expected) in args.iter().zip(native.params.iter().map(|ty| native_ty(*ty))) {
                        let mut value = self.check_expr(cx, arg);
                        coerce_numeric_literal(&mut value, expected);
                        if !value.ty.is_error() && !self.is_assignable(value.ty, expected) {
                            self.error(value.span, format!("argument type {:?} does not match {expected:?}", value.ty));
                        }
                        checked.push(value);
                    }
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: checked },
                        ty: native_ty(native.ret),
                        span,
                    };
                }
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
                    let mut value = self.check_expr(cx, arg);
                    // Instance and free calls narrow a bare numeric literal to
                    // the parameter type; static calls did not, so
                    // `Date.UTC(2020, 0, 1, ...)` failed on literals that the
                    // same arguments would accept through any other call
                    // shape.
                    coerce_numeric_literal(&mut value, expected);
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
                // `x as any` is a widening assertion: box a concrete value into
                // the dynamic carrier (a value already `any` passes through).
                if target == Ty::Any {
                    return if value.ty == Ty::Any || value.ty.is_error() {
                        value
                    } else {
                        hir::Expr {
                            span,
                            kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                            ty: Ty::Any,
                        }
                    };
                }
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
            ast::Expr::NewDynamic { callee, args, .. } => {
                // `new (expr)(...args)`: any expression of proxy type is
                // evaluated to a value, then routed through `proxyConstruct`.
                // Design boundary: only class targets are constructible
                // (Zun has typed classes, no `function.prototype` + `this`
                // construction model), and `newTarget` is approximated by the
                // target (subclass newTarget selection needs a prototype model).
                let callee = self.check_expr(cx, callee);
                if callee.ty != Ty::Proxy {
                    self.error(span, "`new (expr)(...)` requires a Proxy-typed expression");
                    return self.poison(span);
                }
                let elements: Vec<Option<hir::Expr>> =
                    args.iter().map(|a| Some(self.check_expr(cx, a))).collect();
                let args_array = hir::Expr {
                    kind: hir::ExprKind::ArrayLiteral { elements },
                    ty: Ty::Array,
                    span,
                };
                let Some((id, _)) = zun_natives::resolve("collections", "proxyConstruct") else {
                    self.error(span, "missing `proxyConstruct` intrinsic");
                    return self.poison(span);
                };
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin { id, args: vec![callee, args_array] },
                    ty: Ty::Any,
                    span,
                };
            }
            ast::Expr::New { class, args, .. } => {
                let name = self.text(class).to_string();
                if name == "Function" && !self.classes_by_name.contains_key(&name) {
                    return self.lower_function_construct(cx, args, span);
                }
                // `new ns.C(...args)` where `ns` is a dynamic (`any`) local
                // holding a module namespace from `import()`: construct the class
                // export cross-module at run time. The parser collapses `ns.C`
                // into one dotted class name, so split on the last `.` and check
                // the base is a dynamic local (a known class/realm global takes
                // precedence and is handled below).
                if !self.classes_by_name.contains_key(&name)
                    && !self.realm_globals.contains_key(&name)
                {
                    if let Some(dot) = name.rfind('.') {
                        let base = &name[..dot];
                        let member = name[dot + 1..].to_string();
                        let base_is_dynamic = cx
                            .lookup(base)
                            .is_some_and(|slot| matches!(cx.locals[slot as usize], Ty::Any | Ty::DynamicObject));
                        if base_is_dynamic {
                            let base_name = ast::Name {
                                span: Span::new(class.span.start, class.span.start + base.len() as u32),
                            };
                            let base_expr = self.check_expr(cx, &ast::Expr::Ident(base_name));
                            let member_expr = hir::Expr {
                                kind: hir::ExprKind::String(member),
                                ty: Ty::String,
                                span: class.span,
                            };
                            let elements: Vec<Option<hir::Expr>> =
                                args.iter().map(|a| Some(self.check_expr(cx, a))).collect();
                            let args_array = hir::Expr {
                                kind: hir::ExprKind::ArrayLiteral { elements },
                                ty: Ty::Array,
                                span,
                            };
                            let Some((id, _)) = zun_natives::resolve("collections", "dynamicConstruct")
                            else {
                                self.error(span, "missing `dynamicConstruct` intrinsic");
                                return self.poison(span);
                            };
                            return hir::Expr {
                                kind: hir::ExprKind::CallBuiltin {
                                    id,
                                    args: vec![base_expr, member_expr, args_array],
                                },
                                ty: Ty::Any,
                                span,
                            };
                        }
                    }
                }
                // `new proxy(...args)`: a local of proxy type routes through
                // `proxyConstruct` (the handler's `construct` trap, else the
                // wrapped class's default construction).
                if !self.classes_by_name.contains_key(&name)
                    && !self.realm_globals.contains_key(&name)
                    && cx.lookup(&name).is_some_and(|slot| cx.locals[slot as usize] == Ty::Proxy)
                {
                    let callee = self.check_expr(cx, &ast::Expr::Ident(class.clone()));
                    let elements: Vec<Option<hir::Expr>> =
                        args.iter().map(|a| Some(self.check_expr(cx, a))).collect();
                    let args_array = hir::Expr {
                        kind: hir::ExprKind::ArrayLiteral { elements },
                        ty: Ty::Array,
                        span,
                    };
                    let Some((id, _)) = zun_natives::resolve("collections", "proxyConstruct") else {
                        self.error(span, "missing `proxyConstruct` intrinsic");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![callee, args_array] },
                        ty: Ty::Any,
                        span,
                    };
                }
                // Declarative realm globals (PLAN §3.1): a registered base type
                // lowers `new <Name>()` through its recorded constructor shape.
                // On a miss we fall through to the hardcoded paths unchanged.
                if let Some(binding) = self.realm_globals.get(&name) {
                    use crate::realm::CtorLowering;
                    match binding.ctor.clone() {
                        CtorLowering::Intrinsic0 { module, native, ty } => {
                            if !args.is_empty() {
                                self.error(span, format!("`{name}` expects no constructor arguments"));
                                return self.poison(span);
                            }
                            let Some((id, _)) = zun_natives::resolve(module, native) else {
                                self.error(span, format!("missing intrinsic `{native}`"));
                                return self.poison(span);
                            };
                            return hir::Expr {
                                kind: hir::ExprKind::CallBuiltin { id, args: Vec::new() },
                                ty,
                                span,
                            };
                        }
                        CtorLowering::Promise => {
                            // `new Promise()` → an empty pending promise;
                            // `new Promise(executor)` → create the promise plus
                            // its resolve/reject settlers and synchronously run
                            // the executor (reusing the `withResolvers` settler
                            // mechanism in the interpreter floor).
                            match args.len() {
                                0 => {
                                    let Some((id, _)) = zun_natives::resolve("promise", "promiseNew") else {
                                        self.error(span, "missing Promise intrinsic `promiseNew`");
                                        return self.poison(span);
                                    };
                                    return hir::Expr {
                                        kind: hir::ExprKind::CallBuiltin { id, args: Vec::new() },
                                        ty: Ty::Promise,
                                        span,
                                    };
                                }
                                1 => {
                                    // Contextual typing: the executor receives
                                    // the `resolve`/`reject` settlers, so an
                                    // unannotated `(res, rej) => …` gets both
                                    // params typed `Function` (callable) instead
                                    // of the `F64` default — this is what makes
                                    // the textbook `new Promise((res) =>
                                    // setTimeout(() => res(1), 0))` form work.
                                    self.closure_param_hint = Some(vec![
                                        Ty::Function(hir::DYNAMIC_FUNCTION),
                                        Ty::Function(hir::DYNAMIC_FUNCTION),
                                    ]);
                                    let executor = self.check_expr(cx, &args[0]);
                                    self.closure_param_hint = None;
                                    if !executor.ty.is_error()
                                        && !self.is_assignable(
                                            executor.ty,
                                            Ty::Function(hir::DYNAMIC_FUNCTION),
                                        )
                                    {
                                        self.error(
                                            executor.span,
                                            "`new Promise` expects an executor function",
                                        );
                                    }
                                    let Some((id, _)) = zun_natives::resolve("promise", "promiseNewExecutor") else {
                                        self.error(span, "missing Promise intrinsic `promiseNewExecutor`");
                                        return self.poison(span);
                                    };
                                    return hir::Expr {
                                        kind: hir::ExprKind::CallBuiltin { id, args: vec![executor] },
                                        ty: Ty::Promise,
                                        span,
                                    };
                                }
                                _ => {
                                    self.error(
                                        span,
                                        format!(
                                            "`new Promise` expects at most one executor argument, found {}",
                                            args.len()
                                        ),
                                    );
                                    return self.poison(span);
                                }
                            }
                        }
                        CtorLowering::CollectionFrom { module, empty, from, from_iterable, ty } => {
                            // `new Map()`/`new Set()` → the empty allocator;
                            // `new Map(array)`/`new Set(array)` → the array fast
                            // path (`from`); any other built-in iterable (another
                            // Map/Set, a string, a typed array) → `from_iterable`,
                            // which dispatches on the runtime kind. Arbitrary user
                            // `[Symbol.iterator]` objects are deferred.
                            match args.len() {
                                0 => {
                                    let Some((id, _)) = zun_natives::resolve(module, empty) else {
                                        self.error(span, format!("missing intrinsic `{empty}`"));
                                        return self.poison(span);
                                    };
                                    return hir::Expr {
                                        kind: hir::ExprKind::CallBuiltin { id, args: Vec::new() },
                                        ty,
                                        span,
                                    };
                                }
                                1 => {
                                    let argument = self.check_expr(cx, &args[0]);
                                    // An array keeps the dedicated fast path; a
                                    // known built-in iterable or an `any` value
                                    // routes to the general native.
                                    let native = if self.is_assignable(argument.ty, Ty::Array) {
                                        from
                                    } else if Self::is_iterable_ty(argument.ty) {
                                        from_iterable
                                    } else {
                                        if !argument.ty.is_error() {
                                            self.error(
                                                argument.span,
                                                format!(
                                                    "`{name}` constructor argument must be iterable, \
                                                     found {:?}",
                                                    argument.ty
                                                ),
                                            );
                                        }
                                        from
                                    };
                                    let Some((id, _)) = zun_natives::resolve(module, native) else {
                                        self.error(span, format!("missing intrinsic `{native}`"));
                                        return self.poison(span);
                                    };
                                    return hir::Expr {
                                        kind: hir::ExprKind::CallBuiltin { id, args: vec![argument] },
                                        ty,
                                        span,
                                    };
                                }
                                _ => {
                                    self.error(
                                        span,
                                        format!("`{name}` expects at most one constructor argument, found {}", args.len()),
                                    );
                                    return self.poison(span);
                                }
                            }
                        }
                        CtorLowering::IntrinsicN { module, native, ty, min_args, max_args, pad_empty_flags } => {
                            if args.len() < min_args || args.len() > max_args {
                                self.error(
                                    span,
                                    format!("`{name}` expects {min_args} to {max_args} constructor argument(s), found {}", args.len()),
                                );
                                return self.poison(span);
                            }
                            let Some((id, _)) = zun_natives::resolve(module, native) else {
                                self.error(span, format!("missing intrinsic `{native}`"));
                                return self.poison(span);
                            };
                            let mut checked: Vec<hir::Expr> =
                                args.iter().map(|arg| self.check_expr(cx, arg)).collect();
                            if pad_empty_flags && checked.len() == 1 {
                                checked.push(hir::Expr {
                                    kind: hir::ExprKind::String(String::new()),
                                    ty: Ty::String,
                                    span,
                                });
                            }
                            return hir::Expr {
                                kind: hir::ExprKind::CallBuiltin { id, args: checked },
                                ty,
                                span,
                            };
                        }
                        CtorLowering::ArrayNew => {
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
                                kind: hir::ExprKind::ArrayNew { length: Box::new(length) },
                                ty: Ty::Array,
                                span,
                            };
                        }
                        CtorLowering::ArrayBuffer => {
                            return self.check_array_buffer_new(cx, args, span);
                        }
                        CtorLowering::BufferView { native, kind } => {
                            // `new DataView(buffer)` / `new DataView(buffer,
                            // offset)` with no explicit length is
                            // length-tracking (ES2024): its `byteLength`
                            // follows the buffer's current length.
                            if matches!(kind, hir::BufferViewKind::DataView)
                                && (args.len() == 1 || args.len() == 2)
                            {
                                let buffer = self.check_expr(cx, &args[0]);
                                let offset = match args.get(1) {
                                    Some(arg) => {
                                        let mut offset = self.check_expr(cx, arg);
                                        coerce_numeric_literal(&mut offset, Ty::INT);
                                        offset
                                    }
                                    None => hir::Expr {
                                        kind: hir::ExprKind::Number(0.0),
                                        ty: Ty::Int(IntKind::U64),
                                        span,
                                    },
                                };
                                let Some((id, _)) =
                                    zun_natives::resolve("buffer", "dataViewTrackingNew")
                                else {
                                    self.error(span, "missing DataView intrinsic `dataViewTrackingNew`");
                                    return self.poison(span);
                                };
                                return hir::Expr {
                                    kind: hir::ExprKind::CallBuiltin { id, args: vec![buffer, offset] },
                                    ty: Ty::BufferView(kind),
                                    span,
                                };
                            }
                            // `new Uint8Array(n)` — a single NUMERIC argument is
                            // an element count: allocate a fresh zero-filled
                            // buffer of `n * elementSize` bytes and return a
                            // length-tracking view over it (not a view onto an
                            // existing buffer). A non-numeric single argument
                            // (a buffer) falls through to the view path below.
                            if !matches!(kind, hir::BufferViewKind::DataView) && args.len() == 1 {
                                let mut length = self.check_expr(cx, &args[0]);
                                // `new Float64Array([1, 2, 3])` copies an array (or
                                // another typed array) like `<Kind>Array.from`.
                                if matches!(length.ty, Ty::Array | Ty::BufferView(_)) {
                                    if let (Some(discriminant), Some((id, _))) = (
                                        crate::typed_arrays::element_index(kind),
                                        zun_natives::resolve("buffer", "typedArrayFrom"),
                                    ) {
                                        let kind_lit = hir::Expr {
                                            kind: hir::ExprKind::Number(discriminant as f64),
                                            ty: Ty::Int(IntKind::U32),
                                            span,
                                        };
                                        return hir::Expr {
                                            kind: hir::ExprKind::CallBuiltin {
                                                id,
                                                args: vec![length, kind_lit],
                                            },
                                            ty: Ty::BufferView(kind),
                                            span,
                                        };
                                    }
                                }
                                if length.ty.is_int() || matches!(length.ty, Ty::Float(_)) {
                                    coerce_numeric_literal(&mut length, Ty::INT);
                                    let elem = crate::typed_arrays::element_size(kind).unwrap_or(1);
                                    let byte_len = if elem == 1 {
                                        length
                                    } else {
                                        hir::Expr {
                                            kind: hir::ExprKind::Binary {
                                                op: hir::BinaryOp::Mul,
                                                lhs: Box::new(length),
                                                rhs: Box::new(hir::Expr {
                                                    kind: hir::ExprKind::Number(elem as f64),
                                                    ty: Ty::Int(IntKind::U64),
                                                    span,
                                                }),
                                            },
                                            ty: Ty::Int(IntKind::U64),
                                            span,
                                        }
                                    };
                                    let Some((buf_id, _)) = zun_natives::resolve("buffer", "bufferNew") else {
                                        self.error(span, "missing buffer intrinsic `bufferNew`");
                                        return self.poison(span);
                                    };
                                    let buffer = hir::Expr {
                                        kind: hir::ExprKind::CallBuiltin { id: buf_id, args: vec![byte_len] },
                                        ty: Ty::Buffer,
                                        span,
                                    };
                                    let zero = hir::Expr {
                                        kind: hir::ExprKind::Number(0.0),
                                        ty: Ty::Int(IntKind::U64),
                                        span,
                                    };
                                    let kind_lit = hir::Expr {
                                        kind: hir::ExprKind::Number(
                                            crate::typed_arrays::element_index(kind).unwrap_or(0) as f64,
                                        ),
                                        ty: Ty::Int(IntKind::U32),
                                        span,
                                    };
                                    let Some((id, _)) =
                                        zun_natives::resolve("buffer", "typedArrayTrackingNew")
                                    else {
                                        self.error(span, "missing typed-array intrinsic `typedArrayTrackingNew`");
                                        return self.poison(span);
                                    };
                                    return hir::Expr {
                                        kind: hir::ExprKind::CallBuiltin {
                                            id,
                                            args: vec![buffer, zero, kind_lit],
                                        },
                                        ty: Ty::BufferView(kind),
                                        span,
                                    };
                                }
                            }
                            // A typed array (not `DataView`) created with the
                            // length omitted (`new Uint8Array(buffer)` or
                            // `… (buffer, offset)`) is a length-tracking view
                            // over a resizable buffer (ES2024): its element
                            // count follows the buffer's current length.
                            // `DataView` keeps requiring an explicit length;
                            // length-tracking DataViews are a documented
                            // follow-up.
                            let element_index = crate::typed_arrays::element_index(kind);
                            if args.len() < 3 && element_index.is_some() {
                                if args.is_empty() {
                                    self.error(span, format!("`{name}` expects at least a buffer"));
                                    return self.poison(span);
                                }
                                let buffer = self.check_expr(cx, &args[0]);
                                let offset = match args.get(1) {
                                    Some(arg) => {
                                        let mut offset = self.check_expr(cx, arg);
                                        coerce_numeric_literal(&mut offset, Ty::INT);
                                        offset
                                    }
                                    None => hir::Expr {
                                        kind: hir::ExprKind::Number(0.0),
                                        ty: Ty::Int(IntKind::U64),
                                        span,
                                    },
                                };
                                let kind_lit = hir::Expr {
                                    kind: hir::ExprKind::Number(element_index.unwrap() as f64),
                                    ty: Ty::Int(IntKind::U32),
                                    span,
                                };
                                let Some((id, _)) =
                                    zun_natives::resolve("buffer", "typedArrayTrackingNew")
                                else {
                                    self.error(span, "missing typed-array intrinsic `typedArrayTrackingNew`");
                                    return self.poison(span);
                                };
                                return hir::Expr {
                                    kind: hir::ExprKind::CallBuiltin {
                                        id,
                                        args: vec![buffer, offset, kind_lit],
                                    },
                                    ty: Ty::BufferView(kind),
                                    span,
                                };
                            }
                            if args.len() != 3 {
                                self.error(span, format!("`{name}` expects buffer, offset, and length"));
                                return self.poison(span);
                            }
                            let mut call_args: Vec<hir::Expr> =
                                args.iter().map(|arg| self.check_expr(cx, arg)).collect();
                            for arg in call_args.iter_mut().skip(1) {
                                coerce_numeric_literal(arg, Ty::INT);
                            }
                            let Some((id, _)) = zun_natives::resolve("buffer", native) else {
                                self.error(span, format!("missing buffer-view intrinsic `{native}`"));
                                return self.poison(span);
                            };
                            return hir::Expr {
                                kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                                ty: Ty::BufferView(kind),
                                span,
                            };
                        }
                        // The registry owns this name's annotation and descriptor
                        // value; `new <Name>()` keeps using the existing path.
                        // `ClassFallThrough` (Error + subclasses) likewise falls
                        // through to the class-construction path below.
                        CtorLowering::FallThrough | CtorLowering::ClassFallThrough => {}
                    }
                }
                let typed_view = match name.as_str() {
                    "Uint8Array" => Some("u8ArrayNew"),
                    "Uint16Array" => Some("u16ArrayNew"),
                    "Uint32Array" => Some("u32ArrayNew"),
                    "Float64Array" => Some("f64ArrayNew"),
                    _ => None,
                };
                if name == "ArrayBuffer" {
                    return self.check_array_buffer_new(cx, args, span);
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
                    let Some((id, _)) = zun_natives::resolve("buffer", "dataViewNew") else {
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
                    let view_kind = match name.as_str() {
                        "Uint8Array" => hir::BufferViewKind::U8,
                        "Uint16Array" => hir::BufferViewKind::U16,
                        "Uint32Array" => hir::BufferViewKind::U32,
                        "Float64Array" => hir::BufferViewKind::F64,
                        _ => unreachable!(),
                    };
                    // `new <Kind>Array(buffer, offset, length)` is a
                    // fixed-length view. With the length omitted (`(buffer)` or
                    // `(buffer, offset)`) it is a **length-tracking** view over
                    // a resizable buffer (ES2024): its element count follows the
                    // buffer's current length. Tracking routes to a
                    // kind-agnostic native carrying the element discriminant.
                    if args.is_empty() || args.len() > 3 {
                        self.error(
                            span,
                            format!("`{name}` expects a buffer and an optional offset and length"),
                        );
                        return self.poison(span);
                    }
                    if args.len() < 3 {
                        let buffer = self.check_expr(cx, &args[0]);
                        let offset = match args.get(1) {
                            Some(arg) => {
                                let mut offset = self.check_expr(cx, arg);
                                coerce_numeric_literal(&mut offset, Ty::INT);
                                offset
                            }
                            None => hir::Expr {
                                kind: hir::ExprKind::Number(0.0),
                                ty: Ty::Int(IntKind::U64),
                                span,
                            },
                        };
                        let Some(discriminant) = crate::typed_arrays::element_index(view_kind)
                        else {
                            self.error(span, "typed array requires an element kind".to_string());
                            return self.poison(span);
                        };
                        let kind_lit = hir::Expr {
                            kind: hir::ExprKind::Number(discriminant as f64),
                            ty: Ty::Int(IntKind::U32),
                            span,
                        };
                        let Some((id, _)) = zun_natives::resolve("buffer", "typedArrayTrackingNew")
                        else {
                            self.error(span, "missing typed-array intrinsic `typedArrayTrackingNew`");
                            return self.poison(span);
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id,
                                args: vec![buffer, offset, kind_lit],
                            },
                            ty: Ty::BufferView(view_kind),
                            span,
                        };
                    }
                    let mut call_args: Vec<hir::Expr> =
                        args.iter().map(|arg| self.check_expr(cx, arg)).collect();
                    for arg in call_args.iter_mut().skip(1) {
                        coerce_numeric_literal(arg, Ty::INT);
                    }
                    let Some((id, _)) = zun_natives::resolve("buffer", native_name) else {
                        self.error(
                            span,
                            format!("missing typed-array intrinsic `{native_name}`"),
                        );
                        return self.poison(span);
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
                // `BigInt(x)` coercion. A string argument parses (decimal or a
                // `0x`/`0o`/`0b` prefix); a number argument must be
                // integer-valued (non-integers are a runtime RangeError).
                if name == "BigInt" {
                    if args.len() != 1 {
                        self.error(span, "`BigInt` expects one argument");
                        return self.poison(span);
                    }
                    let arg = self.check_expr(cx, &args[0]);
                    let native_name = if arg.ty.is_int() || arg.ty.is_float() {
                        "bigIntFromNumber"
                    } else if arg.ty == Ty::String {
                        "bigIntNew"
                    } else if arg.ty.is_error() {
                        return self.poison(span);
                    } else {
                        self.error(arg.span, "`BigInt` expects a string or number argument");
                        return self.poison(span);
                    };
                    let Some((id, _)) = zun_natives::resolve("collections", native_name) else {
                        self.error(span, format!("missing collection intrinsic `{native_name}`"));
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                        ty: Ty::BigInt,
                        span,
                    };
                }
                // `new Proxy(target, handler)`: a proxy over a *callable* target
                // boxes the function into a GC record (functions do not box into
                // `Any`) and is built with `proxyNewCallable`, so the `apply`
                // trap and direct `proxy(...)` calls can recover the function; a
                // proxy over data uses the plain `proxyNew`.
                if name == "Proxy" {
                    if args.len() != 2 {
                        self.error(span, "`Proxy` expects a target and a handler");
                        return self.poison(span);
                    }
                    let target = self.check_expr(cx, &args[0]);
                    let handler = self.check_expr(cx, &args[1]);
                    // A class descriptor target is boxed into a GC record (`classBox`)
                    // so the proxy can hold it and `new proxy(...)` can recover it.
                    let target = if matches!(target.kind, hir::ExprKind::CallBuiltin { id, .. }
                        if zun_natives::resolve("collections", "realmDescriptor")
                            .map(|(rid, _)| rid) == Some(id))
                    {
                        let Some((box_id, _)) = zun_natives::resolve("collections", "classBox")
                        else {
                            self.error(span, "missing `classBox` intrinsic");
                            return self.poison(span);
                        };
                        let target_span = target.span;
                        hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id: box_id, args: vec![target] },
                            ty: Ty::DynamicObject,
                            span: target_span,
                        }
                    } else {
                        target
                    };
                    let (native_name, target) = if matches!(target.ty, Ty::Function(_)) {
                        let Some((box_id, _)) = zun_natives::resolve("collections", "functionBox")
                        else {
                            self.error(span, "missing `functionBox` intrinsic");
                            return self.poison(span);
                        };
                        let target_span = target.span;
                        let boxed = hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id: box_id, args: vec![target] },
                            ty: Ty::Any,
                            span: target_span,
                        };
                        ("proxyNewCallable", boxed)
                    } else {
                        ("proxyNew", target)
                    };
                    let Some((id, _)) = zun_natives::resolve("collections", native_name) else {
                        self.error(span, format!("missing collection intrinsic `{native_name}`"));
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![target, handler] },
                        ty: Ty::Proxy,
                        span,
                    };
                }
                let managed_collection = match name.as_str() {
                    "Map" => Some(("mapNew", Ty::Map)),
                    "Set" => Some(("setNew", Ty::Set)),
                    "WeakMap" => Some(("weakMapNew", Ty::WeakMap)),
                    "WeakSet" => Some(("weakSetNew", Ty::WeakSet)),
                    "RegExp" => Some(("regexpNew", Ty::RegExp)),
                    "BigInt" => Some(("bigIntNew", Ty::BigInt)),
                    "FinalizationRegistry" => Some((
                        "finalizationRegistryNew",
                        Ty::FinalizationRegistry,
                    )),
                    "Promise" => Some(("promiseNew", Ty::Promise)),
                    _ => None,
                };
                if let Some((native_name, ty)) = managed_collection {
                    let valid_args = (name == "RegExp" && (args.len() == 1 || args.len() == 2))
                        || (name == "BigInt" && args.len() == 1)
                        || (name == "Proxy" && args.len() == 2);
                    if !args.is_empty() && !valid_args {
                        self.error(span, format!("`{name}` expects no constructor arguments"));
                        return self.poison(span);
                    }
                    let module = if name == "Promise" {
                        "promise"
                    } else {
                        "collections"
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
                                let mut checked: Vec<hir::Expr> =
                                    args.iter().map(|arg| self.check_expr(cx, arg)).collect();
                                if name == "RegExp" && checked.len() == 1 {
                                    // `regexpNew(pattern, flags)`: default to no flags.
                                    checked.push(hir::Expr {
                                        kind: hir::ExprKind::String(String::new()),
                                        ty: Ty::String,
                                        span,
                                    });
                                }
                                checked
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
                // `new Boolean(x)`: the wrapper object. Lowered to
                // `new BooleanObject(<truthiness of x>)` (the prelude class in
                // `builtin/boolean.zun`); bare `Boolean(x)` stays a primitive.
                if name == "Boolean" && !self.classes_by_name.contains_key(&name) {
                    let Some(&wrapper) = self.classes_by_name.get("BooleanObject") else {
                        self.error(class.span, "missing `BooleanObject` wrapper class");
                        return self.poison(span);
                    };
                    if args.len() > 1 {
                        self.error(span, "`Boolean` expects at most one argument");
                        return self.poison(span);
                    }
                    let flag = match args.first() {
                        None => hir::Expr {
                            kind: hir::ExprKind::Boolean(false),
                            ty: Ty::Boolean,
                            span,
                        },
                        Some(arg) => {
                            let arg = self.check_expr(cx, arg);
                            if arg.ty.is_error() {
                                return self.poison(span);
                            }
                            if arg.ty == Ty::Boolean {
                                arg
                            } else {
                                let Some((id, _)) = zun_natives::resolve("boolean", "booleanCoerce")
                                else {
                                    self.error(span, "missing Boolean intrinsic `booleanCoerce`");
                                    return self.poison(span);
                                };
                                hir::Expr {
                                    kind: hir::ExprKind::CallBuiltin { id, args: vec![arg] },
                                    ty: Ty::Boolean,
                                    span,
                                }
                            }
                        }
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::New { class: wrapper, args: vec![flag] },
                        ty: Ty::Instance(wrapper),
                        span,
                    };
                }
                let Some(&class_idx) = self.classes_by_name.get(&name) else {
                    self.error(class.span, format!("unknown class `{name}`"));
                    return self.poison(span);
                };
                // ECMAScript Error construction: `new Error(message, { cause })`.
                // The `(message, options)` shape does not fit Zun's one-arg-per-
                // field model, so Error-family classes are built specially here:
                // the message is arg0 (empty string when omitted), `stackFrames`
                // comes from its declared initializer, and `cause` is read from
                // the options object's `.cause` (or `undefined` when no options
                // object is given). Applies to `Error` and its six subclasses,
                // which all inherit the `message`/`stackFrames`/`cause` layout.
                const ERROR_FAMILY: &[&str] = &[
                    "Error",
                    "EvalError",
                    "RangeError",
                    "ReferenceError",
                    "SyntaxError",
                    "TypeError",
                    "URIError",
                ];
                if ERROR_FAMILY.contains(&name.as_str())
                    && self.field_of(class_idx, "cause").map(|(_, ty)| ty) == Some(Ty::Any)
                {
                    return self.check_error_new(cx, class_idx, &name, args, span);
                }
                // A class that declares a `constructor` method runs user code on
                // `new`: fields start at their defaults and the arguments are
                // checked against the constructor's parameters. The class's own
                // `constructor` runs; it may call `super(...)` to chain to the
                // base constructor (see `check_super_call`).
                let own_ctor = self
                    .methods_by_class
                    .get(class_idx as usize)
                    .and_then(|m| m.get("constructor"))
                    .copied();
                if let Some(ctor) = own_ctor {
                    let new = self.check_new_with_constructor(cx, class_idx, &name, ctor, args, span);
                    return self.wrap_hooked_new(class_idx, new, span);
                }
                let field_tys: Vec<Ty> = self
                    .full_fields(class_idx)
                    .into_iter()
                    .map(|(_, ty, _, _)| ty)
                    .collect();
                let mut args: Vec<hir::Expr> =
                    args.iter().map(|a| self.check_expr(cx, a)).collect();
                let initializers = self.full_initializers(class_idx);
                if args.len() < field_tys.len() {
                    for initializer in initializers.iter().skip(args.len()).take(field_tys.len() - args.len()) {
                        let Some(initializer) = initializer else {
                            break;
                        };
                        args.push(self.check_expr(cx, initializer));
                    }
                }
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
                let new = hir::Expr {
                    kind: hir::ExprKind::New {
                        class: class_idx,
                        args,
                    },
                    ty: Ty::Instance(class_idx),
                    span,
                };
                self.wrap_hooked_new(class_idx, new, span)
            }
            ast::Expr::Field { object, field, .. } => {
                let fname = self.text(field).to_string();
                if let ast::Expr::Ident(class_name) = object.as_ref() {
                    // `Enum.Member` folds to the member's compile-time literal.
                    let enum_name = self.text(class_name).to_string();
                    if self.enums.contains_key(&enum_name) {
                        match self.enums[&enum_name].get(&fname).cloned() {
                            Some(types::EnumConst::Num(n)) => {
                                return hir::Expr { kind: hir::ExprKind::Number(n), ty: Ty::F64, span };
                            }
                            Some(types::EnumConst::Str(s)) => {
                                return hir::Expr {
                                    kind: hir::ExprKind::String(s),
                                    ty: Ty::String,
                                    span,
                                };
                            }
                            None => {
                                self.error(
                                    field.span,
                                    format!("enum `{enum_name}` has no member `{fname}`"),
                                );
                                return hir::Expr { kind: hir::ExprKind::Number(0.0), ty: Ty::Error, span };
                            }
                        }
                    }
                    if let Some(&class_index) = self.classes_by_name.get(self.text(class_name)) {
                        if let Some((field_slot, ty)) = self.static_field_of(class_index, &fname) {
                            return hir::Expr {
                                kind: hir::ExprKind::StaticFieldGet { class: class_index, field_slot },
                                ty,
                                span,
                            };
                        }
                    }
                    // `Number.*` numeric constants fold to compile-time `number`
                    // literals. Keyed on the receiver name (the `Number` class
                    // has no static fields, so `static_field_of` above returns
                    // `None` and falls through here).
                    if self.text(class_name) == "Number" {
                        let constant = match fname.as_str() {
                            "MAX_SAFE_INTEGER" => Some(9_007_199_254_740_991.0),
                            "MIN_SAFE_INTEGER" => Some(-9_007_199_254_740_991.0),
                            "MAX_VALUE" => Some(f64::MAX),
                            "MIN_VALUE" => Some(f64::MIN_POSITIVE.min(f64::from_bits(1))),
                            "EPSILON" => Some(f64::EPSILON),
                            "POSITIVE_INFINITY" => Some(f64::INFINITY),
                            "NEGATIVE_INFINITY" => Some(f64::NEG_INFINITY),
                            "NaN" => Some(f64::NAN),
                            _ => None,
                        };
                        if let Some(value) = constant {
                            return hir::Expr {
                                kind: hir::ExprKind::Number(value),
                                ty: Ty::F64,
                                span,
                            };
                        }
                    }
                    // Well-known symbols as first-class values (`Symbol.iterator`,
                    // `Symbol.asyncIterator`, `Symbol.hasInstance`,
                    // `Symbol.toPrimitive`, `Symbol.toStringTag`). Their identities
                    // are stable per isolate.
                    if self.text(class_name) == "Symbol"
                        && !self.classes_by_name.contains_key("Symbol")
                    {
                        // Stable well-known symbol ids. Source of truth:
                        // `zun_core::WellKnownSymbol` (the runtime registry seeds
                        // these at the same ids). Kept inline so the compile-time
                        // crate does not depend on the runtime crate.
                        let well_known_id = match fname.as_str() {
                            "iterator" => Some(0.0),
                            "asyncIterator" => Some(1.0),
                            "hasInstance" => Some(2.0),
                            "toPrimitive" => Some(3.0),
                            "toStringTag" => Some(4.0),
                            "match" => Some(5.0),
                            "replace" => Some(6.0),
                            "search" => Some(7.0),
                            "split" => Some(8.0),
                            "species" => Some(9.0),
                            "isConcatSpreadable" => Some(10.0),
                            "unscopables" => Some(11.0),
                            _ => None,
                        };
                        if let Some(well_known_id) = well_known_id {
                            let Some((id, _)) =
                                zun_natives::resolve("collections", "symbolWellKnown")
                            else {
                                self.error(span, "missing `symbolWellKnown` intrinsic");
                                return self.poison(span);
                            };
                            return hir::Expr {
                                kind: hir::ExprKind::CallBuiltin {
                                    id,
                                    args: vec![hir::Expr {
                                        kind: hir::ExprKind::Number(well_known_id),
                                        ty: Ty::F64,
                                        span,
                                    }],
                                },
                                ty: Ty::Symbol,
                                span,
                            };
                        }
                    }
                }
                let object = self.check_expr(cx, object);
                // `symbol.description` — the creation description or `undefined`.
                if object.ty == Ty::Symbol && fname == "description" {
                    let Some((id, _)) =
                        zun_natives::resolve("collections", "symbolDescription")
                    else {
                        self.error(span, "missing `symbolDescription` intrinsic");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![object] },
                        ty: Ty::Any,
                        span,
                    };
                }
                if object.ty == Ty::Array && fname == "length" {
                    return hir::Expr {
                        kind: hir::ExprKind::ArrayLength {
                            object: Box::new(object),
                        },
                        ty: Ty::Int(IntKind::U64),
                        span,
                    };
                }
                // `str.length` as a PROPERTY (the spec form), lowering to the
                // same `string.length` native the method form uses — intercepted
                // on the primitive receiver ahead of class dispatch, like the
                // `.length` cases for arrays and views above.
                if object.ty == Ty::String && fname == "length" {
                    let Some((id, _)) = zun_natives::resolve("string", "length") else {
                        self.error(span, "missing String intrinsic `length`");
                        return self.poison(span);
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![object] },
                        ty: Ty::Int(IntKind::U64),
                        span,
                    };
                }
                // `view.length` as a property (Node `Buffer.length`, and the
                // typed arrays' element count). Lowers to the kind's `length`
                // intrinsic, so the method form `view.length()` still works too.
                if let Ty::BufferView(kind) = object.ty {
                    if fname == "length" {
                        if let Some((native, ret, _)) = crate::typed_arrays::method(kind, "length") {
                            if let Some((id, _)) = zun_natives::resolve("buffer", native) {
                                return hir::Expr {
                                    kind: hir::ExprKind::CallBuiltin { id, args: vec![object] },
                                    ty: ret,
                                    span,
                                };
                            }
                        }
                    }
                }
                // `map.size`/`set.size` as a PROPERTY (the spec form), lowering
                // to the same `mapSize`/`setSize` native the method form used.
                // Follows the `.length`-on-views/strings precedent above: a
                // field name is intercepted on a known receiver type ahead of
                // class dispatch, so the backing class need not be in scope. The
                // method form `map.size()` still resolves via the method table.
                if fname == "size" {
                    let native = match object.ty {
                        Ty::Map => Some("mapSize"),
                        Ty::Set => Some("setSize"),
                        _ => None,
                    };
                    if let Some(native) = native {
                        let Some((id, _)) = zun_natives::resolve("collections", native) else {
                            self.error(span, format!("missing intrinsic `{native}`"));
                            return self.poison(span);
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: vec![object] },
                            ty: Ty::Int(IntKind::U64),
                            span,
                        };
                    }
                }
                // `Function.prototype.name`/`length` resolve from the compile-time
                // signature of a function value; a dynamic function value (a
                // closure) has no statically known name or declared arity.
                if let Ty::Function(func) = object.ty {
                    if fname == "name" {
                        let name = if matches!(
                            func,
                            hir::DYNAMIC_FUNCTION | hir::DYNAMIC_PREDICATE | hir::DYNAMIC_REDUCER
                        ) {
                            String::new()
                        } else {
                            self.signatures[func as usize].name.clone()
                        };
                        return hir::Expr { kind: hir::ExprKind::String(name), ty: Ty::String, span };
                    }
                    if fname == "length" {
                        let length = if matches!(
                            func,
                            hir::DYNAMIC_FUNCTION | hir::DYNAMIC_PREDICATE | hir::DYNAMIC_REDUCER
                        ) {
                            0
                        } else {
                            let signature = &self.signatures[func as usize];
                            let rest = usize::from(signature.rest_param.is_some());
                            signature
                                .params
                                .len()
                                .saturating_sub(signature.optional_count)
                                .saturating_sub(rest)
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::Number(length as f64),
                            ty: Ty::F64,
                            span,
                        };
                    }
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
                    Ty::DynamicObject | Ty::Any => hir::Expr {
                        kind: hir::ExprKind::DynamicGet {
                            object: Box::new(object),
                            name: fname,
                        },
                        ty: Ty::Any,
                        span,
                    },
                    // `proxy.key` routes through the handler's `get` trap (the
                    // native falls back to the target when the trap is absent).
                    Ty::Proxy => self.proxy_get(object, hir::Expr {
                        kind: hir::ExprKind::String(fname),
                        ty: Ty::String,
                        span,
                    }, span),
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
            ast::Expr::OptionalIndex { object, index, .. } => {
                let object = self.check_expr(cx, object);
                let key = self.check_expr(cx, index);
                if !matches!(object.ty, Ty::Any | Ty::DynamicObject | Ty::Error) {
                    self.error(object.span, "optional computed access requires a dynamic receiver");
                }
                hir::Expr {
                    kind: hir::ExprKind::OptionalDynamicGetValue {
                        object: Box::new(object),
                        key: Box::new(key),
                    },
                    ty: Ty::Any,
                    span,
                }
            }
            ast::Expr::OptionalCall { callee, args, .. } => {
                let callee = self.check_expr(cx, callee);
                let Ty::Function(func) = callee.ty else {
                    self.error(callee.span, "optional call requires a function value");
                    return self.poison(span);
                };
                let checked: Vec<hir::Expr> = args.iter().map(|arg| self.check_expr(cx, arg)).collect();
                let return_ty = match func {
                    hir::DYNAMIC_FUNCTION | hir::DYNAMIC_REDUCER => Ty::Any,
                    hir::DYNAMIC_PREDICATE => Ty::Boolean,
                    _ => self.signatures[func as usize].return_ty,
                };
                hir::Expr {
                    kind: hir::ExprKind::OptionalIndirectCall {
                        callee: Box::new(callee),
                        args: checked,
                        func,
                    },
                    ty: return_ty,
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
                    (Ty::WeakSet, "add") => ("weakSetAdd", Ty::Boolean, vec![Ty::Any]),
                    (Ty::WeakSet, "has") => ("weakSetHas", Ty::Boolean, vec![Ty::Any]),
                    (Ty::WeakSet, "delete") => ("weakSetDelete", Ty::Boolean, vec![Ty::Any]),
                    (Ty::WeakRef, "deref") => ("weakRefDeref", Ty::Any, vec![]),
                    (Ty::RegExp, "isMatch" | "test") => ("regexpIsMatch", Ty::Boolean, vec![Ty::String]),
                    (Ty::RegExp, "exec") => ("regexpExec", Ty::Array, vec![Ty::String]),
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
                    "promise"
                } else {
                    "collections"
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
                if let ast::Expr::String { value: key, .. } = index.as_ref() {
                    if let Ty::Instance(class) = object.ty {
                        if let Some((field_slot, ty)) = self.field_of(class, key) {
                            return hir::Expr {
                                kind: hir::ExprKind::FieldGet {
                                    object: Box::new(object),
                                    class,
                                    field_slot,
                                },
                                ty,
                                span,
                            };
                        }
                    }
                }
                if matches!(object.ty, Ty::Any | Ty::DynamicObject) {
                    let key = self.check_expr(cx, index);
                    return hir::Expr {
                        kind: hir::ExprKind::DynamicGetValue {
                            object: Box::new(object),
                            key: Box::new(key),
                        },
                        ty: Ty::Any,
                        span,
                    };
                }
                if object.ty == Ty::Proxy {
                    let key = self.check_expr(cx, index);
                    return self.proxy_get(object, key, span);
                }
                // `view[i]` on a byte-buffer view (a Buffer / typed array) reads
                // one element, routed to the kind's `get` intrinsic.
                if let Ty::BufferView(kind) = object.ty {
                    let mut index = self.check_expr(cx, index);
                    coerce_numeric_literal(&mut index, Ty::Int(IntKind::U64));
                    if !index.ty.is_int() && !index.ty.is_error() {
                        self.error(index.span, "buffer index must be an integer");
                    }
                    if let Some((native, ret, _)) = crate::typed_arrays::method(kind, "get") {
                        if let Some((id, _)) = zun_natives::resolve("buffer", native) {
                            return hir::Expr {
                                kind: hir::ExprKind::CallBuiltin { id, args: vec![object, index] },
                                ty: ret,
                                span,
                            };
                        }
                    }
                    self.error(span, "missing buffer index intrinsic");
                    return self.poison(span);
                }
                let mut index = self.check_expr(cx, index);
                coerce_numeric_literal(&mut index, Ty::INT);
                if object.ty != Ty::Array && !object.ty.is_error() {
                    self.error(object.span, "indexing requires an Array");
                }
                let index = self.coerce_array_index(index);
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
                // Computed property store on a dynamic receiver: a Symbol key
                // routes to the symbol-keyed slot, a string/number key to the
                // ordinary dynamic property. The key and value are boxed to `Any`.
                if matches!(object.ty, Ty::Any | Ty::DynamicObject) {
                    let key = self.check_expr(cx, index);
                    let value = self.check_expr(cx, value);
                    let boxed = if value.ty == Ty::Any {
                        value
                    } else {
                        hir::Expr {
                            span: value.span,
                            kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                            ty: Ty::Any,
                        }
                    };
                    return hir::Expr {
                        kind: hir::ExprKind::DynamicSetValue {
                            object: Box::new(object),
                            key: Box::new(key),
                            value: Box::new(boxed),
                        },
                        ty: Ty::Any,
                        span,
                    };
                }
                if object.ty == Ty::Proxy {
                    let key = self.check_expr(cx, index);
                    let value = self.check_expr(cx, value);
                    return self.proxy_set(object, key, value, span);
                }
                // `view[i] = v` on a byte-buffer view (Buffer / typed array)
                // stores one element via the kind's `set` intrinsic.
                if let Ty::BufferView(kind) = object.ty {
                    let mut index = self.check_expr(cx, index);
                    coerce_numeric_literal(&mut index, Ty::Int(IntKind::U64));
                    if !index.ty.is_int() && !index.ty.is_error() {
                        self.error(index.span, "buffer index must be an integer");
                    }
                    if let Some((native, _ret, params)) = crate::typed_arrays::method(kind, "set") {
                        // params = [index, element]; the element type is the kind's.
                        let elem_ty = params.get(1).copied().unwrap_or(Ty::Int(IntKind::U8));
                        let mut value = self.check_expr(cx, value);
                        let value_ty = value.ty;
                        coerce_numeric_literal(&mut value, elem_ty);
                        // A JS `number` stored into an integer element is
                        // converted (NaN -> 0, truncation) like a typed-array set.
                        if elem_ty.is_int() && value.ty == Ty::F64 {
                            let value_span = value.span;
                            value = hir::Expr {
                                kind: hir::ExprKind::Convert { value: Box::new(value) },
                                ty: elem_ty,
                                span: value_span,
                            };
                        }
                        if !value.ty.is_error() && !self.is_assignable(value.ty, elem_ty) {
                            self.error(value.span, format!("buffer element must be {elem_ty:?}"));
                        }
                        if let Some((id, _)) = zun_natives::resolve("buffer", native) {
                            // The store intrinsic yields a bool; the expression's
                            // value is the assigned element (JS assignment result).
                            return hir::Expr {
                                kind: hir::ExprKind::CallBuiltin {
                                    id,
                                    args: vec![object, index, value],
                                },
                                ty: if value_ty.is_error() { elem_ty } else { value_ty },
                                span,
                            };
                        }
                    }
                    self.error(span, "missing buffer index intrinsic");
                    return self.poison(span);
                }
                let mut index = self.check_expr(cx, index);
                let value = self.check_expr(cx, value);
                let value_ty = value.ty;
                coerce_numeric_literal(&mut index, Ty::INT);
                if object.ty != Ty::Array && !object.ty.is_error() {
                    self.error(object.span, "indexing requires an Array");
                }
                let index = self.coerce_array_index(index);
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
                    Ty::DynamicObject | Ty::Any => {
                        let boxed = if value.ty == Ty::Any {
                            value
                        } else {
                            hir::Expr {
                                span: value.span,
                                kind: hir::ExprKind::BoxAny {
                                    value: Box::new(value),
                                },
                                ty: Ty::Any,
                            }
                        };
                        hir::Expr {
                            kind: hir::ExprKind::DynamicSet {
                                object: Box::new(object),
                                name: fname,
                                value: Box::new(boxed),
                            },
                            ty: Ty::Any,
                            span,
                        }
                    }
                    // `proxy.key = v` routes through the handler's `set` trap.
                    Ty::Proxy => {
                        let key = hir::Expr {
                            kind: hir::ExprKind::String(fname),
                            ty: Ty::String,
                            span,
                        };
                        self.proxy_set(object, key, value, span)
                    }
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
                        Ty::String => "stringIteratorNew",
                        Ty::Error => "arrayIteratorNew",
                        Ty::BufferView(kind) => match crate::typed_arrays::iterator_new(kind) {
                            Some(new_name) => new_name,
                            None => {
                                self.error(
                                    object.span,
                                    "a DataView has no Symbol.iterator".to_string(),
                                );
                                return self.poison(span);
                            }
                        },
                        other => {
                            self.error(
                                object.span,
                                format!("type {other:?} has no Symbol.iterator"),
                            );
                            return self.poison(span);
                        }
                    };
                    let Some((id, _)) = zun_natives::resolve("buffer", native_name) else {
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
                        // A plain dynamic function value accepts any argument
                        // count (JS); only the fixed-shape predicate/reducer
                        // callbacks enforce their arity.
                        if func != hir::DYNAMIC_FUNCTION && args.len() != expected {
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
        let key = self.check_expr(cx, key);
        let object = self.check_expr(cx, object);
        if !key.ty.is_error() && key.ty != Ty::String {
            self.error(key.span, "left operand of `in` must be a string key");
        }
        let native = match object.ty {
            Ty::Proxy => "proxyHas",
            Ty::DynamicObject | Ty::Any => "reflectHas",
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
                if !key.ty.is_error() && key.ty != Ty::String {
                    self.error(key.span, "`delete obj[k]` requires a string key");
                }
                (object.as_ref(), key)
            }
            _ => {
                self.error(
                    span,
                    "`delete` requires a property reference (`obj.k` or `obj[k]`)",
                );
                return self.poison(span);
            }
        };
        let object = self.check_expr(cx, object_ast);
        let native = match object.ty {
            Ty::Proxy => "proxyDelete",
            Ty::DynamicObject | Ty::Any => "reflectDelete",
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
        let operand = self.check_expr(cx, operand);
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
        hir::Expr {
            kind: hir::ExprKind::String(type_string.to_string()),
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
            Eq | Ne => {
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
mod tests {
#[cfg(test)]
#[path = "../tests.rs"]
mod tests;
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
