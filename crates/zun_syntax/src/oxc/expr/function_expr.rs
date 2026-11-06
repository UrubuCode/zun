use super::*;

/// A named function expression binds its own name inside its body:
/// `(() => { let name = function name() {...}; return name; })()` gives the
/// closure its self-reference (the `let` initializer already supports it).
pub(super) fn convert_named_function_expression(source: &str, function: &OxcFunction<'_>) -> Result<Expr, String> {
    let id = function.id.as_ref().expect("checked above");
    let name = Name { span: Span::new(id.span.start, id.span.end) };
    let whole = Span::new(function.span.start, function.span.end);
    let (body_start, body_end) = function
        .body
        .as_ref()
        .map_or((0, 0), |body| (body.span.start as usize, body.span.end as usize));
    let body_text = &source[body_start..body_end];
    let shadowed = ["var", "let", "const", "function", "class"]
        .iter()
        .any(|keyword| body_text.contains(&format!("{keyword} {}", id.name)))
        || contains_word(&source[function.params.span.start as usize..function.params.span.end as usize], id.name.as_str());
    if !shadowed {
        IMMUTABLE_SELF_NAMES.with(|names| names.borrow_mut().push(id.name.to_string()));
    }
    let inner = convert_oxc_function(source, function);
    if !shadowed {
        IMMUTABLE_SELF_NAMES.with(|names| names.borrow_mut().pop());
    }
    let inner = inner?;
    let statements = vec![
        Stmt::Let { name, ty: None, init: inner, span: whole },
        Stmt::Return { value: Some(Expr::Ident(name)), span: whole },
    ];
    let iife = Expr::Function(FunctionExpr {
        params: Vec::new(),
        return_type: convert_type_annotation(source, None)?,
        body: Block { statements, span: whole },
        is_arrow: true,
        is_async: false,
        is_generator: false,
        span: whole,
    });
    Ok(Expr::ComputedCall { callee: Box::new(iife), args: Vec::new(), span: whole })
}

/// A class expression is lifted to a hoisted class declaration (Zun
/// classes are program-global) and the expression becomes a reference
/// to it, which sema reads as the class constructor descriptor. An
/// anonymous class takes the name of the binding it initializes (set by
/// the declarator converter), else the `class` keyword span.
pub(super) fn convert_class_expression(source: &str, class_expression: &oxc_ast::ast::Class<'_>) -> Result<Expr, String> {
    let hint = CLASS_NAME_HINT.with(|cell| cell.borrow_mut().take());
    // A class bound by a declarator takes the binding's name so that
    // `new Binding()` resolves statically (limitation: an inner
    // reference to a distinct class-expression name is not bound).
    let name_span = match (hint, class_expression.id.as_ref()) {
        (Some(binding), _) => binding,
        (None, Some(id)) => Span::new(id.span.start, id.span.end),
        (None, None) => Span::new(
            class_expression.span.start,
            class_expression.span.start + 5,
        ),
    };
    if let Some(id) = class_expression.id.as_ref() {
        IMMUTABLE_SELF_NAMES.with(|names| names.borrow_mut().push(id.name.to_string()));
    }
    let class = convert_class_declaration(source, class_expression);
    if class_expression.id.is_some() {
        IMMUTABLE_SELF_NAMES.with(|names| names.borrow_mut().pop());
    }
    let mut class = class.map_err(|errors| errors.join("; "))?;
    let trailing = CLASS_TRAILING.with(|cells| std::mem::take(&mut *cells.borrow_mut()));
    if !trailing.is_empty() {
        PENDING_PRELUDE.with(|pending| pending.borrow_mut().extend(trailing));
    }
    class.name = Name { span: name_span };
    HOISTED_CLASSES.with(|cells| cells.borrow_mut().push(class));
    Ok(Expr::Ident(Name { span: name_span }))
}

/// Whether a function expression's body mentions its own name.
pub(super) fn refers_to_own_name(source: &str, function: &OxcFunction<'_>) -> bool {
    function.id.as_ref().is_some_and(|id| {
        function.body.as_ref().is_some_and(|body| {
            contains_word(&source[body.span.start as usize..body.span.end as usize], id.name.as_str())
        })
    })
}
