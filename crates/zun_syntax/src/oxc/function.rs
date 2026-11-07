use super::*;

/// A parameter's default expression; an optional `x?: T` parameter without an
/// initializer defaults to `undefined`.
pub(super) fn param_default(
    source: &str,
    parameter: &oxc_ast::ast::FormalParameter<'_>,
) -> Result<Option<Box<Expr>>, String> {
    if let Some(initializer) = parameter.initializer.as_ref() {
        return Ok(Some(Box::new(convert_expression(source, initializer)?)));
    }
    if parameter.optional {
        let at = Span::new(parameter.span.start, parameter.span.end);
        return Ok(Some(Box::new(Expr::Unary {
            op: crate::ast::UnaryOp::Void,
            operand: Box::new(Expr::Number {
                value: 0.0,
                span: at,
            }),
            span: at,
        })));
    }
    Ok(None)
}

/// Convert an OXC function (an object-method, getter, or setter value) into a
/// Zun anonymous-function expression. Parameters must be plain identifiers;
/// the body reuses the shared statement-list conversion.
/// Convert one formal parameter. An identifier binding becomes a plain `Param`.
/// A destructuring pattern (`{a}`, `[x, y]`) becomes a fresh positional
/// parameter (named by the parameter's source span, which is a unique token) plus
/// a `Destructure` statement pushed onto `prologue`, binding the pattern's names
/// from that parameter at the top of the body — the standard JS desugaring.
pub(super) fn convert_formal_param(
    source: &str,
    parameter: &oxc_ast::ast::FormalParameter<'_>,
    prologue: &mut Vec<Stmt>,
) -> Result<crate::ast::Param, String> {
    let span = |s: oxc_span::Span| Span::new(s.start, s.end);
    if prologue.is_empty() {
        PATTERN_PARAM_TEXT.with(|text| text.borrow_mut().clear());
    }
    match &parameter.pattern {
        BindingPattern::BindingIdentifier(identifier) => {
            let ty = convert_type_annotation(source, parameter.type_annotation.as_deref())?;
            let mut default_value = param_default(source, parameter)?;
            // A default that reads a name bound by an earlier destructuring
            // parameter must run after that destructuring: it moves into the body
            // prologue as `p = p === undefined ? <default> : p`.
            if let (Some(initializer), false, true) = (
                parameter.initializer.as_ref(),
                prologue.is_empty(),
                ty.as_ref().is_none_or(|annotation| {
                    !annotation.is_array
                        && source.get(
                            annotation.name.span.start as usize..annotation.name.span.end as usize,
                        ) == Some("any")
                }),
            ) {
                let init_text =
                    &source[initializer.span().start as usize..initializer.span().end as usize];
                let mentions = PATTERN_PARAM_TEXT.with(|text| {
                    let text = text.borrow();
                    text.split(|c: char| !is_identifier_char(c))
                        .filter(|word| {
                            !word.is_empty()
                                && word.chars().next().is_some_and(|c| !c.is_ascii_digit())
                        })
                        .any(|word| contains_word(init_text, word))
                });
                if mentions {
                    let at = span(parameter.span);
                    let name = Name {
                        span: span(identifier.span),
                    };
                    let undefined = Expr::Unary {
                        op: crate::ast::UnaryOp::Void,
                        operand: Box::new(Expr::Number {
                            value: 0.0,
                            span: at,
                        }),
                        span: at,
                    };
                    let default = default_value.take().expect("initializer was converted");
                    prologue.push(Stmt::Expr(Expr::Assign {
                        target: name,
                        value: Box::new(Expr::Conditional {
                            condition: Box::new(Expr::Binary {
                                op: BinaryOp::Eq,
                                lhs: Box::new(Expr::Ident(name)),
                                rhs: Box::new(undefined),
                                span: at,
                            }),
                            when_true: default,
                            when_false: Box::new(Expr::Ident(name)),
                            span: at,
                        }),
                        span: at,
                    }));
                }
            }
            Ok(crate::ast::Param {
                name: Name {
                    span: span(identifier.span),
                },
                ty,
                default_value,
                is_rest: false,
            })
        }
        other => {
            PATTERN_PARAM_TEXT.with(|text| {
                let mut text = text.borrow_mut();
                text.push(' ');
                text.push_str(&source[parameter.span.start as usize..parameter.span.end as usize]);
            });
            let pattern = convert_binding_pattern(source, other)?;
            let slot = span(parameter.span);
            prologue.push(Stmt::Destructure {
                pattern,
                init: Expr::Ident(Name { span: slot }),
                span: slot,
            });
            Ok(crate::ast::Param {
                name: Name { span: slot },
                ty: None,
                default_value: param_default(source, parameter)?,
                is_rest: false,
            })
        }
    }
}

