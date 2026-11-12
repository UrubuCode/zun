//! Realm-global constructor lowering for `check_new` (see `new.rs`).
//!
//! `new <Name>(...)` for a registered realm base type (PLAN §3.1) lowers
//! through the type's recorded `CtorLowering` shape. `try_realm_ctor` matches
//! the shape and dispatches to one helper per arm; `FallThrough` /
//! `ClassFallThrough` return `None` so `check_new` continues with its
//! hardcoded resolution. Each helper returns the finished `hir::Expr`,
//! preserving the original arm bodies exactly.

use crate::check::*;
use crate::realm::CtorLowering;

impl Checker<'_> {
    /// Lower `new <name>(...)` through the realm registry's ctor shape, if any.
    /// `Some` = handled; `None` = fall through to the hardcoded paths.
    pub(in crate::check) fn try_realm_ctor(
        &mut self,
        cx: &mut FunctionCx,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Option<hir::Expr> {
        let ctor = self.realm_globals.get(name)?.ctor.clone();
        match ctor {
            CtorLowering::Intrinsic0 { module, native, ty } => {
                Some(self.new_ctor_intrinsic0(cx, name, args, span, module, native, ty))
            }
            CtorLowering::Promise => Some(self.new_ctor_promise(cx, args, span)),
            CtorLowering::CollectionFrom {
                module,
                empty,
                from,
                from_iterable,
                ty,
            } => Some(self.new_ctor_collection_from(
                cx,
                name,
                args,
                span,
                module,
                empty,
                from,
                from_iterable,
                ty,
            )),
            CtorLowering::IntrinsicN {
                module,
                native,
                ty,
                min_args,
                max_args,
                pad_empty_flags,
            } => Some(self.new_ctor_intrinsic_n(
                cx,
                name,
                args,
                span,
                module,
                native,
                ty,
                min_args,
                max_args,
                pad_empty_flags,
            )),
            CtorLowering::ArrayNew => Some(self.new_ctor_array_new(cx, args, span)),
            CtorLowering::ArrayBuffer => {
                let before = self.diagnostics.len();
                let typed = self.check_array_buffer_new(cx, args, span);
                if self.diagnostics.len() == before {
                    return Some(typed);
                }
                self.diagnostics.truncate(before);
                Some(self.new_buffer_dynamic(cx, name, args, span))
            }
            CtorLowering::BufferView { native, kind } => {
                if !matches!(kind, hir::BufferViewKind::DataView)
                    && !args.iter().any(|a| matches!(a, ast::Expr::Spread { .. }))
                    && !self.typed_ctor_fits_static(cx, args)
                {
                    return Some(self.new_buffer_dynamic(cx, name, args, span));
                }
                if matches!(kind, hir::BufferViewKind::DataView) {
                    if !self.typed_ctor_fits_static(cx, args) || !self.first_arg_is_buffer(cx, args) {
                        return Some(self.new_buffer_dynamic(cx, name, args, span));
                    }
                    let before = self.diagnostics.len();
                    let typed = self.new_ctor_buffer_view(cx, name, args, span, native, kind);
                    if self.diagnostics.len() == before {
                        return Some(typed);
                    }
                    self.diagnostics.truncate(before);
                    return Some(self.new_buffer_dynamic(cx, name, args, span));
                }
                Some(self.new_ctor_buffer_view(cx, name, args, span, native, kind))
            }
            // The registry owns this name's annotation and descriptor
            // value; `new <Name>()` keeps using the existing path.
            // `ClassFallThrough` (Error + subclasses) likewise falls
            // through to the class-construction path below.
            CtorLowering::FallThrough | CtorLowering::ClassFallThrough => None,
        }
    }

    #[allow(clippy::too_many_arguments)] // mirrors the realm intrinsic constructor context
    fn new_ctor_intrinsic0(
        &mut self,
        cx: &mut FunctionCx,
        name: &str,
        args: &[ast::Expr],
        span: Span,
        module: &'static str,
        native: &'static str,
        ty: Ty,
    ) -> hir::Expr {
        // `new WeakSet(iterable)` / `new WeakMap(iterable)`.
        if args.len() == 1 && matches!(name, "WeakSet" | "WeakMap") {
            let helper = if name == "WeakSet" {
                "__zunWeakSetFrom"
            } else {
                "__zunWeakMapFrom"
            };
            if let Some(&func) = self.by_name.get(helper) {
                let argument = self.check_expr(cx, &args[0]);
                return hir::Expr {
                    kind: hir::ExprKind::Call {
                        func,
                        args: vec![Self::box_to_any(argument)],
                    },
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
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: Vec::new(),
            },
            ty,
            span,
        }
    }

    fn new_ctor_promise(
        &mut self,
        cx: &mut FunctionCx,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
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
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: Vec::new(),
                    },
                    ty: Ty::Promise,
                    span,
                }
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
                    && !self.is_assignable(executor.ty, Ty::Function(hir::DYNAMIC_FUNCTION))
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
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![executor],
                    },
                    ty: Ty::Promise,
                    span,
                }
            }
            _ => {
                self.error(
                    span,
                    format!(
                        "`new Promise` expects at most one executor argument, found {}",
                        args.len()
                    ),
                );
                self.poison(span)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn new_ctor_collection_from(
        &mut self,
        cx: &mut FunctionCx,
        name: &str,
        args: &[ast::Expr],
        span: Span,
        module: &'static str,
        empty: &'static str,
        from: &'static str,
        from_iterable: &'static str,
        ty: Ty,
    ) -> hir::Expr {
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
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: Vec::new(),
                    },
                    ty,
                    span,
                }
            }
            1 => {
                let argument = self.check_expr(cx, &args[0]);
                // An array keeps the dedicated fast path; a
                // known built-in iterable or an `any` value
                // routes to the general native.
                let native = if argument.ty != Ty::Any && self.is_assignable(argument.ty, Ty::Array)
                {
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
                hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![argument],
                    },
                    ty,
                    span,
                }
            }
            _ => {
                self.error(
                    span,
                    format!(
                        "`{name}` expects at most one constructor argument, found {}",
                        args.len()
                    ),
                );
                self.poison(span)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn new_ctor_intrinsic_n(
        &mut self,
        cx: &mut FunctionCx,
        name: &str,
        args: &[ast::Expr],
        span: Span,
        module: &'static str,
        native: &'static str,
        ty: Ty,
        min_args: usize,
        max_args: usize,
        pad_empty_flags: bool,
    ) -> hir::Expr {
        if args.len() < min_args || args.len() > max_args {
            // An argument count outside the typed overload is not a static
            // error: the constructor is built at run time through its class
            // descriptor, which applies the spec's argument rules and throws
            // the appropriate TypeError (e.g. `new WeakRef()` with no target).
            return self.new_buffer_dynamic(cx, name, args, span);
        }
        let Some((id, _)) = zun_natives::resolve(module, native) else {
            self.error(span, format!("missing intrinsic `{native}`"));
            return self.poison(span);
        };
        let mut checked: Vec<hir::Expr> = args.iter().map(|arg| self.check_expr(cx, arg)).collect();
        if pad_empty_flags && checked.len() == 1 {
            checked.push(hir::Expr {
                kind: hir::ExprKind::String(String::new()),
                ty: Ty::String,
                span,
            });
        }
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin { id, args: checked },
            ty,
            span,
        }
    }

    fn new_ctor_array_new(
        &mut self,
        cx: &mut FunctionCx,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        if let Some(call) = self.array_ctor_dynamic(cx, args, span) {
            return call;
        }
        let mut length = self.check_expr(cx, &args[0]);
        coerce_numeric_literal(&mut length, Ty::INT);
        if !length.ty.is_int() {
            self.error(length.span, "array length must be an integer");
        }
        hir::Expr {
            kind: hir::ExprKind::ArrayNew {
                length: Box::new(length),
            },
            ty: Ty::Array,
            span,
        }
    }

    #[allow(clippy::unnecessary_unwrap)] // guarded unwrap reads clearer than if-let here
    fn new_ctor_buffer_view(
        &mut self,
        cx: &mut FunctionCx,
        name: &str,
        args: &[ast::Expr],
        span: Span,
        native: &'static str,
        kind: hir::BufferViewKind,
    ) -> hir::Expr {
        // `new DataView(buffer)` / `new DataView(buffer,
        // offset)` with no explicit length is
        // length-tracking (ES2024): its `byteLength`
        // follows the buffer's current length.
        if matches!(kind, hir::BufferViewKind::DataView) && (args.len() == 1 || args.len() == 2) {
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
            let Some((id, _)) = zun_natives::resolve("buffer", "dataViewTrackingNew") else {
                self.error(span, "missing DataView intrinsic `dataViewTrackingNew`");
                return self.poison(span);
            };
            return hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![buffer, offset],
                },
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
        let mut prechecked_arg0: Option<hir::Expr> = None;
        if !matches!(kind, hir::BufferViewKind::DataView) && args.len() == 1 {
            match self.new_typed_single_arg(cx, args, span, kind) {
                Ok(expr) => return expr,
                // The single argument was neither numeric nor an array (a
                // buffer): it is already checked, so reuse it below instead of
                // checking `args[0]` a second time.
                Err(checked) => prechecked_arg0 = Some(checked),
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
            let buffer = match prechecked_arg0.take() {
                Some(expr) => expr,
                None => self.check_expr(cx, &args[0]),
            };
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
            let Some((id, _)) = zun_natives::resolve("buffer", "typedArrayTrackingNew") else {
                self.error(
                    span,
                    "missing typed-array intrinsic `typedArrayTrackingNew`",
                );
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
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: call_args,
            },
            ty: Ty::BufferView(kind),
            span,
        }
    }

    /// `new <Kind>Array(x)` with a single argument: an element count (numeric)
    /// allocates and zero-fills a tracking view; an array/typed-array copies
    /// like `<Kind>Array.from`. Returns `Err(checked_arg)` when the argument is
    /// neither (a buffer), handing back the already-checked argument so the
    /// caller falls through to the view path without re-checking it.
    fn new_typed_single_arg(
        &mut self,
        cx: &mut FunctionCx,
        args: &[ast::Expr],
        span: Span,
        kind: hir::BufferViewKind,
    ) -> Result<hir::Expr, hir::Expr> {
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
                return Ok(hir::Expr {
                    kind: hir::ExprKind::CallBuiltin {
                        id,
                        args: vec![length, kind_lit],
                    },
                    ty: Ty::BufferView(kind),
                    span,
                });
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
                return Ok(self.poison(span));
            };
            let buffer = hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id: buf_id,
                    args: vec![byte_len],
                },
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
                    crate::typed_arrays::element_index(kind).unwrap_or(0) as f64
                ),
                ty: Ty::Int(IntKind::U32),
                span,
            };
            let Some((id, _)) = zun_natives::resolve("buffer", "typedArrayTrackingNew") else {
                self.error(
                    span,
                    "missing typed-array intrinsic `typedArrayTrackingNew`",
                );
                return Ok(self.poison(span));
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::CallBuiltin {
                    id,
                    args: vec![buffer, zero, kind_lit],
                },
                ty: Ty::BufferView(kind),
                span,
            });
        }
        Err(length)
    }
}

