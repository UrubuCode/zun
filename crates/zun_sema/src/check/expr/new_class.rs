//! Early `new` interceptors and the user-class construction tail for
//! `check_new` (see `new.rs`).
//!
//! `try_new_early` handles the forms that must be caught before the realm and
//! built-in paths (collection-argument sugar, `new Function`, dynamic
//! cross-module construction, and `new <proxy-local>`). `new_class_construct`
//! is the final path: it resolves the class name and builds the instance,
//! delegating the unknown-name fallbacks to `new_unknown_class_fallback`.
//! Interceptor order is preserved exactly.

use crate::check::*;

impl Checker<'_> {
    /// Forms resolved before the realm/built-in paths: `Map`/`Set` with extra
    /// ignored arguments, `new Function`, `new ns.C(...)` cross-module, and
    /// `new <proxy-local>(...)`. `Some` = handled.
    pub(in crate::check) fn try_new_early(
        &mut self,
        cx: &mut FunctionCx,
        class: &ast::Name,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        // `new Map(iterable, extra...)` / `new Set(iterable, extra...)`: the
        // extra arguments are evaluated and ignored.
        if args.len() > 1
            && (name == "Map" || name == "Set")
            && !self.classes_by_name.contains_key(name)
            && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
        {
            let inner = ast::Expr::New {
                class: *class,
                args: args[..1].to_vec(),
                span,
            };
            let mut result = self.check_expr(cx, &inner);
            for extra in &args[1..] {
                let effect = self.check_expr(cx, extra);
                let ty = result.ty;
                result = hir::Expr {
                    kind: hir::ExprKind::Sequence {
                        effect: Box::new(effect),
                        value: Box::new(result),
                    },
                    ty,
                    span,
                };
            }
            return Some(result);
        }
        if name == "Function" && !self.classes_by_name.contains_key(name) {
            return Some(self.lower_function_construct(cx, args, span));
        }
        // `new ns.C(...args)` where `ns` is a dynamic (`any`) local
        // holding a module namespace from `import()`: construct the class
        // export cross-module at run time. The parser collapses `ns.C`
        // into one dotted class name, so split on the last `.` and check
        // the base is a dynamic local (a known class/realm global takes
        // precedence and is handled below).
        if !self.classes_by_name.contains_key(name) && !self.realm_globals.contains_key(name) {
            if let Some(dot) = name.rfind('.') {
                let base = &name[..dot];
                let member = name[dot + 1..].to_string();
                let base_is_dynamic = cx.lookup(base).is_some_and(|slot| {
                    matches!(cx.locals[slot as usize], Ty::Any | Ty::DynamicObject)
                });
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
                        return Some(self.poison(span));
                    };
                    return Some(hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![base_expr, member_expr, args_array],
                        },
                        ty: Ty::Any,
                        span,
                    });
                }
            }
        }
        // `new proxy(...args)`: a local of proxy type routes through
        // `proxyConstruct` (the handler's `construct` trap, else the
        // wrapped class's default construction).
        if !self.classes_by_name.contains_key(name)
            && !self.realm_globals.contains_key(name)
            && cx
                .lookup(name)
                .is_some_and(|slot| matches!(cx.locals[slot as usize], Ty::Proxy | Ty::Any))
        {
            let callee = self.check_expr(cx, &ast::Expr::Ident(*class));
            let elements: Vec<Option<hir::Expr>> =
                args.iter().map(|a| Some(self.check_expr(cx, a))).collect();
            let args_array = hir::Expr {
                kind: hir::ExprKind::ArrayLiteral { elements },
                ty: Ty::Array,
                span,
            };
            let Some((id, _)) = zun_natives::resolve("collections", "proxyConstruct") else {
                self.error(span, "missing `proxyConstruct` intrinsic");
                return Some(self.poison(span));
            };
            return Some(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![callee, args_array],
                },
                ty: Ty::Any,
                span,
            });
        }
        None
    }

    /// The final construction path: resolve the class name and build the
    /// instance (Error-family specially, a declared constructor via
    /// `check_new_with_constructor`, or field-by-field defaults otherwise).
    pub(in crate::check) fn new_class_construct(
        &mut self,
        cx: &mut FunctionCx,
        class: &ast::Name,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let Some(&class_idx) = self.classes_by_name.get(name) else {
            return self.new_unknown_class_fallback(cx, class, name, args, span);
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
        if ERROR_FAMILY.contains(&name)
            && self.field_of(class_idx, "cause").map(|(_, ty)| ty) == Some(Ty::Any)
        {
            return self.check_error_new(cx, class_idx, name, args, span);
        }
        // A class that declares a `constructor` method runs user code on
        // `new`: fields start at their defaults and the arguments are
        // checked against the constructor's parameters. The class's own
        // `constructor` runs; it may call `super(...)` to chain to the
        // base constructor (see `check_super_call`).
        // A derived class without its own constructor inherits the nearest
        // base constructor (JS implicit `constructor(...args) { super(...args) }`).
        let mut own_ctor = None;
        let mut walk = Some(class_idx);
        while let Some(current) = walk {
            own_ctor = self
                .methods_by_class
                .get(current as usize)
                .and_then(|m| m.get("constructor"))
                .copied();
            if own_ctor.is_some() {
                break;
            }
            walk = self.classes.get(current as usize).and_then(|c| c.base);
        }
        if let Some(ctor) = own_ctor {
            let new = self.check_new_with_constructor(cx, class_idx, name, ctor, args, span);
            return self.wrap_hooked_new(class_idx, new, span);
        }
        let field_tys: Vec<Ty> = self
            .full_fields(class_idx)
            .into_iter()
            .map(|(_, ty, _, _)| ty)
            .collect();
        let mut args: Vec<hir::Expr> = args.iter().map(|a| self.check_expr(cx, a)).collect();
        let initializers = self.full_initializers(class_idx);
        if args.len() < field_tys.len() {
            for initializer in initializers
                .iter()
                .skip(args.len())
                .take(field_tys.len() - args.len())
            {
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

    /// Resolution for a `new <name>(...)` whose name is not a known class: a
    /// global read (`ReferenceError` in dead code), a class-valued variable
    /// via `reflectConstruct`, or an `unknown class` error.
    fn new_unknown_class_fallback(
        &mut self,
        cx: &mut FunctionCx,
        class: &ast::Name,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        // An undeclared constructor name is a run-time ReferenceError
        // (it may sit in dead code), like an undeclared identifier read.
        if cx.lookup(name).is_none()
            && !self.by_name.contains_key(name)
            && !self.module_globals.contains(name)
        {
            if let Some((id, _)) = zun_natives::resolve("collections", "globalRead") {
                return hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![Self::string_literal(name.to_string(), span)],
                    },
                    ty: Ty::Any,
                    span,
                };
            }
        }
        // `new x(...)` where `x` is a variable holding a class value, or a
        // declared ordinary function: construct through the value at run time.
        if cx.lookup(name).is_some()
            || self.module_globals.contains(name)
            || self.by_name.contains_key(name)
        {
            let callee = self.check_expr(cx, &ast::Expr::Ident(*class));
            if !callee.ty.is_error()
                && matches!(callee.ty, Ty::Any | Ty::Function(_) | Ty::DynamicObject)
            {
                if let Some(expr) = self.lower_reflect_construct(cx, callee, args, span) {
                    return expr;
                }
            }
        }
        self.error(class.span, format!("unknown class `{name}`"));
        self.poison(span)
    }

    /// Lower `new callee(args)` to the `reflectConstruct` native: the callee is
    /// boxed to `Any` and the arguments are packed into an array literal.
    fn lower_reflect_construct(
        &mut self,
        cx: &mut FunctionCx,
        callee: hir::Expr,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        let (id, _) = zun_natives::resolve("collections", "reflectConstruct")?;
        let elements: Vec<Option<hir::Expr>> =
            args.iter().map(|a| Some(self.check_expr(cx, a))).collect();
        let args_array = hir::Expr {
            kind: hir::ExprKind::ArrayLiteral { elements },
            ty: Ty::Array,
            span,
        };
        Some(hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![Self::box_to_any(callee), args_array],
            },
            ty: Ty::Any,
            span,
        })
    }
}
