//! Native OXC parsing boundary for standard JavaScript and TypeScript.
//!
//! The existing parser remains authoritative for Zun-specific syntax while the
//! HIR adapter is built incrementally. This boundary lets callers validate
//! standard JS/TS with OXC without coupling the rest of the compiler to OXC's
//! arena-owned AST types.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, AssignmentOperator, AssignmentTarget, BinaryOperator,
    BindingPattern, ChainElement, Decorator as OxcDecorator,
    ClassElement, Declaration, ExportDefaultDeclarationKind, Expression, ForStatementInit,
    ForStatementLeft, ImportDeclarationSpecifier, LogicalOperator, MethodDefinitionKind,
    ModuleExportName, ObjectPropertyKind, PropertyKey, PropertyKind, Statement, UnaryOperator,
};
use oxc_ast::ast::Function as OxcFunction;
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use zun_common::Span;

use crate::ast::{BinaryOp, Block, Expr, Function, FunctionExpr, Name, Program, Stmt};

fn convert_type_annotation(
    source: &str,
    annotation: Option<&oxc_ast::ast::TSTypeAnnotation<'_>>,
) -> Result<Option<crate::ast::TypeAnnotation>, String> {
    let Some(annotation) = annotation else {
        return Ok(None);
    };
    let start = annotation.span.start as usize;
    let end = annotation.span.end as usize;
    let raw = source
        .get(start..end)
        .ok_or_else(|| "OXC adapter received an invalid type span".to_owned())?;
    let type_start = raw
        .find(|character: char| !character.is_ascii_whitespace() && character != ':')
        .ok_or_else(|| "OXC adapter received an empty type annotation".to_owned())?;
    let mut type_text = raw[type_start..].trim();
    // Strip trailing `[]` array suffixes; Zun arrays are untyped, so `T[]`
    // resolves to `Array` and the base name stays for diagnostics.
    let mut is_array = false;
    while let Some(stripped) = type_text.strip_suffix("[]") {
        type_text = stripped.trim_end();
        is_array = true;
    }
    // Keep the base name of a generic (`Array<number>` -> `Array`); the type
    // arguments are erased like everything else.
    if let Some(generic) = type_text.find('<') {
        type_text = type_text[..generic].trim_end();
    }
    // Zun erases types (PLAN: typed-TS pragmatism). A plain named type keeps its
    // name (so `Array`/class receivers still resolve); anything the source
    // language cannot name here — a function type `(x) => y`, a union, a tuple,
    // an object or literal type — erases to an untyped (`any`) binding rather
    // than being rejected, matching JS/TS erasure semantics.
    if type_text.is_empty()
        || !type_text
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '$')
    {
        return Ok(None);
    }
    let name_start = start + type_start;
    Ok(Some(crate::ast::TypeAnnotation {
        name: Name {
            span: Span::new(name_start as u32, (name_start + type_text.len()) as u32),
        },
        is_array,
    }))
}

/// Map an OXC decorator node to our [`crate::ast::Decorator`].
///
/// Phase 1 is syntax only. The callee is the decorator expression (an
/// identifier or member-access chain); the `@foo(...)` call form carries its
/// arguments, a bare `@foo` carries `None`. Unsupported callee/argument forms
/// surface as an explicit error (no silent drop) because `convert_expression`
/// and `convert_argument` reject what they cannot represent.
fn convert_decorator(source: &str, decorator: &OxcDecorator<'_>) -> Result<crate::ast::Decorator, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let (callee, args) = match &decorator.expression {
        Expression::CallExpression(call) => {
            let mut args = Vec::with_capacity(call.arguments.len());
            for argument in &call.arguments {
                args.push(convert_argument(source, argument)?);
            }
            (convert_expression(source, &call.callee)?, Some(args))
        }
        other => (convert_expression(source, other)?, None),
    };
    Ok(crate::ast::Decorator {
        callee,
        args,
        span: span(decorator.span),
    })
}

/// Convert an OXC property key used in an object binding pattern.
fn convert_binding_key(source: &str, property: &oxc_ast::ast::BindingProperty<'_>)
    -> Result<crate::ast::PropertyKey, String>
{
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    if property.computed {
        let Some(key_expression) = property.key.as_expression() else {
            return Err("OXC adapter requires an expression computed key".into());
        };
        return Ok(crate::ast::PropertyKey::Computed(Box::new(convert_expression(
            source,
            key_expression,
        )?)));
    }
    match &property.key {
        PropertyKey::StaticIdentifier(identifier) => {
            Ok(crate::ast::PropertyKey::Ident(Name { span: span(identifier.span) }))
        }
        PropertyKey::StringLiteral(string) => {
            let Some(value) = string.value.as_str() else {
                return Err("OXC adapter cannot represent non-UTF-8 string keys".into());
            };
            Ok(crate::ast::PropertyKey::String { value: value.to_owned(), span: span(string.span) })
        }
        PropertyKey::NumericLiteral(number) => {
            Ok(crate::ast::PropertyKey::Number { value: number.value, span: span(number.span) })
        }
        _ => Err("OXC adapter supports identifier, string, and numeric property keys only".into()),
    }
}

/// Convert an OXC binding pattern (identifier, array, object, assignment) into
/// the Zun [`crate::ast::Pattern`]. Nested patterns recurse naturally.
fn convert_binding_pattern(
    source: &str,
    pattern: &BindingPattern<'_>,
) -> Result<crate::ast::Pattern, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => Ok(crate::ast::Pattern::Ident {
            name: Name { span: span(identifier.span) },
            ty: None,
        }),
        BindingPattern::ArrayPattern(array) => {
            let mut elements = Vec::with_capacity(array.elements.len());
            for element in &array.elements {
                match element {
                    None => elements.push(None),
                    Some(binding) => {
                        let (pattern, default) = split_assignment(source, binding)?;
                        elements.push(Some(crate::ast::PatternElement { pattern, default }));
                    }
                }
            }
            let rest = match &array.rest {
                Some(rest) => Some(Box::new(convert_binding_pattern(source, &rest.argument)?)),
                None => None,
            };
            Ok(crate::ast::Pattern::Array { elements, rest, span: span(array.span) })
        }
        BindingPattern::ObjectPattern(object) => {
            let mut properties = Vec::with_capacity(object.properties.len());
            for property in &object.properties {
                let key = convert_binding_key(source, property)?;
                let (value, default) = split_assignment(source, &property.value)?;
                properties.push(crate::ast::PatternProperty { key, value, default });
            }
            let rest = match &object.rest {
                Some(rest) => Some(Box::new(convert_binding_pattern(source, &rest.argument)?)),
                None => None,
            };
            Ok(crate::ast::Pattern::Object { properties, rest, span: span(object.span) })
        }
        BindingPattern::AssignmentPattern(_) => {
            // A top-level `=` default is only meaningful attached to its
            // enclosing element/property, handled by `split_assignment`.
            Err("OXC adapter: unexpected default outside an element or property".into())
        }
    }
}

/// Split a binding pattern that may carry a top-level default (`x = 1`,
/// `{a} = obj`) into its inner pattern and optional default expression.
fn split_assignment(
    source: &str,
    pattern: &BindingPattern<'_>,
) -> Result<(crate::ast::Pattern, Option<Expr>), String> {
    match pattern {
        BindingPattern::AssignmentPattern(assignment) => Ok((
            convert_binding_pattern(source, &assignment.left)?,
            Some(convert_expression(source, &assignment.right)?),
        )),
        other => Ok((convert_binding_pattern(source, other)?, None)),
    }
}

/// Convert one variable declarator into either a `Stmt::Let` (identifier head)
/// or a `Stmt::Destructure` (array/object pattern head).
fn convert_declarator(
    source: &str,
    declarator: &oxc_ast::ast::VariableDeclarator<'_>,
) -> Result<Stmt, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    // An uninitialized binding (`let x;`) defaults to `undefined` (`void 0`).
    let init = match declarator.init.as_ref() {
        Some(initializer) => {
            if let (Expression::ClassExpression(_), BindingPattern::BindingIdentifier(binding)) =
                (initializer, &declarator.id)
            {
                CLASS_NAME_HINT
                    .with(|cell| *cell.borrow_mut() = Some(span(binding.span)));
            }
            let converted = convert_expression(source, initializer);
            CLASS_NAME_HINT.with(|cell| *cell.borrow_mut() = None);
            converted?
        }
        None => Expr::Unary {
            op: crate::ast::UnaryOp::Void,
            operand: Box::new(Expr::Number { value: 0.0, span: span(declarator.span) }),
            span: span(declarator.span),
        },
    };
    match &declarator.id {
        BindingPattern::BindingIdentifier(identifier) => Ok(Stmt::Let {
            name: Name { span: span(identifier.span) },
            ty: convert_type_annotation(source, declarator.type_annotation.as_deref())?,
            init,
            span: span(declarator.span),
        }),
        other => Ok(Stmt::Destructure {
            pattern: convert_binding_pattern(source, other)?,
            init,
            span: span(declarator.span),
        }),
    }
}

fn convert_argument(source: &str, argument: &Argument<'_>) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    // `...spread` is the only argument form that is not itself an expression;
    // every other argument is an ordinary expression, so route it through the
    // full expression converter rather than re-listing a subset here.
    if let Argument::SpreadElement(spread) = argument {
        return Ok(Expr::Spread {
            expr: Box::new(convert_expression(source, &spread.argument)?),
            span: span(spread.span),
        });
    }
    let expression = argument
        .as_expression()
        .ok_or("OXC adapter cannot represent this argument")?;
    // A static `Number.f` / `Math.f` passed as a callback (`xs.filter(Number.isNaN)`)
    // becomes `(f) => Number.f(f)`; the method-name span doubles as the
    // parameter name, which is harmless because the body only reads it once.
    if let Expression::StaticMemberExpression(member) = expression {
        if let Expression::Identifier(object) = &member.object {
            let object_name = source
                .get(object.span.start as usize..object.span.end as usize)
                .unwrap_or("");
            let property = source
                .get(member.property.span.start as usize..member.property.span.end as usize)
                .unwrap_or("");
            let callable = property.chars().next().is_some_and(|c| c.is_ascii_lowercase());
            if matches!(object_name, "Number" | "Math") && callable && !member.optional {
                let property_name = Name { span: span(member.property.span) };
                let call = Expr::MethodCall {
                    receiver: Box::new(Expr::Ident(Name { span: span(object.span) })),
                    method: Name { span: span(member.property.span) },
                    args: vec![Expr::Ident(Name { span: span(member.property.span) })],
                    span: span(member.span),
                };
                return Ok(Expr::Function(FunctionExpr {
                    params: vec![crate::ast::Param {
                        name: property_name,
                        ty: None,
                        default_value: None,
                        is_rest: false,
                    }],
                    return_type: None,
                    body: Block {
                        statements: vec![Stmt::Return {
                            value: Some(call),
                            span: span(member.span),
                        }],
                        span: span(member.span),
                    },
                    is_arrow: true,
                    is_async: false,
                    is_generator: false,
                    span: span(member.span),
                }));
            }
        }
    }
    convert_expression(source, expression)
}

fn convert_block_statement(source: &str, statement: &Statement<'_>) -> Result<Block, String> {
    let Statement::BlockStatement(block) = statement else {
        return Err("OXC adapter requires block branches for conditional statements".into());
    };
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let statements = convert_statement_list(source, &block.body)?;
    Ok(Block {
        statements,
        span: span(block.span),
    })
}

