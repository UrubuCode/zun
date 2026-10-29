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
    let Some(initializer) = declarator.init.as_ref() else {
        return Err("OXC adapter requires an initializer for local bindings".into());
    };
    let init = convert_expression(source, initializer)?;
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
    for statement in body {
        match statement {
            Statement::EmptyStatement(_) | Statement::DebuggerStatement(_) => {}
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
            Statement::ExpressionStatement(expression_statement) => statements.push(Stmt::Expr(
                convert_expression(source, &expression_statement.expression)?,
            )),
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
                statements.push(convert_for_statement(source, for_statement)?);
            }
            Statement::ForOfStatement(for_of_statement) => {
                statements.push(convert_for_of(source, for_of_statement)?);
            }
            _ => {
                return Err(
                    "OXC adapter currently supports return and expression statements in branches"
                        .into(),
                )
            }
        }
    }
    Ok(statements)
}

/// Convert an OXC function (an object-method, getter, or setter value) into a
/// Zun anonymous-function expression. Parameters must be plain identifiers;
/// the body reuses the shared statement-list conversion.
fn convert_oxc_function(source: &str, function: &OxcFunction<'_>) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let Some(body) = function.body.as_ref() else {
        return Err("OXC adapter requires an object method body".into());
    };
    let mut params = Vec::with_capacity(function.params.items.len());
    for parameter in &function.params.items {
        let BindingPattern::BindingIdentifier(identifier) = &parameter.pattern else {
            return Err("OXC adapter supports identifier object-method parameters only".into());
        };
        params.push(crate::ast::Param {
            name: Name {
                span: span(identifier.span),
            },
            ty: convert_type_annotation(source, parameter.type_annotation.as_deref())?,
            default_value: parameter
                .initializer
                .as_ref()
                .map(|initializer| convert_expression(source, initializer))
                .transpose()?
                .map(Box::new),
            is_rest: false,
        });
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
    let statements = convert_statement_list(source, &body.statements)?;
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
) -> Result<Stmt, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let init = match &for_statement.init {
        None => None,
        Some(ForStatementInit::VariableDeclaration(declaration)) => {
            if declaration.declarations.len() != 1 {
                return Err("OXC adapter supports one for-loop initializer binding".into());
            }
            let declarator = &declaration.declarations[0];
            let BindingPattern::BindingIdentifier(identifier) = &declarator.id else {
                return Err("OXC adapter supports identifier for-loop bindings".into());
            };
            let Some(initializer) = declarator.init.as_ref() else {
                return Err("OXC adapter requires an initializer for for-loop bindings".into());
            };
            Some(Box::new(Stmt::Let {
                name: Name { span: span(identifier.span) },
                ty: convert_type_annotation(source, declarator.type_annotation.as_deref())?,
                init: convert_expression(source, initializer)?,
                span: span(declarator.span),
            }))
        }
        Some(init) => Some(Box::new(Stmt::Expr(convert_expression(
            source,
            init.as_expression()
                .ok_or("OXC adapter does not support this for-loop initializer yet")?,
        )?))),
    };
    Ok(Stmt::For {
        init,
        cond: for_statement
            .test
            .as_ref()
            .map(|expression| convert_expression(source, expression))
            .transpose()?,
        update: for_statement
            .update
            .as_ref()
            .map(|expression| convert_expression(source, expression))
            .transpose()?,
        body: convert_if_branch(source, &for_statement.body)?,
        span: span(for_statement.span),
    })
}

