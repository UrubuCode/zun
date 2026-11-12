use crate::check::*;

impl Checker<'_> {
    /// Classes with runtime-decorated instance members run the
    /// instance-initializer / field-transform hooks on the new object.
    pub(in crate::check) fn wrap_hooked_new(
        &mut self,
        class_idx: u32,
        new: hir::Expr,
        span: Span,
    ) -> hir::Expr {
        if self.hooked_classes.contains(&class_idx) {
            if let Some((init_id, _)) = zun_natives::resolve("collections", "classInstanceInit") {
                let boxed = hir::Expr {
                    kind: hir::ExprKind::BoxAny {
                        value: Box::new(new),
                    },
                    ty: Ty::Any,
                    span,
                };
                let hooked = hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id: init_id,
                        args: vec![boxed],
                    },
                    ty: Ty::Any,
                    span,
                };
                return hir::Expr {
                    kind: hir::ExprKind::Convert {
                        value: Box::new(hooked),
                    },
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
    pub(in crate::check) fn check_super_call(
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
            if self.classes[this_class as usize].dynamic_base {
                // The runtime base is not modelled: evaluate the arguments for
                // their effects and treat the call as a no-op.
                for argument in args {
                    self.check_expr(cx, argument);
                }
                return hir::Expr {
                    kind: hir::ExprKind::Null,
                    ty: Ty::Void,
                    span,
                };
            }
            self.error(
                span,
                "`super` used in a class that has no base class (no `extends`)",
            );
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
                    self.error(
                        span,
                        "base class has no constructor; `super(...)` takes no arguments",
                    );
                    return self.poison(span);
                }
                return hir::Expr {
                    kind: hir::ExprKind::Null,
                    ty: Ty::Void,
                    span,
                };
            }
            self.error(mspan, format!("no method `{mname}` on the base class"));
            return self.poison(span);
        };
        if self.signatures[func as usize].is_static {
            self.error(
                mspan,
                format!("static method `{mname}` must be called through its class"),
            );
            return self.poison(span);
        }
        if self.signatures[func as usize].rest_param.is_some()
            || args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
        {
            self.error(
                span,
                "`super` calls do not support rest/spread parameters yet",
            );
            return self.poison(span);
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
        hir::Expr {
            kind: hir::ExprKind::StaticCall {
                func,
                args: call_args,
            },
            ty: return_ty,
            span,
        }
    }

    /// `super.m(args)` inside a static method of `class`: a direct call of the
    /// base class's static `m`, with the current `this` (the receiving class
    /// value) forwarded as its hidden first parameter.
    pub(in crate::check) fn check_static_super_call(
        &mut self,
        cx: &mut FunctionCx,
        class: u32,
        method: &ast::Name,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let Some(base) = self.classes[class as usize].base else {
            self.error(
                span,
                "`super` used in a class that has no base class (no `extends`)",
            );
            return self.poison(span);
        };
        let mname = self.text(method).to_string();
        let Some(func) = self.method_of(base, &mname) else {
            self.error(
                method.span,
                format!("no static method `{mname}` on the base class"),
            );
            return self.poison(span);
        };
        if !self.signatures[func as usize].is_static {
            self.error(
                method.span,
                format!("`{mname}` is not a static method of the base class"),
            );
            return self.poison(span);
        }
        let hidden_class = self.static_this.contains_key(&func);
        let (param_tys, return_ty, optional_count, signature_name) = {
            let sig = &self.signatures[func as usize];
            (
                sig.params[hidden_class as usize..].to_vec(),
                sig.return_ty,
                sig.optional_count,
                sig.name.clone(),
            )
        };
        let mut call_args: Vec<hir::Expr> = args.iter().map(|a| self.check_expr(cx, a)).collect();
        if call_args.len() > param_tys.len()
            || call_args.len() < param_tys.len().saturating_sub(optional_count)
        {
            self.error(
                span,
                format!(
                    "`{mname}` expects {} argument(s), found {}",
                    param_tys.len(),
                    call_args.len()
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
                hir::Expr {
                    ty: cx.locals[this_slot as usize],
                    kind: hir::ExprKind::Local(this_slot),
                    span,
                },
            );
        }
        hir::Expr {
            kind: hir::ExprKind::StaticCall {
                func,
                args: call_args,
            },
            ty: return_ty,
            span,
        }
    }

    /// Lower `new C(args)` for a class that declares `constructor(...)`.
    /// Fields are allocated at their default initializer (or the typed zero);
    /// the constructor then runs with `this` = the instance. Arguments are
    /// checked against the constructor's parameters (arity, optional
    /// parameters, types), not the field list.
    pub(in crate::check) fn check_new_with_constructor(
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
            let initializer = initializers
                .get(slot)
                .and_then(|init| init.as_ref())
                .filter(|init| {
                    field_ty == Ty::Any
                        || !matches!(
                            init.as_ref(),
                            ast::Expr::Unary {
                                op: ast::UnaryOp::Void,
                                ..
                            }
                        )
                });
            let mut value = match initializer {
                Some(init) => {
                    let init = init.clone();
                    self.check_expr(cx, &init)
                }
                None => match field_ty {
                    Ty::Int(_) | Ty::Float(_) => hir::Expr {
                        kind: hir::ExprKind::Number(0.0),
                        ty: Ty::F64,
                        span,
                    },
                    Ty::Boolean => hir::Expr {
                        kind: hir::ExprKind::Boolean(false),
                        ty: Ty::Boolean,
                        span,
                    },
                    Ty::String => hir::Expr {
                        kind: hir::ExprKind::String(String::new()),
                        ty: Ty::String,
                        span,
                    },
                    // `undefined`: a missing key on an empty dynamic object.
                    Ty::Any => hir::Expr {
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
                    },
                    other => hir::Expr {
                        kind: hir::ExprKind::Null,
                        ty: other,
                        span,
                    },
                },
            };
            coerce_numeric_literal(&mut value, field_ty);
            defaults.push(value);
        }

        let (param_tys, optional_count, rest_param, sig_name) = {
            let sig = &self.signatures[ctor as usize];
            (
                sig.params.clone(),
                sig.optional_count,
                sig.rest_param,
                sig.name.clone(),
            )
        };
        if let Some(rest_slot) = rest_param {
            // The arguments from the rest position on are gathered (spreads
            // flattened) into one array passed as the last constructor argument.
            let fixed = rest_slot - 1;
            if args.len() < fixed.saturating_sub(optional_count)
                || args[..args.len().min(fixed)]
                    .iter()
                    .any(|a| matches!(a, ast::Expr::Spread { .. }))
            {
                self.error(
                    span,
                    format!("`{name}` constructor expects at least {fixed} fixed argument(s)"),
                );
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
                kind: hir::ExprKind::NewWithConstructor {
                    class: class_idx,
                    defaults,
                    func: ctor,
                    args: call_args,
                },
                ty: Ty::Instance(class_idx),
                span,
            };
        }
        // param_tys[0] is `this`.
        let max = param_tys.len() - 1;
        // Extra arguments beyond the declared parameters are evaluated for
        // their effects and then ignored, as in JS.
        let (args, extra_args) =
            if args.len() > max && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. })) {
                args.split_at(max)
            } else {
                (args, &args[0..0])
            };
        let min = max.saturating_sub(optional_count);
        // A missing `any` parameter reads as `undefined` (JS passes fewer arguments freely).
        let padded: Vec<ast::Expr>;
        let args = if args.len() < min
            && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
            && param_tys[1 + args.len()..=min]
                .iter()
                .all(|ty| *ty == Ty::Any)
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
            kind: hir::ExprKind::NewWithConstructor {
                class: class_idx,
                defaults,
                func: ctor,
                args: call_args,
            },
            ty: Ty::Instance(class_idx),
            span,
        };
        for extra in extra_args {
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
    pub(in crate::check) fn check_error_new(
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
            None => hir::Expr {
                kind: hir::ExprKind::String(String::new()),
                ty: Ty::String,
                span,
            },
        };
        coerce_numeric_literal(&mut message, Ty::String);
        if !message.ty.is_error() && !self.is_assignable(message.ty, Ty::String) {
            self.error(message.span, format!("`{name}` message must be a string"));
        }
        // `cause` = `options.cause` (a missing key reads back as `undefined`).
        // A nullish options argument (`new Error("m", null)`) yields
        // `undefined` through an optional-chain guard instead of a TypeError.
        let cause = match args.get(1) {
            Some(arg) => {
                let guarded = ast::Expr::OptionalChain {
                    object: Box::new(arg.clone()),
                    body: Box::new(ast::Expr::Field {
                        object: Box::new(ast::Expr::ChainRoot { depth: 0, span }),
                        field: ast::synthetic_ident("cause"),
                        span,
                    }),
                    span,
                };
                let checked = self.check_expr(cx, &guarded);
                if checked.ty == Ty::Any || checked.ty.is_error() {
                    checked
                } else {
                    hir::Expr {
                        kind: hir::ExprKind::BoxAny {
                            value: Box::new(checked),
                        },
                        ty: Ty::Any,
                        span,
                    }
                }
            }
            None => hir::Expr {
                kind: hir::ExprKind::DynamicGet {
                    object: Box::new(hir::Expr {
                        kind: hir::ExprKind::DynamicObjectLiteral {
                            entries: Vec::new(),
                        },
                        ty: Ty::DynamicObject,
                        span,
                    }),
                    name: "cause".to_string(),
                },
                ty: Ty::Any,
                span,
            },
        };
        // `hasCause`: whether the options object carries the `cause` key. An
        // object literal with static keys answers at compile time; a plain
        // identifier is probed at run time; any other (possibly impure)
        // expression keeps the field default (`cause !== undefined` still
        // reports presence).
        let mut has_cause: Option<hir::Expr> = match args.get(1) {
            Some(ast::Expr::ObjectLiteral { properties, .. }) => {
                let mut keys = Vec::new();
                let mut simple = true;
                for member in properties {
                    match member {
                        ast::ObjectMember::Property(p) => match &p.key {
                            ast::PropertyKey::Ident(n) => keys.push(self.text(n).to_string()),
                            ast::PropertyKey::String { value, .. } => keys.push(value.clone()),
                            _ => simple = false,
                        },
                        _ => simple = false,
                    }
                }
                simple.then(|| hir::Expr {
                    kind: hir::ExprKind::Boolean(keys.iter().any(|k| k == "cause")),
                    ty: Ty::Boolean,
                    span,
                })
            }
            Some(arg @ ast::Expr::Ident(_)) => {
                match self.by_name.get("__zunErrorHasCause").copied() {
                    Some(func) => {
                        let options = self.check_expr(cx, arg);
                        Some(hir::Expr {
                            kind: hir::ExprKind::Call {
                                func,
                                args: vec![Self::box_to_any(options)],
                            },
                            ty: Ty::Boolean,
                            span,
                        })
                    }
                    None => None,
                }
            }
            _ => None,
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
                "hasCause" if has_cause.is_some() => has_cause.take().unwrap(),
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
            kind: hir::ExprKind::New {
                class: class_idx,
                args: new_args,
            },
            ty: Ty::Instance(class_idx),
            span,
        }
    }

    /// `new ArrayBuffer(length)` or `new ArrayBuffer(length, { maxByteLength })`.
    /// Without the options bag the buffer is fixed-length (`bufferNew`); with a
    /// `maxByteLength` it is resizable (`bufferResizableNew`, ES2024).
    pub(in crate::check) fn check_array_buffer_new(
        &mut self,
        cx: &mut FunctionCx,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        if args.is_empty() || args.len() > 2 {
            self.error(
                span,
                "`ArrayBuffer` expects a length and an optional options object",
            );
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
                    self.error(
                        other.span(),
                        "`ArrayBuffer` second argument must be an options object",
                    );
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
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: call_args,
            },
            ty: Ty::Buffer,
            span,
        }
    }
}