/// Convert an OXC statement list into Zun statements, reused by block
/// statements and by object-method / function bodies.
fn convert_statement_list(source: &str, body: &[Statement<'_>]) -> Result<Vec<Stmt>, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let mut statements = Vec::with_capacity(body.len());
    let saved_prelude = PENDING_PRELUDE.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    let mut prev_start = 0usize;
    let ordered = hoist_function_declarations(source, body);
    for statement in ordered.iter().copied() {
        flush_prelude(&mut statements, prev_start);
        prev_start = statements.len();
        match statement {
            Statement::EmptyStatement(_) | Statement::DebuggerStatement(_) => {}
            Statement::VariableDeclaration(declaration)
                if declaration.kind == oxc_ast::ast::VariableDeclarationKind::Var
                    && declaration.declarations.iter().any(|d| match &d.id {
                        BindingPattern::BindingIdentifier(id) => HOISTED_VARS.with(|stack| {
                            stack
                                .borrow()
                                .last()
                                .map(|set| set.contains(id.name.as_str()))
                                .unwrap_or(false)
                        }),
                        _ => false,
                    }) =>
            {
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
                        statements.push(convert_declarator(source, declarator)?);
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
            }
            Statement::VariableDeclaration(declaration) => {
                for declarator in &declaration.declarations {
                    statements.push(convert_declarator(source, declarator)?);
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
                    continue;
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
            }
            // A bare `{ … }` block. One that declares `let`/`const` keeps its own
            // scope (modelled as `if (true) { … }`); other blocks are flattened.
            Statement::BlockStatement(block) => {
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
            }
            // A nested function declaration binds a function value (hoisting to
            // the top of the scope is approximated by its textual position).
            Statement::FunctionDeclaration(function) => {
                let Some(id) = function.id.as_ref() else {
                    return Err("OXC adapter requires named function declarations".into());
                };
                statements.push(Stmt::Let {
                    name: Name { span: span(id.span) },
                    ty: None,
                    init: convert_oxc_function(source, function)?,
                    span: span(function.span),
                });
            }
            // A block-local / nested named class declaration is hoisted to the
            // program's class list (Zun classes are program-global); its name
            // binds through `classes_by_name` in sema, so no statement is
            // emitted here.
            Statement::ClassDeclaration(class_declaration) => {
                let class = convert_class_declaration(source, class_declaration)
                    .map_err(|errors| errors.join("; "))?;
                HOISTED_CLASSES.with(|cells| cells.borrow_mut().push(class));
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

/// Insert the statements queued by expression conversion (the `yield*` loop)
/// before the statement(s) converted since `start`.
fn flush_prelude(statements: &mut Vec<Stmt>, start: usize) {
    let prelude = PENDING_PRELUDE.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    if prelude.is_empty() {
        return;
    }
    let tail = statements.split_off(start.min(statements.len()));
    statements.extend(prelude);
    statements.extend(tail);
}

/// A parameter's default expression; an optional `x?: T` parameter without an
/// initializer defaults to `undefined`.
fn param_default(
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
            operand: Box::new(Expr::Number { value: 0.0, span: at }),
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
fn convert_formal_param(
    source: &str,
    parameter: &oxc_ast::ast::FormalParameter<'_>,
    prologue: &mut Vec<Stmt>,
) -> Result<crate::ast::Param, String> {
    let span = |s: oxc_span::Span| Span::new(s.start, s.end);
    match &parameter.pattern {
        BindingPattern::BindingIdentifier(identifier) => Ok(crate::ast::Param {
            name: Name { span: span(identifier.span) },
            ty: convert_type_annotation(source, parameter.type_annotation.as_deref())?,
            default_value: param_default(source, parameter)?,
            is_rest: false,
        }),
        other => {
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

fn convert_oxc_function(source: &str, function: &OxcFunction<'_>) -> Result<Expr, String> {
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
    statements.extend(convert_function_statements(source, &body.statements, &param_names(&function.params))?);
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

fn convert_if_branch(source: &str, statement: &Statement<'_>) -> Result<Block, String> {
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

/// Convert a C-style `for (init; cond; update) body` statement.
fn convert_for_statement(
    source: &str,
    for_statement: &oxc_ast::ast::ForStatement<'_>,
) -> Result<(Vec<Stmt>, Stmt), String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let mut pre = Vec::new();
    let init = match &for_statement.init {
        None => None,
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

/// Desugar a `switch` to an `if`/`else if` chain comparing the discriminant with
/// `===` against each case, with the `default` as the final `else`. `break`
/// statements at the top of a case body are dropped (the common break-per-case
/// form; fall-through is not modelled), matching the native parser's lowering.
fn convert_switch(
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

/// Convert a `for (… in …)` statement to the `ForIn` node (lowered to key
/// iteration in sema). The binding is a declared identifier or pattern.
fn convert_for_in(
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
fn convert_for_await(
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
fn convert_for_of(
    source: &str,
    for_of_statement: &oxc_ast::ast::ForOfStatement<'_>,
) -> Result<Vec<Stmt>, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    if for_of_statement.r#await {
        return convert_for_await(source, for_of_statement);
    }
    let ForStatementLeft::VariableDeclaration(declaration) = &for_of_statement.left else {
        return Err("OXC adapter supports declared for-of bindings only".into());
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
    Ok(vec![Stmt::ForOf {
        pattern,
        iterable: convert_expression(source, &for_of_statement.right)?,
        body: convert_if_branch(source, &for_of_statement.body)?,
        span: span(for_of_statement.span),
    }])
}

fn convert_if_statement(
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

fn convert_try_statement(
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

fn convert_expression(source: &str, expression: &Expression<'_>) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    match expression {
        Expression::NumericLiteral(number) => Ok(Expr::Number {
            value: number.value,
            span: span(number.span),
        }),
        Expression::BigIntLiteral(bigint) => Ok(Expr::BigInt {
            value: bigint.value.as_str().to_owned(),
            span: span(bigint.span),
        }),
        Expression::StringLiteral(string) => {
            let Some(value) = string.value.as_str() else {
                return Err("OXC adapter cannot represent non-UTF-8 string literals".into());
            };
            Ok(Expr::String {
                value: value.to_owned(),
                span: span(string.span),
            })
        }
        Expression::BooleanLiteral(boolean) => Ok(Expr::Bool {
            value: boolean.value,
            span: span(boolean.span),
        }),
        Expression::NullLiteral(null) => Ok(Expr::Null {
            span: span(null.span),
        }),
        Expression::RegExpLiteral(regex) => Ok(Expr::RegexLiteral {
            pattern: regex.regex.pattern.text.to_string(),
            flags: regex.regex.flags.to_string(),
            span: span(regex.span),
        }),
        Expression::TemplateLiteral(template) => {
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
        Expression::TaggedTemplateExpression(tagged) => {
            let tag = Box::new(convert_expression(source, &tagged.tag)?);
            let quasi = &tagged.quasi;
            let mut cooked = Vec::with_capacity(quasi.quasis.len());
            let mut raw = Vec::with_capacity(quasi.quasis.len());
            for element in &quasi.quasis {
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
                raw,
                exprs,
                span: span(tagged.span),
            })
        }
        Expression::Identifier(identifier) => {
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
        // `super.method(...)`: the receiver name is the source text `super`.
        Expression::Super(super_expression) => Ok(Expr::Ident(Name {
            span: span(super_expression.span),
        })),
        Expression::ThisExpression(this_expression) => Ok(Expr::Ident(Name {
            span: span(this_expression.span),
        })),
        // `super` is modelled as an identifier whose text is `super`; sema
        // recognises it in call/method-call receiver position and lowers the
        // base-class dispatch. The span covers the `super` keyword in source.
        Expression::Super(super_keyword) => Ok(Expr::Ident(Name {
            span: span(super_keyword.span),
        })),
        Expression::ParenthesizedExpression(parenthesized) => {
            convert_expression(source, &parenthesized.expression)
        }
        Expression::SequenceExpression(sequence) => {
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
        Expression::TSAsExpression(assertion) => Ok(Expr::Cast {
            expr: Box::new(convert_expression(source, &assertion.expression)?),
            ty: crate::ast::TypeAnnotation {
                name: Name {
                    span: Span::new(
                        assertion.type_annotation.span().start,
                        assertion.type_annotation.span().end,
                    ),
                },
                is_array: false,
            },
            span: span(assertion.span),
        }),
        Expression::TSTypeAssertion(assertion) => Ok(Expr::Cast {
            expr: Box::new(convert_expression(source, &assertion.expression)?),
            ty: crate::ast::TypeAnnotation {
                name: Name {
                    span: Span::new(
                        assertion.type_annotation.span().start,
                        assertion.type_annotation.span().end,
                    ),
                },
                is_array: false,
            },
            span: span(assertion.span),
        }),
        Expression::TSSatisfiesExpression(assertion) => Ok(Expr::Cast {
            expr: Box::new(convert_expression(source, &assertion.expression)?),
            ty: crate::ast::TypeAnnotation {
                name: Name {
                    span: Span::new(
                        assertion.type_annotation.span().start,
                        assertion.type_annotation.span().end,
                    ),
                },
                is_array: false,
            },
            span: span(assertion.span),
        }),
        Expression::TSNonNullExpression(assertion) => convert_expression(source, &assertion.expression),
        Expression::StaticMemberExpression(member) => {
            let object = Box::new(convert_expression(source, &member.object)?);
            let field = Name {
                span: span(member.property.span),
            };
            if member.optional {
                Ok(Expr::OptionalField {
                    object,
                    field,
                    span: span(member.span),
                })
            } else {
                Ok(Expr::Field {
                    object,
                    field,
                    span: span(member.span),
                })
            }
        }
        Expression::ComputedMemberExpression(member) => {
            let object = Box::new(convert_expression(source, &member.object)?);
            // `obj['name']` with a plain identifier-shaped literal is `obj.name`.
            if let Expression::StringLiteral(text) = &member.expression {
                if text.value.as_str().is_some_and(is_plain_identifier)
                    && (text.span.end - text.span.start) as usize
                        == text.value.as_str().map_or(0, str::len) + 2
                {
                    let field = Name { span: Span::new(text.span.start + 1, text.span.end - 1) };
                    return Ok(if member.optional {
                        Expr::OptionalField { object, field, span: span(member.span) }
                    } else {
                        Expr::Field { object, field, span: span(member.span) }
                    });
                }
            }
            let index = Box::new(convert_expression(source, &member.expression)?);
            if member.optional {
                return Ok(Expr::OptionalIndex {
                    object,
                    index,
                    span: span(member.span),
                });
            }
            Ok(Expr::Index {
                object,
                index,
                span: span(member.span),
            })
        }
        Expression::PrivateFieldExpression(member) => {
            if member.optional {
                return Err("OXC adapter does not support optional private members yet".into());
            }
            Ok(Expr::PrivateField {
                object: Box::new(convert_expression(source, &member.object)?),
                field: Name {
                    span: span(member.field.span),
                },
                span: span(member.span),
            })
        }
        Expression::ChainExpression(chain) => convert_chain(source, chain),
        Expression::BinaryExpression(binary) => {
            let op = match binary.operator {
                BinaryOperator::Equality => BinaryOp::LooseEq,
                BinaryOperator::StrictEquality => BinaryOp::Eq,
                BinaryOperator::Inequality => BinaryOp::LooseNe,
                BinaryOperator::StrictInequality => BinaryOp::Ne,
                BinaryOperator::LessThan => BinaryOp::Lt,
                BinaryOperator::LessEqualThan => BinaryOp::Le,
                BinaryOperator::GreaterThan => BinaryOp::Gt,
                BinaryOperator::GreaterEqualThan => BinaryOp::Ge,
                BinaryOperator::Addition => BinaryOp::Add,
                BinaryOperator::Subtraction => BinaryOp::Sub,
                BinaryOperator::Multiplication => BinaryOp::Mul,
                BinaryOperator::Division => BinaryOp::Div,
                BinaryOperator::Remainder => BinaryOp::Rem,
                BinaryOperator::Exponential => BinaryOp::Pow,
                BinaryOperator::BitwiseAnd => BinaryOp::BitAnd,
                BinaryOperator::BitwiseOR => BinaryOp::BitOr,
                BinaryOperator::BitwiseXOR => BinaryOp::BitXor,
                BinaryOperator::ShiftLeft => BinaryOp::Shl,
                BinaryOperator::ShiftRight => BinaryOp::Shr,
                BinaryOperator::ShiftRightZeroFill => BinaryOp::UShr,
                BinaryOperator::In => BinaryOp::In,
                BinaryOperator::Instanceof => BinaryOp::Instanceof,
                _ => return Err("OXC adapter does not support this binary operator yet".into()),
            };
            Ok(Expr::Binary {
                op,
                lhs: Box::new(convert_expression(source, &binary.left)?),
                rhs: Box::new(convert_expression(source, &binary.right)?),
                span: span(binary.span),
            })
        }
        Expression::LogicalExpression(logical) => {
            let op = match logical.operator {
                LogicalOperator::And => BinaryOp::And,
                LogicalOperator::Or => BinaryOp::Or,
                LogicalOperator::Coalesce => BinaryOp::Nullish,
            };
            Ok(Expr::Binary {
                op,
                lhs: Box::new(convert_expression(source, &logical.left)?),
                rhs: Box::new(convert_expression(source, &logical.right)?),
                span: span(logical.span),
            })
        }
        Expression::ConditionalExpression(conditional) => Ok(Expr::Conditional {
            condition: Box::new(convert_expression(source, &conditional.test)?),
            when_true: Box::new(convert_expression(source, &conditional.consequent)?),
            when_false: Box::new(convert_expression(source, &conditional.alternate)?),
            span: span(conditional.span),
        }),
        Expression::UnaryExpression(unary) => {
            // Unary plus (`+x`) is numeric coercion; the language has no such
            // operator, so desugar to `x - 0`, which applies ToNumber to the
            // operand with the same result for every primitive.
            if matches!(unary.operator, UnaryOperator::UnaryPlus) {
                // `x as any - 0` boxes the operand so the subtraction takes the
                // dynamic arithmetic path, which applies ToNumber at run time.
                let operand = convert_expression(source, &unary.argument)?;
                let boxed = Expr::Cast {
                    expr: Box::new(operand),
                    ty: crate::ast::TypeAnnotation {
                        name: Name { span: Span::new(unary.span.start, unary.span.start) },
                        is_array: false,
                    },
                    span: span(unary.span),
                };
                return Ok(Expr::Binary {
                    op: BinaryOp::Sub,
                    lhs: Box::new(boxed),
                    rhs: Box::new(Expr::Number { value: 0.0, span: span(unary.span) }),
                    span: span(unary.span),
                });
            }
            let op = match unary.operator {
                UnaryOperator::UnaryNegation => crate::ast::UnaryOp::Neg,
                UnaryOperator::LogicalNot => crate::ast::UnaryOp::Not,
                UnaryOperator::BitwiseNot => crate::ast::UnaryOp::BitNot,
                UnaryOperator::Void => crate::ast::UnaryOp::Void,
                UnaryOperator::Typeof => crate::ast::UnaryOp::Typeof,
                UnaryOperator::Delete => crate::ast::UnaryOp::Delete,
                _ => return Err("OXC adapter does not support this unary operator yet".into()),
            };
            Ok(Expr::Unary {
                op,
                operand: Box::new(convert_expression(source, &unary.argument)?),
                span: span(unary.span),
            })
        }
        Expression::ImportExpression(import) => {
            if !import.options.is_none() {
                return Err("OXC adapter does not support import options yet".into());
            }
            Ok(Expr::ImportCall {
                specifier: Box::new(convert_expression(source, &import.source)?),
                span: span(import.span),
            })
        }
        Expression::AwaitExpression(await_expression) => Ok(Expr::Await {
            operand: Box::new(convert_expression(source, &await_expression.argument)?),
            span: span(await_expression.span),
        }),
        Expression::YieldExpression(yield_expression) if yield_expression.delegate => {
            // `yield* source`: drive the delegate by hand. The iteration setup and
            // the forwarding loop become statements placed before the statement
            // that contains the expression (see `PENDING_PRELUDE`); the expression
            // itself reads the delegate's return value.
            let Some(argument) = yield_expression.argument.as_ref() else {
                return Err("OXC adapter: `yield*` requires an operand".into());
            };
            let sp = span(yield_expression.span);
            let operand = convert_expression(source, argument)?;
            let n = YIELD_STAR_COUNTER.with(|c| {
                c.set(c.get() + 1);
                c.get()
            });
            let it = crate::ast::synthetic_ident(&format!("__ys_it{n}"));
            let result = crate::ast::synthetic_ident(&format!("__ys_r{n}"));
            let ok = crate::ast::synthetic_ident(&format!("__ys_ok{n}"));
            let threw = crate::ast::synthetic_ident(&format!("__ys_th{n}"));
            let sent = crate::ast::synthetic_ident(&format!("__ys_s{n}"));
            let caught = crate::ast::synthetic_ident(&format!("__ys_c{n}"));
            let ret_answer = crate::ast::synthetic_ident(&format!("__ys_rr{n}"));
            let string = |text: &str| Expr::String { value: text.to_owned(), span: sp };
            let index = |name: Name, key: &str| Expr::Index {
                object: Box::new(Expr::Ident(name)),
                index: Box::new(string(key)),
                span: sp,
            };
            let step = |args: Vec<Expr>| Expr::ComputedCall { callee: Box::new(index(it, "next")), args, span: sp };
            // `typeof it.key !== "undefined"`: a delegate method that is present.
            let defined = |key: &str| Expr::Binary {
                op: BinaryOp::Ne,
                lhs: Box::new(Expr::Unary {
                    op: crate::ast::UnaryOp::Typeof,
                    operand: Box::new(index(it, key)),
                    span: sp,
                }),
                rhs: Box::new(string("undefined")),
                span: sp,
            };
            let get_iterator = Expr::ComputedCall {
                callee: Box::new(Expr::Index {
                    object: Box::new(operand),
                    index: Box::new(Expr::Field {
                        object: Box::new(Expr::Ident(crate::ast::synthetic_ident("Symbol"))),
                        field: crate::ast::synthetic_ident("iterator"),
                        span: sp,
                    }),
                    span: sp,
                }),
                args: Vec::new(),
                span: sp,
            };
            let prelude = vec![
                Stmt::Let { name: it, ty: None, init: get_iterator, span: sp },
                Stmt::Let { name: result, ty: None, init: step(Vec::new()), span: sp },
                Stmt::While {
                    cond: Expr::Unary {
                        op: crate::ast::UnaryOp::Not,
                        operand: Box::new(index(result, "done")),
                        span: sp,
                    },
                    body: Block {
                        statements: vec![
                            // The sent value, a thrown exception, or a return request on the
                            // outer generator is forwarded to the delegate.
                            Stmt::Let { name: ok, ty: None, init: Expr::Bool { value: false, span: sp }, span: sp },
                            Stmt::Let { name: threw, ty: None, init: Expr::Bool { value: false, span: sp }, span: sp },
                            Stmt::Let { name: sent, ty: None, init: Expr::Bool { value: false, span: sp }, span: sp },
                            Stmt::Try {
                                body: Block {
                                    statements: vec![
                                        Stmt::Expr(Expr::Assign {
                                            target: sent,
                                            value: Box::new(Expr::Yield {
                                                value: Box::new(index(result, "value")),
                                                span: sp,
                                            }),
                                            span: sp,
                                        }),
                                        Stmt::Expr(Expr::Assign {
                                            target: ok,
                                            value: Box::new(Expr::Bool { value: true, span: sp }),
                                            span: sp,
                                        }),
                                    ],
                                    span: sp,
                                },
                                catches: vec![crate::ast::CatchClause {
                                    binding: caught,
                                    ty: crate::ast::TypeAnnotation {
                                        name: Name { span: Span::new(0, 0) },
                                        is_array: false,
                                    },
                                    body: Block {
                                        statements: vec![
                                            Stmt::Expr(Expr::Assign {
                                                target: threw,
                                                value: Box::new(Expr::Bool { value: true, span: sp }),
                                                span: sp,
                                            }),
                                            Stmt::Expr(Expr::Assign {
                                                target: sent,
                                                value: Box::new(Expr::Ident(caught)),
                                                span: sp,
                                            }),
                                        ],
                                        span: sp,
                                    },
                                }],
                                finally: Some(Block {
                                    statements: vec![Stmt::If {
                                        cond: Expr::Unary {
                                            op: crate::ast::UnaryOp::Not,
                                            operand: Box::new(Expr::Ident(ok)),
                                            span: sp,
                                        },
                                        then_block: Block {
                                            statements: vec![Stmt::If {
                                                cond: Expr::Unary {
                                                    op: crate::ast::UnaryOp::Not,
                                                    operand: Box::new(Expr::Ident(threw)),
                                                    span: sp,
                                                },
                                                then_block: Block {
                                                    statements: vec![Stmt::If {
                                                        cond: defined("return"),
                                                        then_block: Block {
                                                            statements: vec![
                                                                // Forward the return request, validate the
                                                                // answer, and finish when the delegate did.
                                                                Stmt::Let {
                                                                    name: ret_answer,
                                                                    ty: None,
                                                                    init: Expr::ComputedCall {
                                                                        callee: Box::new(index(it, "return")),
                                                                        args: vec![Expr::Call {
                                                                            callee: crate::ast::synthetic_ident("__generatorReturnValue"),
                                                                            args: Vec::new(),
                                                                            span: sp,
                                                                        }],
                                                                        span: sp,
                                                                    },
                                                                    span: sp,
                                                                },
                                                                Stmt::If {
                                                                    cond: Expr::Binary {
                                                                        op: BinaryOp::Ne,
                                                                        lhs: Box::new(Expr::Unary {
                                                                            op: crate::ast::UnaryOp::Typeof,
                                                                            operand: Box::new(Expr::Ident(ret_answer)),
                                                                            span: sp,
                                                                        }),
                                                                        rhs: Box::new(string("object")),
                                                                        span: sp,
                                                                    },
                                                                    then_block: Block {
                                                                        statements: vec![Stmt::Throw {
                                                                            value: Expr::New {
                                                                                class: crate::ast::synthetic_ident("TypeError"),
                                                                                args: vec![string("iterator.return() did not return an object")],
                                                                                span: sp,
                                                                            },
                                                                            span: sp,
                                                                        }],
                                                                        span: sp,
                                                                    },
                                                                    else_block: None,
                                                                    span: sp,
                                                                },
                                                                Stmt::If {
                                                                    cond: index(ret_answer, "done"),
                                                                    then_block: Block {
                                                                        statements: vec![Stmt::Return {
                                                                            value: Some(index(ret_answer, "value")),
                                                                            span: sp,
                                                                        }],
                                                                        span: sp,
                                                                    },
                                                                    else_block: None,
                                                                    span: sp,
                                                                },
                                                            ],
                                                            span: sp,
                                                        },
                                                        else_block: None,
                                                        span: sp,
                                                    }],
                                                    span: sp,
                                                },
                                                else_block: None,
                                                span: sp,
                                            }],
                                            span: sp,
                                        },
                                        else_block: None,
                                        span: sp,
                                    }],
                                    span: sp,
                                }),
                                span: sp,
                            },
                            Stmt::If {
                                cond: Expr::Ident(ok),
                                then_block: Block {
                                    statements: vec![Stmt::Expr(Expr::Assign {
                                        target: result,
                                        value: Box::new(step(vec![Expr::Ident(sent)])),
                                        span: sp,
                                    })],
                                    span: sp,
                                },
                                else_block: Some(Block {
                                    statements: vec![
                                    // A delegate without `throw` is closed via `return`
                                    // (when present) before the call below raises a TypeError.
                                    Stmt::If {
                                        cond: Expr::Unary {
                                            op: crate::ast::UnaryOp::Not,
                                            operand: Box::new(defined("throw")),
                                            span: sp,
                                        },
                                        then_block: Block {
                                            statements: vec![Stmt::If {
                                                cond: defined("return"),
                                                then_block: Block {
                                                    statements: vec![Stmt::Expr(Expr::ComputedCall {
                                                        callee: Box::new(index(it, "return")),
                                                        args: Vec::new(),
                                                        span: sp,
                                                    })],
                                                    span: sp,
                                                },
                                                else_block: None,
                                                span: sp,
                                            }],
                                            span: sp,
                                        },
                                        else_block: None,
                                        span: sp,
                                    },
                                    Stmt::Expr(Expr::Assign {
                                        target: result,
                                        value: Box::new(Expr::ComputedCall {
                                            callee: Box::new(index(it, "throw")),
                                            args: vec![Expr::Ident(sent)],
                                            span: sp,
                                        }),
                                        span: sp,
                                    })],
                                    span: sp,
                                }),
                                span: sp,
                            },
                        ],
                        span: sp,
                    },
                    span: sp,
                },
            ];
            PENDING_PRELUDE.with(|pending| pending.borrow_mut().extend(prelude));
            Ok(index(result, "value"))
        }
        Expression::YieldExpression(yield_expression) => {
            if yield_expression.delegate {
                return Err("OXC adapter does not support delegated yield yet".into());
            }
            Ok(Expr::Yield {
                value: Box::new(match yield_expression.argument.as_ref() {
                    Some(argument) => convert_expression(source, argument)?,
                    None => Expr::Null {
                        span: span(yield_expression.span),
                    },
                }),
                span: span(yield_expression.span),
            })
        }
        Expression::AssignmentExpression(assignment) => {
            let compound = match assignment.operator {
                AssignmentOperator::Assign => None,
                AssignmentOperator::Addition => Some(BinaryOp::Add),
                AssignmentOperator::Subtraction => Some(BinaryOp::Sub),
                AssignmentOperator::Multiplication => Some(BinaryOp::Mul),
                AssignmentOperator::Division => Some(BinaryOp::Div),
                AssignmentOperator::Remainder => Some(BinaryOp::Rem),
                AssignmentOperator::Exponential => Some(BinaryOp::Pow),
                AssignmentOperator::BitwiseAnd => Some(BinaryOp::BitAnd),
                AssignmentOperator::BitwiseOR => Some(BinaryOp::BitOr),
                AssignmentOperator::BitwiseXOR => Some(BinaryOp::BitXor),
                AssignmentOperator::ShiftLeft => Some(BinaryOp::Shl),
                AssignmentOperator::ShiftRight => Some(BinaryOp::Shr),
                AssignmentOperator::ShiftRightZeroFill => Some(BinaryOp::UShr),
                // Logical assignments desugar to `a = a OP b`; the resulting
                // value matches the short-circuit form for every operand.
                AssignmentOperator::LogicalAnd => Some(BinaryOp::And),
                AssignmentOperator::LogicalOr => Some(BinaryOp::Or),
                AssignmentOperator::LogicalNullish => Some(BinaryOp::Nullish),
                _ => {
                    return Err("OXC adapter does not support compound assignment yet".into());
                }
            };
            if let Some(op) = compound {
                let whole = span(assignment.span);
                let logical = matches!(
                    assignment.operator,
                    AssignmentOperator::LogicalAnd
                        | AssignmentOperator::LogicalOr
                        | AssignmentOperator::LogicalNullish
                );
                let member = match &assignment.left {
                    AssignmentTarget::StaticMemberExpression(m) => Some(MemberRef::Static(m)),
                    AssignmentTarget::ComputedMemberExpression(m) => Some(MemberRef::Computed(m)),
                    AssignmentTarget::PrivateFieldExpression(m) => Some(MemberRef::Private(m)),
                    _ => None,
                };
                if let Some(member) = member {
                    // The target's object and key are evaluated exactly once;
                    // a logical assignment only stores when it does not
                    // short-circuit.
                    let (binds, read, store) = decompose_member(source, member)?;
                    let rhs = convert_expression(source, &assignment.right)?;
                    let body = if logical {
                        Expr::Binary {
                            op,
                            lhs: Box::new(read),
                            rhs: Box::new(store.build(rhs, whole)),
                            span: whole,
                        }
                    } else {
                        store.build(
                            Expr::Binary { op, lhs: Box::new(read), rhs: Box::new(rhs), span: whole },
                            whole,
                        )
                    };
                    return Ok(wrap_binds(binds, body, whole));
                }
                if logical {
                    if let AssignmentTarget::AssignmentTargetIdentifier(identifier) = &assignment.left {
                        let name = Name { span: span(identifier.span) };
                        let rhs = convert_expression(source, &assignment.right)?;
                        return Ok(Expr::Binary {
                            op,
                            lhs: Box::new(Expr::Ident(name)),
                            rhs: Box::new(Expr::Assign { target: name, value: Box::new(rhs), span: whole }),
                            span: whole,
                        });
                    }
                }
            }
            let mut value = convert_expression(source, &assignment.right)?;
            if let Some(op) = compound {
                // `a OP= b` desugars to `a = (a OP b)`. Known v1 limitation: a
                // target with side-effecting sub-expressions is evaluated twice.
                let read = match &assignment.left {
                    AssignmentTarget::AssignmentTargetIdentifier(identifier) => {
                        Expr::Ident(Name { span: span(identifier.span) })
                    }
                    AssignmentTarget::StaticMemberExpression(member) => Expr::Field {
                        object: Box::new(convert_expression(source, &member.object)?),
                        field: Name { span: span(member.property.span) },
                        span: span(member.span),
                    },
                    AssignmentTarget::ComputedMemberExpression(member) => Expr::Index {
                        object: Box::new(convert_expression(source, &member.object)?),
                        index: Box::new(convert_expression(source, &member.expression)?),
                        span: span(member.span),
                    },
                    AssignmentTarget::PrivateFieldExpression(member) => Expr::PrivateField {
                        object: Box::new(convert_expression(source, &member.object)?),
                        field: Name { span: span(member.field.span) },
                        span: span(member.span),
                    },
                    _ => {
                        return Err(
                            "OXC adapter supports identifier, field, and index assignments only"
                                .into(),
                        );
                    }
                };
                value = Expr::Binary {
                    op,
                    lhs: Box::new(read),
                    rhs: Box::new(value),
                    span: span(assignment.span),
                };
            }
            let value = Box::new(value);
            match &assignment.left {
                AssignmentTarget::AssignmentTargetIdentifier(identifier) => Ok(Expr::Assign {
                    target: Name {
                        span: span(identifier.span),
                    },
                    value,
                    span: span(assignment.span),
                }),
                AssignmentTarget::StaticMemberExpression(member) => Ok(Expr::FieldAssign {
                    object: Box::new(convert_expression(source, &member.object)?),
                    field: Name {
                        span: span(member.property.span),
                    },
                    value,
                    span: span(assignment.span),
                }),
                // `obj['name'] = v` with a plain identifier-shaped literal is `obj.name = v`.
                AssignmentTarget::ComputedMemberExpression(member)
                    if matches!(&member.expression, Expression::StringLiteral(text)
                        if text.value.as_str().is_some_and(is_plain_identifier)
                            && (text.span.end - text.span.start) as usize
                                == text.value.as_str().map_or(0, str::len) + 2) =>
                {
                    let Expression::StringLiteral(text) = &member.expression else { unreachable!() };
                    Ok(Expr::FieldAssign {
                        object: Box::new(convert_expression(source, &member.object)?),
                        field: Name { span: Span::new(text.span.start + 1, text.span.end - 1) },
                        value,
                        span: span(assignment.span),
                    })
                }
                AssignmentTarget::ComputedMemberExpression(member) => Ok(Expr::IndexAssign {
                    object: Box::new(convert_expression(source, &member.object)?),
                    index: Box::new(convert_expression(source, &member.expression)?),
                    value,
                    span: span(assignment.span),
                }),
                // Sema lowers a private store from `FieldAssign` over a
                // `PrivateField` target (the shape the hand parser emits).
                AssignmentTarget::PrivateFieldExpression(member) => Ok(Expr::FieldAssign {
                    object: Box::new(Expr::PrivateField {
                        object: Box::new(convert_expression(source, &member.object)?),
                        field: Name {
                            span: span(member.field.span),
                        },
                        span: span(member.span),
                    }),
                    field: Name {
                        span: span(member.field.span),
                    },
                    value,
                    span: span(assignment.span),
                }),
                _ => {
                    Err("OXC adapter supports identifier, field, and index assignments only".into())
                }
            }
        }
        Expression::CallExpression(call) => {
            let mut args = Vec::with_capacity(call.arguments.len());
            for argument in &call.arguments {
                args.push(convert_argument(source, argument)?);
            }
            // Strip parentheses and `as` casts around the callee so
            // `(Math.sqrt as any)(x)` is the plain call `Math.sqrt(x)`.
            let mut callee_expression = &call.callee;
            loop {
                match callee_expression {
                    Expression::ParenthesizedExpression(inner) => {
                        callee_expression = &inner.expression
                    }
                    Expression::TSAsExpression(inner) => callee_expression = &inner.expression,
                    _ => break,
                }
            }
            match callee_expression {
                // `super(args)`: the callee name is the source text `super`.
                Expression::Super(super_expression) => Ok(Expr::Call {
                    callee: Name {
                        span: span(super_expression.span),
                    },
                    args,
                    span: span(call.span),
                }),
                Expression::Identifier(identifier) => Ok(Expr::Call {
                    callee: Name {
                        span: span(identifier.span),
                    },
                    args,
                    span: span(call.span),
                }),
                // `Intl.fn(args)`: a dotted callee name the checker maps onto a
                // prelude function (see `Checker::text`).
                Expression::StaticMemberExpression(member)
                    if !member.optional
                        && matches!(&member.object, Expression::Identifier(id) if id.name == "Intl") =>
                {
                    Ok(Expr::Call {
                        callee: Name {
                            span: span(member.span),
                        },
                        args,
                        span: span(call.span),
                    })
                }
                // `this.#m(args)`: a private method is an ordinary vtable
                // method whose name keeps the `#` (so it cannot collide with a
                // public member).
                Expression::PrivateFieldExpression(member) if !member.optional => {
                    Ok(Expr::MethodCall {
                        receiver: Box::new(convert_expression(source, &member.object)?),
                        method: Name {
                            span: span(member.field.span),
                        },
                        args,
                        span: span(call.span),
                    })
                }
                Expression::StaticMemberExpression(member) => {
                    // `String.prototype.m.call(thisArg, ...rest)` becomes
                    // `__thisString(thisArg).m(...rest)` (the checker maps the
                    // `String.prototype` callee text onto the prelude helper).
                    if !member.optional && member.property.name == "call" {
                        if let Expression::StaticMemberExpression(method_member) = &member.object {
                            if let Expression::StaticMemberExpression(proto) = &method_member.object {
                                if !method_member.optional
                                    && !proto.optional
                                    && proto.property.name == "prototype"
                                    && matches!(&proto.object, Expression::Identifier(id) if id.name == "String")
                                    && !args.is_empty()
                                    && !matches!(args[0], Expr::Spread { .. })
                                {
                                    let mut rest = args;
                                    let this_arg = rest.remove(0);
                                    return Ok(Expr::MethodCall {
                                        receiver: Box::new(Expr::Call {
                                            callee: Name { span: span(proto.span) },
                                            args: vec![this_arg],
                                            span: span(proto.span),
                                        }),
                                        method: Name { span: span(method_member.property.span) },
                                        args: rest,
                                        span: span(call.span),
                                    });
                                }
                            }
                        }
                    }
                    let receiver = Box::new(convert_expression(source, &member.object)?);
                    let method = Name {
                        span: span(member.property.span),
                    };
                    if member.optional {
                        Ok(Expr::OptionalMethodCall {
                            object: receiver,
                            method,
                            args,
                            span: span(call.span),
                        })
                    } else {
                        Ok(Expr::MethodCall {
                            receiver,
                            method,
                            args,
                            span: span(call.span),
                        })
                    }
                }
                // `obj['name'](args)` with a plain identifier-shaped literal key is
                // the method call `obj.name(args)` (the name span skips the quotes).
                Expression::ComputedMemberExpression(member)
                    if !member.optional
                        && matches!(&member.expression, Expression::StringLiteral(text)
                            if text.value.as_str().is_some_and(is_plain_identifier)
                                && (text.span.end - text.span.start) as usize == text.value.as_str().map_or(0, str::len) + 2) =>
                {
                    let Expression::StringLiteral(text) = &member.expression else { unreachable!() };
                    Ok(Expr::MethodCall {
                        receiver: Box::new(convert_expression(source, &member.object)?),
                        method: Name { span: Span::new(text.span.start + 1, text.span.end - 1) },
                        args,
                        span: span(call.span),
                    })
                }
                // Any other callee is an arbitrary function-valued expression
                // (`a[b]()`, `f()()`, `(cond ? f : g)(…)`): call it indirectly.
                other => Ok(Expr::ComputedCall {
                    callee: Box::new(convert_expression(source, other)?),
                    args,
                    span: span(call.span),
                }),
            }
        }
        Expression::ObjectExpression(object) => {
            let mut properties = Vec::with_capacity(object.properties.len());
            for property in &object.properties {
                let property = match property {
                    ObjectPropertyKind::ObjectProperty(property) => property,
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        properties.push(crate::ast::ObjectMember::Spread {
                            expr: convert_expression(source, &spread.argument)?,
                            span: span(spread.span),
                        });
                        continue;
                    }
                };
                // Method shorthand and accessors carry a function value; a plain
                // data property is `PropertyKind::Init` and not a method.
                let member_kind = if property.method {
                    crate::ast::PropertyKind::Method
                } else {
                    match property.kind {
                        PropertyKind::Init => crate::ast::PropertyKind::Init,
                        PropertyKind::Get => crate::ast::PropertyKind::Get,
                        PropertyKind::Set => crate::ast::PropertyKind::Set,
                    }
                };
                let key = if property.computed {
                    let Some(key_expression) = property.key.as_expression() else {
                        return Err("OXC adapter requires an expression computed key".into());
                    };
                    crate::ast::PropertyKey::Computed(Box::new(convert_expression(source, key_expression)?))
                } else {
                    match &property.key {
                        PropertyKey::StaticIdentifier(identifier) => {
                            crate::ast::PropertyKey::Ident(Name {
                                span: span(identifier.span),
                            })
                        }
                        PropertyKey::StringLiteral(string) => {
                            let Some(value) = string.value.as_str() else {
                                return Err(
                                    "OXC adapter cannot represent non-UTF-8 string keys".into(),
                                );
                            };
                            crate::ast::PropertyKey::String {
                                value: value.to_owned(),
                                span: span(string.span),
                            }
                        }
                        PropertyKey::NumericLiteral(number) => crate::ast::PropertyKey::Number {
                            value: number.value,
                            span: span(number.span),
                        },
                        _ => {
                            return Err(
                                "OXC adapter supports identifier, string, and numeric property keys only"
                                    .into(),
                            )
                        }
                    }
                };
                let value = match member_kind {
                    crate::ast::PropertyKind::Init => convert_expression(source, &property.value)?,
                    // Method / getter / setter: the value is a function.
                    _ => {
                        let Expression::FunctionExpression(function) = &property.value else {
                            return Err(
                                "OXC adapter expected a function value for an object method or accessor"
                                    .into(),
                            );
                        };
                        convert_oxc_function(source, function)?
                    }
                };
                properties.push(crate::ast::ObjectMember::Property(crate::ast::ObjectProperty {
                    key,
                    value,
                    kind: member_kind,
                }));
            }
            Ok(Expr::ObjectLiteral {
                properties,
                span: span(object.span),
            })
        }
        Expression::ArrayExpression(array) => {
            let mut elements = Vec::with_capacity(array.elements.len());
            for element in &array.elements {
                match element {
                    ArrayExpressionElement::Elision(_) => {
                        elements.push(crate::ast::ArrayElement::Hole);
                    }
                    ArrayExpressionElement::SpreadElement(spread) => {
                        elements.push(crate::ast::ArrayElement::Spread(
                            convert_expression(source, &spread.argument)?,
                        ));
                    }
                    other => {
                        let Some(expression) = other.as_expression() else {
                            return Err(
                                "OXC adapter expected an array element expression".into()
                            );
                        };
                        elements.push(crate::ast::ArrayElement::Expr(
                            convert_expression(source, expression)?,
                        ));
                    }
                }
            }
            Ok(Expr::ArrayLiteral {
                elements,
                span: span(array.span),
            })
        }
        Expression::NewExpression(new_expression) => {
            let mut args = Vec::with_capacity(new_expression.arguments.len());
            for argument in &new_expression.arguments {
                args.push(convert_argument(source, argument)?);
            }
            // A bare identifier constructor resolves a named class; any other
            // callee (`new ns.Ctor()`, `new (expr)()`) becomes a dynamic new on
            // the evaluated callee expression.
            match &new_expression.callee {
                Expression::Identifier(identifier) => Ok(Expr::New {
                    class: Name {
                        span: span(identifier.span),
                    },
                    args,
                    span: span(new_expression.span),
                }),
                // `new Intl.X(...)`: a dotted class name the checker maps onto
                // the prelude `IntlX` classes (see `Checker::text`).
                Expression::StaticMemberExpression(member)
                    if matches!(&member.object, Expression::Identifier(id) if id.name == "Intl") =>
                {
                    Ok(Expr::New {
                        class: Name {
                            span: span(member.span),
                        },
                        args,
                        span: span(new_expression.span),
                    })
                }
                callee => Ok(Expr::NewDynamic {
                    callee: Box::new(convert_expression(source, callee)?),
                    args,
                    span: span(new_expression.span),
                }),
            }
        }
        Expression::ArrowFunctionExpression(arrow) => convert_arrow(source, arrow),
        Expression::FunctionExpression(function) => convert_oxc_function(source, function),
        // A class expression is lifted to a hoisted class declaration (Zun
        // classes are program-global) and the expression becomes a reference
        // to it, which sema reads as the class constructor descriptor. An
        // anonymous class takes the name of the binding it initializes (set by
        // the declarator converter), else the `class` keyword span.
        Expression::ClassExpression(class_expression) => {
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
            let mut class = convert_class_declaration(source, class_expression)
                .map_err(|errors| errors.join("; "))?;
            class.name = Name { span: name_span };
            HOISTED_CLASSES.with(|cells| cells.borrow_mut().push(class));
            Ok(Expr::Ident(Name { span: name_span }))
        }
        Expression::UpdateExpression(update) => convert_update(source, update, true),
        other => {
            let dbg = format!("{other:?}");
            let kind = dbg.split(|c: char| !c.is_alphanumeric()).find(|s| !s.is_empty()).unwrap_or("?");
            Err(format!("OXC adapter: unsupported expression `{kind}`"))
        }
    }
}

/// Desugar `x++`/`++x`/`x--`/`--x` to `x = x ± 1` (the language has no update
/// operator). Correct for prefix form and for statement/for-update position
/// (the value is unused); a postfix result used as a value would see the new
/// value instead of the old — a known v1 limitation.
/// Convert an expression evaluated only for its effect (an expression
/// statement or a `for` update clause): a postfix update there needs no old
/// value.
fn convert_effect_expression(
    source: &str,
    expression: &Expression<'_>,
) -> Result<Expr, String> {
    match expression {
        Expression::UpdateExpression(update) => convert_update(source, update, false),
        other => convert_expression(source, other),
    }
}

/// Convert `x++`/`x--`. In value position a postfix form yields the old value,
/// computed as `(x = x ± 1) ∓ 1`.
fn convert_update(
    source: &str,
    update: &oxc_ast::ast::UpdateExpression<'_>,
    value_used: bool,
) -> Result<Expr, String> {
    let updated = convert_update_assign(source, update)?;
    if value_used && !update.prefix {
        let whole = Span::new(update.span.start, update.span.end);
        let undo = match update.operator {
            oxc_ast::ast::UpdateOperator::Increment => BinaryOp::Sub,
            oxc_ast::ast::UpdateOperator::Decrement => BinaryOp::Add,
        };
        return Ok(Expr::Binary {
            op: undo,
            lhs: Box::new(updated),
            rhs: Box::new(Expr::Number { value: 1.0, span: whole }),
            span: whole,
        });
    }
    Ok(updated)
}

fn convert_update_assign(
    source: &str,
    update: &oxc_ast::ast::UpdateExpression<'_>,
) -> Result<Expr, String> {
    use oxc_ast::ast::SimpleAssignmentTarget;
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let op = match update.operator {
        oxc_ast::ast::UpdateOperator::Increment => BinaryOp::Add,
        oxc_ast::ast::UpdateOperator::Decrement => BinaryOp::Sub,
    };
    let whole = span(update.span);
    let one = || Expr::Number { value: 1.0, span: whole };
    let step = |read: Expr| Expr::Binary { op, lhs: Box::new(read), rhs: Box::new(one()), span: whole };
    let member = match &update.argument {
        SimpleAssignmentTarget::StaticMemberExpression(m) => Some(MemberRef::Static(m)),
        SimpleAssignmentTarget::ComputedMemberExpression(m) => Some(MemberRef::Computed(m)),
        SimpleAssignmentTarget::PrivateFieldExpression(m) => Some(MemberRef::Private(m)),
        _ => None,
    };
    if let Some(member) = member {
        let (binds, read, store) = decompose_member(source, member)?;
        return Ok(wrap_binds(binds, store.build(step(read), whole), whole));
    }
    match &update.argument {
        SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier) => {
            let target = Name { span: span(identifier.span) };
            Ok(Expr::Assign {
                target,
                value: Box::new(step(Expr::Ident(Name { span: span(identifier.span) }))),
                span: whole,
            })
        }
        SimpleAssignmentTarget::StaticMemberExpression(member) => {
            let object = convert_expression(source, &member.object)?;
            let read = Expr::Field {
                object: Box::new(convert_expression(source, &member.object)?),
                field: Name { span: span(member.property.span) },
                span: span(member.span),
            };
            Ok(Expr::FieldAssign {
                object: Box::new(object),
                field: Name { span: span(member.property.span) },
                value: Box::new(step(read)),
                span: whole,
            })
        }
        SimpleAssignmentTarget::ComputedMemberExpression(member) => {
            let read = Expr::Index {
                object: Box::new(convert_expression(source, &member.object)?),
                index: Box::new(convert_expression(source, &member.expression)?),
                span: span(member.span),
            };
            Ok(Expr::IndexAssign {
                object: Box::new(convert_expression(source, &member.object)?),
                index: Box::new(convert_expression(source, &member.expression)?),
                value: Box::new(step(read)),
                span: whole,
            })
        }
        SimpleAssignmentTarget::PrivateFieldExpression(member) => {
            let private = |source: &str| -> Result<Expr, String> {
                Ok(Expr::PrivateField {
                    object: Box::new(convert_expression(source, &member.object)?),
                    field: Name { span: span(member.field.span) },
                    span: span(member.span),
                })
            };
            let read = private(source)?;
            Ok(Expr::FieldAssign {
                object: Box::new(private(source)?),
                field: Name { span: span(member.field.span) },
                value: Box::new(step(read)),
                span: whole,
            })
        }
        _ => Err("OXC adapter supports identifier, field, and index update targets only".into()),
    }
}

/// Convert an arrow function (`(a) => expr` or `(a) => { … }`) to a
/// `FunctionExpr` flagged as an arrow. A concise expression body becomes a
/// single `return`.
fn convert_arrow(
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
            name: Name { span: span(identifier.span) },
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
        Block { statements, span: span(arrow.span) }
    } else if let oxc_ast::ast::ArrowFunctionBody::FunctionBody(function_body) = &arrow.body {
        let mut statements = prologue;
        statements.extend(convert_function_statements(source, &function_body.statements, &param_names(&arrow.params))?);
        Block { statements, span: span(function_body.span) }
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

/// Parse standard JavaScript or TypeScript with OXC.
///
/// The returned diagnostics are formatted at the boundary so callers do not
/// need to retain OXC's arena or AST lifetimes. An empty vector means parsing
/// succeeded; semantic analysis remains a separate compiler stage.
pub fn diagnostics(source: &str, typescript: bool) -> Vec<String> {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs().with_typescript(typescript);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    parsed
        .diagnostics
        .into_iter()
        .map(|diagnostic| format!("{diagnostic:?}"))
        .collect()
}

/// Return whether OXC accepts the source as standard JS/TS syntax.
pub fn parses(source: &str, typescript: bool) -> bool {
    diagnostics(source, typescript).is_empty()
}

/// Convert the first OXC-supported subset into the existing Zun AST.
///
/// Unsupported constructs return a diagnostic instead of being silently
/// approximated; additional statement and expression adapters can be added
/// without changing the public frontend boundary.
pub fn parse_subset(source: &str, typescript: bool) -> Result<Program, Vec<String>> {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs().with_typescript(typescript);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(parsed
            .diagnostics
            .into_iter()
            .map(|diagnostic| format!("{diagnostic:?}"))
            .collect());
    }

    // Reset the hoisted-class collector for this parse; the statement converter
    // pushes block-local / nested class declarations here while converting the
    // module body, and they are drained into `classes` before the Program is
    // built.
    HOISTED_CLASSES.with(|cells| cells.borrow_mut().clear());

    let mut functions = Vec::new();
    let mut imports = Vec::new();
    let mut classes = Vec::new();
    let mut top_level = Vec::new();
    let mut hoisted_functions: Vec<Stmt> = Vec::new();
    TOP_LEVEL_NAMES.with(|names| {
        let mut names = names.borrow_mut();
        names.clear();
        for statement in parsed.program.body.iter() {
            if let Statement::VariableDeclaration(decl) = statement {
                for d in decl.declarations.iter() {
                    if let BindingPattern::BindingIdentifier(id) = &d.id {
                        names.insert(id.name.to_string());
                    }
                }
            }
        }
    });
    for statement in parsed.program.body {
        TOP_LEVEL_BLOCK.with(|cell| {
            cell.set(match &statement {
                Statement::BlockStatement(block) => block.span.start,
                _ => u32::MAX,
            })
        });
        // TS type-only declarations erase: a type alias or interface carries no
        // runtime value, so it is dropped (annotations that name it resolve to
        // `any` on this path, like the native parser's erasure).
        if matches!(
            statement,
            Statement::TSTypeAliasDeclaration(_) | Statement::TSInterfaceDeclaration(_)
        ) {
            continue;
        }
        if let Statement::ImportDeclaration(import_declaration) = statement {
            let Some(module) = import_declaration.source.value.as_str() else {
                return Err(vec![
                    "OXC adapter cannot represent non-UTF-8 import sources".into(),
                ]);
            };
            let mut names = Vec::new();
            if let Some(specifiers) = import_declaration.specifiers.as_deref() {
                for specifier in specifiers {
                    match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                            let ModuleExportName::IdentifierName(imported) = &specifier.imported
                            else {
                                return Err(vec![
                                    "OXC adapter supports identifier named imports only".into(),
                                ]);
                            };
                            names.push(crate::ast::ImportName {
                                imported: Name {
                                    span: Span::new(imported.span.start, imported.span.end),
                                },
                                local: Name {
                                    span: Span::new(
                                        specifier.local.span.start,
                                        specifier.local.span.end,
                                    ),
                                },
                                is_default: false,
                                is_namespace: false,
                            });
                        }
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                            names.push(crate::ast::ImportName {
                                imported: Name {
                                    span: Span::new(
                                        specifier.local.span.start,
                                        specifier.local.span.end,
                                    ),
                                },
                                local: Name {
                                    span: Span::new(
                                        specifier.local.span.start,
                                        specifier.local.span.end,
                                    ),
                                },
                                is_default: true,
                                is_namespace: false,
                            });
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                            names.push(crate::ast::ImportName {
                                imported: Name {
                                    span: Span::new(specifier.span.start, specifier.span.start),
                                },
                                local: Name {
                                    span: Span::new(
                                        specifier.local.span.start,
                                        specifier.local.span.end,
                                    ),
                                },
                                is_default: false,
                                is_namespace: true,
                            });
                        }
                    }
                }
            }
            imports.push(crate::ast::Import {
                names,
                module: module.to_owned(),
                span: Span::new(import_declaration.span.start, import_declaration.span.end),
            });
            continue;
        }
        if let Statement::ExportNamedDeclaration(export) = &statement {
            for specifier in &export.specifiers {
                let local = specifier.local.span();
                let exported = specifier.exported.span();
                if source.get(local.start as usize..local.end as usize)
                    != source.get(exported.start as usize..exported.end as usize)
                {
                    return Err(vec![
                        "OXC adapter supports unchanged local export names only".into(),
                    ]);
                }
            }
            continue;
        }
        let class_declaration = match &statement {
            Statement::ClassDeclaration(class_declaration) => Some(class_declaration),
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::ClassDeclaration(class_declaration) => Some(class_declaration),
                _ => None,
            },
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::ClassDeclaration(class_declaration) => {
                    Some(class_declaration)
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(class_declaration) = class_declaration {
            classes.push(convert_class_declaration(source, class_declaration)?);
            continue;
        }
        let function = match &statement {
            Statement::FunctionDeclaration(function) => Some(function),
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::FunctionDeclaration(function) => Some(function),
                _ => None,
            },
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(function) => Some(function),
                _ => None,
            },
            _ => None,
        };
        // A top-level function declaration that the program reassigns
        // (`f = function () {}`, the string-array obfuscator idiom) is a mutable
        // binding: lower it to a `let` hoisted to the head of the module body.
        if let Some(function) = function {
            if let Some(id) = function.id.as_ref() {
                let name = &source[id.span.start as usize..id.span.end as usize];
                if !function.generator
                    && !function.r#async
                    && name_is_reassigned(&source[function.span.start as usize..function.span.end as usize], name) {
                    hoisted_functions.extend(
                        convert_statement_list(source, std::slice::from_ref(&statement))
                            .map_err(|error| vec![error])?,
                    );
                    continue;
                }
            }
        }
        let Some(function) = function else {
            // Any other top-level node is an executable module statement (a JS/TS
            // module body): convert it and collect it to run as the program.
            top_level.extend(
                convert_statement_list(source, std::slice::from_ref(&statement))
                    .map_err(|error| vec![error])?,
            );
            continue;
        };
        let Some(id) = function.id.as_ref() else {
            return Err(vec!["OXC adapter requires named functions".into()]);
        };
        let Some(body) = function.body.as_ref() else {
            return Err(vec!["OXC adapter requires function bodies".into()]);
        };
        let span = |span: oxc_span::Span| Span::new(span.start, span.end);
        let function_span = span(function.span);
        let mut params = Vec::with_capacity(function.params.items.len());
        let mut prologue = Vec::new();
        for parameter in &function.params.items {
            params.push(
                convert_formal_param(source, parameter, &mut prologue)
                    .map_err(|error| vec![error])?,
            );
        }
        // The function body converts through the single shared statement
        // converter (switch, labelled statements, loops, etc.), after any
        // destructured-parameter prologue.
        let mut statements = prologue;
        statements.extend(
            convert_function_statements(source, &body.statements, &param_names(&function.params)).map_err(|error| vec![error])?,
        );
        if let Some(rest) = function.params.rest.as_ref() {
            let BindingPattern::BindingIdentifier(identifier) = &rest.rest.argument else {
                return Err(vec![
                    "OXC adapter supports identifier rest parameters only".into()
                ]);
            };
            params.push(crate::ast::Param {
                name: Name {
                    span: span(identifier.span),
                },
                ty: convert_type_annotation(source, rest.type_annotation.as_deref())
                    .map_err(|error| vec![error])?,
                default_value: None,
                is_rest: true,
            });
        }
        inject_arguments_object(source, function_span, &mut params, &mut statements);
        functions.push(Function {
            name: Name {
                span: span(id.span),
            },
            // OXC adapter erases generic type parameters (see above).
            type_params: Vec::new(),
            decorators: Vec::new(),
            params,
            return_type: convert_type_annotation(source, function.return_type.as_deref())
                .map_err(|error| vec![error])?,
            is_extern: false,
            is_static: false,
            accessor: None,
            external_module: None,
            is_async: function.r#async,
            is_generator: function.generator,
            body: Block {
                statements,
                span: span(body.span),
            },
            span: function_span,
        });
    }
    // Drain the block-local / nested classes collected while converting the
    // module body (and function/method bodies) into the program's class list.
    HOISTED_CLASSES.with(|cells| classes.append(&mut cells.borrow_mut()));
    hoisted_functions.append(&mut top_level);
    let top_level = hoisted_functions;
    Ok(Program {
        functions,
        classes,
        imports,
        // The OXC adapter does not surface type aliases/interfaces yet; they
        // erase, so this only limits alias resolution on that secondary path.
        type_aliases: Vec::new(),
        // Likewise, enum declarations are not surfaced on the OXC path yet.
        enums: Vec::new(),
        top_level,
    })
}

#[cfg(test)]
mod tests {
    use super::{diagnostics, parse_subset, parses};
    use zun_common::Span;

    #[test]
    fn parses_standard_typescript_without_using_zun_ast() {
        assert!(parses("const answer: number = 42;", true));
    }

    #[test]
    fn reports_standard_typescript_syntax_errors() {
        let errors = diagnostics("const answer: = 42;", true);
        assert!(!errors.is_empty());
    }

    #[test]
    fn converts_a_simple_oxc_function_into_the_zun_ast() {
        let program = parse_subset("function main() { return 42; }", true).unwrap();
        assert_eq!(program.functions.len(), 1);
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Number { value, .. }),
                ..
            } if value == 42.0
        ));
    }

    #[test]
    fn converts_string_and_boolean_literals_into_the_zun_ast() {
        for (source, expected) in [
            ("function text() { return 'zun'; }", "zun"),
            ("function flag() { return true; }", "true"),
        ] {
            let program = parse_subset(source, true).unwrap();
            let crate::ast::Stmt::Return {
                value: Some(value), ..
            } = &program.functions[0].body.statements[0]
            else {
                panic!("expected a return statement");
            };
            match (value, expected) {
                (crate::ast::Expr::String { value, .. }, "zun") => assert_eq!(value, "zun"),
                (crate::ast::Expr::Bool { value, .. }, "true") => assert!(*value),
                _ => panic!("unexpected converted literal"),
            }
        }
    }

    #[test]
    fn converts_class_and_member_decorators() {
        let program = parse_subset(
            "@foo @bar(1) class C { @log x: number; @ns.dec m() { return 0; } }",
            true,
        )
        .unwrap();
        let class = &program.classes[0];
        assert_eq!(class.decorators.len(), 2);
        // `@foo` is a bare identifier form; `@bar(1)` is the call form.
        assert!(class.decorators[0].args.is_none());
        let bar_args = class.decorators[1].args.as_ref().expect("call form args");
        assert_eq!(bar_args.len(), 1);
        // Field and method decorators are carried through too.
        assert_eq!(class.fields[0].decorators.len(), 1);
        assert_eq!(class.methods[0].decorators.len(), 1);
        assert!(matches!(
            class.methods[0].decorators[0].callee,
            crate::ast::Expr::Field { .. }
        ));
    }

    #[test]
    fn converts_identifier_parameters_and_returns() {
        let program = parse_subset("function identity(value) { return value; }", false).unwrap();
        assert_eq!(program.functions[0].params.len(), 1);
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Ident(_)),
                ..
            }
        ));
    }

    #[test]
    fn converts_binary_return_expressions() {
        let program = parse_subset("function add(value) { return value + 2; }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Binary {
                    op: crate::ast::BinaryOp::Add,
                    ..
                }),
                ..
            }
        ));
    }

    #[test]
    fn converts_bigint_literals() {
        let program =
            parse_subset("function main() { return 12345678901234567890n; }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::BigInt { ref value, .. }),
                ..
            } if value == "12345678901234567890"
        ));
    }

    #[test]
    fn accepts_empty_statements_in_functions_and_branches() {
        let program = parse_subset(
            "function main(value) { ; if (value) ; return value; }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements.first(),
            Some(crate::ast::Stmt::If { then_block, .. })
                if then_block.statements.is_empty()
        ));
    }

    #[test]
    fn accepts_debugger_statements_as_no_ops() {
        let program = parse_subset(
            "function main(value) { debugger; if (value) debugger; }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements.first(),
            Some(crate::ast::Stmt::If { then_block, .. })
                if then_block.statements.is_empty()
        ));
    }

    #[test]
    fn converts_try_catch_finally_statements() {
        let program = parse_subset(
            "function main(value) { try { throw value; } catch (error) { return error; } finally { debugger; } }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements.first(),
            Some(crate::ast::Stmt::Try { catches, finally: Some(_), .. })
                if catches.len() == 1 && catches[0].ty.name.span == Span::new(0, 0)
        ));
    }

    #[test]
    fn converts_logical_and_unary_return_expressions() {
        let program =
            parse_subset("function choose(value) { return !value && true; }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Binary {
                    op: crate::ast::BinaryOp::And,
                    ref lhs,
                    ..
                }),
                ..
            } if matches!(**lhs, crate::ast::Expr::Unary { op: crate::ast::UnaryOp::Not, .. })
        ));
    }

    #[test]
    fn converts_identifier_calls_and_arguments() {
        let program = parse_subset("function main() { return add(20, 22); }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Call { ref callee, ref args, .. }),
                ..
            } if callee.span.start < callee.span.end && args.len() == 2
        ));
    }

    #[test]
    fn converts_multiple_supported_body_statements() {
        let program = parse_subset("function main() { add(20, 22); return 42; }", false).unwrap();
        assert_eq!(program.functions[0].body.statements.len(), 2);
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Expr(crate::ast::Expr::Call { .. })
        ));
    }

    #[test]
    fn converts_initialized_local_declarations() {
        let program =
            parse_subset("function main() { let value = 42; return value; }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Let { .. }
        ));
    }

    #[test]
    fn preserves_typed_local_declarations() {
        let program = parse_subset(
            "function main() { let value: int = 42; return value; }",
            true,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Let { ref ty, .. } if ty.as_ref().is_some_and(|ty| ty.name.span.start < ty.name.span.end)
        ));
    }

    #[test]
    fn converts_static_member_access_and_method_calls() {
        let program = parse_subset("function main(value) { return value.size(); }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::MethodCall { .. }),
                ..
            }
        ));
        let field = parse_subset("function read(value) { return value.size; }", false).unwrap();
        assert!(matches!(
            field.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Field { .. }),
                ..
            }
        ));
    }

    #[test]
    fn converts_if_statements_with_block_branches() {
        let program = parse_subset(
            "function choose(value) { if (value) { return 1; } else { return 2; } }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::If {
                else_block: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn converts_local_bindings_inside_block_branches() {
        let program = parse_subset(
            "function main(value) { if (value) { let result: int = 42; return result; } return 0; }",
            true,
        )
        .unwrap();
        let crate::ast::Stmt::If { ref then_block, .. } = program.functions[0].body.statements[0]
        else {
            panic!("expected an if statement");
        };
        assert!(matches!(
            then_block.statements.first(),
            Some(crate::ast::Stmt::Let { ty: Some(_), .. })
        ));
    }

    #[test]
    fn converts_else_if_branches() {
        let program = parse_subset(
            "function main(value) { if (value) { return 1; } else if (value + 1) { return 2; } return 0; }",
            true,
        )
        .unwrap();
        let crate::ast::Stmt::If { ref else_block, .. } = program.functions[0].body.statements[0]
        else {
            panic!("expected an if statement");
        };
        assert!(matches!(
            else_block
                .as_ref()
                .and_then(|block| block.statements.first()),
            Some(crate::ast::Stmt::If { .. })
        ));
    }

    #[test]
    fn converts_single_statement_conditional_branches() {
        let program = parse_subset(
            "function main(value) { if (value) return 1; else return 0; }",
            true,
        )
        .unwrap();
        let crate::ast::Stmt::If {
            ref then_block,
            ref else_block,
            ..
        } = program.functions[0].body.statements[0]
        else {
            panic!("expected an if statement");
        };
        assert!(matches!(
            then_block.statements.first(),
            Some(crate::ast::Stmt::Return { .. })
        ));
        assert!(matches!(
            else_block
                .as_ref()
                .and_then(|block| block.statements.first()),
            Some(crate::ast::Stmt::Return { .. })
        ));
    }

    #[test]
    fn converts_single_statement_local_branch() {
        let program = parse_subset(
            "function main(value) { if (value) var result: int = 42; }",
            true,
        )
        .unwrap();
        let crate::ast::Stmt::If { ref then_block, .. } = program.functions[0].body.statements[0]
        else {
            panic!("expected an if statement");
        };
        assert!(matches!(
            then_block.statements.first(),
            Some(crate::ast::Stmt::Let { ty: Some(_), .. })
        ));
    }

    #[test]
    fn converts_single_statement_while_branch() {
        let program = parse_subset(
            "function main(value) { if (value) while (value) break; }",
            true,
        )
        .unwrap();
        let crate::ast::Stmt::If { ref then_block, .. } = program.functions[0].body.statements[0]
        else {
            panic!("expected an if statement");
        };
        assert!(matches!(
            then_block.statements.first(),
            Some(crate::ast::Stmt::While { .. })
        ));
    }

    #[test]
    fn converts_single_statement_do_while_branch() {
        let program = parse_subset(
            "function main(value) { if (value) do break; while (value); }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::If {
                then_block: ref block,
                ..
            } if matches!(block.statements.first(), Some(crate::ast::Stmt::DoWhile { .. }))
        ));
    }

    #[test]
    fn converts_single_statement_for_branch() {
        let program = parse_subset(
            "function main(value) { if (value) for (let i = 0; i < value; i = i + 1) break; }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::If {
                then_block: ref block,
                ..
            } if matches!(block.statements.first(), Some(crate::ast::Stmt::For { .. }))
        ));
    }

    #[test]
    fn converts_single_statement_for_of_branch() {
        let program = parse_subset(
            "function main(values, flag) { if (flag) for (let value of values) break; }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::If {
                then_block: ref block,
                ..
            } if matches!(block.statements.first(), Some(crate::ast::Stmt::ForOf { .. }))
        ));
    }

    #[test]
    fn converts_loops_inside_block_branches() {
        let program = parse_subset(
            "function main(value, values) { if (value) { while (value) break; for (let item of values) break; } }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::If {
                then_block: ref block,
                ..
            } if matches!(block.statements.first(), Some(crate::ast::Stmt::While { .. }))
                && matches!(block.statements.get(1), Some(crate::ast::Stmt::ForOf { .. }))
        ));
    }

    #[test]
    fn converts_while_statements_with_block_bodies() {
        let program = parse_subset(
            "function count(value) { while (value) { return 1; } return 0; }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::While { .. }
        ));
    }

    #[test]
    fn converts_for_statements_with_initializer_and_update() {
        let program = parse_subset(
            "function count() { for (let value = 0; value < 2; value + 1) { return value; } }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::For {
                init: Some(_),
                cond: Some(_),
                update: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn preserves_typed_for_initializer_bindings() {
        let program = parse_subset(
            "function main() { for (let index: int = 0; index < 1; index = index + 1) { } }",
            true,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::For {
                init: Some(ref init),
                ..
            } if matches!(init.as_ref(), crate::ast::Stmt::Let { ty: Some(ty), .. } if ty.name.span.start < ty.name.span.end)
        ));
    }

    #[test]
    fn converts_for_of_statements_with_identifier_bindings() {
        let program = parse_subset(
            "function visit(items) { for (const item of items) { return item; } }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::ForOf { .. }
        ));
    }

    #[test]
    fn converts_do_while_statements_with_block_bodies() {
        let program = parse_subset(
            "function retry(value) { do { return value; } while (value); }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::DoWhile { .. }
        ));
    }

    #[test]
    fn converts_loop_control_statements() {
        let program = parse_subset(
            "function scan(items) { for (const item of items) { if (item) { continue; } break; } }",
            false,
        )
        .unwrap();
        let crate::ast::Stmt::ForOf { ref body, .. } = program.functions[0].body.statements[0]
        else {
            panic!("expected for-of loop");
        };
        assert!(matches!(body.statements[0], crate::ast::Stmt::If { .. }));
        assert!(matches!(body.statements[1], crate::ast::Stmt::Break { .. }));
    }

    #[test]
    fn converts_throw_statements() {
        let program = parse_subset("function fail(error) { throw error; }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Throw { .. }
        ));
    }

    #[test]
    fn converts_identifier_assignments() {
        let program =
            parse_subset("function set(value) { value = 42; return value; }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Expr(crate::ast::Expr::Assign { .. })
        ));
    }

    #[test]
    fn converts_named_and_default_imports() {
        let program = parse_subset(
            "import value, { helper as local } from 'zun:std'; function main() { return local; }",
            false,
        )
        .unwrap();
        assert_eq!(program.imports.len(), 1);
        assert_eq!(program.imports[0].module, "zun:std");
        assert_eq!(program.imports[0].names.len(), 2);
        assert!(program.imports[0].names[0].is_default);
        assert!(!program.imports[0].names[1].is_default);
    }

    #[test]
    fn converts_namespace_imports() {
        let program = parse_subset("import * as math from 'zun:core'; function main() {}", true).unwrap();
        let binding = &program.imports[0].names[0];
        assert!(binding.is_namespace);
        assert!(!binding.is_default);
        assert_eq!(binding.local.span, Span::new(12, 16));
    }

    #[test]
    fn converts_initialized_class_fields() {
        let program = parse_subset("class Point { x: number = 5; }", true).unwrap();
        let field = &program.classes[0].fields[0];
        assert!(matches!(field.initializer.as_deref(), Some(crate::ast::Expr::Number { value, .. }) if *value == 5.0));
    }

    #[test]
    fn converts_optional_computed_members() {
        let program = parse_subset("function main(object: any, key: any): any { return object?.[key]; }", true).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return { value: Some(crate::ast::Expr::OptionalIndex { .. }), .. }
        ));
    }

    #[test]
    fn converts_optional_indirect_calls() {
        let program = parse_subset("function main(f: Function): any { return f?.(); }", true).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return { value: Some(crate::ast::Expr::OptionalCall { ref args, .. }), .. } if args.is_empty()
        ));
    }

    #[test]
    fn converts_identifier_constructor_calls() {
        let program = parse_subset("function make() { return new Point(1, 2); }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::New { ref args, .. }),
                ..
            } if args.len() == 2
        ));
    }

    #[test]
    fn converts_uninitialized_class_fields_and_heritage() {
        let program = parse_subset(
            "class Child extends Base { value: number; #secret: string; get read() { return 1; } } function main() {}",
            true,
        )
        .unwrap();
        assert_eq!(program.classes.len(), 1);
        assert!(program.classes[0].base.is_some());
        assert_eq!(program.classes[0].fields.len(), 2);
        assert!(program.classes[0].fields[1].is_private);
        assert_eq!(program.classes[0].methods.len(), 1);
        assert_eq!(
            program.classes[0].methods[0].accessor,
            Some(crate::ast::AccessorKind::Get)
        );
    }

    #[test]
    fn converts_template_literals_and_interpolations() {
        let program =
            parse_subset("function greet(name) { return `hello ${name}`; }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Template { ref parts, .. }),
                ..
            } if parts.len() == 3
        ));
    }

    #[test]
    fn converts_regex_literals_and_keeps_division() {
        let program = parse_subset(
            "function f(a, b) { return /a[/]b/gi; } function g(a, b) { return a / b; }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::RegexLiteral { ref pattern, ref flags, .. }),
                ..
            } if pattern == "a[/]b" && flags == "gi"
        ));
        assert!(matches!(
            program.functions[1].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Binary { .. }),
                ..
            }
        ));
    }

    #[test]
    fn converts_null_and_conditional_expressions() {
        let program =
            parse_subset("function choose(value) { return value ? null : 0; }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Conditional { .. }),
                ..
            }
        ));
    }

    #[test]
    fn converts_optional_static_members() {
        let program = parse_subset("function read(value) { return value?.size; }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::OptionalField { .. }),
                ..
            }
        ));
    }

    #[test]
    fn converts_computed_member_access() {
        let program = parse_subset(
            "function read(values, index) { return values[index]; }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Index { .. }),
                ..
            }
        ));
    }

    #[test]
    fn converts_field_and_index_assignments() {
        let program = parse_subset(
            "function write(value, values, index) { value.size = 1; values[index] = 2; }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Expr(crate::ast::Expr::FieldAssign { .. })
        ));
        assert!(matches!(
            program.functions[0].body.statements[1],
            crate::ast::Stmt::Expr(crate::ast::Expr::IndexAssign { .. })
        ));
    }

    #[test]
    fn converts_private_member_access_and_assignment() {
        let program = parse_subset(
            "function update(value) { value.#secret = 1; return value.#secret; }",
            false,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Expr(crate::ast::Expr::FieldAssign { .. })
        ));
        assert!(matches!(
            program.functions[0].body.statements[1],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::PrivateField { .. }),
                ..
            }
        ));
    }

    #[test]
    fn converts_spread_call_arguments() {
        let program =
            parse_subset("function call(values) { return merge(...values); }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Call { ref args, .. }),
                ..
            } if matches!(args.first(), Some(crate::ast::Expr::Spread { .. }))
        ));
    }

    #[test]
    fn converts_await_and_yield_expressions() {
        let async_program =
            parse_subset("async function wait(value) { return await value; }", false).unwrap();
        assert!(matches!(
            async_program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Await { .. }),
                ..
            }
        ));
        let generator = parse_subset("function* values() { yield 1; }", false).unwrap();
        assert!(matches!(
            generator.functions[0].body.statements[0],
            crate::ast::Stmt::Expr(crate::ast::Expr::Yield { .. })
        ));
    }

    #[test]
    fn converts_this_member_access() {
        let program = parse_subset(
            "class Box { value: number; read() { return this.value; } } function main() {}",
            true,
        )
        .unwrap();
        assert!(matches!(
            program.classes[0].methods[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Field { .. }),
                ..
            }
        ));
    }

    #[test]
    fn converts_loops_in_class_methods() {
        let program = parse_subset(
            "class Box { value: number; read(value) { while (value) break; return this.value; } } function main() {}",
            true,
        )
        .unwrap();
        assert!(matches!(
            program.classes[0].methods[0].body.statements.first(),
            Some(crate::ast::Stmt::While { .. })
        ));
    }

    #[test]
    fn converts_locals_and_conditionals_in_class_methods() {
        let program = parse_subset(
            "class Box { value: number; read(value) { let local: number = value; if (local) return this.value; return value; } } function main() {}",
            true,
        )
        .unwrap();
        assert!(matches!(
            program.classes[0].methods[0].body.statements.first(),
            Some(crate::ast::Stmt::Let { .. })
        ));
        assert!(matches!(
            program.classes[0].methods[0].body.statements.get(1),
            Some(crate::ast::Stmt::If { .. })
        ));
    }

    #[test]
    fn converts_control_flow_and_throw_in_class_methods() {
        let program = parse_subset(
            "class Box { value: number; read(value) { while (value) { if (value) continue; break; } throw value; } } function main() {}",
            true,
        )
        .unwrap();
        assert!(matches!(
            program.classes[0].methods[0].body.statements.get(1),
            Some(crate::ast::Stmt::Throw { .. })
        ));
    }

    #[test]
    fn converts_parenthesized_expressions() {
        let program = parse_subset("function add(value) { return (value + 1); }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Binary { .. }),
                ..
            }
        ));
    }

    #[test]
    fn converts_typescript_assertions() {
        let program = parse_subset(
            "function cast(value: unknown) { return value as number; }",
            true,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Cast { .. }),
                ..
            }
        ));
    }

    #[test]
    fn converts_satisfies_and_non_null_assertions() {
        let program = parse_subset(
            "function check(value: unknown) { return value! satisfies number; }",
            true,
        )
        .unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Cast { .. }),
                ..
            }
        ));
    }

    #[test]
    fn converts_default_parameters() {
        let program = parse_subset("function greet(name = 'zun') { return name; }", false).unwrap();
        assert!(program.functions[0].params[0].default_value.is_some());
    }

    #[test]
    fn converts_rest_parameters() {
        let program =
            parse_subset("function collect(...values) { return values; }", false).unwrap();
        assert!(program.functions[0].params[0].is_rest);
    }

    #[test]
    fn converts_typed_rest_parameters() {
        let program = parse_subset(
            "function collect(...values: Values) { return values; }",
            true,
        )
        .unwrap();
        assert!(program.functions[0].params[0].ty.is_some());
    }

    #[test]
    fn converts_named_typescript_function_types() {
        let source = "function add(value: int): int { return value; }";
        let program = parse_subset(source, true).unwrap();
        let function = &program.functions[0];
        assert_eq!(
            function.params[0].ty.as_ref().unwrap().name.span,
            Span::new(20, 23)
        );
        assert_eq!(
            function.return_type.as_ref().unwrap().name.span,
            Span::new(26, 29)
        );
    }

    #[test]
    fn preserves_typescript_class_field_types() {
        let program =
            parse_subset("class Box { value: number; } function main() {}", true).unwrap();
        assert_eq!(program.classes[0].fields[0].ty.name.span, Span::new(19, 25));
    }

    #[test]
    fn converts_optional_method_calls() {
        let program =
            parse_subset("function read(value) { return value?.trim(); }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::OptionalMethodCall { .. }),
                ..
            }
        ));
    }

    #[test]
    fn converts_exported_declarations() {
        let program = parse_subset(
            "export function main(): int { return 7; } export class Box { value: number; }",
            true,
        )
        .unwrap();
        assert_eq!(program.functions.len(), 1);
        assert_eq!(program.classes.len(), 1);
    }

    #[test]
    fn converts_named_default_declarations() {
        let function =
            parse_subset("export default function main(): int { return 7; }", true).unwrap();
        let class = parse_subset("export default class Box {}", true).unwrap();
        assert_eq!(function.functions.len(), 1);
        assert_eq!(class.classes.len(), 1);
    }

    #[test]
    fn accepts_unchanged_export_lists_and_rejects_aliases() {
        let program = parse_subset("function main() {} export { main };", false).unwrap();
        assert_eq!(program.functions.len(), 1);
        let error =
            parse_subset("function main() {} export { main as entry };", false).unwrap_err();
        assert!(error
            .iter()
            .any(|message| message.contains("unchanged local export names")));
        assert!(parse_subset("export { main } from \"other\";", false).is_err());
    }

    #[test]
    fn converts_object_literals_with_methods_accessors_and_spread() {
        let program = parse_subset(
            "function main() { let x = 0; let o = { a: 1, x, [\"k\"]: 2 }; }",
            false,
        )
        .unwrap();
        let crate::ast::Stmt::Let {
            init: crate::ast::Expr::ObjectLiteral { properties, .. },
            ..
        } = &program.functions[0].body.statements[1]
        else {
            panic!("expected an object literal");
        };
        assert_eq!(properties.len(), 3);
        let prop = |index: usize| match &properties[index] {
            crate::ast::ObjectMember::Property(property) => property,
            _ => panic!("expected a keyed property"),
        };
        assert!(matches!(prop(0).key, crate::ast::PropertyKey::Ident(_)));
        assert!(matches!(prop(1).value, crate::ast::Expr::Ident(_)));
        assert!(matches!(prop(2).key, crate::ast::PropertyKey::Computed(_)));

        // Method shorthand, getters/setters, and spread now convert.
        assert!(parse_subset("function main() { let o = { ...rest }; }", false).is_ok());
        assert!(parse_subset(
            "function main() { let o = { m() { return 1; } }; }",
            false
        )
        .is_ok());
        assert!(parse_subset(
            "function main() { let o = { get a() { return 1; } }; }",
            false
        )
        .is_ok());
    }

    #[test]
    fn converts_array_literals_with_holes_and_spread() {
        let program = parse_subset(
            "function main() { let xs = [1, 2]; let a = [0, ...xs, , 3]; }",
            false,
        )
        .unwrap();
        let crate::ast::Stmt::Let {
            init: crate::ast::Expr::ArrayLiteral { elements, .. },
            ..
        } = &program.functions[0].body.statements[1]
        else {
            panic!("expected an array literal");
        };
        assert_eq!(elements.len(), 4);
        assert!(matches!(elements[0], crate::ast::ArrayElement::Expr(_)));
        assert!(matches!(elements[1], crate::ast::ArrayElement::Spread(_)));
        assert!(matches!(elements[2], crate::ast::ArrayElement::Hole));
        assert!(matches!(elements[3], crate::ast::ArrayElement::Expr(_)));
    }
}