impl Checker<'_> {
    /// `new <TypedArray | DataView | ArrayBuffer>(...)` resolved at run time
    /// through the constructor's class descriptor: the argument list is
    /// coerced by ECMAScript's rules (ToIndex, iterables, array-likes, ...)
    /// rather than by the static overloads.
    pub(in crate::check) fn new_buffer_dynamic(
        &mut self,
        cx: &mut FunctionCx,
        name: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> hir::Expr {
        let Some(token) = self.realm_globals.get(name).map(|binding| binding.token) else {
            return self.poison(span);
        };
        let elements: Vec<Option<hir::Expr>> = args
            .iter()
            .map(|a| Some(Self::box_to_any(self.check_expr(cx, a))))
            .collect();
        let args_array = hir::Expr {
            kind: hir::ExprKind::ArrayLiteral { elements },
            ty: Ty::Array,
            span,
        };
        let Some((id, _)) = zun_natives::resolve("collections", "proxyConstruct") else {
            return self.poison(span);
        };
        hir::Expr {
            kind: hir::ExprKind::CallBuiltin {
                id,
                args: vec![self.realm_descriptor_expr(token, span), args_array],
            },
            ty: Ty::Any,
            span,
        }
    }

    /// Whether a typed-array constructor call fits one of the statically
    /// typed overloads; anything else is built at run time.
    fn typed_ctor_fits_static(&mut self, cx: &mut FunctionCx, args: &[ast::Expr]) -> bool {
        let before = self.diagnostics.len();
        let checked: Vec<hir::Expr> = args.iter().map(|a| self.check_expr(cx, a)).collect();
        self.diagnostics.truncate(before);
        let literal_int = |e: &hir::Expr| {
            matches!(e.kind, hir::ExprKind::Number(n) if n.fract() == 0.0 && n >= 0.0)
        };
        let int_like = |e: &hir::Expr| e.ty.is_int() || literal_int(e);
        match checked.as_slice() {
            [only] => {
                int_like(only)
                    || matches!(only.ty, Ty::Array | Ty::BufferView(_) | Ty::Buffer)
            }
            [buffer, offset] => buffer.ty == Ty::Buffer && int_like(offset),
            [buffer, offset, length] => {
                buffer.ty == Ty::Buffer && int_like(offset) && int_like(length)
            }
            _ => false,
        }
    }
}

impl Checker<'_> {
    /// Whether the first constructor argument is statically an `ArrayBuffer`.
    fn first_arg_is_buffer(&mut self, cx: &mut FunctionCx, args: &[ast::Expr]) -> bool {
        let Some(first) = args.first() else {
            return false;
        };
        let before = self.diagnostics.len();
        let checked = self.check_expr(cx, first);
        self.diagnostics.truncate(before);
        checked.ty == Ty::Buffer
    }
}
