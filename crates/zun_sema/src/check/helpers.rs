use crate::check::*;

impl Checker<'_> {
    /// `(this as any).#x`: the cast only silences the type checker, so inside a
    /// class body a cast-wrapped `this` is the typed instance receiver again.
    pub(in crate::check) fn peel_this_cast(&self, cx: &FunctionCx, expr: &ast::Expr) -> ast::Expr {
        let mut inner = expr;
        while let ast::Expr::Cast { expr: next, .. } = inner {
            inner = next;
        }
        match inner {
            ast::Expr::Ident(name) if cx.this_class.is_some() && self.text(name) == "this" => {
                inner.clone()
            }
            _ => expr.clone(),
        }
    }

    /// A private name resolves lexically to the enclosing class, so a receiver
    /// the checker only knows as `any` (`other.#x` after a `v is Point` guard)
    /// is downcast to that class; a wrong-brand value then fails the cast with a
    /// TypeError at run time.
    pub(in crate::check) fn assume_private_receiver(
        &mut self,
        cx: &FunctionCx,
        object: hir::Expr,
        name: &str,
    ) -> hir::Expr {
        if !matches!(object.ty, Ty::Any | Ty::DynamicObject) {
            return object;
        }
        let Some(class) = cx.this_class else {
            return object;
        };
        if self.private_field_of(class, name).is_none() && !self.has_private_member(class, name) {
            return object;
        }
        let span = object.span;
        hir::Expr {
            kind: hir::ExprKind::Convert {
                value: Box::new(Self::box_to_any(object)),
            },
            ty: Ty::Instance(class),
            span,
        }
    }

    /// An expression that throws a `TypeError` when evaluated (calling
    /// `undefined`), used for writes the language rejects at run time.
    pub(in crate::check) fn runtime_type_error(
        &mut self,
        cx: &mut FunctionCx,
        span: Span,
    ) -> hir::Expr {
        let callee = self.check_expr(
            cx,
            &ast::Expr::Unary {
                op: ast::UnaryOp::Void,
                operand: Box::new(ast::Expr::Number { value: 0.0, span }),
                span,
            },
        );
        hir::Expr {
            kind: hir::ExprKind::IndirectCall {
                callee: Box::new(callee),
                args: Vec::new(),
                func: hir::DYNAMIC_FUNCTION,
            },
            ty: Ty::Any,
            span,
        }
    }

    /// An iterable spread source that is not statically an Array (a string, a Set,
    /// an `any` value) is materialized through the iteration protocol.
    pub(in crate::check) fn spread_source_to_array(&mut self, source: hir::Expr) -> hir::Expr {
        if source.ty == Ty::Array || source.ty.is_error() {
            return source;
        }
        let span = source.span;
        match zun_natives::resolve("array", "arrayFrom") {
            Some((id, _)) => hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![Self::box_to_any(source)],
                },
                ty: Ty::Array,
                span,
            },
            None => source,
        }
    }

    /// `apply`/`Reflect.apply`/`Reflect.construct` argument lists: an Array passes
    /// through; any other object is read as an array-like (no iteration).
    pub(crate) fn array_like_to_array(&mut self, source: hir::Expr) -> hir::Expr {
        if source.ty == Ty::Array || source.ty.is_error() {
            return source;
        }
        let span = source.span;
        match zun_natives::resolve("array", "arrayFromArrayLike") {
            Some((id, _)) => hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![Self::box_to_any(source)],
                },
                ty: Ty::Array,
                span,
            },
            None => source,
        }
    }

    /// Resolve a `collections`-module native, reporting a missing intrinsic.
    pub(in crate::check) fn resolve_collection(&mut self, span: Span, name: &str) -> Option<u32> {
        match zun_natives::resolve("collections", name) {
            Some((id, _)) => Some(id),
            None => {
                self.error(span, format!("missing collections intrinsic `{name}`"));
                None
            }
        }
    }

    /// Lower `str.replace(search, fn)` / `str.replaceAll(search, fn)`.
    pub(in crate::check) fn replace_with_function(
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
        let flag = hir::Expr {
            kind: hir::ExprKind::Boolean(all),
            ty: Ty::Boolean,
            span,
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![search, receiver, callback, flag],
            },
            ty: Ty::String,
            span,
        }
    }

    /// Recognise a simple type guard over an untyped (`any`) local and return
    /// the local's name, slot and narrowed type: `typeof x === "string" |
    /// "number" | "boolean"` and `Array.isArray(x)`.
    pub(in crate::check) fn type_guard_narrowing(
        &self,
        cx: &FunctionCx,
        cond: &ast::Expr,
    ) -> Option<(String, hir::LocalSlot, Ty)> {
        let (ident, narrowed) = match cond {
            ast::Expr::Binary {
                op: ast::BinaryOp::Eq,
                lhs,
                rhs,
                ..
            } => {
                let (typeof_side, literal_side) = match (&**lhs, &**rhs) {
                    (ast::Expr::Unary { .. }, ast::Expr::String { .. }) => (&**lhs, &**rhs),
                    (ast::Expr::String { .. }, ast::Expr::Unary { .. }) => (&**rhs, &**lhs),
                    _ => return None,
                };
                let ast::Expr::Unary {
                    op: ast::UnaryOp::Typeof,
                    operand,
                    ..
                } = typeof_side
                else {
                    return None;
                };
                let ast::Expr::Ident(name) = &**operand else {
                    return None;
                };
                let ast::Expr::String { value, .. } = literal_side else {
                    return None;
                };
                let narrowed = match value.as_str() {
                    "string" => Ty::String,
                    "number" => Ty::F64,
                    "boolean" => Ty::Boolean,
                    _ => return None,
                };
                (*name, narrowed)
            }
            ast::Expr::StaticMethodCall {
                class,
                method,
                args,
                ..
            } if self.text(class) == "Array"
                && self.text(method) == "isArray"
                && args.len() == 1 =>
            {
                let ast::Expr::Ident(name) = &args[0] else {
                    return None;
                };
                (*name, Ty::Array)
            }
            ast::Expr::MethodCall {
                receiver,
                method,
                args,
                ..
            } if matches!(&**receiver, ast::Expr::Ident(class) if self.text(class) == "Array")
                && self.text(method) == "isArray"
                && args.len() == 1 =>
            {
                let ast::Expr::Ident(name) = &args[0] else {
                    return None;
                };
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
    pub(in crate::check) fn unbox_to(value: hir::Expr, target: Ty) -> hir::Expr {
        if value.ty == Ty::Any && matches!(target, Ty::F64 | Ty::Boolean | Ty::String) {
            let span = value.span;
            return hir::Expr {
                kind: hir::ExprKind::Convert {
                    value: Box::new(value),
                },
                ty: target,
                span,
            };
        }
        value
    }

    pub(in crate::check) fn box_to_any(value: hir::Expr) -> hir::Expr {
        if value.ty == Ty::Any || value.ty.is_error() {
            value
        } else {
            let span = value.span;
            hir::Expr {
                kind: hir::ExprKind::BoxAny {
                    value: Box::new(value),
                },
                ty: Ty::Any,
                span,
            }
        }
    }

    /// Whether a read of the module-scope local `name` (in slot `slot`) must go
    /// through the realm store because a function body assigns that global.
    pub(in crate::check) fn reads_through_global(
        &self,
        cx: &FunctionCx,
        name: &str,
        slot: hir::LocalSlot,
    ) -> bool {
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
    pub(in crate::check) fn string_literal(text: String, span: Span) -> hir::Expr {
        hir::Expr {
            kind: hir::ExprKind::String(text),
            ty: Ty::String,
            span,
        }
    }

    /// `globalSet(name, box(value), false)` — store a module global.
    pub(in crate::check) fn global_set_expr(
        &self,
        name: &str,
        value: hir::Expr,
        span: Span,
    ) -> hir::Expr {
        let (id, _) = zun_natives::resolve("collections", "globalSet")
            .expect("globalSet intrinsic must exist");
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![
                    Self::string_literal(name.to_string(), span),
                    Self::box_to_any(value),
                    hir::Expr {
                        kind: hir::ExprKind::Boolean(false),
                        ty: Ty::Boolean,
                        span,
                    },
                ],
            },
            ty: Ty::Any,
            span,
        }
    }

    /// `globalGet(name)` — read a module global (dynamic).
    pub(in crate::check) fn global_get_expr(&self, name: &str, span: Span) -> hir::Expr {
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
}