thread_local! {
    /// Statements a converted expression needs executed before the statement
    /// that contains it (the `yield*` forwarding loop). Drained by
    /// `convert_statement_list` right after each statement is converted.
    static PENDING_PRELUDE: std::cell::RefCell<Vec<Stmt>> = const { std::cell::RefCell::new(Vec::new()) };
    static YIELD_STAR_COUNTER: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

thread_local! {
    /// Names bound by top-level variable declarations of the module.
    static TOP_LEVEL_NAMES: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

thread_local! {
    /// Start offset of the bare block currently being converted as a direct
    /// child of the module body. Such a block stays flattened into the module
    /// scope (its bindings are module globals); every other bare block that
    /// declares `let`/`const` keeps its own scope.
    static TOP_LEVEL_BLOCK: std::cell::Cell<u32> = const { std::cell::Cell::new(u32::MAX) };
}

thread_local! {
    /// Name span of the binding an anonymous class expression initializes
    /// (`const C = class {}`), consumed by the class-expression converter.
    static CLASS_NAME_HINT: std::cell::RefCell<Option<Span>> =
        const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// Block-local / nested class declarations collected while converting a
    /// module body. Drained into the program's class list in `parse_subset`.
    /// Thread-local so the recursive statement converter can reach it without
    /// threading a collector through every conversion function.
    static HOISTED_CLASSES: std::cell::RefCell<Vec<crate::ast::Class>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Convert a single OXC class declaration into the AST class (shared by the
/// top-level scan and the statement converter for block-local classes).
fn convert_class_declaration(source: &str, class_declaration: &oxc_ast::ast::Class<'_>) -> Result<crate::ast::Class, Vec<String>> {
            // Anonymous class expressions get their name patched by the caller.
            let id_span = class_declaration
                .id
                .as_ref()
                .map(|id| id.span)
                .unwrap_or(class_declaration.span);
            let base = class_declaration
                .heritage
                .as_ref()
                .map(|heritage| {
                    let Expression::Identifier(identifier) = &heritage.expression else {
                        return Err("OXC adapter supports identifier class bases only".into());
                    };
                    Ok(Name {
                        span: Span::new(identifier.span.start, identifier.span.end),
                    })
                })
                .transpose()
                .map_err(|error| vec![error])?;
            let mut fields = Vec::new();
            let mut methods = Vec::new();
            let mut symbol_methods = Vec::new();
            for element in &class_declaration.body.body {
                match element {
                    ClassElement::PropertyDefinition(property) => {
                        let (field_span, is_private) = match &property.key {
                            PropertyKey::StaticIdentifier(identifier) => {
                                (Span::new(identifier.span.start, identifier.span.end), false)
                            }
                            PropertyKey::PrivateIdentifier(identifier) => {
                                (Span::new(identifier.span.start, identifier.span.end), true)
                            }
                            _ => {
                                return Err(vec![
                                    "OXC adapter supports identifier class fields only".into(),
                                ])
                            }
                        };
                        // A field without an explicit (or nameable) type erases
                        // to `any`, encoded as a zero-width type span that sema
                        // resolves to `Ty::Any` (JS/TS erasure semantics).
                        let type_annotation =
                            convert_type_annotation(source, property.type_annotation.as_deref())
                                .map_err(|error| vec![error])?
                                .unwrap_or(crate::ast::TypeAnnotation {
                                    name: Name {
                                        span: Span::new(field_span.start, field_span.start),
                                    },
                                    is_array: false,
                                });
                        fields.push(crate::ast::ClassField {
                            name: Name { span: field_span },
                            decorators: property
                                .decorators
                                .iter()
                                .map(|decorator| convert_decorator(source, decorator))
                                .collect::<Result<Vec<_>, _>>()
                                .map_err(|error| vec![error])?,
                            ty: type_annotation,
                            is_static: property.r#static,
                            is_private,
                            // A field without an initializer defaults to
                            // `undefined` (`void 0`), so the implicit
                            // constructor always has a value for every slot.
                            initializer: Some(Box::new(
                                match property.value.as_ref() {
                                    Some(expression) => convert_expression(source, expression)
                                        .map_err(|error| vec![error])?,
                                    None => Expr::Unary {
                                        op: crate::ast::UnaryOp::Void,
                                        operand: Box::new(Expr::Number {
                                            value: 0.0,
                                            span: field_span,
                                        }),
                                        span: field_span,
                                    },
                                },
                            )),
                        });
                    }
                    ClassElement::MethodDefinition(method) => {
                        // A computed key (`[Symbol.iterator]() {}`) is lowered
                        // to a Symbol-keyed side-table entry; a plain name is a
                        // vtable method. `name_span` carries the key span in the
                        // computed case (used only for diagnostics).
                        // `['name']() {}` with a plain identifier-shaped literal is a named method.
                        let literal_name = match (&method.key, method.computed) {
                            (PropertyKey::StringLiteral(text), true)
                                if text.value.as_str().is_some_and(is_plain_identifier)
                                    && (text.span.end - text.span.start) as usize
                                        == text.value.as_str().map_or(0, str::len) + 2 =>
                            {
                                Some(Span::new(text.span.start + 1, text.span.end - 1))
                            }
                            _ => None,
                        };
                        let (name_span, computed_key) = if let Some(literal) = literal_name {
                            (literal, None)
                        } else if method.computed {
                            let key_expression = method.key.as_expression().ok_or_else(|| {
                                vec!["OXC adapter requires an expression computed method key".to_owned()]
                            })?;
                            let key = convert_expression(source, key_expression)
                                .map_err(|error| vec![error])?;
                            (key.span(), Some(key))
                        } else if let PropertyKey::StaticIdentifier(identifier) = &method.key {
                            (Span::new(identifier.span.start, identifier.span.end), None)
                        } else if let PropertyKey::PrivateIdentifier(identifier) = &method.key {
                            (Span::new(identifier.span.start, identifier.span.end), None)
                        } else {
                            return Err(vec![
                                "OXC adapter supports identifier class method names only".into(),
                            ]);
                        };
                        let Some(body) = method.value.body.as_ref() else {
                            return Err(vec!["OXC adapter requires class method bodies".into()]);
                        };
                        // The method body uses the unified statement converter so
                        // class methods support the same statement surface as any
                        // other function body.
                        let mut method_prologue = Vec::new();
                        let mut params = Vec::with_capacity(method.value.params.items.len());
                        for parameter in &method.value.params.items {
                            params.push(
                                convert_formal_param(source, parameter, &mut method_prologue)
                                    .map_err(|error| vec![error])?,
                            );
                        }
                        let mut method_statements = method_prologue;
                        method_statements.extend(
                            convert_function_statements(source, &body.statements, &param_names(&method.value.params))
                                .map_err(|error| vec![error])?,
                        );
                        if let Some(rest) = method.value.params.rest.as_ref() {
                            let BindingPattern::BindingIdentifier(identifier) = &rest.rest.argument
                            else {
                                return Err(vec![
                                    "OXC adapter supports identifier rest parameters only".into(),
                                ]);
                            };
                            params.push(crate::ast::Param {
                                name: Name {
                                    span: Span::new(identifier.span.start, identifier.span.end),
                                },
                                ty: convert_type_annotation(
                                    source,
                                    rest.type_annotation.as_deref(),
                                )
                                .map_err(|error| vec![error])?,
                                default_value: None,
                                is_rest: true,
                            });
                        }
                        inject_arguments_object(
                            source,
                            Span::new(method.span.start, method.span.end),
                            &mut params,
                            &mut method_statements,
                        );
                        let decorators = method
                            .decorators
                            .iter()
                            .map(|decorator| convert_decorator(source, decorator))
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(|error| vec![error])?;
                        let function = Function {
                            name: Name { span: name_span },
                            // The OXC adapter is a partial secondary path; it
                            // erases generic type parameters (the hand parser
                            // carries them). Generics are erased either way.
                            type_params: Vec::new(),
                            decorators,
                            params,
                            return_type: convert_type_annotation(
                                source,
                                method.value.return_type.as_deref(),
                            )
                            .map_err(|error| vec![error])?,
                            is_extern: false,
                            // A computed-key member is modeled as a static
                            // method (no implicit `this`).
                            is_static: method.r#static,
                            accessor: match method.kind {
                                MethodDefinitionKind::Get => Some(crate::ast::AccessorKind::Get),
                                MethodDefinitionKind::Set => Some(crate::ast::AccessorKind::Set),
                                _ => None,
                            },
                            external_module: None,
                            is_async: method.value.r#async,
                            is_generator: method.value.generator,
                            body: Block {
                                statements: method_statements,
                                span: Span::new(body.span.start, body.span.end),
                            },
                            span: Span::new(method.span.start, method.span.end),
                        };
                        match computed_key {
                            Some(key) => symbol_methods.push(crate::ast::ClassSymbolMethod {
                                key,
                                function,
                                span: Span::new(method.span.start, method.span.end),
                            }),
                            None => {
                                // A later declaration of the same member (same
                                // name, accessor kind, staticness) replaces the
                                // earlier one, as in JS.
                                let text = |f: &Function| {
                                    source[f.name.span.start as usize..f.name.span.end as usize]
                                        .to_owned()
                                };
                                let key = (text(&function), function.accessor, function.is_static);
                                if let Some(existing) = methods
                                    .iter_mut()
                                    .find(|m| (text(m), m.accessor, m.is_static) == key)
                                {
                                    *existing = function;
                                } else {
                                    methods.push(function);
                                }
                            }
                        }
                    }
                    _ => {
                        return Err(vec![
                            "OXC adapter does not support this class element yet".into()
                        ])
                    }
                }
            }
            // JS classes create fields by assignment: `this.name = …` in a
            // constructor of a base-less class declares an `any` field.
            if base.is_none() {
                for element in &class_declaration.body.body {
                    let ClassElement::MethodDefinition(method) = element else { continue };
                    if method.kind != oxc_ast::ast::MethodDefinitionKind::Constructor {
                        continue;
                    }
                    let (start, end) = (method.span.start as usize, method.span.end as usize);
                    let text = &source[start..end];
                    let bytes = text.as_bytes();
                    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
                    // Both `this.name =` and `this['name'] =` spellings.
                    let mut candidates: Vec<(usize, usize)> = Vec::new();
                    for (marker, skip) in [("this.", 5usize), ("this['", 6), ("this[\"", 6)] {
                        let mut from = 0;
                        while let Some(found) = text[from..].find(marker) {
                            let at = from + found;
                            from = at + skip;
                            if at > 0 && (is_word(bytes[at - 1]) || bytes[at - 1] == b'.') {
                                continue;
                            }
                            candidates.push((at, skip));
                        }
                    }
                    for (at, skip) in candidates {
                        let quoted = skip == 6;
                        let name_start = at + skip;
                        let mut name_end = name_start;
                        while name_end < bytes.len() && is_word(bytes[name_end]) {
                            name_end += 1;
                        }
                        if name_end == name_start || !is_plain_identifier(&text[name_start..name_end]) {
                            continue;
                        }
                        let mut cursor = name_end;
                        if quoted {
                            // closing quote and bracket
                            if bytes.get(cursor).is_none_or(|b| *b != 39u8 && *b != b'"')
                                || bytes.get(cursor + 1) != Some(&b']')
                            {
                                continue;
                            }
                            cursor += 2;
                        }
                        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                            cursor += 1;
                        }
                        let assigns = cursor < bytes.len()
                            && bytes[cursor] == b'='
                            && !matches!(bytes.get(cursor + 1), Some(b'=') | Some(b'>'));
                        if !assigns {
                            continue;
                        }
                        let field_name = &text[name_start..name_end];
                        let known = fields.iter().any(|field: &crate::ast::ClassField| {
                            source.get(field.name.span.start as usize..field.name.span.end as usize)
                                == Some(field_name)
                        }) || methods.iter().any(|method: &crate::ast::Function| {
                            source.get(method.name.span.start as usize..method.name.span.end as usize)
                                == Some(field_name)
                        });
                        if known {
                            continue;
                        }
                        let field_span = Span::new((start + name_start) as u32, (start + name_end) as u32);
                        fields.push(crate::ast::ClassField {
                            name: Name { span: field_span },
                            decorators: Vec::new(),
                            ty: crate::ast::TypeAnnotation {
                                name: Name { span: Span::new(field_span.start, field_span.start) },
                                is_array: false,
                            },
                            is_static: false,
                            is_private: false,
                            initializer: Some(Box::new(Expr::Unary {
                                op: crate::ast::UnaryOp::Void,
                                operand: Box::new(Expr::Number { value: 0.0, span: field_span }),
                                span: field_span,
                            })),
                        });
                    }
                }
            }
            return Ok(crate::ast::Class {
                name: Name {
                    span: Span::new(id_span.start, id_span.end),
                },
                // OXC adapter erases generic type parameters (see above).
                type_params: Vec::new(),
                base,
                decorators: class_declaration
                    .decorators
                    .iter()
                    .map(|decorator| convert_decorator(source, decorator))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| vec![error])?,
                fields,
                methods,
                symbol_methods,
                span: Span::new(class_declaration.span.start, class_declaration.span.end),
            });
}

/// Find the first whole-word occurrence of `arguments` inside `[start, end)` of
/// the source, returning its span.
fn find_arguments_word(source: &str, start: u32, end: u32) -> Option<Span> {
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
fn inject_arguments_object(
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
            init: Expr::ArrayLiteral { elements, span: word },
            span: word,
        },
    );
}

/// Whether the member/call spine of `expression` contains an optional link
/// (`?.`), i.e. whether it is part of an optional chain.
fn spine_has_optional(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::StaticMemberExpression(member) => {
            member.optional || spine_has_optional(&member.object)
        }
        Expression::ComputedMemberExpression(member) => {
            member.optional || spine_has_optional(&member.object)
        }
        Expression::CallExpression(call) => call.optional || spine_has_optional(&call.callee),
        Expression::TSNonNullExpression(inner) => spine_has_optional(&inner.expression),
        _ => false,
    }
}

