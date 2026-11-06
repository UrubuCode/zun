use super::*;

/// Convert a C-style `for (init; cond; update) body` statement.
pub(super) fn convert_for_statement(
    source: &str,
    for_statement: &oxc_ast::ast::ForStatement<'_>,
) -> Result<(Vec<Stmt>, Stmt), String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let mut pre = Vec::new();
    let mut extra_init: Vec<Stmt> = Vec::new();
    let init = match &for_statement.init {
        None => None,
        Some(ForStatementInit::VariableDeclaration(declaration))
            if declaration.declarations.len() != 1
                && declaration.kind != oxc_ast::ast::VariableDeclarationKind::Var
                && declaration
                    .declarations
                    .iter()
                    .all(|d| matches!(d.id, BindingPattern::BindingIdentifier(_))) =>
        {
            // Several `let`/`const` declarators stay in the init clause so each
            // is a per-iteration binding.
            let mut lowered = Vec::new();
            for declarator in &declaration.declarations {
                let BindingPattern::BindingIdentifier(identifier) = &declarator.id else {
                    return Err("OXC adapter supports identifier for-loop bindings".into());
                };
                let init = match declarator.init.as_ref() {
                    Some(initializer) => convert_expression(source, initializer)?,
                    None => Expr::Unary {
                        op: crate::ast::UnaryOp::Void,
                        operand: Box::new(Expr::Number { value: 0.0, span: span(declarator.span) }),
                        span: span(declarator.span),
                    },
                };
                lowered.push(Stmt::Let {
                    name: Name { span: span(identifier.span) },
                    ty: convert_type_annotation(source, declarator.type_annotation.as_deref())?,
                    init,
                    span: span(declarator.span),
                });
            }
            let mut lowered = lowered.into_iter();
            let first = lowered.next();
            extra_init.extend(lowered);
            first.map(Box::new)
        }
        Some(ForStatementInit::VariableDeclaration(declaration))
            if declaration.declarations.len() != 1 =>
        {
            // Several declarators: hoist them as `let`s before the loop.
            for declarator in &declaration.declarations {
                let BindingPattern::BindingIdentifier(identifier) = &declarator.id else {
                    return Err("OXC adapter supports identifier for-loop bindings".into());
                };
                let init = match declarator.init.as_ref() {
                    Some(initializer) => convert_expression(source, initializer)?,
                    // `for (let i; ...)` — an uninitialized binding is `undefined`.
                    None => Expr::Unary {
                        op: crate::ast::UnaryOp::Void,
                        operand: Box::new(Expr::Number { value: 0.0, span: span(declarator.span) }),
                        span: span(declarator.span),
                    },
                };
                pre.push(Stmt::Let {
                    name: Name { span: span(identifier.span) },
                    ty: convert_type_annotation(source, declarator.type_annotation.as_deref())?,
                    init,
                    span: span(declarator.span),
                });
            }
            None
        }
        Some(ForStatementInit::VariableDeclaration(declaration)) => {
            let declarator = &declaration.declarations[0];
            let BindingPattern::BindingIdentifier(identifier) = &declarator.id else {
                return Err("OXC adapter supports identifier for-loop bindings".into());
            };
            let init = match declarator.init.as_ref() {
                Some(initializer) => convert_expression(source, initializer)?,
                // `for (let i; ...)` — an uninitialized binding is `undefined`.
                None => Expr::Unary {
                    op: crate::ast::UnaryOp::Void,
                    operand: Box::new(Expr::Number { value: 0.0, span: span(declarator.span) }),
                    span: span(declarator.span),
                },
            };
            let binding = Stmt::Let {
                name: Name { span: span(identifier.span) },
                ty: convert_type_annotation(source, declarator.type_annotation.as_deref())?,
                init,
                span: span(declarator.span),
            };
            if declaration.kind == oxc_ast::ast::VariableDeclarationKind::Var {
                // `var` is function-scoped: one binding shared by every
                // iteration (no per-iteration copy).
                pre.push(binding);
                None
            } else {
                Some(Box::new(binding))
            }
        }
        Some(init) => Some(Box::new(Stmt::Expr(convert_expression(
            source,
            init.as_expression()
                .ok_or("OXC adapter does not support this for-loop initializer yet")?,
        )?))),
    };
    Ok((pre, Stmt::For {
        init,
        extra_init,
        cond: for_statement
            .test
            .as_ref()
            .map(|expression| convert_expression(source, expression))
            .transpose()?,
        update: for_statement
            .update
            .as_ref()
            .map(|expression| convert_effect_expression(source, expression))
            .transpose()?,
        body: convert_if_branch(source, &for_statement.body)?,
        span: span(for_statement.span),
    }))
}

