//! Run-time classes: classes that are values rather than static declarations.
//!
//! Zun hoists a class to one program-global TypeDef, so every evaluation of
//! its source text yields the same descriptor. Two shapes cannot be modelled
//! that way:
//!
//! * a class whose **base is only a value at run time** (`class extends Base`
//!   with `Base` a parameter or a variable: a mixin) -- there is no static
//!   layout to inherit from, and `super` must reach whatever the value is;
//! * a class expression used **as a value inside a function**
//!   (`return class { ... }`) -- each call must produce its own constructor
//!   and prototype, and its members close over the function's locals.
//!
//! Such a class is lowered here, at the syntax boundary, to the closure form
//! the dynamic object model already supports: the constructor is a function
//! value, the members are function values installed on its `prototype`, and
//! the whole definition sits in an immediately invoked arrow so that each
//! evaluation gets fresh bindings. The prelude's `__zunClass*` helpers
//! (`builtin/runtime_class.zun`) carry the class semantics (heritage checks,
//! `super(...)`, descriptors). No type is invented at run time and nothing
//! crosses the compile/run boundary except ordinary IL.
//!
//! Members that need per-class static knowledge (`#private` names,
//! decorators, parameter properties) are not lowered; a class using them
//! keeps the static path, including its diagnostics.

use super::*;

/// Where a class appears, which decides whether it must be a run-time value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ClassSite {
    /// A `class Name {}` statement.
    Declaration,
    /// A class expression that directly initializes a binding or is the
    /// callee of `new`: evaluated where it is named, so it stays static.
    Bound,
    /// A class expression in any other position: a first-class value.
    Value,
}

/// The source extent of a function being converted, for scope questions that
/// the adapter answers textually.
#[derive(Clone, Copy)]
struct FunctionScope {
    params: (u32, u32),
    whole: (u32, u32),
}

/// What `super` and `new.target` mean inside the run-time class member being
/// converted.
#[derive(Clone, Copy)]
pub(super) struct RuntimeSuper {
    /// The binding holding the class constructor.
    class: Name,
    /// Whether the member is static (its home object is the constructor).
    is_static: bool,
    /// In a derived constructor: the binding that holds `this` once
    /// `super(...)` has run.
    ctor_this: Option<Name>,
    /// The binding holding the field-initializer function, if any.
    init: Option<Name>,
    /// Whether the member is the constructor (so `new.target` is defined).
    in_ctor: bool,
}

