use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_new_dynamic(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::NewDynamic { callee, args, .. } = expr else { unreachable!() };
        // `new (expr)(...args)`: any expression of proxy type is
        // evaluated to a value, then routed through `proxyConstruct`.
        // Design boundary: only class targets are constructible
        // (Zun has typed classes, no `function.prototype` + `this`
        // construction model), and `newTarget` is approximated by the
        // target (subclass newTarget selection needs a prototype model).
        // `new ns.C(...args)` where `ns` is a dynamic (`any`) local holding a
        // module namespace from `import()`: construct the class export
        // cross-module at run time (the hand parser spelled this as a
        // dotted class name; OXC spells it as a member-expression callee).
        if let ast::Expr::Field { object, field, .. } = callee.as_ref() {
            if let ast::Expr::Ident(base) = object.as_ref() {
                let base_text = self.text(base).to_string();
                let base_is_dynamic = !self.classes_by_name.contains_key(&base_text)
                    && !self.realm_globals.contains_key(&base_text)
                    && cx
                        .lookup(&base_text)
                        .is_some_and(|slot| matches!(cx.locals[slot as usize], Ty::Any | Ty::DynamicObject));
                if base_is_dynamic {
                    let member = self.text(field).to_string();
                    let base_expr = self.check_expr(cx, &ast::Expr::Ident(*base));
                    let member_expr = hir::Expr {
                        kind: hir::ExprKind::String(member),
                        ty: Ty::String,
                        span: field.span,
                    };
                    let elements: Vec<Option<hir::Expr>> =
                        args.iter().map(|a| Some(self.check_expr(cx, a))).collect();
                    let args_array = hir::Expr {
                        kind: hir::ExprKind::ArrayLiteral { elements },
                        ty: Ty::Array,
                        span,
                    };
                    if let Some((id, _)) = zun_natives::resolve("collections", "dynamicConstruct") {
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
        }
        let callee = self.check_expr(cx, callee);
        if matches!(callee.ty, Ty::Any | Ty::Function(_)) {
            // A dynamic callee reaches `new` as a plain value: only classes
            // are constructible, so anything else throws a TypeError
            // ("... is not a constructor") once the arguments are evaluated.
            let mut result = self.runtime_type_error(cx, span);
            for arg in args.iter().rev() {
                let effect = self.check_expr(cx, arg);
                result = hir::Expr {
                    kind: hir::ExprKind::Sequence { effect: Box::new(effect), value: Box::new(result) },
                    ty: Ty::Any,
                    span,
                };
            }
            return result;
        }
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


    pub(in crate::check) fn check_new(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::New { class, args, .. } = expr else { unreachable!() };
        let name = self.text(class).to_string();
        // Forms that must be caught before the realm/built-in paths
        // (collection-argument sugar, `new Function`, cross-module and
        // proxy-local construction).
        if let Some(expr) = self.try_new_early(cx, class, &name, args, span) {
            return expr;
        }
        // Declarative realm globals (PLAN §3.1): a registered base type
        // lowers `new <Name>()` through its recorded constructor shape.
        // On a miss we fall through to the hardcoded paths unchanged.
        if let Some(expr) = self.try_realm_ctor(cx, &name, args, span) {
            return expr;
        }
        // The hardcoded built-in constructors (buffer views, `BigInt`,
        // `Proxy`, managed collections, `Array`, and the wrapper objects).
        if let Some(expr) = self.try_new_builtin(cx, &name, class, args, span) {
            return expr;
        }
        // The final user-class construction path.
        self.new_class_construct(cx, class, &name, args, span)
    }
}
