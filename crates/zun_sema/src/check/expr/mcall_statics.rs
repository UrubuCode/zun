//! AST-phase interceptors for `check_method_call` (see `method_call.rs`).
//!
//! These run before the receiver is checked, matching on the raw AST: the
//! `.call`/`.apply` prototype forms, the namespace statics reached through a
//! bare identifier receiver (`Math.max`, `Object.keys`, `Array.from`, ...),
//! and the static-regex `.test` fast path. Each returns `Some` when it handles
//! the call and `None` to fall through, preserving the original order.

use crate::check::*;

impl Checker<'_> {
    /// `recv.method(...spread)` on a dynamic receiver: dispatch at run time
    /// through `methodCallSpread`.
    pub(in crate::check) fn try_mcall_spread_dynamic(
        &mut self,
        cx: &mut FunctionCx,
        receiver: &ast::Expr,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        if args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
            && !matches!(self.text(method), "call" | "apply" | "bind")
            && !matches!(receiver, ast::Expr::Ident(_) | ast::Expr::Field { .. } | ast::Expr::StaticField { .. } if false)
        {
            let is_namespace = matches!(receiver, ast::Expr::Ident(n) if cx.lookup(self.text(n)).is_none());
            if !is_namespace {
                let probe = self.check_expr(cx, receiver);
                if matches!(probe.ty, Ty::Any | Ty::DynamicObject) {
                    let elements = args
                        .iter()
                        .map(|arg| match arg {
                            ast::Expr::Spread { expr, .. } => ast::ArrayElement::Spread((**expr).clone()),
                            other => ast::ArrayElement::Expr(other.clone()),
                        })
                        .collect();
                    let literal = ast::Expr::ArrayLiteral { elements, span };
                    let list = self.check_expr(cx, &literal);
                    if let Some((id, _)) = zun_natives::resolve("array", "methodCallSpread") {
                        let name = hir::Expr {
                            kind: hir::ExprKind::String(self.text(method).to_string()),
                            ty: Ty::String,
                            span,
                        };
                        return Some(hir::Expr {
                            kind: hir::ExprKind::CallBuiltin {
                                id,
                                args: vec![Self::box_to_any(probe), Self::box_to_any(name), list],
                            },
                            ty: Ty::Any,
                            span,
                        });
                    }
                }
            }
        }
        None
    }

    /// `Object.prototype.hasOwnProperty.call(obj, key)` folded to
    /// `Object.hasOwn(obj, key)` (with compile-time answers for built-in and
    /// user-class prototypes).
    pub(in crate::check) fn try_mcall_object_hasown(
        &mut self,
        cx: &mut FunctionCx,
        receiver: &ast::Expr,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        if self.text(method) == "call" && args.len() == 2 && !self.classes_by_name.contains_key("Object") {
            let is_has_own_call = match receiver {
                ast::Expr::Field { object: proto, field: member, .. } => {
                    self.text(member) == "hasOwnProperty"
                        && (matches!(proto.as_ref(),
                            ast::Expr::Field { object: base, field: p, .. }
                                if self.text(p) == "prototype"
                                    && matches!(base.as_ref(), ast::Expr::Ident(n) if self.text(n) == "Object"))
                            || matches!(proto.as_ref(),
                                ast::Expr::StaticField { class, field: p, .. }
                                    if self.text(p) == "prototype" && self.text(class) == "Object"))
                }
                _ => false,
            };
            if is_has_own_call {
                // `Object.prototype.hasOwnProperty.call(Ctor.prototype, "name")` on a
                // built-in constructor: answered from the built-in method table.
                let proto_owner = match &args[0] {
                    ast::Expr::Field { object: base, field: p, .. } if self.text(p) == "prototype" => match base.as_ref() {
                        ast::Expr::Ident(n) => Some(self.text(n).to_string()),
                        _ => None,
                    },
                    ast::Expr::StaticField { class, field: p, .. } if self.text(p) == "prototype" => {
                        Some(self.text(class).to_string())
                    }
                    _ => None,
                };
                if let (Some(owner), ast::Expr::String { value, .. }) = (&proto_owner, &args[1]) {
                    // A user class: its prototype owns exactly the declared
                    // instance methods and accessors (plus `constructor`).
                    if let Some(&class_idx) = self.classes_by_name.get(owner) {
                        let own = &self.methods_by_class[class_idx as usize];
                        let present = value == "constructor"
                            || [value.clone(), format!("get_{value}"), format!("set_{value}")]
                                .iter()
                                .any(|n| {
                                    own.get(n).is_some_and(|&f| !self.signatures[f as usize].is_static)
                                });
                        return Some(hir::Expr { kind: hir::ExprKind::Boolean(present), ty: Ty::Boolean, span });
                    }
                }
                // A well-known Symbol key (`Symbol.toPrimitive`) on a user class
                // prototype: present when the class declares that symbol method.
                if let (Some(owner), ast::Expr::Field { .. }) = (&proto_owner, &args[1]) {
                    if let Some(&class_idx) = self.classes_by_name.get(owner) {
                        if let Some(symbol) = self.well_known_symbol_of_key(&args[1]) {
                            let present = self
                                .class_symbol_methods
                                .iter()
                                .any(|&(c, s, _)| c == class_idx && s == symbol.id().get());
                            return Some(hir::Expr { kind: hir::ExprKind::Boolean(present), ty: Ty::Boolean, span });
                        }
                    }
                }
                let proto_owner = proto_owner;
                if let (Some(owner), ast::Expr::String { .. }) = (proto_owner, &args[1]) {
                    if !self.classes_by_name.contains_key(&owner) {
                        if let ast::Expr::String { value, .. } = &args[1] {
                            if builtin_function_length(&[owner.as_str(), "prototype", value.as_str()]).is_some() {
                                return Some(hir::Expr { kind: hir::ExprKind::Boolean(true), ty: Ty::Boolean, span });
                            }
                        }
                    }
                }
                return Some(self.check_object_static(cx, "hasOwn", method.span, args, span));
            }
        }
        None
    }

    /// `Array.prototype.<m>.call(arrayLike, ...rest)`: mutate an array-like
    /// in place (push/pop/fill/reverse) or convert it with `Array.from` for a
    /// read-only method.
    pub(in crate::check) fn try_mcall_array_proto_call(
        &mut self,
        cx: &mut FunctionCx,
        receiver: &ast::Expr,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        if self.text(method) == "call" && !args.is_empty() && !self.classes_by_name.contains_key("Array") {
            let array_method = match receiver {
                ast::Expr::Field { object: proto, field: member, .. }
                    if matches!(proto.as_ref(),
                        ast::Expr::Field { object: base, field: p, .. }
                            if self.text(p) == "prototype"
                                && matches!(base.as_ref(), ast::Expr::Ident(n) if self.text(n) == "Array"))
                        || matches!(proto.as_ref(),
                            ast::Expr::StaticField { class, field: p, .. }
                                if self.text(p) == "prototype" && self.text(class) == "Array") =>
                {
                    Some(self.text(member).to_string())
                }
                _ => None,
            };
            const READ_ONLY: &[&str] = &[
                "reduce", "reduceRight", "map", "filter", "forEach", "some", "every", "find",
                "findIndex", "findLast", "findLastIndex", "indexOf", "lastIndexOf", "includes",
                "join", "slice", "concat", "at", "flat", "flatMap", "entries", "keys", "values",
            ];
            if let Some(name) = array_method {
                // `Array.prototype.push/pop.call(receiver, ...)` mutates an Array
                // or an array-like through the generic algorithm.
                if (name == "push" || name == "pop" || name == "fill" || name == "reverse")
                    && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
                {
                    let native = match name.as_str() {
                        "push" => "arrayLikePush",
                        "fill" => "arrayLikeFill",
                        "reverse" => "arrayLikeReverse",
                        _ => "arrayLikePop",
                    };
                    if let Some((id, _)) = zun_natives::resolve("array", native) {
                        let source = self.check_expr(cx, &args[0]);
                        let mut call_args = vec![Self::box_to_any(source)];
                        if name == "push" || name == "fill" || name == "reverse" {
                            let literal = ast::Expr::ArrayLiteral {
                                elements: args[1..].iter().cloned().map(ast::ArrayElement::Expr).collect(),
                                span,
                            };
                            let list = self.check_expr(cx, &literal);
                            call_args.push(list);
                        }
                        return Some(hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: call_args },
                            ty: Ty::Any,
                            span,
                        });
                    }
                }
                if READ_ONLY.contains(&name.as_str()) && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. })) {
                    if let Some((from_id, _)) = zun_natives::resolve("array", "arrayFromArrayLike") {
                        let source = self.check_expr(cx, &args[0]);
                        let (slot, tmp) = self.synthetic_local(cx, Ty::Array);
                        let call = ast::Expr::MethodCall {
                            receiver: Box::new(ast::Expr::Ident(tmp)),
                            method: ast::synthetic_ident(&name),
                            args: args[1..].to_vec(),
                            span,
                        };
                        let body = self.check_expr(cx, &call);
                        let ty = body.ty;
                        return Some(hir::Expr {
                            kind: hir::ExprKind::Sequence {
                                effect: Box::new(hir::Expr {
                                    kind: hir::ExprKind::Assign {
                                        slot,
                                        value: Box::new(hir::Expr {
                                            kind: hir::ExprKind::CallBuiltin {
                                                id: from_id,
                                                args: vec![Self::box_to_any(source)],
                                            },
                                            ty: Ty::Array,
                                            span,
                                        }),
                                    },
                                    ty: Ty::Array,
                                    span,
                                }),
                                value: Box::new(body),
                            },
                            ty,
                            span,
                        });
                    }
                }
            }
        }
        None
    }

    /// The remaining `.apply`/`.call` prototype forms: `String.prototype.concat.apply`,
    /// `Error.prototype.toString.call`, and `Object.prototype.toString.call`.
    pub(in crate::check) fn try_mcall_proto_apply_tostring(
        &mut self,
        cx: &mut FunctionCx,
        receiver: &ast::Expr,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        // `String.prototype.concat.apply(base, args)`.
        if self.text(method) == "apply" && args.len() == 2 && !self.classes_by_name.contains_key("String") {
            let is_string_concat = match receiver {
                ast::Expr::Field { object: proto, field: member, .. } => {
                    self.text(member) == "concat"
                        && matches!(proto.as_ref(),
                            ast::Expr::Field { object: base, field: p, .. }
                                if self.text(p) == "prototype"
                                    && matches!(base.as_ref(), ast::Expr::Ident(n) if self.text(n) == "String"))
                }
                _ => false,
            };
            if is_string_concat {
                if let Some(&func) = self.by_name.get("__zunStringConcatArgs") {
                    let base = self.check_expr(cx, &args[0]);
                    let base = if base.ty == Ty::String {
                        base
                    } else {
                        self.to_string_for_parse(base)
                    };
                    let list = self.check_expr(cx, &args[1]);
                    let list = self.spread_source_to_array(list);
                    return Some(hir::Expr {
                        kind: hir::ExprKind::Call { func, args: vec![base, list] },
                        ty: Ty::String,
                        span,
                    });
                }
            }
        }
        // `Error.prototype.toString.call(value)`.
        if self.text(method) == "call" && args.len() == 1 {
            let is_error_proto_to_string = match receiver {
                ast::Expr::Field { object: proto, field: member, .. } => {
                    self.text(member) == "toString"
                        && matches!(proto.as_ref(),
                            ast::Expr::Field { object: base, field: p, .. }
                                if self.text(p) == "prototype"
                                    && matches!(base.as_ref(), ast::Expr::Ident(n) if self.text(n) == "Error"))
                        || matches!(proto.as_ref(),
                            ast::Expr::StaticField { class, field: p, .. }
                                if self.text(p) == "prototype" && self.text(class) == "Error")
                }
                _ => false,
            };
            if is_error_proto_to_string {
                if let Some(&func) = self.by_name.get("__zunErrorProtoToString") {
                    let value = self.check_expr(cx, &args[0]);
                    return Some(hir::Expr {
                        kind: hir::ExprKind::Call { func, args: vec![Self::box_to_any(value)] },
                        ty: Ty::String,
                        span,
                    });
                }
            }
        }
        // `Object.prototype.toString.call(value)`.
        if self.text(method) == "call" && args.len() == 1 && !self.classes_by_name.contains_key("Object") {
            let is_proto_to_string = match receiver {
                ast::Expr::Field { object: proto, field: member, .. } => {
                    self.text(member) == "toString"
                        && match proto.as_ref() {
                            ast::Expr::Field { object: base, field: p, .. } => {
                                self.text(p) == "prototype"
                                    && matches!(base.as_ref(), ast::Expr::Ident(n) if self.text(n) == "Object")
                            }
                            ast::Expr::StaticField { class, field: p, .. } => {
                                self.text(p) == "prototype" && self.text(class) == "Object"
                            }
                            _ => false,
                        }
                }
                _ => false,
            };
            if is_proto_to_string {
                if let Some((id, _)) = zun_natives::resolve_name("objectToStringTag") {
                    let value = self.check_expr(cx, &args[0]);
                    let value = if value.ty == Ty::Any {
                        value
                    } else {
                        hir::Expr {
                            span: value.span,
                            kind: hir::ExprKind::BoxAny { value: Box::new(value) },
                            ty: Ty::Any,
                        }
                    };
                    return Some(hir::Expr {
                        kind: hir::ExprKind::CallBuiltin { id, args: vec![value] },
                        ty: Ty::String,
                        span,
                    });
                }
            }
        }
        None
    }

    /// `Math.<fn>.apply(thisArg, array)` / `Math.<fn>.call(thisArg, ...args)`.
    pub(in crate::check) fn try_mcall_math_apply(
        &mut self,
        cx: &mut FunctionCx,
        receiver: &ast::Expr,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        if let ast::Expr::Field { object: inner_obj, field: inner_field, .. } = receiver {
            if let ast::Expr::Ident(m) = inner_obj.as_ref() {
                let m_name = self.text(m);
                let verb = self.text(method).to_string();
                let fn_name = self.text(inner_field).to_string();
                if m_name == "Math"
                    && !self.classes_by_name.contains_key("Math")
                    && math_function_length(&fn_name).is_some()
                    && (verb == "apply" || verb == "call")
                {
                    if verb == "apply" {
                        if let Some(array) = args.get(1) {
                            return Some(self.math_apply_expr(cx, &fn_name, array, span));
                        }
                        return Some(self.check_math_static(cx, &fn_name, method.span, &[], span));
                    }
                    let rest = if args.is_empty() { args } else { &args[1..] };
                    return Some(self.check_math_static(cx, &fn_name, method.span, rest, span));
                }
            }
        }
        None
    }

    /// The static-regex fast path: `/re/.test(str)` / `.isMatch(str)` on a
    /// compile-time-known pattern, recorded for `zun_ilgen` to synthesize a
    /// native matcher.
    pub(in crate::check) fn try_mcall_static_regex(
        &mut self,
        cx: &mut FunctionCx,
        receiver: &ast::Expr,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        let mname_peek = self.text(method).to_string();
        if (mname_peek == "test" || mname_peek == "isMatch") && args.len() == 1 {
            if let Some((pattern, flags)) = self.static_regex(receiver) {
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
                return Some(hir::Expr {
                    kind: hir::ExprKind::RegexMatch { matcher, args: vec![subject] },
                    ty: Ty::Boolean,
                    span,
                });
            }
        }
        None
    }
}