/// Convert one link of an optional chain. Every `?.` link pushes the value
/// computed so far onto `guards` and continues on a [`Expr::ChainRoot`]
/// placeholder, so a nullish guard short-circuits the *rest* of the chain.
fn chain_link(
    source: &str,
    expression: &Expression<'_>,
    guards: &mut Vec<Expr>,
) -> Result<Expr, String> {
    if !spine_has_optional(expression) {
        return convert_expression(source, expression);
    }
    match expression {
        Expression::StaticMemberExpression(member) => chain_static(source, member, guards),
        Expression::ComputedMemberExpression(member) => chain_computed(source, member, guards),
        Expression::CallExpression(call) => chain_call(source, call, guards),
        Expression::TSNonNullExpression(inner) => chain_link(source, &inner.expression, guards),
        other => convert_expression(source, other),
    }
}

fn chain_guard(object: Expr, optional: bool, guards: &mut Vec<Expr>) -> Expr {
    if optional {
        let span = object.span();
        guards.push(object);
        Expr::ChainRoot { depth: 0, span }
    } else {
        object
    }
}

fn chain_static(
    source: &str,
    member: &oxc_ast::ast::StaticMemberExpression<'_>,
    guards: &mut Vec<Expr>,
) -> Result<Expr, String> {
    let object = chain_link(source, &member.object, guards)?;
    let object = chain_guard(object, member.optional, guards);
    Ok(Expr::Field {
        object: Box::new(object),
        field: Name { span: Span::new(member.property.span.start, member.property.span.end) },
        span: Span::new(member.span.start, member.span.end),
    })
}