/// Convert a `for (… in …)` statement to the `ForIn` node (lowered to key
/// iteration in sema). The binding is a declared identifier or pattern.
pub(super) fn convert_for_in(
    source: &str,
    for_in_statement: &oxc_ast::ast::ForInStatement<'_>,
) -> Result<Stmt, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let ForStatementLeft::VariableDeclaration(declaration) = &for_in_statement.left else {
        return Err("OXC adapter supports declared for-in bindings only".into());
    };
    if declaration.declarations.len() != 1 {
        return Err("OXC adapter supports one for-in binding".into());
    }
    let declarator = &declaration.declarations[0];
    let pattern = match &declarator.id {
        BindingPattern::BindingIdentifier(identifier) => crate::ast::Pattern::Ident {
            name: Name { span: span(identifier.span) },
            ty: convert_type_annotation(source, declarator.type_annotation.as_deref())?,
        },
        other => convert_binding_pattern(source, other)?,
    };
    // `for (var k in o)`: one function-scoped binding shared by every iteration,
    // assigned from a per-iteration temporary at the top of the body. The
    // declaration is emitted ahead of the loop as a separate statement.
    if declaration.kind == oxc_ast::ast::VariableDeclarationKind::Var {
        if let (BindingPattern::BindingIdentifier(identifier), crate::ast::Pattern::Ident { ty, .. }) =
            (&declarator.id, &pattern)
        {
            let var_name = Name { span: span(identifier.span) };
            let temp = crate::ast::synthetic_ident("__for_in_var");
            let mut body = convert_if_branch(source, &for_in_statement.body)?;
            body.statements.insert(
                0,
                Stmt::Expr(Expr::Assign {
                    target: var_name,
                    value: Box::new(Expr::Ident(temp)),
                    span: span(identifier.span),
                }),
            );
            let declare = Stmt::Let {
                name: var_name,
                ty: ty.clone(),
                init: Expr::Unary {
                    op: crate::ast::UnaryOp::Void,
                    operand: Box::new(Expr::Number { value: 0.0, span: span(identifier.span) }),
                    span: span(identifier.span),
                },
                span: span(declarator.span),
            };
            let looped = Stmt::ForIn {
                pattern: crate::ast::Pattern::Ident { name: temp, ty: None },
                object: convert_expression(source, &for_in_statement.right)?,
                body,
                span: span(for_in_statement.span),
            };
            PENDING_PRELUDE.with(|pending| pending.borrow_mut().push(declare));
            return Ok(looped);
        }
    }
    Ok(Stmt::ForIn {
        pattern,
        object: convert_expression(source, &for_in_statement.right)?,
        body: convert_if_branch(source, &for_in_statement.body)?,
        span: span(for_in_statement.span),
    })
}