pub(super) fn convert_oxc_function(
    source: &str,
    function: &OxcFunction<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let Some(body) = function.body.as_ref() else {
        return Err("OXC adapter requires an object method body".into());
    };
    let mut params = Vec::with_capacity(function.params.items.len());
    let mut prologue = Vec::new();
    for parameter in &function.params.items {
        params.push(convert_formal_param(source, parameter, &mut prologue)?);
    }
    if let Some(rest) = function.params.rest.as_ref() {
        let BindingPattern::BindingIdentifier(identifier) = &rest.rest.argument else {
            return Err("OXC adapter supports identifier rest parameters only".into());
        };
        params.push(crate::ast::Param {
            name: Name {
                span: span(identifier.span),
            },
            ty: convert_type_annotation(source, rest.type_annotation.as_deref())?,
            default_value: None,
            is_rest: true,
        });
    }
    let mut statements = prologue;
    statements.extend(convert_function_statements(
        source,
        &body.statements,
        &param_names(&function.params),
    )?);
    #[allow(clippy::redundant_locals)] // rebind to obtain a mutable binding for injection
    let mut params = params;
    inject_arguments_object(source, span(function.span), &mut params, &mut statements);
    Ok(Expr::Function(FunctionExpr {
        params,
        return_type: convert_type_annotation(source, function.return_type.as_deref())?,
        body: Block {
            statements,
            span: span(body.span),
        },
        is_arrow: false,
        is_async: function.r#async,
        is_generator: function.generator,
        span: span(function.span),
    }))
}

/// Convert an arrow function (`(a) => expr` or `(a) => { … }`) to a
/// `FunctionExpr` flagged as an arrow. A concise expression body becomes a
/// single `return`.
pub(super) fn convert_arrow(
    source: &str,
    arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>,
) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let mut params = Vec::with_capacity(arrow.params.items.len());
    let mut prologue = Vec::new();
    for parameter in &arrow.params.items {
        params.push(convert_formal_param(source, parameter, &mut prologue)?);
    }
    if let Some(rest) = arrow.params.rest.as_ref() {
        let BindingPattern::BindingIdentifier(identifier) = &rest.rest.argument else {
            return Err("OXC adapter supports identifier rest parameters only".into());
        };
        params.push(crate::ast::Param {
            name: Name {
                span: span(identifier.span),
            },
            ty: convert_type_annotation(source, rest.type_annotation.as_deref())?,
            default_value: None,
            is_rest: true,
        });
    }
    // A concise body (`=> expr`) returns its expression; a block body
    // (`=> { … }`) converts its statement list. Any destructured-parameter
    // prologue runs first.
    let body = if let Some(expression) = arrow.get_expression() {
        let mut statements = prologue;
        statements.push(Stmt::Return {
            value: Some(convert_expression(source, expression)?),
            span: span(expression.span()),
        });
        Block {
            statements,
            span: span(arrow.span),
        }
    } else if let oxc_ast::ast::ArrowFunctionBody::FunctionBody(function_body) = &arrow.body {
        let mut statements = prologue;
        statements.extend(convert_function_statements(
            source,
            &function_body.statements,
            &param_names(&arrow.params),
        )?);
        Block {
            statements,
            span: span(function_body.span),
        }
    } else {
        return Err("OXC adapter: unexpected arrow body".into());
    };
    Ok(Expr::Function(FunctionExpr {
        params,
        return_type: convert_type_annotation(source, arrow.return_type.as_deref())?,
        body,
        is_arrow: true,
        is_async: arrow.r#async,
        is_generator: false,
        span: span(arrow.span),
    }))
}

