use super::*;

pub(super) fn convert_template_literal(source: &str, template: &oxc_ast::ast::TemplateLiteral<'_>) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let mut parts = Vec::with_capacity(template.quasis.len() + template.expressions.len());
    for (index, quasi) in template.quasis.iter().enumerate() {
        let text = quasi
            .value
            .cooked
            .as_ref()
            .and_then(|value| value.as_str())
            .or_else(|| Some(quasi.value.raw.as_str()))
            .ok_or_else(|| {
                "OXC adapter cannot represent this template segment".to_owned()
            })?;
        parts.push(crate::ast::TemplatePart::Text(text.to_owned()));
        if let Some(expression) = template.expressions.get(index) {
            parts.push(crate::ast::TemplatePart::Expr(Box::new(
                convert_expression(source, expression)?,
            )));
        }
    }
    Ok(Expr::Template {
        parts,
        span: span(template.span),
    })
}

pub(super) fn convert_tagged_template(source: &str, tagged: &oxc_ast::ast::TaggedTemplateExpression<'_>) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let tag = Box::new(convert_expression(source, &tagged.tag)?);
    let quasi = &tagged.quasi;
    let mut cooked = Vec::with_capacity(quasi.quasis.len());
    let mut raw = Vec::with_capacity(quasi.quasis.len());
    let mut cooked_undefined = Vec::with_capacity(quasi.quasis.len());
    for element in &quasi.quasis {
        cooked_undefined.push(element.value.cooked.is_none());
        let cooked_text = element
            .value
            .cooked
            .as_ref()
            .and_then(|value| value.as_str())
            .unwrap_or_else(|| element.value.raw.as_str());
        cooked.push(cooked_text.to_owned());
        raw.push(element.value.raw.as_str().to_owned());
    }
    let mut exprs = Vec::with_capacity(quasi.expressions.len());
    for expression in &quasi.expressions {
        exprs.push(Box::new(convert_expression(source, expression)?));
    }
    Ok(Expr::TaggedTemplate {
        tag,
        cooked,
        cooked_undefined,
        raw,
        exprs,
        span: span(tagged.span),
    })
}

pub(super) fn convert_sequence(source: &str, sequence: &oxc_ast::ast::SequenceExpression<'_>) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    // Desugar `(a, b, c)` into an immediately-invoked arrow
    // `(() => { a; b; return c; })()` so evaluation order and side
    // effects of the earlier expressions are preserved while the whole
    // sequence still yields the last expression's value.
    let whole = span(sequence.span);
    let (last, leading) = sequence
        .expressions
        .split_last()
        .ok_or("OXC adapter: empty sequence expression")?;
    // A sequence containing `yield` cannot live in a closure: hoist the
    // leading expressions to statements before the containing statement.
    if source[sequence.span.start as usize..sequence.span.end as usize].contains("yield") {
        let mut prelude = Vec::with_capacity(leading.len());
        for expression in leading {
            prelude.push(Stmt::Expr(convert_expression(source, expression)?));
        }
        PENDING_PRELUDE.with(|pending| pending.borrow_mut().extend(prelude));
        return convert_expression(source, last);
    }
    let mut statements = Vec::with_capacity(sequence.expressions.len());
    for expression in leading {
        statements.push(Stmt::Expr(convert_expression(source, expression)?));
    }
    statements.push(Stmt::Return {
        value: Some(convert_expression(source, last)?),
        span: span(last.span()),
    });
    let iife = Expr::Function(FunctionExpr {
        params: Vec::new(),
        return_type: convert_type_annotation(source, None)?,
        body: Block { statements, span: whole },
        is_arrow: true,
        is_async: false,
        is_generator: false,
        span: whole,
    });
    Ok(Expr::ComputedCall {
        callee: Box::new(iife),
        args: Vec::new(),
        span: whole,
    })
}

/// `expr as T` / `<T>expr` / `expr satisfies T`: a cast whose type name span is
/// the source range of the type annotation.
pub(super) fn convert_ts_cast(
    source: &str,
    inner: &Expression<'_>,
    type_span: oxc_span::Span,
    whole: oxc_span::Span,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    Ok(Expr::Cast {
        expr: Box::new(convert_expression(source, inner)?),
        ty: crate::ast::TypeAnnotation {
            name: Name {
                span: Span::new(type_span.start, type_span.end),
            },
            is_array: false,
        },
        span: span(whole),
    })
}

pub(super) fn convert_identifier(identifier: &oxc_ast::ast::IdentifierReference<'_>) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    // The global literal identifiers have no variable binding: map them
    // to their values so they resolve like JS (`undefined` ≡ `void 0`,
    // `NaN`/`Infinity` are float literals).
    match identifier.name.as_str() {
        "undefined" => Ok(Expr::Unary {
            op: crate::ast::UnaryOp::Void,
            operand: Box::new(Expr::Number { value: 0.0, span: span(identifier.span) }),
            span: span(identifier.span),
        }),
        "NaN" => Ok(Expr::Number { value: f64::NAN, span: span(identifier.span) }),
        "Infinity" => Ok(Expr::Number { value: f64::INFINITY, span: span(identifier.span) }),
        _ => Ok(Expr::Ident(Name { span: span(identifier.span) })),
    }
}
