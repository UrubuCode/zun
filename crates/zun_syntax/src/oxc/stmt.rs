use super::*;

/// Convert an OXC statement list into Zun statements, reused by block
/// statements and by object-method / function bodies.
pub(super) fn convert_statement_list(source: &str, body: &[Statement<'_>]) -> Result<Vec<Stmt>, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let mut statements = Vec::with_capacity(body.len());
    let saved_prelude = PENDING_PRELUDE.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    let ordered = hoist_function_declarations(source, body);
    let forward_names = forward_declared_names(source, &ordered);
    predeclare_forward_functions(source, &ordered, &forward_names, &mut statements)?;
    let mut prev_start = statements.len();
    for statement in ordered.iter().copied() {
        flush_prelude(&mut statements, prev_start);
        prev_start = statements.len();
        match statement {
            Statement::EmptyStatement(_) | Statement::DebuggerStatement(_) => {}
            Statement::VariableDeclaration(declaration) if declares_hoisted_var(declaration) => {
                convert_hoisted_var_declaration(source, declaration, &mut statements)?;
            }
            Statement::VariableDeclaration(declaration) => {
                for declarator in &declaration.declarations {
                    statements.extend(convert_declarator_stmts(source, declarator)?);
                }
            }
            Statement::ReturnStatement(return_statement) => statements.push(Stmt::Return {
                value: return_statement
                    .argument
                    .as_ref()
                    .map(|expression| convert_expression(source, expression))
                    .transpose()?,
                span: span(return_statement.span),
            }),
            Statement::ExpressionStatement(expression_statement) => {
                convert_expression_statement(source, expression_statement, &mut statements)?;
            }
            Statement::BreakStatement(break_statement) => statements.push(Stmt::Break {
                label: break_statement.label.as_ref().map(|label| Name {
                    span: span(label.span),
                }),
                span: span(break_statement.span),
            }),
            Statement::ContinueStatement(continue_statement) => statements.push(Stmt::Continue {
                label: continue_statement.label.as_ref().map(|label| Name {
                    span: span(label.span),
                }),
                span: span(continue_statement.span),
            }),
            Statement::ThrowStatement(throw_statement) => statements.push(Stmt::Throw {
                value: convert_expression(source, &throw_statement.argument)?,
                span: span(throw_statement.span),
            }),
            Statement::IfStatement(if_statement) => {
                statements.push(convert_if_statement(source, if_statement)?);
            }
            Statement::TryStatement(try_statement) => {
                statements.push(convert_try_statement(source, try_statement)?);
            }
            Statement::WhileStatement(while_statement) => {
                statements.push(Stmt::While {
                    cond: convert_expression(source, &while_statement.test)?,
                    body: convert_if_branch(source, &while_statement.body)?,
                    span: span(while_statement.span),
                });
            }
            Statement::DoWhileStatement(do_while_statement) => {
                statements.push(Stmt::DoWhile {
                    body: convert_if_branch(source, &do_while_statement.body)?,
                    cond: convert_expression(source, &do_while_statement.test)?,
                    span: span(do_while_statement.span),
                });
            }
            Statement::ForStatement(for_statement) => {
                let (pre, stmt) = convert_for_statement(source, for_statement)?;
                statements.extend(pre);
                statements.push(stmt);
            }
            Statement::ForOfStatement(for_of_statement) => {
                statements.extend(convert_for_of(source, for_of_statement)?);
            }
            Statement::ForInStatement(for_in_statement) => {
                statements.push(convert_for_in(source, for_in_statement)?);
            }
            Statement::SwitchStatement(switch) => {
                statements.push(convert_switch(source, switch)?);
            }
            Statement::LabeledStatement(labeled) => {
                convert_labeled_statement(source, labeled, &mut statements)?;
            }
            Statement::BlockStatement(block) => {
                convert_bare_block(source, block, &mut statements)?;
            }
            Statement::FunctionDeclaration(function) => {
                convert_function_declaration(source, function, &forward_names, &mut statements)?;
            }
            Statement::ClassDeclaration(class_declaration) => {
                convert_class_statement(source, class_declaration, &mut statements)?;
            }
            other => {
                let dbg = format!("{other:?}");
                let kind = dbg
                    .split(|c: char| !c.is_alphanumeric())
                    .find(|s| !s.is_empty())
                    .unwrap_or("?");
                return Err(format!("OXC adapter: unsupported statement `{kind}`"));
            }
        }
    }
    flush_prelude(&mut statements, prev_start);
    PENDING_PRELUDE.with(|pending| *pending.borrow_mut() = saved_prelude);
    Ok(statements)
}

/// Sibling function declarations that mention a *later* sibling (mutual
/// recursion) cannot be ordered: the later name is pre-declared with a
/// placeholder closure and its declaration becomes an assignment, so earlier
/// closures capture the (cell-boxed) variable. Returns those later names.
fn forward_declared_names(
    source: &str,
    ordered: &[&Statement<'_>],
) -> std::collections::HashSet<String> {
    let mut forward_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    {
        let declared: Vec<&oxc_ast::ast::Function<'_>> = ordered
            .iter()
            .filter_map(|statement| match statement {
                Statement::FunctionDeclaration(function) => Some(&**function),
                _ => None,
            })
            .collect();
        for (position, earlier) in declared.iter().enumerate() {
            let Some(text) = source.get(earlier.span.start as usize..earlier.span.end as usize) else { continue };
            for later in declared.iter().skip(position + 1) {
                if let Some(id) = later.id.as_ref() {
                    if contains_word(text, id.name.as_str()) {
                        forward_names.insert(id.name.to_string());
                    }
                }
            }
        }
    }
    forward_names
}

/// Emit the placeholder `let` of every forward-referenced function name.
fn predeclare_forward_functions(
    source: &str,
    ordered: &[&Statement<'_>],
    forward_names: &std::collections::HashSet<String>,
    statements: &mut Vec<Stmt>,
) -> Result<(), String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    for statement in ordered.iter().copied() {
        let Statement::FunctionDeclaration(function) = statement else { continue };
        if let Some(id) = function.id.as_ref() {
            if forward_names.contains(id.name.as_str()) {
                let whole = span(function.span);
                statements.push(Stmt::Let {
                    name: Name { span: span(id.span) },
                    ty: None,
                    init: Expr::Function(FunctionExpr {
                        params: Vec::new(),
                        return_type: convert_type_annotation(source, None)?,
                        body: Block { statements: Vec::new(), span: whole },
                        is_arrow: true,
                        is_async: false,
                        is_generator: false,
                        span: whole,
                    }),
                    span: whole,
                });
            }
        }
    }
    Ok(())
}

/// Whether a `var` declaration declares a name hoisted to function scope.
fn declares_hoisted_var(declaration: &oxc_ast::ast::VariableDeclaration<'_>) -> bool {
    declaration.kind == oxc_ast::ast::VariableDeclarationKind::Var
        && declaration.declarations.iter().any(|d| match &d.id {
            BindingPattern::BindingIdentifier(id) => HOISTED_VARS.with(|stack| {
                stack
                    .borrow()
                    .last()
                    .map(|set| set.contains(id.name.as_str()))
                    .unwrap_or(false)
            }),
            _ => false,
        })
}

fn convert_hoisted_var_declaration(
    source: &str,
    declaration: &oxc_ast::ast::VariableDeclaration<'_>,
    statements: &mut Vec<Stmt>,
) -> Result<(), String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    for declarator in &declaration.declarations {
        let hoisted = match &declarator.id {
            BindingPattern::BindingIdentifier(id) => HOISTED_VARS.with(|stack| {
                stack
                    .borrow()
                    .last()
                    .map(|set| set.contains(id.name.as_str()))
                    .unwrap_or(false)
            }),
            _ => false,
        };
        if !hoisted {
            statements.extend(convert_declarator_stmts(source, declarator)?);
            continue;
        }
        // Hoisted: the declaration lives at function scope, so the
        // initializer becomes a plain assignment (and a bare
        // `var x;` leaves the value untouched).
        if let (Some(initializer), BindingPattern::BindingIdentifier(id)) =
            (declarator.init.as_ref(), &declarator.id)
        {
            let whole = span(declarator.span);
            statements.push(Stmt::Expr(Expr::Assign {
                target: Name { span: span(id.span) },
                value: Box::new(convert_expression(source, initializer)?),
                span: whole,
            }));
        }
    }
    Ok(())
}

fn convert_expression_statement(
    source: &str,
    expression_statement: &oxc_ast::ast::ExpressionStatement<'_>,
    statements: &mut Vec<Stmt>,
) -> Result<(), String> {
    // A statement-level comma sequence (`a = 1, b = 2;`) is split
    // into one statement per operand: same evaluation order, no
    // closure per execution.
    let mut inner = &expression_statement.expression;
    while let Expression::ParenthesizedExpression(parenthesized) = inner {
        inner = &parenthesized.expression;
    }
    match inner {
        Expression::SequenceExpression(sequence)
            if !source[sequence.span.start as usize..sequence.span.end as usize].contains("yield") =>
        {
            for operand in sequence.expressions.iter() {
                statements.push(Stmt::Expr(convert_effect_expression(source, operand)?));
            }
        }
        _ => statements.push(Stmt::Expr(convert_effect_expression(
            source,
            &expression_statement.expression,
        )?)),
    }
    Ok(())
}

fn convert_labeled_statement(
    source: &str,
    labeled: &oxc_ast::ast::LabeledStatement<'_>,
    statements: &mut Vec<Stmt>,
) -> Result<(), String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let span_all = span(labeled.span);
    if let Statement::BlockStatement(block) = &labeled.body {
        // A labeled block: `if (true) { ... }` carries the whole block
        // as the single labeled statement.
        let body = Block {
            statements: convert_statement_list(source, &block.body)?,
            span: span(block.span),
        };
        statements.push(Stmt::Labeled {
            label: Name { span: span(labeled.label.span) },
            statement: Box::new(Stmt::If {
                cond: Expr::Bool { value: true, span: span(block.span) },
                then_block: body,
                else_block: None,
                span: span(block.span),
            }),
            span: span_all,
        });
        return Ok(());
    }
    let inner = convert_statement_list(source, std::slice::from_ref(&labeled.body))?;
    // The labelled body is a single statement; wrap a multi-statement
    // body (rare) in a block-equivalent by taking the first.
    let mut inner = inner;
    let statement = inner
        .pop()
        .unwrap_or(Stmt::Expr(Expr::Null { span: span_all }));
    statements.extend(inner);
    statements.push(Stmt::Labeled {
        label: Name { span: span(labeled.label.span) },
        statement: Box::new(statement),
        span: span_all,
    });
    Ok(())
}

