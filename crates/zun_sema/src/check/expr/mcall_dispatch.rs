//! The generic instance-method dispatch tail for `check_method_call` (see
//! `method_call.rs`).
//!
//! Reached once every receiver-type interceptor has fallen through: resolve
//! the receiver's class, pick the method (honouring lexically-bound private
//! methods), and invoke it after arity/type checking. This is the method's
//! final path and always produces a result; the two helpers keep the
//! algorithm whole while staying within size limits.

use crate::check::*;

impl Checker<'_> {
    /// Resolve the receiver's class and the method to call (including the
    /// `addEventListener` object-form reroute and lexical private binding),
    /// then invoke it via `mcall_instance_invoke`.
    pub(in crate::check) fn mcall_instance_dispatch(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        mname: String,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
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
        // `addEventListener` / `removeEventListener` accept either a
        // function or a `{ handleEvent }` object. A function cannot be
        // stored in `any`, so the object form routes to a sibling
        // `<name>Object` method taking an `any` listener.
        let mut mname = mname;
        if matches!(mname.as_str(), "addEventListener" | "removeEventListener")
            && args.len() >= 2
            && !matches!(args[1], ast::Expr::Function(_))
            && self.method_of(class, &format!("{mname}Object")).is_some()
        {
            let probe = self.check_expr(cx, &args[1]);
            if matches!(probe.ty, Ty::DynamicObject | Ty::Any | Ty::Instance(_)) {
                mname = format!("{mname}Object");
            }
        }
        // Private methods (`#m`) bind lexically to the class that declares
        // the call site, so a same-named subclass `#m` never overrides them.
        let mut private_direct = false;
        let lexical = match cx.this_class {
            Some(tc) if mname.starts_with('#') && self.is_subclass(class, tc) => {
                self.method_of(tc, &mname).inspect(|_| private_direct = true)
            }
            _ => None,
        };
        let Some(func) = lexical.or_else(|| self.method_of(class, &mname)) else {
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
        self.mcall_instance_invoke(cx, receiver, func, mname, private_direct, args, span)
    }

    /// Check arguments against the resolved method's signature (spread,
    /// rest-parameter, or fixed) and build the final method-call expression,
    /// applying any routed-method rewrite.
    #[allow(clippy::too_many_arguments)]
    fn mcall_instance_invoke(
        &mut self,
        cx: &mut FunctionCx,
        receiver: hir::Expr,
        func: hir::FuncIndex,
        mname: String,
        private_direct: bool,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        if let Some(ast::Expr::Spread { expr, .. }) = args.last() {
            let rest_index = self.signatures[func as usize].rest_param;
            let fixed_count = args.len().saturating_sub(1);
            if rest_index == Some(fixed_count + 1) {
                let mut fixed_args = Vec::with_capacity(fixed_count);
                for argument in &args[..fixed_count] {
                    fixed_args.push(self.check_expr(cx, argument));
                }
                let array = self.check_expr(cx, expr);
                let array = self.spread_source_to_array(array);
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
        let listener_hint = if mname == "addEventListener" {
            self.classes_by_name.get("Event").map(|&event| Ty::Instance(event))
        } else {
            None
        };
        for (arg_index, a) in args.iter().enumerate() {
            // Contextual typing: an unannotated listener closure
            // receives the `Event`.
            if arg_index == 1 && matches!(a, ast::Expr::Function(_)) {
                if let Some(event_ty) = listener_hint {
                    self.closure_param_hint = Some(vec![event_ty]);
                }
            }
            call_args.push(self.check_expr(cx, a));
            self.closure_param_hint = None;
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
            } else if private_direct {
                hir::ExprKind::StaticCall { func, args: call_args }
            } else {
                hir::ExprKind::MethodCall { func, args: call_args }
            },
            ty: return_ty,
            span,
        }
    }
}