/// Find the first whole-word occurrence of `arguments` inside `[start, end)` of
/// the source, returning its span.
pub(super) fn find_arguments_word(source: &str, start: u32, end: u32) -> Option<Span> {
    let text = source.get(start as usize..end as usize)?;
    let bytes = text.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let mut from = 0;
    while let Some(found) = text[from..].find("arguments") {
        let at = from + found;
        let after = at + "arguments".len();
        let before_ok = at == 0 || !is_ident(bytes[at - 1]);
        let after_ok = after >= bytes.len() || !is_ident(bytes[after]);
        if before_ok && after_ok {
            return Some(Span::new(start + at as u32, start + after as u32));
        }
        from = after;
    }
    None
}

/// Give a non-arrow function its implicit `arguments` object: an Array local
/// `arguments = [p1, ..., pn, ...rest]` built at the top of the body. A function
/// that does not already end in a rest parameter gets a synthetic one so extra
/// call arguments are collected.
pub(super) fn inject_arguments_object(
    source: &str,
    function_span: Span,
    params: &mut Vec<crate::ast::Param>,
    statements: &mut Vec<Stmt>,
) {
    let Some(word) = find_arguments_word(source, function_span.start, function_span.end) else {
        return;
    };
    let rest_name = match params.last() {
        Some(last) if last.is_rest => last.name,
        _ => {
            let name = Name {
                span: Span::new(
                    crate::ast::SYNTHETIC_NAME_BASE + 0x0800_0000 + function_span.start,
                    crate::ast::SYNTHETIC_NAME_BASE + 0x0800_0000 + function_span.start,
                ),
            };
            params.push(crate::ast::Param {
                name,
                ty: None,
                default_value: None,
                is_rest: true,
            });
            name
        }
    };
    let mut elements: Vec<crate::ast::ArrayElement> = params
        .iter()
        .filter(|param| !param.is_rest)
        .map(|param| crate::ast::ArrayElement::Expr(Expr::Ident(param.name)))
        .collect();
    elements.push(crate::ast::ArrayElement::Spread(Expr::Ident(rest_name)));
    statements.insert(
        0,
        Stmt::Let {
            name: Name { span: word },
            ty: None,
            init: Expr::ArrayLiteral {
                elements,
                span: word,
            },
            span: word,
        },
    );
}

pub(super) fn param_names(params: &oxc_ast::ast::FormalParameters<'_>) -> Vec<String> {
    // Side channel for `convert_function_statements`: a list of plain
    // identifier parameters (no defaults, patterns or rest) shares one scope
    // with the body, so a body `var` of a parameter name is that parameter.
    LAST_PARAMS_SIMPLE.with(|flag| {
        flag.set(
            params.rest.is_none()
                && params.items.iter().all(|item| {
                    matches!(item.pattern, BindingPattern::BindingIdentifier(_))
                        && item.initializer.is_none()
                }),
        )
    });
    let mut names = Vec::new();
    for item in &params.items {
        if let BindingPattern::BindingIdentifier(identifier) = &item.pattern {
            names.push(identifier.name.to_string());
        }
    }
    if let Some(rest) = &params.rest {
        if let BindingPattern::BindingIdentifier(identifier) = &rest.rest.argument {
            names.push(identifier.name.to_string());
        }
    }
    names
}