thread_local! {
    /// The functions enclosing the node being converted, innermost last.
    static FUNCTION_SCOPES: std::cell::RefCell<Vec<FunctionScope>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// What a `this` expression converts to, per enclosing non-arrow function
    /// (arrows inherit). `None` is the ordinary `this`; otherwise the binding
    /// that stands for it, and whether that binding is a one-element cell
    /// (see [`this_expr`]).
    static THIS_ALIAS: std::cell::RefCell<Vec<Option<(Name, bool)>>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The `super` meaning per enclosing non-arrow function; `None` leaves
    /// `super` to the checker (a static class member).
    static RUNTIME_SUPER: std::cell::RefCell<Vec<Option<RuntimeSuper>>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Frames handed to the next non-arrow function converted (the member
    /// function of a run-time class), consumed by [`enter_function`].
    static NEXT_THIS_ALIAS: std::cell::Cell<Option<(Name, bool)>> =
        const { std::cell::Cell::new(None) };
    static NEXT_RUNTIME_SUPER: std::cell::Cell<Option<RuntimeSuper>> =
        const { std::cell::Cell::new(None) };
    /// Set while the callee of `new (class {})` is converted.
    static NEW_CALLEE_CLASS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Pops the frames pushed by [`enter_function`] / [`enter_member_frames`].
pub(super) struct FrameGuard {
    scope: bool,
    frames: bool,
}

impl Drop for FrameGuard {
    fn drop(&mut self) {
        if self.scope {
            FUNCTION_SCOPES.with(|scopes| {
                scopes.borrow_mut().pop();
            });
        }
        if self.frames {
            THIS_ALIAS.with(|stack| {
                stack.borrow_mut().pop();
            });
            RUNTIME_SUPER.with(|stack| {
                stack.borrow_mut().pop();
            });
        }
    }
}

/// Record that a function is being converted. A non-arrow function starts a
/// fresh `this` / `super` context (taking the frames queued for it, if it is
/// a run-time class member); an arrow inherits its enclosing one.
pub(super) fn enter_function(
    params: oxc_span::Span,
    whole: oxc_span::Span,
    is_arrow: bool,
) -> FrameGuard {
    FUNCTION_SCOPES.with(|scopes| {
        scopes.borrow_mut().push(FunctionScope {
            params: (params.start, params.end),
            whole: (whole.start, whole.end),
        })
    });
    if !is_arrow {
        let alias = NEXT_THIS_ALIAS.with(std::cell::Cell::take);
        let home = NEXT_RUNTIME_SUPER.with(std::cell::Cell::take);
        THIS_ALIAS.with(|stack| stack.borrow_mut().push(alias));
        RUNTIME_SUPER.with(|stack| stack.borrow_mut().push(home));
    }
    FrameGuard {
        scope: true,
        frames: !is_arrow,
    }
}

/// Start a `this` / `super` context for code that is not a function of its
/// own: a static class body (both `None`), or a run-time class's static
/// initializers.
pub(super) fn enter_member_frames(alias: Option<Name>, home: Option<RuntimeSuper>) -> FrameGuard {
    THIS_ALIAS.with(|stack| stack.borrow_mut().push(alias.map(|name| (name, false))));
    RUNTIME_SUPER.with(|stack| stack.borrow_mut().push(home));
    FrameGuard {
        scope: false,
        frames: true,
    }
}

/// Mark the class expression about to be converted as the callee of `new`.
pub(super) fn set_new_callee_class(value: bool) {
    NEW_CALLEE_CLASS.with(|flag| flag.set(value));
}

/// Whether the class expression being converted was marked as the callee of
/// `new`; reading the mark clears it, so it never reaches a nested class.
pub(super) fn take_new_callee_class() -> bool {
    NEW_CALLEE_CLASS.with(|flag| flag.replace(false))
}

/// Whether the node being converted sits in a member of a run-time class, so
/// `super` is lowered here instead of being left to the checker.
pub(super) fn in_runtime_class() -> bool {
    runtime_super().is_some()
}

fn runtime_super() -> Option<RuntimeSuper> {
    RUNTIME_SUPER.with(|stack| stack.borrow().last().copied().flatten())
}

/// The expression a `this` keyword at `span` converts to.
///
/// In a derived run-time class constructor `this` is whatever `super(...)`
/// returned, so it lives in a binding that is written after it is declared
/// and read by arrows created later. That binding is a one-element array
/// (`[value]`) that is never reassigned, read as `cell[0]`: an arrow then
/// captures the array by value and still sees the write. A plain reassigned
/// local would need the checker to box it up front, which it decides from the
/// closure's source text -- where this compiler-made name does not appear.
pub(super) fn this_expr(span: Span) -> Expr {
    match THIS_ALIAS.with(|stack| stack.borrow().last().copied().flatten()) {
        Some((cell, true)) => this_cell_read(cell, span),
        Some((alias, false)) => Expr::Ident(alias),
        None => Expr::Ident(Name { span }),
    }
}

fn this_cell_read(cell: Name, span: Span) -> Expr {
    Expr::Index {
        object: Box::new(Expr::Ident(cell)),
        index: Box::new(Expr::Number { value: 0.0, span }),
        span,
    }
}

fn ident(text: &str) -> Expr {
    Expr::Ident(crate::ast::synthetic_ident(text))
}

fn helper_call(name: &str, args: Vec<Expr>, span: Span) -> Expr {
    Expr::Call {
        callee: crate::ast::synthetic_ident(name),
        args,
        span,
    }
}

fn undefined(span: Span) -> Expr {
    Expr::Unary {
        op: crate::ast::UnaryOp::Void,
        operand: Box::new(Expr::Number { value: 0.0, span }),
        span,
    }
}

fn any_type() -> Option<crate::ast::TypeAnnotation> {
    Some(crate::ast::TypeAnnotation {
        name: crate::ast::synthetic_ident("any"),
        is_array: false,
    })
}

fn let_any(name: Name, init: Expr, span: Span) -> Stmt {
    Stmt::Let {
        name,
        ty: any_type(),
        init,
        span,
    }
}

/// The home object of the member being converted: the base's prototype for
/// an instance member, the base constructor for a static one.
fn home_object(home: RuntimeSuper, span: Span) -> Expr {
    let subject = if home.is_static {
        Expr::Ident(home.class)
    } else {
        Expr::Field {
            object: Box::new(Expr::Ident(home.class)),
            field: crate::ast::synthetic_ident("prototype"),
            span,
        }
    };
    Expr::MethodCall {
        receiver: Box::new(ident("Object")),
        method: crate::ast::synthetic_ident("getPrototypeOf"),
        args: vec![subject],
        span,
    }
}

/// `super(args)` inside a run-time class constructor, or `None` when `super`
/// belongs to a static class (the checker lowers that one).
pub(super) fn runtime_super_call(args: Vec<Expr>, span: Span) -> Option<Result<Expr, String>> {
    let home = runtime_super()?;
    let Some(this_binding) = home.ctor_this else {
        return Some(Err(
            "`super(...)` is only valid in the constructor of a class with a base".into(),
        ));
    };
    let elements = args
        .into_iter()
        .map(|arg| match arg {
            Expr::Spread { expr, .. } => crate::ast::ArrayElement::Spread(*expr),
            other => crate::ast::ArrayElement::Expr(other),
        })
        .collect();
    let init = match home.init {
        Some(init) => Expr::Ident(init),
        None => undefined(span),
    };
    Some(Ok(Expr::IndexAssign {
        object: Box::new(Expr::Ident(this_binding)),
        index: Box::new(Expr::Number { value: 0.0, span }),
        value: Box::new(helper_call(
            "__zunSuperCall",
            vec![
                Expr::Ident(home.class),
                ident("this"),
                Expr::ArrayLiteral { elements, span },
                init,
            ],
            span,
        )),
        span,
    }))
}

/// `super.name(args)` / `super[key](args)` inside a run-time class member:
/// the base's member, called on the current `this`.
pub(super) fn runtime_super_member_call(
    key: Result<Name, Expr>,
    args: Vec<Expr>,
    span: Span,
) -> Option<Expr> {
    let home = runtime_super()?;
    let target = match key {
        Ok(field) => Expr::Field {
            object: Box::new(home_object(home, span)),
            field,
            span,
        },
        Err(index) => Expr::Index {
            object: Box::new(home_object(home, span)),
            index: Box::new(index),
            span,
        },
    };
    let mut call_args = vec![this_expr(crate::ast::synthetic_ident("this").span)];
    call_args.extend(args);
    Some(Expr::MethodCall {
        receiver: Box::new(target),
        method: crate::ast::synthetic_ident("call"),
        args: call_args,
        span,
    })
}

/// `super.name` / `super[key]` read inside a run-time class member: a lookup
/// that starts at the home object with the current `this` as the receiver.
pub(super) fn runtime_super_member_read(key: Expr, span: Span) -> Option<Expr> {
    let home = runtime_super()?;
    Some(Expr::MethodCall {
        receiver: Box::new(ident("Reflect")),
        method: crate::ast::synthetic_ident("get"),
        args: vec![
            home_object(home, span),
            key,
            this_expr(crate::ast::synthetic_ident("this").span),
        ],
        span,
    })
}

/// `new.target` inside a run-time class constructor.
pub(super) fn runtime_new_target(span: Span) -> Option<Expr> {
    let home = runtime_super().filter(|home| home.in_ctor)?;
    Some(helper_call(
        "__zunNewTarget",
        vec![Expr::Ident(home.class), ident("this")],
        span,
    ))
}

/// Built-in constructors the checker already gives a (base-less) meaning to
/// as a heritage name; they keep that path.
const CHECKER_BASES: &[&str] = &[
    "Iterator", "BigInt", "Symbol", "Object", "Function", "Array", "String", "Number", "Boolean",
    "Error", "Proxy", "Reflect", "Date", "RegExp", "Map", "Set", "WeakMap", "WeakSet", "Promise",
];

/// Byte offsets of the whole-word occurrences of `word` in `text`.
fn word_positions<'t>(text: &'t str, word: &'t str) -> impl Iterator<Item = usize> + 't {
    let mut from = 0;
    std::iter::from_fn(move || {
        while let Some(found) = text.get(from..)?.find(word) {
            let start = from + found;
            let end = start + word.len();
            from = end;
            let before = text[..start].chars().next_back();
            let after = text[end..].chars().next();
            if !before.is_some_and(is_identifier_char) && !after.is_some_and(is_identifier_char) {
                return Some(start);
            }
        }
        None
    })
}

/// Whether the text following a binding name initializes it with a class
/// expression (`= class ...`, optionally after a type annotation).
fn initializer_is_class(after_name: &str) -> bool {
    let rest = after_name.trim_start();
    let rest = if let Some(annotated) = rest.strip_prefix(':') {
        let mut found = None;
        let bytes = annotated.as_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            match byte {
                b'=' if bytes.get(index + 1) != Some(&b'>') => {
                    found = Some(index);
                    break;
                }
                b';' | b'\n' => break,
                _ => {}
            }
        }
        match found {
            Some(index) => &annotated[index..],
            None => return false,
        }
    } else {
        rest
    };
    let Some(value) = rest.strip_prefix('=') else {
        return false;
    };
    let value = value.trim_start();
    value
        .strip_prefix("class")
        .is_some_and(|tail| !tail.chars().next().is_some_and(is_identifier_char))
}

/// How `text` declares `name` as a variable or function, if it does:
/// `Some(true)` for a binding initialized by a class expression (which is
/// hoisted under the binding's name, so it is a static class).
fn declared_binding(text: &str, name: &str) -> Option<bool> {
    let mut declared = None;
    for start in word_positions(text, name) {
        let before = text[..start].trim_end();
        let keyword = ["const", "let", "var", "function"].iter().any(|keyword| {
            before.strip_suffix(keyword).is_some_and(|head| {
                !head.chars().next_back().is_some_and(is_identifier_char)
            })
        });
        if !keyword {
            continue;
        }
        if initializer_is_class(&text[start + name.len()..]) {
            return Some(true);
        }
        declared = Some(false);
    }
    declared
}

/// Whether `name` is a class the program declares (so it can be a static
/// base): a class declaration, a built-in carrier root, a binding initialized
/// by a class expression, or an imported name.
fn is_known_class_name(source: &str, name: &str) -> bool {
    let carrier = format!("__Builtin{name}");
    for candidate in [name, carrier.as_str()] {
        for start in word_positions(source, candidate) {
            let before = source[..start].trim_end();
            if before.strip_suffix("class").is_some_and(|head| {
                !head.chars().next_back().is_some_and(is_identifier_char)
            }) {
                return true;
            }
        }
    }
    if declared_binding(source, name) == Some(true) {
        return true;
    }
    for start in word_positions(source, "import") {
        let tail = &source[start..];
        let clause = tail.find("from").map_or("", |end| &tail[..end]);
        if clause.len() < 4096 && contains_word(clause, name) {
            return true;
        }
    }
    false
}

/// Whether the identifier `name`, used as a heritage at source offset `at`,
/// names a run-time value rather than a static class.
fn identifier_is_runtime_value(source: &str, name: &str, at: u32) -> bool {
    let scopes = FUNCTION_SCOPES.with(|scopes| scopes.borrow().clone());
    for scope in scopes.iter().rev() {
        let params = source
            .get(scope.params.0 as usize..scope.params.1 as usize)
            .unwrap_or_default();
        if contains_word(params, name) {
            return true;
        }
        let end = at.clamp(scope.whole.0, scope.whole.1);
        let body = source
            .get(scope.whole.0 as usize..end as usize)
            .unwrap_or_default();
        match declared_binding(body, name) {
            Some(true) => return false,
            Some(false) => return true,
            None => {}
        }
    }
    if CHECKER_BASES.contains(&name) {
        return false;
    }
    if is_known_class_name(source, name) {
        return false;
    }
    // A value has to come from somewhere: a name the program mentions nowhere
    // but in a heritage clause is neither a class nor a binding, and stays on
    // the static path so the checker reports the unknown base class. (This
    // also covers bindings no keyword scan would find: a destructured loop
    // variable, a catch parameter.)
    word_positions(source, name).nth(1).is_some()
}

/// Whether the class's base is only known at run time.
fn heritage_is_dynamic(source: &str, class: &oxc_ast::ast::Class<'_>) -> bool {
    let Some(heritage) = class.heritage.as_ref() else {
        return false;
    };
    match peel_heritage(&heritage.expression) {
        Expression::NullLiteral(_) => false,
        Expression::Identifier(identifier) => {
            let own = class.id.as_ref().map(|id| id.name.as_str());
            own != Some(identifier.name.as_str())
                && identifier_is_runtime_value(source, identifier.name.as_str(), class.span.start)
        }
        // A class expression is hoisted like any other; it is a static base
        // unless it is itself a run-time class.
        Expression::ClassExpression(inner) => {
            heritage_is_dynamic(source, inner)
        }
        // `f()` where `function f(...): typeof Name` names its base statically.
        Expression::CallExpression(call) => !matches!(&call.callee,
            Expression::Identifier(callee)
                if declared_typeof_return(source, callee.name.as_str()).is_some()),
        _ => true,
    }
}

/// Whether every member of the class can be expressed in the closure form.
fn lowering_supported(class: &oxc_ast::ast::Class<'_>) -> bool {
    if class.declare || !class.decorators.is_empty() {
        return false;
    }
    class.body.body.iter().all(|element| match element {
        ClassElement::MethodDefinition(method) => {
            method.decorators.is_empty()
                && !matches!(method.key, PropertyKey::PrivateIdentifier(_))
                && !(method.kind == MethodDefinitionKind::Constructor
                    && method
                        .value
                        .params
                        .items
                        .iter()
                        .any(|parameter| parameter.accessibility.is_some() || parameter.readonly))
        }
        ClassElement::PropertyDefinition(property) => {
            property.decorators.is_empty()
                && !matches!(property.key, PropertyKey::PrivateIdentifier(_))
        }
        ClassElement::StaticBlock(_) | ClassElement::TSIndexSignature(_) => true,
        ClassElement::AccessorProperty(_) => false,
    })
}

/// Whether the class must be lowered as a run-time value: its base is one, or
/// it is a first-class value inside a function (evaluated once per call).
pub(super) fn wants_runtime_class(
    source: &str,
    class: &oxc_ast::ast::Class<'_>,
    site: ClassSite,
) -> bool {
    if !lowering_supported(class) {
        return false;
    }
    if heritage_is_dynamic(source, class) {
        return true;
    }
    site == ClassSite::Value && FUNCTION_SCOPES.with(|scopes| !scopes.borrow().is_empty())
}

/// The property key of a class member as an expression.
fn member_key(source: &str, key: &PropertyKey<'_>, computed: bool) -> Result<Expr, String> {
    match key {
        PropertyKey::StaticIdentifier(identifier) if !computed => Ok(Expr::String {
            value: identifier.name.to_string(),
            span: Span::new(identifier.span.start, identifier.span.end),
        }),
        PropertyKey::PrivateIdentifier(_) => {
            Err("private members are not supported in a run-time class".into())
        }
        other => {
            let expression = other
                .as_expression()
                .ok_or("OXC adapter requires an expression class member key")?;
            convert_expression(source, expression)
        }
    }
}

/// Lower a class to the closure form (see the module documentation). The
/// result is an expression evaluating to the class constructor; `name` is the
/// class's `name` (its own, or the binding it initializes).
pub(super) fn convert_runtime_class(
    source: &str,
    class: &oxc_ast::ast::Class<'_>,
    name: &str,
) -> Result<Expr, String> {
    let whole = Span::new(class.span.start, class.span.end);
    let unique = class.span.start;
    let class_binding = crate::ast::synthetic_ident(&format!("__zcC{unique}"));
    let base_binding = crate::ast::synthetic_ident(&format!("__zcB{unique}"));
    let init_binding = crate::ast::synthetic_ident(&format!("__zcI{unique}"));
    let this_binding = crate::ast::synthetic_ident(&format!("__zcT{unique}"));
    let mut statements: Vec<Stmt> = Vec::new();

    // The heritage is evaluated first, in the enclosing `this` / scope.
    let has_base = class.heritage.is_some();
    if let Some(heritage) = class.heritage.as_ref() {
        let base = convert_expression(source, peel_heritage(&heritage.expression))?;
        statements.push(let_any(base_binding, base, whole));
    }

    if let Some(id) = class.id.as_ref() {
        IMMUTABLE_SELF_NAMES.with(|names| names.borrow_mut().push(id.name.to_string()));
    }
    let members = convert_members(
        source,
        class,
        class_binding,
        init_binding,
        this_binding,
        has_base,
        whole,
    );
    if class.id.is_some() {
        IMMUTABLE_SELF_NAMES.with(|names| names.borrow_mut().pop());
    }
    let members = members?;

    // Declared first and assigned second: the members refer to the class
    // binding (for `super` and `new.target`) and to the class's own name,
    // which they capture like any other enclosing local. A closure only
    // captures a binding that already exists when it is created.
    statements.push(let_any(class_binding, undefined(whole), whole));
    let own_name = class.id.as_ref().map(|id| Name {
        span: Span::new(id.span.start, id.span.end),
    });
    if let Some(own_name) = own_name {
        statements.push(let_any(own_name, undefined(whole), whole));
    }
    statements.extend(members.before_constructor);
    statements.push(Stmt::Expr(Expr::Assign {
        target: class_binding,
        value: Box::new(members.constructor),
        span: whole,
    }));
    statements.push(Stmt::Expr(helper_call(
        "__zunClassBegin",
        vec![
            Expr::Ident(class_binding),
            Expr::String {
                value: name.to_string(),
                span: whole,
            },
            Expr::Bool {
                value: has_base,
                span: whole,
            },
            if has_base {
                Expr::Ident(base_binding)
            } else {
                undefined(whole)
            },
        ],
        whole,
    )));
    // The class's own name is a binding visible to its members (immutable to
    // them: an assignment in a member body is converted to a throw).
    if let Some(own_name) = own_name {
        statements.push(Stmt::Expr(Expr::Assign {
            target: own_name,
            value: Box::new(Expr::Ident(class_binding)),
            span: whole,
        }));
    }
    statements.extend(members.after_constructor);
    statements.push(Stmt::Return {
        value: Some(Expr::Ident(class_binding)),
        span: whole,
    });
    let definition = Expr::Function(FunctionExpr {
        params: Vec::new(),
        return_type: convert_type_annotation(source, None)?,
        body: Block {
            statements,
            span: whole,
        },
        is_arrow: true,
        is_async: false,
        is_generator: false,
        this_class: None,
        span: whole,
    });
    Ok(Expr::ComputedCall {
        callee: Box::new(definition),
        args: Vec::new(),
        span: whole,
    })
}

/// The pieces of a lowered class body.
struct LoweredMembers {
    /// Computed field keys and the field-initializer function.
    before_constructor: Vec<Stmt>,
    /// The constructor function expression.
    constructor: Expr,
    /// Method, accessor and static installations, in source order.
    after_constructor: Vec<Stmt>,
}

#[allow(clippy::too_many_arguments)] // the bindings of one lowered class
fn convert_members(
    source: &str,
    class: &oxc_ast::ast::Class<'_>,
    class_binding: Name,
    init_binding: Name,
    this_binding: Name,
    has_base: bool,
    whole: Span,
) -> Result<LoweredMembers, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let unique = class.span.start;
    let instance_home = RuntimeSuper {
        class: class_binding,
        is_static: false,
        ctor_this: None,
        init: None,
        in_ctor: false,
    };
    let static_home = RuntimeSuper {
        is_static: true,
        ..instance_home
    };
    let mut before_constructor = Vec::new();
    let mut after_constructor = Vec::new();
    let mut field_statements = Vec::new();
    let mut constructor_method = None;

    for (index, element) in class.body.body.iter().enumerate() {
        match element {
            ClassElement::TSIndexSignature(_) => {}
            ClassElement::AccessorProperty(_) => {
                return Err("`accessor` fields are not supported in a run-time class".into())
            }
            ClassElement::MethodDefinition(method) => {
                if method.value.body.is_none() {
                    // An overload or abstract signature has no run-time form.
                    continue;
                }
                if method.kind == MethodDefinitionKind::Constructor {
                    constructor_method = Some(method);
                    continue;
                }
                let key = member_key(source, &method.key, method.computed)?;
                let home = if method.r#static {
                    static_home
                } else {
                    instance_home
                };
                // The computed `super[key]` forms resolve their home object
                // through the shared class context.
                let _context = push_super_context(Some(class_binding), method.r#static);
                NEXT_RUNTIME_SUPER.with(|next| next.set(Some(home)));
                let function = convert_oxc_function(source, &method.value);
                NEXT_RUNTIME_SUPER.with(|next| next.set(None));
                let target = if method.r#static {
                    Expr::Ident(class_binding)
                } else {
                    Expr::Field {
                        object: Box::new(Expr::Ident(class_binding)),
                        field: crate::ast::synthetic_ident("prototype"),
                        span: span(method.span),
                    }
                };
                let helper = match method.kind {
                    MethodDefinitionKind::Get => "__zunClassGetter",
                    MethodDefinitionKind::Set => "__zunClassSetter",
                    _ => "__zunClassMethod",
                };
                after_constructor.push(Stmt::Expr(helper_call(
                    helper,
                    vec![target, key, function?],
                    span(method.span),
                )));
            }
            ClassElement::PropertyDefinition(property) => {
                if property.declare {
                    continue;
                }
                let at = span(property.span);
                let key = member_key(source, &property.key, property.computed)?;
                if property.r#static {
                    // A static initializer runs with `this` bound to the class.
                    let _context = push_super_context(Some(class_binding), true);
                    let _frames = enter_member_frames(Some(class_binding), Some(static_home));
                    let value = match property.value.as_ref() {
                        Some(expression) => convert_expression(source, expression)?,
                        None => undefined(at),
                    };
                    after_constructor.push(Stmt::Expr(helper_call(
                        "__zunClassStatic",
                        vec![Expr::Ident(class_binding), key, value],
                        at,
                    )));
                    continue;
                }
                // A computed key is evaluated once, when the class is defined.
                let key = if property.computed {
                    let key_binding =
                        crate::ast::synthetic_ident(&format!("__zcK{unique}_{index}"));
                    before_constructor.push(let_any(key_binding, key, at));
                    Expr::Ident(key_binding)
                } else {
                    key
                };
                // The initializer runs inside the field-initializer function,
                // whose `this` is the instance.
                let _context = push_super_context(Some(class_binding), false);
                let _frames = enter_member_frames(None, Some(instance_home));
                let value = match property.value.as_ref() {
                    Some(expression) => convert_expression(source, expression)?,
                    None => undefined(at),
                };
                field_statements.push(Stmt::Expr(Expr::IndexAssign {
                    object: Box::new(ident("this")),
                    index: Box::new(key),
                    value: Box::new(value),
                    span: at,
                }));
            }
            ClassElement::StaticBlock(block) => {
                let at = span(block.span);
                let _context = push_super_context(Some(class_binding), true);
                let _frames = enter_member_frames(Some(class_binding), Some(static_home));
                let body = convert_function_statements(source, &block.body, &[])?;
                let arrow = Expr::Function(FunctionExpr {
                    params: Vec::new(),
                    return_type: convert_type_annotation(source, None)?,
                    body: Block {
                        statements: body,
                        span: at,
                    },
                    is_arrow: true,
                    is_async: false,
                    is_generator: false,
                    this_class: None,
                    span: at,
                });
                after_constructor.push(Stmt::Expr(Expr::ComputedCall {
                    callee: Box::new(arrow),
                    args: Vec::new(),
                    span: at,
                }));
            }
        }
    }

    let has_fields = !field_statements.is_empty();
    if has_fields {
        let initializer = Expr::Function(FunctionExpr {
            params: Vec::new(),
            return_type: convert_type_annotation(source, None)?,
            body: Block {
                statements: field_statements,
                span: whole,
            },
            is_arrow: false,
            is_async: false,
            is_generator: false,
            this_class: None,
            span: whole,
        });
        before_constructor.push(let_any(init_binding, initializer, whole));
    }
    let init = has_fields.then_some(init_binding);
    let run_initializers = || {
        Stmt::Expr(Expr::MethodCall {
            receiver: Box::new(Expr::Ident(init_binding)),
            method: crate::ast::synthetic_ident("call"),
            args: vec![ident("this")],
            span: whole,
        })
    };

    // A class constructor refuses a plain call (no receiver).
    let call_check = || {
        Stmt::Expr(helper_call(
            "__zunClassCallCheck",
            vec![Expr::Ident(class_binding), ident("this")],
            whole,
        ))
    };

    let constructor = match constructor_method {
        Some(method) => {
            let home = RuntimeSuper {
                ctor_this: has_base.then_some(this_binding),
                init,
                in_ctor: true,
                ..instance_home
            };
            let _context = push_super_context(Some(class_binding), false);
            NEXT_RUNTIME_SUPER.with(|next| next.set(Some(home)));
            // In a derived constructor `this` is the object `super(...)`
            // returns, held in a local that arrows capture.
            NEXT_THIS_ALIAS.with(|next| next.set(has_base.then_some((this_binding, true))));
            let function = convert_oxc_function(source, &method.value);
            NEXT_RUNTIME_SUPER.with(|next| next.set(None));
            NEXT_THIS_ALIAS.with(|next| next.set(None));
            let Expr::Function(mut function) = function? else {
                return Err("OXC adapter: a constructor did not convert to a function".into());
            };
            if has_base {
                function
                    .body
                    .statements
                    .insert(
                        0,
                        let_any(
                            this_binding,
                            Expr::ArrayLiteral {
                                elements: vec![crate::ast::ArrayElement::Expr(undefined(whole))],
                                span: whole,
                            },
                            whole,
                        ),
                    );
                function.body.statements.push(Stmt::Return {
                    value: Some(helper_call(
                        "__zunDerivedThis",
                        vec![this_cell_read(this_binding, whole)],
                        whole,
                    )),
                    span: whole,
                });
            } else if has_fields {
                function.body.statements.insert(0, run_initializers());
            }
            function.body.statements.insert(0, call_check());
            Expr::Function(function)
        }
        None => {
            let mut params = Vec::new();
            let mut body = Vec::new();
            if has_base {
                // `constructor(...args) { super(...args); }`
                let rest = crate::ast::synthetic_ident(&format!("__zcA{unique}"));
                params.push(crate::ast::Param {
                    name: rest,
                    ty: None,
                    default_value: None,
                    is_rest: true,
                });
                body.push(Stmt::Return {
                    value: Some(helper_call(
                        "__zunSuperCall",
                        vec![
                            Expr::Ident(class_binding),
                            ident("this"),
                            Expr::Ident(rest),
                            match init {
                                Some(init) => Expr::Ident(init),
                                None => undefined(whole),
                            },
                        ],
                        whole,
                    )),
                    span: whole,
                });
            } else if has_fields {
                body.push(run_initializers());
            }
            body.insert(0, call_check());
            Expr::Function(FunctionExpr {
                params,
                return_type: convert_type_annotation(source, None)?,
                body: Block {
                    statements: body,
                    span: whole,
                },
                is_arrow: false,
                is_async: false,
                is_generator: false,
                this_class: None,
                span: whole,
            })
        }
    };
    Ok(LoweredMembers {
        before_constructor,
        constructor,
        after_constructor,
    })
}
