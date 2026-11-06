use crate::check::*;

impl Checker<'_> {
    pub(in crate::check) fn check_method_call(&mut self, cx: &mut FunctionCx, expr: &ast::Expr) -> hir::Expr {
        let span = expr.span();
        let ast::Expr::MethodCall { receiver, method, args, .. } = expr else { unreachable!() };
        // AST-phase interceptors (the `.call`/`.apply` prototype forms,
        // identifier-receiver namespace statics, and the static-regex fast
        // path), in their original order (see `mcall_statics.rs`).
        if let Some(e) = self.try_mcall_spread_dynamic(cx, receiver.as_ref(), method, args, span) { return e; }
        if let Some(e) = self.try_mcall_object_hasown(cx, receiver.as_ref(), method, args, span) { return e; }
        if let Some(e) = self.try_mcall_array_proto_call(cx, receiver.as_ref(), method, args, span) { return e; }
        if let Some(e) = self.try_mcall_proto_apply_tostring(cx, receiver.as_ref(), method, args, span) { return e; }
        if let Some(e) = self.try_mcall_math_apply(cx, receiver.as_ref(), method, args, span) { return e; }
        if let Some(e) = self.try_mcall_ident_receiver(cx, receiver.as_ref(), method, args, span) { return e; }
        if let Some(e) = self.try_mcall_static_regex(cx, receiver.as_ref(), method, args, span) { return e; }
        let mut receiver = self.check_expr(cx, receiver);
        let mname = self.text(method).to_string();
        // An integer-typed receiver (`s.codePointAt(0)!`) is a JS number:
        // widen it so the `Number.prototype` methods apply.
        if receiver.ty.is_int()
            && matches!(
                mname.as_str(),
                "toString" | "toFixed" | "toExponential" | "toPrecision" | "toLocaleString" | "valueOf"
            )
        {
            let receiver_span = receiver.span;
            receiver = hir::Expr {
                kind: hir::ExprKind::Convert { value: Box::new(receiver) },
                ty: Ty::F64,
                span: receiver_span,
            };
        }
        let receiver = match self.mcall_array_mutators(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_array_push_splice(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_array_splice_sorted(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_array_with_spliced(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_array_concat_reduce(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_promise_dynamic(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        // `String.prototype.concat(...strs)` — variadic. Folded into a
        // chain of the two-argument string `concat` native; zero
        let receiver = match self.mcall_string_concat(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_string_dynamic(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_string_slice_split(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_string_replace_all(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_string_replace_match(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_buffer_views(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_buffer_set(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        // synthesized when it is omitted. The ICU4X-backed
        // `stringNormalize` native validates the form at run time.
        let receiver = match self.mcall_string_normalize(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_managed(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_number_a(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_number_b(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_number_c(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        let receiver = match self.mcall_primitive_tail(cx, receiver, &mname, method, args, span) { Ok(e) => return e, Err(r) => r };
        self.mcall_instance_dispatch(cx, receiver, mname, method, args, span)
    }
}