/// A bare `{ … }` block. One that declares `let`/`const` keeps its own
/// scope (modelled as `if (true) { … }`); other blocks are flattened.
fn convert_bare_block(
    source: &str,
    block: &oxc_ast::ast::BlockStatement<'_>,
    statements: &mut Vec<Stmt>,
) -> Result<(), String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let shadows_top_level = TOP_LEVEL_BLOCK.with(|cell| cell.get()) == block.span.start
        && block.body.iter().any(|inner| {
            matches!(
                inner,
                Statement::VariableDeclaration(decl)
                    if !matches!(decl.kind, oxc_ast::ast::VariableDeclarationKind::Var)
                        && decl.declarations.iter().any(|d| matches!(
                            &d.id,
                            BindingPattern::BindingIdentifier(id)
                                if TOP_LEVEL_NAMES.with(|names| names.borrow().contains(id.name.as_str()))
                        ))
            )
        });
    let declares_lexical = (TOP_LEVEL_BLOCK.with(|cell| cell.get()) != block.span.start || shadows_top_level) && block.body.iter().any(|inner| {
        matches!(
            inner,
            Statement::VariableDeclaration(decl)
                if !matches!(decl.kind, oxc_ast::ast::VariableDeclarationKind::Var)
        )
    });
    let inner = convert_statement_list(source, &block.body)?;
    if declares_lexical {
        statements.push(Stmt::If {
            cond: Expr::Bool { value: true, span: span(block.span) },
            then_block: Block { statements: inner, span: span(block.span) },
            else_block: None,
            span: span(block.span),
        });
    } else {
        statements.extend(inner);
    }
    Ok(())
}

