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
        // `new Map(iterable, extra...)` / `new Set(iterable, extra...)`: the
        // extra arguments are evaluated and ignored.
        if args.len() > 1
            && (name == "Map" || name == "Set")
            && !self.classes_by_name.contains_key(&name)
            && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
        {
            let inner = ast::Expr::New { class: *class, args: args[..1].to_vec(), span };
            let mut result = self.check_expr(cx, &inner);
            for extra in &args[1..] {
                let effect = self.check_expr(cx, extra);
                let ty = result.ty;
                result = hir::Expr {
                    kind: hir::ExprKind::Sequence { effect: Box::new(effect), value: Box::new(result) },
                    ty,
                    span,
                };
            }
            return result;
        }
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
                    // `new WeakSet(iterable)` / `new WeakMap(iterable)`.
                    if args.len() == 1 && matches!(name.as_str(), "WeakSet" | "WeakMap") {
                        let helper = if name == "WeakSet" { "__zunWeakSetFrom" } else { "__zunWeakMapFrom" };
                        if let Some(&func) = self.by_name.get(helper) {
                            let argument = self.check_expr(cx, &args[0]);
                            return hir::Expr {
                                kind: hir::ExprKind::Call { func, args: vec![Self::box_to_any(argument)] },
                                ty,
                                span,
                            };
                        }
                    }
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
                                // A non-callable executor (reachable through an
                                // `as any` cast) throws a TypeError at runtime.
                                let throw = self.runtime_type_error(cx, span);
                                return hir::Expr {
                                    kind: hir::ExprKind::Sequence {
                                        effect: Box::new(executor),
                                        value: Box::new(throw),
                                    },
                                    ty: Ty::Any,
                                    span,
                                };
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
                            let native = if argument.ty != Ty::Any && self.is_assignable(argument.ty, Ty::Array) {
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
                    if let Some(call) = self.array_ctor_dynamic(cx, args, span) {
                        return call;
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
                        if name == "RegExp" {
                            // A dynamic (`any`) pattern or flags operand goes
                            // through ToString, as `new RegExp(x)` does.
                            let mut converted = Vec::with_capacity(checked.len());
                            for arg in checked.drain(..) {
                                converted.push(if arg.ty == Ty::Any {
                                    self.to_string_for_parse(arg)
                                } else {
                                    arg
                                });
                            }
                            checked = converted;
                        }
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
            if let Some(call) = self.array_ctor_dynamic(cx, args, span) {
                return call;
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
        // `new Number(x)`: the wrapper object, lowered to
        // `new NumberObject(<ToNumber of x>)`.
        if name == "Number" && !self.classes_by_name.contains_key(&name) {
            let Some(&wrapper) = self.classes_by_name.get("NumberObject") else {
                self.error(class.span, "missing `NumberObject` wrapper class");
                return self.poison(span);
            };
            if args.len() > 1 {
                self.error(span, "`Number` expects at most one argument");
                return self.poison(span);
            }
            let value = match args.first() {
                None => hir::Expr {
                    kind: hir::ExprKind::Number(0.0),
                    ty: Ty::F64,
                    span,
                },
                Some(arg) => {
                    let mut arg = self.check_expr(cx, arg);
                    coerce_numeric_literal(&mut arg, Ty::F64);
                    self.to_number_for_arg(arg)
                }
            };
            return hir::Expr {
                kind: hir::ExprKind::New { class: wrapper, args: vec![value] },
                ty: Ty::Instance(wrapper),
                span,
            };
        }
        if name == "String" && !self.classes_by_name.contains_key(&name) {
            let Some(&wrapper) = self.classes_by_name.get("StringObject") else {
                self.error(class.span, "missing `StringObject` wrapper class");
                return self.poison(span);
            };
            if args.len() > 1 {
                self.error(span, "`String` expects at most one argument");
                return self.poison(span);
            }
            let text = match args.first() {
                None => hir::Expr {
                    kind: hir::ExprKind::String(String::new()),
                    ty: Ty::String,
                    span,
                },
                Some(arg) => {
                    let arg = self.check_expr(cx, arg);
                    if arg.ty.is_error() {
                        return self.poison(span);
                    }
                    if arg.ty == Ty::String {
                        arg
                    } else if let Some(&func) = self.by_name.get("__coerceString") {
                        hir::Expr {
                            kind: hir::ExprKind::Call {
                                func,
                                args: vec![Self::box_to_any(arg)],
                            },
                            ty: Ty::String,
                            span,
                        }
                    } else {
                        self.error(span, "missing `__coerceString` helper");
                        return self.poison(span);
                    }
                }
            };
            return hir::Expr {
                kind: hir::ExprKind::New { class: wrapper, args: vec![text] },
                ty: Ty::Instance(wrapper),
                span,
            };
        }
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
            // An undeclared constructor name is a run-time ReferenceError
            // (it may sit in dead code), like an undeclared identifier read.
            if cx.lookup(&name).is_none() && !self.by_name.contains_key(&name) && !self.module_globals.contains(&name) {
                if let Some((id, _)) = zun_natives::resolve("collections", "globalRead") {
                    return hir::Expr {
                        kind: hir::ExprKind::CallBuiltin {
                            id,
                            args: vec![Self::string_literal(name.clone(), span)],
                        },
                        ty: Ty::Any,
                        span,
                    };
                }
            }
            // `new x(...)` where `x` is a variable holding a class value:
            // construct through the class descriptor at run time.
            if cx.lookup(&name).is_some() || self.module_globals.contains(&name) {
                let callee = self.check_expr(cx, &ast::Expr::Ident(*class));
                if !callee.ty.is_error() && matches!(callee.ty, Ty::Any | Ty::Function(_) | Ty::DynamicObject) {
                    if let Some((id, _)) = zun_natives::resolve("collections", "reflectConstruct") {
                        let elements: Vec<Option<hir::Expr>> =
                            args.iter().map(|a| Some(self.check_expr(cx, a))).collect();
                        let args_array = hir::Expr {
                            kind: hir::ExprKind::ArrayLiteral { elements },
                            ty: Ty::Array,
                            span,
                        };
                        return hir::Expr {
                            kind: hir::ExprKind::CallBuiltin { id, args: vec![Self::box_to_any(callee), args_array] },
                            ty: Ty::Any,
                            span,
                        };
                    }
                }
            }
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
}