/// Convert a `for (… of …)` statement, with an identifier or destructuring
/// pattern binding.
fn convert_for_of(
    source: &str,
    for_of_statement: &oxc_ast::ast::ForOfStatement<'_>,
) -> Result<Stmt, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    if for_of_statement.r#await {
        return Err("OXC adapter does not support await in for-of loops yet".into());
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
    Ok(Stmt::ForOf {
        pattern,
        iterable: convert_expression(source, &for_of_statement.right)?,
        body: convert_if_branch(source, &for_of_statement.body)?,
        span: span(for_of_statement.span),
    })
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
        let mut statements = Vec::new();
        for statement in &block.body {
            statements.extend(convert_if_branch(source, statement)?.statements);
        }
        Ok(Block {
            statements,
            span: Span::new(block.span.start, block.span.end),
        })
    }

    let catches = if let Some(handler) = try_statement.handler.as_ref() {
        let Some(parameter) = handler.param.as_ref() else {
            return Err("OXC adapter requires an identifier catch binding".into());
        };
        let BindingPattern::BindingIdentifier(identifier) = &parameter.pattern else {
            return Err("OXC adapter requires an identifier catch binding".into());
        };
        vec![crate::ast::CatchClause {
            binding: Name {
                span: Span::new(identifier.span.start, identifier.span.end),
            },
            ty: crate::ast::TypeAnnotation {
                name: Name {
                    span: Span::new(0, 0),
                },
                is_array: false,
            },
            body: convert_block_node(source, &handler.body)?,
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
        Expression::Identifier(identifier) => Ok(Expr::Ident(Name {
            span: span(identifier.span),
        })),
        Expression::ThisExpression(this_expression) => Ok(Expr::Ident(Name {
            span: span(this_expression.span),
        })),
        Expression::ParenthesizedExpression(parenthesized) => {
            convert_expression(source, &parenthesized.expression)
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
        Expression::ChainExpression(chain) => match &chain.expression {
            ChainElement::StaticMemberExpression(member) => Ok(Expr::OptionalField {
                object: Box::new(convert_expression(source, &member.object)?),
                field: Name {
                    span: span(member.property.span),
                },
                span: span(chain.span),
            }),
            ChainElement::ComputedMemberExpression(member) => Ok(Expr::OptionalIndex {
                object: Box::new(convert_expression(source, &member.object)?),
                index: Box::new(convert_expression(source, &member.expression)?),
                span: span(chain.span),
            }),
            ChainElement::CallExpression(call) => {
                let mut args = Vec::with_capacity(call.arguments.len());
                for argument in &call.arguments {
                    args.push(convert_argument(source, argument)?);
                }
                if let Expression::StaticMemberExpression(member) = &call.callee {
                    return Ok(Expr::OptionalMethodCall {
                        object: Box::new(convert_expression(source, &member.object)?),
                        method: Name { span: span(member.property.span) },
                        args,
                        span: span(chain.span),
                    });
                }
                Ok(Expr::OptionalCall {
                    callee: Box::new(convert_expression(source, &call.callee)?),
                    args,
                    span: span(chain.span),
                })
            }
            _ => Err("OXC adapter currently supports optional static members only".into()),
        },
        Expression::BinaryExpression(binary) => {
            let op = match binary.operator {
                BinaryOperator::Equality | BinaryOperator::StrictEquality => BinaryOp::Eq,
                BinaryOperator::Inequality | BinaryOperator::StrictInequality => BinaryOp::Ne,
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
                _ => {
                    return Err("OXC adapter does not support logical assignments yet".into());
                }
            };
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
                AssignmentTarget::ComputedMemberExpression(member) => Ok(Expr::IndexAssign {
                    object: Box::new(convert_expression(source, &member.object)?),
                    index: Box::new(convert_expression(source, &member.expression)?),
                    value,
                    span: span(assignment.span),
                }),
                AssignmentTarget::PrivateFieldExpression(member) => Ok(Expr::PrivateFieldAssign {
                    object: Box::new(convert_expression(source, &member.object)?),
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
            match &call.callee {
                Expression::Identifier(identifier) => Ok(Expr::Call {
                    callee: Name {
                        span: span(identifier.span),
                    },
                    args,
                    span: span(call.span),
                }),
                Expression::StaticMemberExpression(member) => {
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
            let Expression::Identifier(identifier) = &new_expression.callee else {
                return Err("OXC adapter supports identifier constructors only".into());
            };
            let mut args = Vec::with_capacity(new_expression.arguments.len());
            for argument in &new_expression.arguments {
                args.push(convert_argument(source, argument)?);
            }
            Ok(Expr::New {
                class: Name {
                    span: span(identifier.span),
                },
                args,
                span: span(new_expression.span),
            })
        }
        Expression::ArrowFunctionExpression(arrow) => convert_arrow(source, arrow),
        _ => {
            Err("OXC adapter currently supports literal, identifier, and binary expressions".into())
        }
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
    for parameter in &arrow.params.items {
        let BindingPattern::BindingIdentifier(identifier) = &parameter.pattern else {
            return Err("OXC adapter supports identifier arrow parameters only".into());
        };
        params.push(crate::ast::Param {
            name: Name { span: span(identifier.span) },
            ty: convert_type_annotation(source, parameter.type_annotation.as_deref())?,
            default_value: parameter
                .initializer
                .as_ref()
                .map(|initializer| convert_expression(source, initializer))
                .transpose()?
                .map(Box::new),
            is_rest: false,
        });
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
    // (`=> { … }`) converts its statement list.
    let body = if let Some(expression) = arrow.get_expression() {
        Block {
            statements: vec![Stmt::Return {
                value: Some(convert_expression(source, expression)?),
                span: span(expression.span()),
            }],
            span: span(arrow.span),
        }
    } else if let oxc_ast::ast::ArrowFunctionBody::FunctionBody(function_body) = &arrow.body {
        Block {
            statements: convert_statement_list(source, &function_body.statements)?,
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

    let mut functions = Vec::new();
    let mut imports = Vec::new();
    let mut classes = Vec::new();
    let mut top_level = Vec::new();
    for statement in parsed.program.body {
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
            let Some(id) = class_declaration.id.as_ref() else {
                return Err(vec!["OXC adapter requires named classes".into()]);
            };
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
                        let Some(type_annotation) =
                            convert_type_annotation(source, property.type_annotation.as_deref())
                                .map_err(|error| vec![error])?
                        else {
                            return Err(vec![
                                "OXC adapter requires class fields to have named types".into(),
                            ]);
                        };
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
                            initializer: property
                                .value
                                .as_ref()
                                .map(|expression| convert_expression(source, expression))
                                .transpose()
                                .map_err(|error| vec![error])?
                                .map(Box::new),
                        });
                    }
                    ClassElement::MethodDefinition(method) => {
                        // A computed key (`[Symbol.iterator]() {}`) is lowered
                        // to a Symbol-keyed side-table entry; a plain name is a
                        // vtable method. `name_span` carries the key span in the
                        // computed case (used only for diagnostics).
                        let (name_span, computed_key) = if method.computed {
                            let key_expression = method.key.as_expression().ok_or_else(|| {
                                vec!["OXC adapter requires an expression computed method key".to_owned()]
                            })?;
                            let key = convert_expression(source, key_expression)
                                .map_err(|error| vec![error])?;
                            (key.span(), Some(key))
                        } else if let PropertyKey::StaticIdentifier(identifier) = &method.key {
                            (Span::new(identifier.span.start, identifier.span.end), None)
                        } else {
                            return Err(vec![
                                "OXC adapter supports identifier class method names only".into(),
                            ]);
                        };
                        let Some(body) = method.value.body.as_ref() else {
                            return Err(vec!["OXC adapter requires class method bodies".into()]);
                        };
                        let mut method_statements = Vec::with_capacity(body.statements.len());
                        for statement in &body.statements {
                            match statement {
                                Statement::EmptyStatement(_) | Statement::DebuggerStatement(_) => {}
                                Statement::VariableDeclaration(declaration) => {
                                    for declarator in &declaration.declarations {
                                        let BindingPattern::BindingIdentifier(identifier) =
                                            &declarator.id
                                        else {
                                            return Err(vec![
                                                "OXC adapter currently supports identifier local bindings".into(),
                                            ]);
                                        };
                                        let Some(initializer) = declarator.init.as_ref() else {
                                            return Err(vec![
                                                "OXC adapter requires an initializer for local bindings".into(),
                                            ]);
                                        };
                                        method_statements.push(Stmt::Let {
                                            name: Name {
                                                span: Span::new(
                                                    identifier.span.start,
                                                    identifier.span.end,
                                                ),
                                            },
                                            ty: convert_type_annotation(
                                                source,
                                                declarator.type_annotation.as_deref(),
                                            )
                                            .map_err(|error| vec![error])?,
                                            init: convert_expression(source, initializer)
                                                .map_err(|error| vec![error])?,
                                            span: Span::new(
                                                declarator.span.start,
                                                declarator.span.end,
                                            ),
                                        });
                                    }
                                }
                                Statement::ReturnStatement(return_statement) => {
                                    method_statements.push(Stmt::Return {
                                        value: return_statement
                                            .argument
                                            .as_ref()
                                            .map(|expression| convert_expression(source, expression))
                                            .transpose()
                                            .map_err(|error| vec![error])?,
                                        span: Span::new(
                                            return_statement.span.start,
                                            return_statement.span.end,
                                        ),
                                    });
                                }
                                Statement::ExpressionStatement(expression_statement) => {
                                    method_statements.push(Stmt::Expr(
                                        convert_expression(source, &expression_statement.expression)
                                            .map_err(|error| vec![error])?,
                                    ));
                                }
                                Statement::BreakStatement(break_statement) => {
                                    method_statements.push(Stmt::Break {
                                        label: break_statement.label.as_ref().map(|label| Name {
                                            span: Span::new(label.span.start, label.span.end),
                                        }),
                                        span: Span::new(
                                            break_statement.span.start,
                                            break_statement.span.end,
                                        ),
                                    });
                                }
                                Statement::ContinueStatement(continue_statement) => {
                                    method_statements.push(Stmt::Continue {
                                        label: continue_statement.label.as_ref().map(|label| Name {
                                            span: Span::new(label.span.start, label.span.end),
                                        }),
                                        span: Span::new(
                                            continue_statement.span.start,
                                            continue_statement.span.end,
                                        ),
                                    });
                                }
                                Statement::ThrowStatement(throw_statement) => {
                                    method_statements.push(Stmt::Throw {
                                        value: convert_expression(source, &throw_statement.argument)
                                            .map_err(|error| vec![error])?,
                                        span: Span::new(
                                            throw_statement.span.start,
                                            throw_statement.span.end,
                                        ),
                                    });
                                }
                                Statement::IfStatement(if_statement) => {
                                    method_statements.push(
                                        convert_if_statement(source, if_statement)
                                            .map_err(|error| vec![error])?,
                                    );
                                }
                                Statement::TryStatement(try_statement) => {
                                    method_statements.push(
                                        convert_try_statement(source, try_statement)
                                            .map_err(|error| vec![error])?,
                                    );
                                }
                                Statement::WhileStatement(while_statement) => {
                                    method_statements.push(Stmt::While {
                                        cond: convert_expression(source, &while_statement.test)
                                            .map_err(|error| vec![error])?,
                                        body: convert_if_branch(source, &while_statement.body)
                                            .map_err(|error| vec![error])?,
                                        span: Span::new(
                                            while_statement.span.start,
                                            while_statement.span.end,
                                        ),
                                    });
                                }
                                Statement::DoWhileStatement(do_while_statement) => {
                                    method_statements.push(Stmt::DoWhile {
                                        body: convert_if_branch(source, &do_while_statement.body)
                                            .map_err(|error| vec![error])?,
                                        cond: convert_expression(source, &do_while_statement.test)
                                            .map_err(|error| vec![error])?,
                                        span: Span::new(
                                            do_while_statement.span.start,
                                            do_while_statement.span.end,
                                        ),
                                    });
                                }
                                Statement::ForStatement(_) => {
                                    let nested = convert_if_branch(source, statement)
                                        .map_err(|error| vec![error])?;
                                    method_statements.extend(nested.statements);
                                }
                                Statement::ForOfStatement(_) => {
                                    let nested = convert_if_branch(source, statement)
                                        .map_err(|error| vec![error])?;
                                    method_statements.extend(nested.statements);
                                }
                                _ => return Err(vec![
                                    "OXC adapter class methods currently support local, conditional, loop-control, throw, return, expression, and loop statements".into(),
                                ]),
                            }
                        }
                        let mut params = Vec::with_capacity(method.value.params.items.len());
                        for parameter in &method.value.params.items {
                            let (identifier_span, is_rest) = match &parameter.pattern {
                                BindingPattern::BindingIdentifier(identifier) => {
                                    (identifier.span, false)
                                }
                                _ => {
                                    return Err(vec![
                                    "OXC adapter supports identifier class method parameters only"
                                        .into(),
                                ])
                                }
                            };
                            params.push(crate::ast::Param {
                                name: Name {
                                    span: Span::new(identifier_span.start, identifier_span.end),
                                },
                                ty: convert_type_annotation(
                                    source,
                                    parameter.type_annotation.as_deref(),
                                )
                                .map_err(|error| vec![error])?,
                                default_value: parameter
                                    .initializer
                                    .as_ref()
                                    .map(|initializer| convert_expression(source, initializer))
                                    .transpose()
                                    .map_err(|error| vec![error])?
                                    .map(Box::new),
                                is_rest,
                            });
                        }
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
                            is_static: method.r#static || computed_key.is_some(),
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
                            None => methods.push(function),
                        }
                    }
                    _ => {
                        return Err(vec![
                            "OXC adapter does not support this class element yet".into()
                        ])
                    }
                }
            }
            classes.push(crate::ast::Class {
                name: Name {
                    span: Span::new(id.span.start, id.span.end),
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
        let mut statements = Vec::with_capacity(body.statements.len());
        for statement in &body.statements {
            match statement {
                Statement::EmptyStatement(_) | Statement::DebuggerStatement(_) => {}
                Statement::VariableDeclaration(declaration) => {
                    for declarator in &declaration.declarations {
                        statements.push(
                            convert_declarator(source, declarator).map_err(|error| vec![error])?,
                        );
                    }
                }
                Statement::ReturnStatement(return_statement) => {
                    let value = return_statement
                        .argument
                        .as_ref()
                        .map(|expression| convert_expression(source, expression))
                        .transpose()
                        .map_err(|error| vec![error])?;
                    statements.push(Stmt::Return {
                        value,
                        span: span(return_statement.span),
                    });
                }
                Statement::ExpressionStatement(expression_statement) => {
                    statements.push(Stmt::Expr(
                        convert_expression(source, &expression_statement.expression)
                            .map_err(|error| vec![error])?,
                    ));
                }
                Statement::BreakStatement(break_statement) => {
                    statements.push(Stmt::Break {
                        label: break_statement.label.as_ref().map(|label| Name {
                            span: span(label.span),
                        }),
                        span: span(break_statement.span),
                    });
                }
                Statement::ContinueStatement(continue_statement) => {
                    statements.push(Stmt::Continue {
                        label: continue_statement.label.as_ref().map(|label| Name {
                            span: span(label.span),
                        }),
                        span: span(continue_statement.span),
                    });
                }
                Statement::ThrowStatement(throw_statement) => {
                    statements.push(Stmt::Throw {
                        value: convert_expression(source, &throw_statement.argument)
                            .map_err(|error| vec![error])?,
                        span: span(throw_statement.span),
                    });
                }
                Statement::IfStatement(if_statement) => {
                    statements.push(convert_if_statement(source, if_statement).map_err(|error| vec![error])?);
                }
                Statement::TryStatement(try_statement) => {
                    statements.push(convert_try_statement(source, try_statement).map_err(|error| vec![error])?);
                }
                Statement::WhileStatement(while_statement) => {
                    let body = convert_block_statement(source, &while_statement.body)
                        .map_err(|error| vec![error])?;
                    statements.push(Stmt::While {
                        cond: convert_expression(source, &while_statement.test)
                            .map_err(|error| vec![error])?,
                        body,
                        span: span(while_statement.span),
                    });
                }
                Statement::ForStatement(for_statement) => {
                    let init = match &for_statement.init {
                        None => None,
                        Some(ForStatementInit::VariableDeclaration(declaration)) => {
                            if declaration.declarations.len() != 1 {
                                return Err(vec![
                                    "OXC adapter supports one for-loop initializer binding".into(),
                                ]);
                            }
                            let declarator = &declaration.declarations[0];
                            let BindingPattern::BindingIdentifier(identifier) = &declarator.id
                            else {
                                return Err(vec![
                                    "OXC adapter supports identifier for-loop bindings".into(),
                                ]);
                            };
                            let Some(initializer) = declarator.init.as_ref() else {
                                return Err(vec![
                                    "OXC adapter requires an initializer for for-loop bindings".into(),
                                ]);
                            };
                            Some(Box::new(Stmt::Let {
                                name: Name {
                                    span: span(identifier.span),
                                },
                                ty: convert_type_annotation(
                                    source,
                                    declarator.type_annotation.as_deref(),
                                )
                                .map_err(|error| vec![error])?,
                                init: convert_expression(source, initializer)
                                    .map_err(|error| vec![error])?,
                                span: span(declarator.span),
                            }))
                        }
                        Some(init) => {
                            let Some(expression) = init.as_expression() else {
                                return Err(vec![
                                    "OXC adapter does not support this for-loop initializer yet".into(),
                                ]);
                            };
                            Some(Box::new(Stmt::Expr(
                                convert_expression(source, expression).map_err(|error| vec![error])?,
                            )))
                        }
                    };
                    let body = convert_block_statement(source, &for_statement.body)
                        .map_err(|error| vec![error])?;
                    statements.push(Stmt::For {
                        init,
                        cond: for_statement
                            .test
                            .as_ref()
                            .map(|expression| convert_expression(source, expression))
                            .transpose()
                            .map_err(|error| vec![error])?,
                        update: for_statement
                            .update
                            .as_ref()
                            .map(|expression| convert_expression(source, expression))
                            .transpose()
                            .map_err(|error| vec![error])?,
                        body,
                        span: span(for_statement.span),
                    });
                }
                Statement::ForOfStatement(for_of_statement) => {
                    if for_of_statement.r#await {
                        return Err(vec![
                            "OXC adapter does not support await in for-of loops yet".into(),
                        ]);
                    }
                    let ForStatementLeft::VariableDeclaration(declaration) = &for_of_statement.left
                    else {
                        return Err(vec![
                            "OXC adapter supports identifier for-of bindings only".into(),
                        ]);
                    };
                    if declaration.declarations.len() != 1 {
                        return Err(vec![
                            "OXC adapter supports one for-of binding".into(),
                        ]);
                    }
                    let declarator = &declaration.declarations[0];
                    let pattern = match &declarator.id {
                        BindingPattern::BindingIdentifier(identifier) => crate::ast::Pattern::Ident {
                            name: Name { span: span(identifier.span) },
                            ty: convert_type_annotation(source, declarator.type_annotation.as_deref())
                                .map_err(|error| vec![error])?,
                        },
                        other => {
                            convert_binding_pattern(source, other).map_err(|error| vec![error])?
                        }
                    };
                    let body = convert_block_statement(source, &for_of_statement.body)
                        .map_err(|error| vec![error])?;
                    statements.push(Stmt::ForOf {
                        pattern,
                        iterable: convert_expression(source, &for_of_statement.right)
                            .map_err(|error| vec![error])?,
                        body,
                        span: span(for_of_statement.span),
                    });
                }
                Statement::DoWhileStatement(do_while_statement) => {
                    let body = convert_block_statement(source, &do_while_statement.body)
                        .map_err(|error| vec![error])?;
                    statements.push(Stmt::DoWhile {
                        body,
                        cond: convert_expression(source, &do_while_statement.test)
                            .map_err(|error| vec![error])?,
                        span: span(do_while_statement.span),
                    });
                }
                _ => {
                    return Err(vec![
                        "OXC adapter currently supports local declarations, conditionals, loops, break, continue, throw, return, and expression statements".into(),
                    ])
                }
            }
        }
        let mut params = Vec::with_capacity(function.params.items.len());
        for parameter in &function.params.items {
            let (identifier_span, is_rest) = match &parameter.pattern {
                BindingPattern::BindingIdentifier(identifier) => (identifier.span, false),
                _ => {
                    return Err(vec![
                        "OXC adapter currently supports identifier parameters".into()
                    ])
                }
            };
            params.push(crate::ast::Param {
                name: Name {
                    span: span(identifier_span),
                },
                ty: convert_type_annotation(source, parameter.type_annotation.as_deref())
                    .map_err(|error| vec![error])?,
                default_value: parameter
                    .initializer
                    .as_ref()
                    .map(|initializer| convert_expression(source, initializer))
                    .transpose()
                    .map_err(|error| vec![error])?
                    .map(Box::new),
                is_rest,
            });
        }
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
            crate::ast::Stmt::Expr(crate::ast::Expr::PrivateFieldAssign { .. })
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