/// A nested function declaration binds a function value (hoisting to the top
/// of the scope is approximated by its textual position).
fn convert_function_declaration(
    source: &str,
    function: &OxcFunction<'_>,
    forward_names: &std::collections::HashSet<String>,
    statements: &mut Vec<Stmt>,
) -> Result<(), String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let Some(id) = function.id.as_ref() else {
        return Err("OXC adapter requires named function declarations".into());
    };
    // A declaration named like a parameter overrides it at entry:
    // skipped here, emitted as an assignment at the body head.
    if PARAM_SHADOWS.with(|names| names.borrow().iter().any(|n| n == id.name.as_str())) {
        if EMIT_SHADOW.with(|flag| flag.get()) {
            statements.push(Stmt::Expr(Expr::Assign {
                target: Name { span: span(id.span) },
                value: Box::new(convert_oxc_function(source, function)?),
                span: span(function.span),
            }));
        }
        return Ok(());
    }
    if forward_names.contains(id.name.as_str()) {
        statements.push(Stmt::Expr(Expr::Assign {
            target: Name { span: span(id.span) },
            value: Box::new(convert_oxc_function(source, function)?),
            span: span(function.span),
        }));
        return Ok(());
    }
    statements.push(Stmt::Let {
        name: Name { span: span(id.span) },
        ty: None,
        init: convert_oxc_function(source, function)?,
        span: span(function.span),
    });
    Ok(())
}