fn chain_computed(
    source: &str,
    member: &oxc_ast::ast::ComputedMemberExpression<'_>,
    guards: &mut Vec<Expr>,
) -> Result<Expr, String> {
    let object = chain_link(source, &member.object, guards)?;
    let object = chain_guard(object, member.optional, guards);
    Ok(Expr::Index {
        object: Box::new(object),
        index: Box::new(convert_expression(source, &member.expression)?),
        span: Span::new(member.span.start, member.span.end),
    })
}

fn chain_call(
    source: &str,
    call: &oxc_ast::ast::CallExpression<'_>,
    guards: &mut Vec<Expr>,
) -> Result<Expr, String> {
    let span = Span::new(call.span.start, call.span.end);
    let mut args = Vec::with_capacity(call.arguments.len());
    for argument in &call.arguments {
        args.push(convert_argument(source, argument)?);
    }
    match &call.callee {
        Expression::StaticMemberExpression(member) => {
            let object = chain_link(source, &member.object, guards)?;
            let object = chain_guard(object, member.optional, guards);
            let method = Name { span: Span::new(member.property.span.start, member.property.span.end) };
            if call.optional {
                Ok(Expr::OptionalCall {
                    callee: Box::new(Expr::Field {
                        object: Box::new(object),
                        field: method,
                        span: Span::new(member.span.start, member.span.end),
                    }),
                    args,
                    span,
                })
            } else {
                Ok(Expr::MethodCall { receiver: Box::new(object), method, args, span })
            }
        }
        Expression::ComputedMemberExpression(member) => {
            let object = chain_link(source, &member.object, guards)?;
            let object = chain_guard(object, member.optional, guards);
            let callee = Expr::Index {
                object: Box::new(object),
                index: Box::new(convert_expression(source, &member.expression)?),
                span: Span::new(member.span.start, member.span.end),
            };
            if call.optional {
                Ok(Expr::OptionalCall { callee: Box::new(callee), args, span })
            } else {
                Ok(Expr::ComputedCall { callee: Box::new(callee), args, span })
            }
        }
        other => {
            let callee = chain_link(source, other, guards)?;
            if call.optional {
                Ok(Expr::OptionalCall { callee: Box::new(callee), args, span })
            } else {
                Ok(Expr::ComputedCall { callee: Box::new(callee), args, span })
            }
        }
    }
}

