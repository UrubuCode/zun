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
            for (name, is_object) in extra_globals {
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
                    hir::Expr {
                        kind: hir::ExprKind::BoxAny {
                            value: Box::new(hir::Expr {
                                kind: hir::ExprKind::DynamicObjectLiteral {
                                    entries: Vec::new(),
                                },
                                ty: Ty::DynamicObject,
                                span,
                            }),
                        },
                        ty: Ty::Any,
                        span,
                    }
                } else {
                    let token = next_token;
                    next_token += 1;
                    self.realm_descriptor_expr(token, span)
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
                        ],
                    },
                    ty: Ty::Void,
                    span,
                }));
            }
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
            let undefined = hir::Expr {
                kind: hir::ExprKind::DynamicGet {
                    object: Box::new(hir::Expr {
                        kind: hir::ExprKind::DynamicObjectLiteral {
                            entries: Vec::new(),
                        },
                        ty: Ty::DynamicObject,
                        span,
                    }),
                    name: "undefined".to_string(),
                },
                ty: Ty::Any,
                span,
            };
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
}