/// A block-local / nested named class declaration is hoisted to the program's
/// class list (Zun classes are program-global); its name binds through
/// `classes_by_name` in sema, so no statement is emitted here.
fn convert_class_statement(
    source: &str,
    class_declaration: &oxc_ast::ast::Class<'_>,
    statements: &mut Vec<Stmt>,
) -> Result<(), String> {
    let class = convert_class_declaration(source, class_declaration)
        .map_err(|errors| errors.join("; "))?;
    HOISTED_CLASSES.with(|cells| cells.borrow_mut().push(class));
    statements.extend(CLASS_TRAILING.with(|cells| std::mem::take(&mut *cells.borrow_mut())));
    Ok(())
}

/// Insert the statements queued by expression conversion (the `yield*` loop)
/// before the statement(s) converted since `start`.
pub(super) fn flush_prelude(statements: &mut Vec<Stmt>, start: usize) {
    let prelude = PENDING_PRELUDE.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    if prelude.is_empty() {
        return;
    }
    let tail = statements.split_off(start.min(statements.len()));
    statements.extend(prelude);
    statements.extend(tail);
}

pub(super) fn convert_if_branch(source: &str, statement: &Statement<'_>) -> Result<Block, String> {
    // A branch is a block (convert its inner list) or any single statement
    // (a one-element list). Delegating to `convert_statement_list` keeps one
    // statement converter, so a branch supports exactly what a block body does.
    let statements = match statement {
        Statement::BlockStatement(block) => convert_statement_list(source, &block.body)?,
        other => convert_statement_list(source, std::slice::from_ref(other))?,
    };
    Ok(Block {
        statements,
        span: Span::new(statement.span().start, statement.span().end),
    })
}