fn convert_chain(
    source: &str,
    chain: &oxc_ast::ast::ChainExpression<'_>,
) -> Result<Expr, String> {
    let mut guards: Vec<Expr> = Vec::new();
    let chain_span = Span::new(chain.span.start, chain.span.end);
    let body = match &chain.expression {
        ChainElement::StaticMemberExpression(member) => chain_static(source, member, &mut guards)?,
        ChainElement::ComputedMemberExpression(member) => {
            chain_computed(source, member, &mut guards)?
        }
        ChainElement::CallExpression(call) => chain_call(source, call, &mut guards)?,
        ChainElement::TSNonNullExpression(inner) => {
            chain_link(source, &inner.expression, &mut guards)?
        }
        _ => return Err("OXC adapter does not support optional private members yet".into()),
    };
    if guards.is_empty() {
        return Ok(body);
    }
    if guards.len() == 1 {
        let is_root = |expr: &Expr| matches!(expr, Expr::ChainRoot { .. });
        match body {
            Expr::Field { object, field, .. } if is_root(&object) => {
                let guard = guards.pop().unwrap();
                return Ok(Expr::OptionalField { object: Box::new(guard), field, span: chain_span });
            }
            Expr::Index { object, index, .. } if is_root(&object) => {
                let guard = guards.pop().unwrap();
                return Ok(Expr::OptionalIndex { object: Box::new(guard), index, span: chain_span });
            }
            Expr::MethodCall { receiver, method, args, .. } if is_root(&receiver) => {
                let guard = guards.pop().unwrap();
                return Ok(Expr::OptionalMethodCall {
                    object: Box::new(guard),
                    method,
                    args,
                    span: chain_span,
                });
            }
            other => {
                let guard = guards.pop().unwrap();
                return Ok(Expr::OptionalChain {
                    object: Box::new(guard),
                    body: Box::new(other),
                    span: chain_span,
                });
            }
        }
    }
    let mut result = body;
    for guard in guards.into_iter().rev() {
        result = Expr::OptionalChain {
            object: Box::new(guard),
            body: Box::new(result),
            span: chain_span,
        };
    }
    Ok(result)
}

