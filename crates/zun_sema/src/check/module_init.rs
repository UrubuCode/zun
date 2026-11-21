use crate::check::*;

impl Checker<'_> {
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
    pub(in crate::check) fn check_top_level(&mut self, statements: &[ast::Stmt]) -> hir::Function {
        let block = ast::Block {
            statements: statements.to_vec(),
            span: Span::new(0, 0),
        };
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
            pending_lexical: Vec::new(),
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
            // A nested-block / function-local class runs its static initializers
            // at its declaration-site marker instead.
            if self.classes[class_idx].local {
                continue;
            }
            match last_start {
                Some(last) if self.classes[class_idx].span_start < last => {
                    self.pending_static_classes.push_back(class_idx as u32)
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

    /// Source offset where a top-level statement starts, when known.
    pub(in crate::check) fn stmt_start(statement: &ast::Stmt) -> Option<u32> {
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
    pub(in crate::check) fn block_returns_only_this(&self, block: &ast::Block) -> bool {
        fn walk(
            checker: &Checker<'_>,
            block: &ast::Block,
            this_count: &mut usize,
            other: &mut bool,
        ) {
            for statement in &block.statements {
                stmt(checker, statement, this_count, other);
            }
        }
        fn stmt(
            checker: &Checker<'_>,
            statement: &ast::Stmt,
            this_count: &mut usize,
            other: &mut bool,
        ) {
            match statement {
                ast::Stmt::Return {
                    value: Some(value), ..
                } => match value {
                    ast::Expr::Ident(name) if checker.text(name) == "this" => *this_count += 1,
                    _ => *other = true,
                },
                ast::Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
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
                ast::Stmt::Try {
                    body,
                    catches,
                    finally,
                    ..
                } => {
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

    pub(in crate::check) fn build_module_initializer(
        &self,
        program: &ast::Program,
    ) -> Option<hir::Function> {
        let span = Span::new(0, 0);
        let mut body: Vec<hir::Stmt> = Vec::new();

        // 1) Declarative realm seed (base-type class descriptors).
        let registry = crate::realm::registry();
        if !registry.is_empty() {
            let (set_id, _) = zun_natives::resolve("realm", "globalDefineBuiltin")
                .expect("globalSet intrinsic must exist");
            // Namespace globals are plain objects; each function-like global gets a
            // descriptor token of its own (past the registry's) so `typeof
            // globalThis[name]` reports a function while a run-time lookup keyed on
            // the token (statics, `name`) can tell `Proxy` from `Object`.
            let first_free_token = registry.iter().map(|(_, b)| b.token + 1).max().unwrap_or(0);
            let mut next_token = first_free_token;
            let extra_globals: Vec<(&str, bool)> = vec![
                ("Math", true),
                ("JSON", true),
                ("Reflect", true),
                ("Iterator", true),
                ("Proxy", false),
                ("parseInt", false),
                ("parseFloat", false),
                ("isNaN", false),
                ("isFinite", false),
                ("encodeURIComponent", false),
                ("decodeURIComponent", false),
                ("encodeURI", false),
                ("decodeURI", false),
                ("btoa", false),
                ("atob", false),
                ("escape", false),
                ("unescape", false),
            ];
            for &(name, is_object) in &extra_globals {
                if registry.iter().any(|(n, _)| n.as_str() == name) {
                    continue;
                }
                // A namespace the prelude declares as a class (`JSON`) is that
                // class value, so `globalThis.JSON === JSON` and its statics read
                // through the global exactly like the bare identifier.
                let value = if let Some(&class_idx) =
                    self.classes_by_name.get(name).filter(|_| is_object)
                {
                    self.realm_descriptor_expr(zun_natives::USER_CLASS_FLAG | (class_idx + 1), span)
                } else if is_object {
                    // `Iterator` is a constructor, not a namespace: `typeof
                    // Iterator` is "function" and `class X extends Iterator`
                    // works. It still has to be an *object*, because the realm
                    // hangs `Iterator.from` and the shared `%Iterator.prototype%`
                    // on it at run time -- making it an opaque descriptor token
                    // instead fixed `typeof` and lost both. The `__classDesc`
                    // record is the hook `typeof` already consults for exactly
                    // this case; the realm re-defines it non-enumerably.
                    // The descriptor travels as a hidden global the realm reads
                    // while it populates the namespace -- not as an entry of the
                    // literal: a populated dynamic literal is a `stdyn`, which
                    // put a dynamic site in the initializer of *every* module
                    // and switched the trimmer to its conservative fallback.
                    if name == "Iterator" {
                        let token = next_token;
                        next_token += 1;
                        let descriptor = self.realm_descriptor_expr(token, span);
                        let flag = |value: bool| hir::Expr {
                            kind: hir::ExprKind::Boolean(value),
                            ty: Ty::Boolean,
                            span,
                        };
                        body.push(hir::Stmt::Expr(hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id: set_id,
                                args: vec![
                                    hir::Expr {
                                        kind: hir::ExprKind::String("$IteratorDesc".to_string()),
                                        ty: Ty::String,
                                        span,
                                    },
                                    descriptor,
                                    flag(false),
                                    flag(false),
                                ],
                            },
                            ty: Ty::Void,
                            span,
                        }));
                    }
                    let entries = Vec::new();
                    hir::Expr {
                        kind: hir::ExprKind::BoxAny {
                            value: Box::new(hir::Expr {
                                kind: hir::ExprKind::DynamicObjectLiteral { entries },
                                ty: Ty::DynamicObject,
                                span,
                            }),
                        },
                        ty: Ty::Any,
                        span,
                    }
                } else {
                    // The token is always consumed so the numbering of the
                    // later globals never depends on the program text.
                    let token = next_token;
                    next_token += 1;
                    // A function the prelude implements (`btoa`, `escape`, ...)
                    // is its function value, so `globalThis.btoa === btoa`; when
                    // the prelude function is absent from this program (the
                    // checker drops a prelude function the user source appears
                    // to redefine) the opaque token keeps the global present.
                    match self
                        .by_name
                        .get(name)
                        .filter(|_| PRELUDE_FUNCTION_GLOBALS.contains(&name))
                        .or_else(|| timer_value_helper(name).and_then(|h| self.by_name.get(h)))
                    {
                        Some(&func) => Self::box_to_any(hir::Expr {
                            kind: hir::ExprKind::FunctionRef { func },
                            ty: Ty::Function(func),
                            span,
                        }),
                        None => self.realm_descriptor_expr(token, span),
                    }
                };
                body.push(hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: set_id,
                        args: vec![
                            hir::Expr {
                                kind: hir::ExprKind::String(name.to_string()),
                                ty: Ty::String,
                                span,
                            },
                            value,
                            hir::Expr {
                                kind: hir::ExprKind::Boolean(false),
                                ty: Ty::Boolean,
                                span,
                            },
                            hir::Expr {
                                kind: hir::ExprKind::Boolean(Self::platform_global_enumerable(name)),
                                ty: Ty::Boolean,
                                span,
                            },
                        ],
                    },
                    ty: Ty::Void,
                    span,
                }));
            }
            for (name, binding) in registry.iter() {
                let descriptor = self.realm_descriptor_expr(binding.token, span);
                body.push(hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: set_id,
                        args: vec![
                            hir::Expr {
                                kind: hir::ExprKind::String(name.clone()),
                                ty: Ty::String,
                                span,
                            },
                            descriptor,
                            hir::Expr {
                                kind: hir::ExprKind::Boolean(false),
                                ty: Ty::Boolean,
                                span,
                            },
                            hir::Expr {
                                kind: hir::ExprKind::Boolean(false),
                                ty: Ty::Boolean,
                                span,
                            },
                        ],
                    },
                    ty: Ty::Void,
                    span,
                }));
            }
            // Platform globals the prelude implements as ordinary top-level
            // functions and classes (not base types with compile-time facets, so
            // the realm registry above does not carry them). Each resolvable
            // name becomes an own property of the global object holding the same
            // value the bare identifier evaluates to: a function is the function
            // value, a class is its descriptor. Names the prelude does not define
            // are skipped.
            let already_seeded: std::collections::HashSet<&str> = registry
                .iter()
                .map(|(n, _)| n.as_str())
                .chain(extra_globals.iter().map(|(n, _)| *n))
                .collect();
            for name in PLATFORM_FUNCTIONS.iter().chain(PLATFORM_CLASSES) {
                if already_seeded.contains(name) {
                    continue;
                }
                // A timer global's value lives under a mangled helper name, so
                // the global property and the bare identifier resolve to the
                // same function and `globalThis.setTimeout === setTimeout`.
                let resolved = self.by_name.get(*name).copied().or_else(|| {
                    timer_value_helper(name).and_then(|helper| self.by_name.get(helper).copied())
                });
                let value = if let Some(func) = resolved {
                    Self::box_to_any(hir::Expr {
                        kind: hir::ExprKind::FunctionRef { func },
                        ty: Ty::Function(func),
                        span,
                    })
                } else if let Some(&class_idx) = self.classes_by_name.get(*name) {
                    self.realm_descriptor_expr(zun_natives::USER_CLASS_FLAG | (class_idx + 1), span)
                } else {
                    continue;
                };
                let flag = |value: bool| hir::Expr {
                    kind: hir::ExprKind::Boolean(value),
                    ty: Ty::Boolean,
                    span,
                };
                body.push(hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: set_id,
                        args: vec![
                            hir::Expr {
                                kind: hir::ExprKind::String((*name).to_string()),
                                ty: Ty::String,
                                span,
                            },
                            value,
                            flag(false),
                            flag(Self::platform_global_enumerable(name)),
                        ],
                    },
                    ty: Ty::Void,
                    span,
                }));
            }
            // The frozen value properties (`undefined`, `NaN`, `Infinity`) and
            // the self reference `globalThis`.
            let number = |value: f64| hir::Expr {
                kind: hir::ExprKind::BoxAny {
                    value: Box::new(hir::Expr {
                        kind: hir::ExprKind::Number(value),
                        ty: Ty::F64,
                        span,
                    }),
                },
                ty: Ty::Any,
                span,
            };
            // `void` of a constant, not a read of a missing key off a fresh
            // dynamic object: that read is an `lddyn`, a dynamic site in the
            // initializer of every module, which alone switched the trimmer to
            // its conservative fallback for every program.
            let undefined = crate::check::default_args::missing_arg(Ty::Any, span);
            let global_self = hir::Expr {
                kind: hir::ExprKind::BoxAny {
                    value: Box::new(hir::Expr {
                        kind: hir::ExprKind::RealmGlobal,
                        ty: Ty::DynamicObject,
                        span,
                    }),
                },
                ty: Ty::Any,
                span,
            };
            for (name, value, constant) in [
                ("undefined", undefined, true),
                ("NaN", number(f64::NAN), true),
                ("Infinity", number(f64::INFINITY), true),
                ("globalThis", global_self, false),
            ] {
                body.push(hir::Stmt::Expr(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: set_id,
                        args: vec![
                            hir::Expr {
                                kind: hir::ExprKind::String(name.to_string()),
                                ty: Ty::String,
                                span,
                            },
                            value,
                            hir::Expr {
                                kind: hir::ExprKind::Boolean(constant),
                                ty: Ty::Boolean,
                                span,
                            },
                            hir::Expr {
                                kind: hir::ExprKind::Boolean(false),
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
        self.append_webidl_iterable_aliases(&mut body, span);

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
}

/// Globals that are function-valued prelude functions when the program has them.
const PRELUDE_FUNCTION_GLOBALS: &[&str] = &[
    "encodeURIComponent",
    "decodeURIComponent",
    "encodeURI",
    "decodeURI",
    "btoa",
    "atob",
    "escape",
    "unescape",
];

/// The prelude helper that carries the *value* of a timer global. The timer
/// surface is lowered at the call site, so the bare names had no function
/// value at all (`setTimeout.length`, `globalThis.setTimeout === setTimeout`).
/// They cannot be declared under their own names: the prelude is concatenated
/// after the user source, so zun-node's own `setTimeout` would become a
/// duplicate declaration (issue #111).
pub(in crate::check) fn timer_value_helper(name: &str) -> Option<&'static str> {
    Some(match name {
        "setTimeout" => "__zunSetTimeout",
        "setInterval" => "__zunSetInterval",
        "clearTimeout" => "__zunClearTimeout",
        "clearInterval" => "__zunClearInterval",
        "queueMicrotask" => "__zunQueueMicrotask",
        "setImmediate" => "__zunSetImmediate",
        // Not timers, but the same shape: call-site lowered globals whose
        // value must be one function object shared with `Number.parseInt`.
        "parseInt" => "__zunParseInt",
        "parseFloat" => "__zunParseFloat",
        _ => return None,
    })
}

/// Prelude functions that are own properties of the global object in Node.
const PLATFORM_FUNCTIONS: &[&str] = &[
    "eval",
    "structuredClone",
    "queueMicrotask",
    "setTimeout",
    "clearTimeout",
    "setInterval",
    "clearInterval",
    "setImmediate",
    "clearImmediate",
    "fetch",
];

/// Prelude classes (constructors, and the `Atomics` and `crypto` namespaces) that are own
/// properties of the global object in Node.
const PLATFORM_CLASSES: &[&str] = &[
    "Date",
    "Atomics",
    "crypto",
    "DOMException",
    "TextEncoder",
    "TextDecoder",
    "URL",
    "URLSearchParams",
    "AbortController",
    "AbortSignal",
    "Event",
    "EventTarget",
    "CustomEvent",
    "Headers",
    "Request",
    "Response",
    "FormData",
    "Blob",
    "File",
    "ReadableStream",
    "WritableStream",
    "TransformStream",
];

impl Checker<'_> {
    /// Whether Node reports the global's own property as enumerable. Every
    /// ECMAScript and web-platform class, namespace and function is not; the
    /// Node timer, microtask, clone and base64 functions, `fetch` and `crypto` are.
    fn platform_global_enumerable(name: &str) -> bool {
        matches!(
            name,
            "structuredClone"
                | "queueMicrotask"
                | "setTimeout"
                | "clearTimeout"
                | "setInterval"
                | "clearInterval"
                | "setImmediate"
                | "clearImmediate"
                | "fetch"
                | "atob"
                | "btoa"
                | "crypto"
        )
    }
}