/// Desugar a `switch` to an `if`/`else if` chain comparing the discriminant with
/// `===` against each case, with the `default` as the final `else`. `break`
/// statements at the top of a case body are dropped (the common break-per-case
/// form; fall-through is not modelled), matching the native parser's lowering.
pub(super) fn convert_switch(
    source: &str,
    switch: &oxc_ast::ast::SwitchStatement<'_>,
) -> Result<Stmt, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let whole = span(switch.span);
    let discriminant = convert_expression(source, &switch.discriminant)?;
    let mut cases = Vec::new();
    for case in &switch.cases {
        let stmts = convert_statement_list(source, &case.consequent)?;
        cases.push(crate::ast::SwitchCase {
            test: case
                .test
                .as_ref()
                .map(|test| convert_expression(source, test))
                .transpose()?,
            body: Block { statements: stmts, span: span(case.span) },
        });
    }
    Ok(Stmt::Switch { discriminant, cases, span: whole })
}

pub(super) fn convert_if_statement(
    source: &str,
    if_statement: &oxc_ast::ast::IfStatement<'_>,
) -> Result<Stmt, String> {
    Ok(Stmt::If {
        cond: convert_expression(source, &if_statement.test)?,
        then_block: convert_if_branch(source, &if_statement.consequent)?,
        else_block: if_statement
            .alternate
            .as_ref()
            .map(|statement| convert_if_branch(source, statement))
            .transpose()?,
        span: Span::new(if_statement.span.start, if_statement.span.end),
    })
}

pub(super) fn convert_try_statement(
    source: &str,
    try_statement: &oxc_ast::ast::TryStatement<'_>,
) -> Result<Stmt, String> {
    fn convert_block_node(
        source: &str,
        block: &oxc_ast::ast::BlockStatement<'_>,
    ) -> Result<Block, String> {
        let statements = convert_statement_list(source, &block.body)?;
        Ok(Block {
            statements,
            span: Span::new(block.span.start, block.span.end),
        })
    }

    let catches = if let Some(handler) = try_statement.handler.as_ref() {
        // `catch { … }` with no binding, or a binding pattern we cannot name
        // directly, uses a zero-width binding span (an unnamed, unused local).
        let binding_span = match handler.param.as_ref() {
            Some(parameter) => match &parameter.pattern {
                BindingPattern::BindingIdentifier(identifier) => {
                    Span::new(identifier.span.start, identifier.span.end)
                }
                // A destructuring pattern binds a hidden local named by the
                // pattern's own source text (no identifier can collide); the
                // body is prefixed with the destructuring of that local.
                _ => Span::new(parameter.span.start, parameter.span.end),
            },
            None => Span::new(handler.body.span.start, handler.body.span.start),
        };
        let mut catch_body = convert_block_node(source, &handler.body)?;
        if let Some(parameter) = handler.param.as_ref() {
            if !matches!(&parameter.pattern, BindingPattern::BindingIdentifier(_)) {
                let pattern = convert_binding_pattern(source, &parameter.pattern)?;
                catch_body.statements.insert(
                    0,
                    Stmt::Destructure {
                        pattern,
                        init: crate::ast::Expr::Ident(Name { span: binding_span }),
                        span: Span::new(binding_span.start, binding_span.end),
                    },
                );
            }
        }
        vec![crate::ast::CatchClause {
            binding: Name { span: binding_span },
            ty: crate::ast::TypeAnnotation {
                name: Name {
                    span: Span::new(0, 0),
                },
                is_array: false,
            },
            body: catch_body,
        }]
    } else {
        Vec::new()
    };
    Ok(Stmt::Try {
        body: convert_block_node(source, &try_statement.block)?,
        catches,
        finally: try_statement
            .finalizer
            .as_ref()
            .map(|finalizer| convert_block_node(source, finalizer))
            .transpose()?,
        span: Span::new(try_statement.span.start, try_statement.span.end),
    })
}