/// A member-expression assignment target (`o.f`, `o[k]`, `o.#p`).
enum MemberRef<'a, 'b> {
    Static(&'a oxc_ast::ast::StaticMemberExpression<'b>),
    Computed(&'a oxc_ast::ast::ComputedMemberExpression<'b>),
    Private(&'a oxc_ast::ast::PrivateFieldExpression<'b>),
}

/// How a decomposed member target stores a value.
enum MemberStore {
    Field { object: Expr, field: Name },
    Index { object: Expr, index: Expr },
}

impl MemberStore {
    fn build(&self, value: Expr, span: Span) -> Expr {
        match self {
            MemberStore::Field { object, field } => Expr::FieldAssign {
                object: Box::new(object.clone()),
                field: *field,
                value: Box::new(value),
                span,
            },
            MemberStore::Index { object, index } => Expr::IndexAssign {
                object: Box::new(object.clone()),
                index: Box::new(index.clone()),
                value: Box::new(value),
                span,
            },
        }
    }
}

fn is_simple_operand(expression: &Expression<'_>) -> bool {
    matches!(
        expression,
        Expression::Identifier(_)
            | Expression::ThisExpression(_)
            | Expression::Super(_)
            | Expression::StringLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
    )
}

/// Split a member target into the sub-expressions that must be evaluated once
/// (`binds`, outermost first), a read of the target, and a store builder; the
/// read/store reference the bound values through [`Expr::ChainRoot`].
fn decompose_member(
    source: &str,
    member: MemberRef<'_, '_>,
) -> Result<(Vec<Expr>, Expr, MemberStore), String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let (object_ast, key_ast) = match &member {
        MemberRef::Static(m) => (&m.object, None),
        MemberRef::Computed(m) => (&m.object, Some(&m.expression)),
        MemberRef::Private(m) => (&m.object, None),
    };
    let bind_object = !is_simple_operand(object_ast);
    let bind_key = key_ast.map(|key| !is_simple_operand(key)).unwrap_or(false);
    let count = bind_object as u32 + bind_key as u32;
    let mut binds = Vec::new();
    let object_span = span(object_ast.span());
    let object_expr = convert_expression(source, object_ast)?;
    let object = if bind_object {
        binds.push(object_expr);
        Expr::ChainRoot { depth: count - 1, span: object_span }
    } else {
        object_expr
    };
    let key = match key_ast {
        Some(key) => {
            let key_span = span(key.span());
            let key_expr = convert_expression(source, key)?;
            Some(if bind_key {
                binds.push(key_expr);
                Expr::ChainRoot { depth: 0, span: key_span }
            } else {
                key_expr
            })
        }
        None => None,
    };
    Ok(match member {
        MemberRef::Static(m) => {
            let field = Name { span: span(m.property.span) };
            (
                binds,
                Expr::Field { object: Box::new(object.clone()), field, span: span(m.span) },
                MemberStore::Field { object, field },
            )
        }
        MemberRef::Computed(m) => {
            let index = key.expect("computed member has a key");
            (
                binds,
                Expr::Index {
                    object: Box::new(object.clone()),
                    index: Box::new(index.clone()),
                    span: span(m.span),
                },
                MemberStore::Index { object, index },
            )
        }
        MemberRef::Private(m) => {
            let field = Name { span: span(m.field.span) };
            let private = Expr::PrivateField {
                object: Box::new(object),
                field,
                span: span(m.span),
            };
            (
                binds,
                private.clone(),
                MemberStore::Field { object: private, field },
            )
        }
    })
}