/// Desugar `for await (<binding> of <iterable>) body` into an explicit
/// async-iterator loop (spec: AsyncIteratorClose on abrupt exit):
///
/// ```text
/// const it = __zunAsyncIterOpen(iterable);
/// let close = false;
/// try {
///   while (true) {
///     close = false;
///     const r = await it.next();
///     if (r.done) break;
///     close = true;
///     <binding> = r.value;
///     body
///   }
/// } finally { if (close) await __zunAsyncIterClose(it); }
/// ```
pub(super) fn convert_for_await(
    source: &str,
    for_of_statement: &oxc_ast::ast::ForOfStatement<'_>,
) -> Result<Vec<Stmt>, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let sp = span(for_of_statement.span);
    let ForStatementLeft::VariableDeclaration(declaration) = &for_of_statement.left else {
        return Err("OXC adapter supports declared for-await bindings only".into());
    };
    if declaration.declarations.len() != 1 {
        return Err("OXC adapter supports one for-await binding".into());
    }
    let declarator = &declaration.declarations[0];
    let pattern = match &declarator.id {
        BindingPattern::BindingIdentifier(identifier) => crate::ast::Pattern::Ident {
            name: Name { span: span(identifier.span) },
            ty: convert_type_annotation(source, declarator.type_annotation.as_deref())?,
        },
        other => convert_binding_pattern(source, other)?,
    };
    let n = YIELD_STAR_COUNTER.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    let it = crate::ast::synthetic_ident(&format!("__fa_it{n}"));
    let result = crate::ast::synthetic_ident(&format!("__fa_r{n}"));
    let close = crate::ast::synthetic_ident(&format!("__fa_close{n}"));
    let field = |name: Name, key: &str| Expr::Index {
        object: Box::new(Expr::Ident(name)),
        index: Box::new(Expr::String { value: key.to_owned(), span: sp }),
        span: sp,
    };
    let set_close = |value: bool| {
        Stmt::Expr(Expr::Assign {
            target: close,
            value: Box::new(Expr::Bool { value, span: sp }),
            span: sp,
        })
    };
    let awaited = |operand: Expr| Expr::Await { operand: Box::new(operand), span: sp };
    let mut loop_body = vec![
        set_close(false),
        Stmt::Let {
            name: result,
            ty: None,
            init: awaited(Expr::ComputedCall {
                callee: Box::new(field(it, "next")),
                args: Vec::new(),
                span: sp,
            }),
            span: sp,
        },
        Stmt::If {
            cond: field(result, "done"),
            then_block: Block { statements: vec![Stmt::Break { label: None, span: sp }], span: sp },
            else_block: None,
            span: sp,
        },
        set_close(true),
    ];
    loop_body.push(match pattern {
        crate::ast::Pattern::Ident { name, ty } => {
            Stmt::Let { name, ty, init: field(result, "value"), span: sp }
        }
        other => Stmt::Destructure { pattern: other, init: field(result, "value"), span: sp },
    });
    loop_body.extend(convert_if_branch(source, &for_of_statement.body)?.statements);
    Ok(vec![
        Stmt::Let {
            name: it,
            ty: None,
            init: Expr::Call {
                callee: crate::ast::synthetic_ident("__zunAsyncIterOpen"),
                args: vec![convert_expression(source, &for_of_statement.right)?],
                span: sp,
            },
            span: sp,
        },
        Stmt::Let { name: close, ty: None, init: Expr::Bool { value: false, span: sp }, span: sp },
        Stmt::Try {
            body: Block {
                statements: vec![Stmt::While {
                    cond: Expr::Bool { value: true, span: sp },
                    body: Block { statements: loop_body, span: sp },
                    span: sp,
                }],
                span: sp,
            },
            catches: Vec::new(),
            finally: Some(Block {
                statements: vec![Stmt::If {
                    cond: Expr::Ident(close),
                    then_block: Block {
                        statements: vec![Stmt::Expr(awaited(Expr::Call {
                            callee: crate::ast::synthetic_ident("__zunAsyncIterClose"),
                            args: vec![Expr::Ident(it)],
                            span: sp,
                        }))],
                        span: sp,
                    },
                    else_block: None,
                    span: sp,
                }],
                span: sp,
            }),
            span: sp,
        },
    ])
}