/// Wrap `body` in `TempBind`s for `binds` (outermost first).
fn wrap_binds(binds: Vec<Expr>, body: Expr, span: Span) -> Expr {
    let mut result = body;
    for value in binds.into_iter().rev() {
        result = Expr::TempBind { value: Box::new(value), body: Box::new(result), span };
    }
    result
}

fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Whether `text` mentions `word` as a whole identifier.
fn contains_word(text: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(found) = text[from..].find(word) {
        let start = from + found;
        let end = start + word.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        if !before.map(is_identifier_char).unwrap_or(false)
            && !after.map(is_identifier_char).unwrap_or(false)
        {
            return true;
        }
        from = end;
    }
    false
}

/// JS hoists function declarations to the top of their block. The adapter
/// lowers a nested declaration to a `let` of a closure, so a declaration that
/// an earlier statement mentions is moved in front of the first such
/// statement (not to the very top, so it still captures the bindings declared
/// before that use).
fn hoist_function_declarations<'a, 'b>(
    source: &str,
    body: &'a [Statement<'b>],
) -> Vec<&'a Statement<'b>> {
    let mut order: Vec<&Statement> = body.iter().collect();
    let mut index = 0;
    while index < order.len() {
        if let Statement::FunctionDeclaration(function) = order[index] {
            if let Some(id) = function.id.as_ref() {
                let name = id.name.as_str();
                let first_use = order[..index].iter().position(|earlier| {
                    let span = earlier.span();
                    source
                        .get(span.start as usize..span.end as usize)
                        .map(|text| contains_word(text, name))
                        .unwrap_or(false)
                });
                if let Some(first) = first_use {
                    let moved = order.remove(index);
                    order.insert(first, moved);
                }
            }
        }
        index += 1;
    }
    order
}

thread_local! {
    /// `var` names hoisted to the function currently being converted
    /// (innermost last). A `var x = e` of a hoisted name converts to the
    /// assignment `x = e`; the declaration itself is emitted once at the top
    /// of the function body.
    static HOISTED_VARS: std::cell::RefCell<Vec<std::collections::HashSet<String>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// A `var` binding found in a function body.
struct VarSite {
    name: String,
    /// Index of the enclosing top-level statement of the function body.
    top_index: usize,
    /// Declared inside a nested block / loop / branch.
    nested: bool,
    /// Declared in a `for` / `for-in` / `for-of` head (converted separately).
    in_head: bool,
}

fn collect_var_declaration(
    declaration: &oxc_ast::ast::VariableDeclaration<'_>,
    top_index: usize,
    nested: bool,
    in_head: bool,
    out: &mut Vec<VarSite>,
) {
    if declaration.kind != oxc_ast::ast::VariableDeclarationKind::Var {
        return;
    }
    for declarator in &declaration.declarations {
        if let BindingPattern::BindingIdentifier(identifier) = &declarator.id {
            out.push(VarSite { name: identifier.name.to_string(), top_index, nested, in_head });
        }
    }
}

fn collect_vars_in_statement(
    statement: &Statement<'_>,
    top_index: usize,
    nested: bool,
    out: &mut Vec<VarSite>,
) {
    match statement {
        Statement::VariableDeclaration(declaration) => {
            collect_var_declaration(declaration, top_index, nested, false, out)
        }
        Statement::BlockStatement(block) => {
            for inner in &block.body {
                collect_vars_in_statement(inner, top_index, true, out);
            }
        }
        Statement::IfStatement(if_statement) => {
            collect_vars_in_statement(&if_statement.consequent, top_index, true, out);
            if let Some(alternate) = &if_statement.alternate {
                collect_vars_in_statement(alternate, top_index, true, out);
            }
        }
        Statement::ForStatement(for_statement) => {
            if let Some(ForStatementInit::VariableDeclaration(declaration)) = &for_statement.init {
                collect_var_declaration(declaration, top_index, true, true, out);
            }
            collect_vars_in_statement(&for_statement.body, top_index, true, out);
        }
        Statement::ForInStatement(for_in) => {
            if let ForStatementLeft::VariableDeclaration(declaration) = &for_in.left {
                collect_var_declaration(declaration, top_index, true, true, out);
            }
            collect_vars_in_statement(&for_in.body, top_index, true, out);
        }
        Statement::ForOfStatement(for_of) => {
            if let ForStatementLeft::VariableDeclaration(declaration) = &for_of.left {
                collect_var_declaration(declaration, top_index, true, true, out);
            }
            collect_vars_in_statement(&for_of.body, top_index, true, out);
        }
        Statement::WhileStatement(while_statement) => {
            collect_vars_in_statement(&while_statement.body, top_index, true, out)
        }
        Statement::DoWhileStatement(do_while) => {
            collect_vars_in_statement(&do_while.body, top_index, true, out)
        }
        Statement::LabeledStatement(labeled) => {
            collect_vars_in_statement(&labeled.body, top_index, nested, out)
        }
        Statement::TryStatement(try_statement) => {
            for inner in &try_statement.block.body {
                collect_vars_in_statement(inner, top_index, true, out);
            }
            if let Some(handler) = &try_statement.handler {
                for inner in &handler.body.body {
                    collect_vars_in_statement(inner, top_index, true, out);
                }
            }
            if let Some(finalizer) = &try_statement.finalizer {
                for inner in &finalizer.body {
                    collect_vars_in_statement(inner, top_index, true, out);
                }
            }
        }
        Statement::SwitchStatement(switch) => {
            for case in &switch.cases {
                for inner in &case.consequent {
                    collect_vars_in_statement(inner, top_index, true, out);
                }
            }
        }
        _ => {}
    }
}

/// Convert a function body, hoisting `var` declarations to function scope when
/// they are declared in a nested block or read before their declaration.
fn convert_function_statements(
    source: &str,
    body: &[Statement<'_>],
    params: &[String],
) -> Result<Vec<Stmt>, String> {
    let mut sites = Vec::new();
    for (index, statement) in body.iter().enumerate() {
        collect_vars_in_statement(statement, index, false, &mut sites);
    }
    let mut hoisted: Vec<String> = Vec::new();
    for site in &sites {
        if site.in_head
            || params.contains(&site.name)
            || hoisted.contains(&site.name)
            || sites.iter().any(|other| other.in_head && other.name == site.name)
        {
            continue;
        }
        let used_early = !site.nested
            && body[..site.top_index].iter().any(|earlier| {
                let span = earlier.span();
                source
                    .get(span.start as usize..span.end as usize)
                    .map(|text| contains_word(text, &site.name))
                    .unwrap_or(false)
            });
        if site.nested || used_early {
            hoisted.push(site.name.clone());
        }
    }
    HOISTED_VARS.with(|stack| stack.borrow_mut().push(hoisted.iter().cloned().collect()));
    let converted = convert_statement_list(source, body);
    HOISTED_VARS.with(|stack| stack.borrow_mut().pop());
    let converted = converted?;
    if hoisted.is_empty() {
        return Ok(converted);
    }
    let mut statements = Vec::with_capacity(converted.len() + hoisted.len());
    for name in &hoisted {
        let span = find_var_name_span(body, name)
            .ok_or_else(|| "OXC adapter lost a hoisted var span".to_string())?;
        statements.push(Stmt::Let {
            name: Name { span },
            ty: None,
            init: Expr::Unary {
                op: crate::ast::UnaryOp::Void,
                operand: Box::new(Expr::Number { value: 0.0, span }),
                span,
            },
            span,
        });
    }
    statements.extend(converted);
    Ok(statements)
}

fn find_var_name_span(body: &[Statement<'_>], name: &str) -> Option<Span> {
    fn in_declaration(
        declaration: &oxc_ast::ast::VariableDeclaration<'_>,
        name: &str,
    ) -> Option<Span> {
        for declarator in &declaration.declarations {
            if let BindingPattern::BindingIdentifier(identifier) = &declarator.id {
                if identifier.name == name {
                    return Some(Span::new(identifier.span.start, identifier.span.end));
                }
            }
        }
        None
    }
    fn in_statement(statement: &Statement<'_>, name: &str) -> Option<Span> {
        match statement {
            Statement::VariableDeclaration(declaration) => in_declaration(declaration, name),
            Statement::BlockStatement(block) => {
                block.body.iter().find_map(|s| in_statement(s, name))
            }
            Statement::IfStatement(s) => in_statement(&s.consequent, name)
                .or_else(|| s.alternate.as_ref().and_then(|a| in_statement(a, name))),
            Statement::ForStatement(s) => in_statement(&s.body, name),
            Statement::ForInStatement(s) => in_statement(&s.body, name),
            Statement::ForOfStatement(s) => in_statement(&s.body, name),
            Statement::WhileStatement(s) => in_statement(&s.body, name),
            Statement::DoWhileStatement(s) => in_statement(&s.body, name),
            Statement::LabeledStatement(s) => in_statement(&s.body, name),
            Statement::TryStatement(s) => s
                .block
                .body
                .iter()
                .find_map(|x| in_statement(x, name))
                .or_else(|| {
                    s.handler
                        .as_ref()
                        .and_then(|h| h.body.body.iter().find_map(|x| in_statement(x, name)))
                })
                .or_else(|| {
                    s.finalizer
                        .as_ref()
                        .and_then(|f| f.body.iter().find_map(|x| in_statement(x, name)))
                }),
            Statement::SwitchStatement(s) => s
                .cases
                .iter()
                .find_map(|c| c.consequent.iter().find_map(|x| in_statement(x, name))),
            _ => None,
        }
    }
    body.iter().find_map(|statement| in_statement(statement, name))
}

fn param_names(params: &oxc_ast::ast::FormalParameters<'_>) -> Vec<String> {
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

/// Whether `name` appears as the target of a plain `=` assignment anywhere in
/// `source` (a textual approximation; only used for function declarations).
fn name_is_reassigned(source: &str, name: &str) -> bool {
    let bytes = source.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let mut from = 0;
    while let Some(found) = source[from..].find(name) {
        let start = from + found;
        let end = start + name.len();
        from = end;
        if start > 0 && (is_word(bytes[start - 1]) || bytes[start - 1] == b'.') {
            continue;
        }
        if end < bytes.len() && is_word(bytes[end]) {
            continue;
        }
        let mut cursor = end;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor < bytes.len()
            && bytes[cursor] == b'='
            && !matches!(bytes.get(cursor + 1), Some(b'=') | Some(b'>'))
        {
            let before = source[..start].trim_end();
            if before.ends_with("let") || before.ends_with("const") || before.ends_with("var") {
                continue;
            }
            return true;
        }
    }
    false
}

/// Whether `text` is a plain ASCII identifier (usable as a method name).
fn is_plain_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}