/// Convert a `for (… of …)` statement, with an identifier or destructuring
/// pattern binding.
pub(super) fn convert_for_of(
    source: &str,
    for_of_statement: &oxc_ast::ast::ForOfStatement<'_>,
) -> Result<Vec<Stmt>, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    if for_of_statement.r#await {
        return convert_for_await(source, for_of_statement);
    }
    let ForStatementLeft::VariableDeclaration(declaration) = &for_of_statement.left else {
        // `for (target of iterable)`: assign each element to an existing binding or
        // member (or destructure it into several) at the top of the body.
        let Some(target) = for_of_statement.left.as_assignment_target() else {
            return Err("OXC adapter supports declared for-of bindings only".into());
        };
        let temp = fresh_destructure_temp();
        let whole = span(for_of_statement.span);
        let store = match target {
            AssignmentTarget::ArrayAssignmentTarget(_) | AssignmentTarget::ObjectAssignmentTarget(_) => {
                convert_destructuring_assignment(source, target, Expr::Ident(temp), whole)?
            }
            plain => store_into_target(source, plain, Expr::Ident(temp), whole)?,
        };
        let mut body = convert_if_branch(source, &for_of_statement.body)?;
        body.statements.insert(0, Stmt::Expr(store));
        return Ok(vec![Stmt::ForOf {
            pattern: crate::ast::Pattern::Ident { name: temp, ty: None },
            iterable: convert_expression(source, &for_of_statement.right)?,
            body,
            span: whole,
        }]);
    };
    if declaration.declarations.len() != 1 {
        return Err("OXC adapter supports one for-of binding".into());
    }
    let declarator = &declaration.declarations[0];
    let pattern = match &declarator.id {
        BindingPattern::BindingIdentifier(identifier) => crate::ast::Pattern::Ident {
            name: Name { span: span(identifier.span) },
            ty: convert_type_annotation(source, declarator.type_annotation.as_deref())?,
        },
        other => convert_binding_pattern(source, other)?,
    };
    // `for (var x of ...)`: one function-scoped binding shared by every
    // iteration — declared before the loop and assigned from a per-iteration
    // temporary at the top of the body.
    if declaration.kind == oxc_ast::ast::VariableDeclarationKind::Var {
        if let (BindingPattern::BindingIdentifier(identifier), crate::ast::Pattern::Ident { ty, .. }) =
            (&declarator.id, &pattern)
        {
            let var_name = Name { span: span(identifier.span) };
            let temp = crate::ast::synthetic_ident("__for_of_var");
            let mut body = convert_if_branch(source, &for_of_statement.body)?;
            body.statements.insert(
                0,
                Stmt::Expr(Expr::Assign {
                    target: var_name,
                    value: Box::new(Expr::Ident(temp)),
                    span: span(identifier.span),
                }),
            );
            return Ok(vec![
                Stmt::Let {
                    name: var_name,
                    ty: ty.clone(),
                    init: Expr::Unary {
                        op: crate::ast::UnaryOp::Void,
                        operand: Box::new(Expr::Number { value: 0.0, span: span(identifier.span) }),
                        span: span(identifier.span),
                    },
                    span: span(declarator.span),
                },
                Stmt::ForOf {
                    pattern: crate::ast::Pattern::Ident { name: temp, ty: None },
                    iterable: convert_expression(source, &for_of_statement.right)?,
                    body,
                    span: span(for_of_statement.span),
                },
            ]);
        }
    }
    // `for (var [a, b] of ...)`: like the plain `var` form, the names are
    // function-scoped bindings shared by every iteration; each round assigns
    // them from the element at the top of the body.
    if declaration.kind == oxc_ast::ast::VariableDeclarationKind::Var {
        if let BindingPattern::ArrayPattern(array) = &declarator.id {
            let names: Option<Vec<oxc_span::Span>> = array
                .elements
                .iter()
                .map(|element| match element {
                    Some(BindingPattern::BindingIdentifier(identifier)) => Some(identifier.span),
                    _ => None,
                })
                .collect();
            if let (Some(names), None) = (names, array.rest.as_ref()) {
                let temp = crate::ast::synthetic_ident("__for_of_var");
                let mut body = convert_if_branch(source, &for_of_statement.body)?;
                let mut lets = Vec::new();
                for (position, name_span) in names.iter().enumerate() {
                    let var_name = Name { span: span(*name_span) };
                    lets.push(Stmt::Let {
                        name: var_name,
                        ty: None,
                        init: Expr::Unary {
                            op: crate::ast::UnaryOp::Void,
                            operand: Box::new(Expr::Number { value: 0.0, span: span(*name_span) }),
                            span: span(*name_span),
                        },
                        span: span(declarator.span),
                    });
                    body.statements.insert(
                        position,
                        Stmt::Expr(Expr::Assign {
                            target: var_name,
                            value: Box::new(Expr::Index {
                                object: Box::new(Expr::Ident(temp)),
                                index: Box::new(Expr::Number {
                                    value: position as f64,
                                    span: span(*name_span),
                                }),
                                span: span(*name_span),
                            }),
                            span: span(*name_span),
                        }),
                    );
                }
                lets.push(Stmt::ForOf {
                    pattern: crate::ast::Pattern::Ident { name: temp, ty: None },
                    iterable: convert_expression(source, &for_of_statement.right)?,
                    body,
                    span: span(for_of_statement.span),
                });
                return Ok(lets);
            }
        }
    }
    Ok(vec![Stmt::ForOf {
        pattern,
        iterable: convert_expression(source, &for_of_statement.right)?,
        body: convert_if_branch(source, &for_of_statement.body)?,
        span: span(for_of_statement.span),
    }])
}
