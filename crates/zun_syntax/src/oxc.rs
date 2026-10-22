//! Native OXC parsing boundary for standard JavaScript and TypeScript.
//!
//! The existing parser remains authoritative for Zun-specific syntax while the
//! HIR adapter is built incrementally. This boundary lets callers validate
//! standard JS/TS with OXC without coupling the rest of the compiler to OXC's
//! arena-owned AST types.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, AssignmentOperator, AssignmentTarget, BinaryOperator, BindingPattern, ChainElement,
    Declaration, ExportDefaultDeclarationKind,
    Expression,
    ClassElement, ForStatementInit, ForStatementLeft, ImportDeclarationSpecifier, LogicalOperator,
    MethodDefinitionKind, ModuleExportName, PropertyKey, Statement,
    UnaryOperator,
};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use zun_common::Span;

use crate::ast::{BinaryOp, Block, Expr, Function, Name, Program, Stmt};

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
    let type_text = raw[type_start..].trim();
    if type_text.is_empty()
        || !type_text
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err("OXC adapter supports named TypeScript types only".into());
    }
    let name_start = start + type_start;
    Ok(Some(crate::ast::TypeAnnotation {
        name: Name {
            span: Span::new(name_start as u32, (name_start + type_text.len()) as u32),
        },
    }))
}

fn convert_argument(argument: &Argument<'_>) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    match argument {
        Argument::NumericLiteral(number) => Ok(Expr::Number {
            value: number.value,
            span: span(number.span),
        }),
        Argument::StringLiteral(string) => {
            let Some(value) = string.value.as_str() else {
                return Err("OXC adapter cannot represent non-UTF-8 string literals".into());
            };
            Ok(Expr::String {
                value: value.to_owned(),
                span: span(string.span),
            })
        }
        Argument::BooleanLiteral(boolean) => Ok(Expr::Bool {
            value: boolean.value,
            span: span(boolean.span),
        }),
        Argument::Identifier(identifier) => Ok(Expr::Ident(Name {
            span: span(identifier.span),
        })),
        Argument::SpreadElement(spread) => Ok(Expr::Spread {
            expr: Box::new(convert_expression(&spread.argument)?),
            span: span(spread.span),
        }),
        _ => Err("OXC adapter currently supports literal, identifier, and spread arguments".into()),
    }
}

fn convert_block_statement(statement: &Statement<'_>) -> Result<Block, String> {
    let Statement::BlockStatement(block) = statement else {
        return Err("OXC adapter requires block branches for conditional statements".into());
    };
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    let mut statements = Vec::with_capacity(block.body.len());
    for statement in &block.body {
        match statement {
            Statement::ReturnStatement(return_statement) => statements.push(Stmt::Return {
                value: return_statement
                    .argument
                    .as_ref()
                    .map(convert_expression)
                    .transpose()?,
                span: span(return_statement.span),
            }),
            Statement::ExpressionStatement(expression_statement) => statements.push(Stmt::Expr(
                convert_expression(&expression_statement.expression)?,
            )),
            Statement::BreakStatement(break_statement) => statements.push(Stmt::Break {
                label: break_statement.label.as_ref().map(|label| Name {
                    span: span(label.span),
                }),
                span: span(break_statement.span),
            }),
            Statement::ContinueStatement(continue_statement) => {
                statements.push(Stmt::Continue {
                    label: continue_statement.label.as_ref().map(|label| Name {
                        span: span(label.span),
                    }),
                    span: span(continue_statement.span),
                })
            }
            Statement::ThrowStatement(throw_statement) => statements.push(Stmt::Throw {
                value: convert_expression(&throw_statement.argument)?,
                span: span(throw_statement.span),
            }),
            Statement::IfStatement(if_statement) => {
                let then_block = convert_block_statement(&if_statement.consequent)?;
                let else_block = if_statement
                    .alternate
                    .as_ref()
                    .map(convert_block_statement)
                    .transpose()?;
                statements.push(Stmt::If {
                    cond: convert_expression(&if_statement.test)?,
                    then_block,
                    else_block,
                    span: span(if_statement.span),
                });
            }
            _ => {
                return Err(
                    "OXC adapter currently supports return and expression statements in branches"
                        .into(),
                )
            }
        }
    }
    Ok(Block {
        statements,
        span: span(block.span),
    })
}

fn convert_expression(expression: &Expression<'_>) -> Result<Expr, String> {
    let span = |span: oxc_span::Span| Span::new(span.start, span.end);
    match expression {
        Expression::NumericLiteral(number) => Ok(Expr::Number {
            value: number.value,
            span: span(number.span),
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
        Expression::TemplateLiteral(template) => {
            let mut parts = Vec::with_capacity(template.quasis.len() + template.expressions.len());
            for (index, quasi) in template.quasis.iter().enumerate() {
                let text = quasi
                    .value
                    .cooked
                    .as_ref()
                    .and_then(|value| value.as_str())
                    .or_else(|| Some(quasi.value.raw.as_str()))
                    .ok_or_else(|| "OXC adapter cannot represent this template segment".to_owned())?;
                parts.push(crate::ast::TemplatePart::Text(text.to_owned()));
                if let Some(expression) = template.expressions.get(index) {
                    parts.push(crate::ast::TemplatePart::Expr(Box::new(
                        convert_expression(expression)?,
                    )));
                }
            }
            Ok(Expr::Template {
                parts,
                span: span(template.span),
            })
        }
        Expression::Identifier(identifier) => Ok(Expr::Ident(Name {
            span: span(identifier.span),
        })),
        Expression::ThisExpression(this_expression) => Ok(Expr::Ident(Name {
            span: span(this_expression.span),
        })),
        Expression::ParenthesizedExpression(parenthesized) => {
            convert_expression(&parenthesized.expression)
        }
        Expression::TSAsExpression(assertion) => Ok(Expr::Cast {
            expr: Box::new(convert_expression(&assertion.expression)?),
            ty: crate::ast::TypeAnnotation {
                name: Name {
                    span: Span::new(
                        assertion.type_annotation.span().start,
                        assertion.type_annotation.span().end,
                    ),
                },
            },
            span: span(assertion.span),
        }),
        Expression::TSTypeAssertion(assertion) => Ok(Expr::Cast {
            expr: Box::new(convert_expression(&assertion.expression)?),
            ty: crate::ast::TypeAnnotation {
                name: Name {
                    span: Span::new(
                        assertion.type_annotation.span().start,
                        assertion.type_annotation.span().end,
                    ),
                },
            },
            span: span(assertion.span),
        }),
        Expression::TSSatisfiesExpression(assertion) => Ok(Expr::Cast {
            expr: Box::new(convert_expression(&assertion.expression)?),
            ty: crate::ast::TypeAnnotation {
                name: Name {
                    span: Span::new(
                        assertion.type_annotation.span().start,
                        assertion.type_annotation.span().end,
                    ),
                },
            },
            span: span(assertion.span),
        }),
        Expression::TSNonNullExpression(assertion) => {
            convert_expression(&assertion.expression)
        }
        Expression::StaticMemberExpression(member) => {
            let object = Box::new(convert_expression(&member.object)?);
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
            let object = Box::new(convert_expression(&member.object)?);
            let index = Box::new(convert_expression(&member.expression)?);
            if member.optional {
                return Err("OXC adapter does not support optional computed members yet".into());
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
                object: Box::new(convert_expression(&member.object)?),
                field: Name {
                    span: span(member.field.span),
                },
                span: span(member.span),
            })
        }
        Expression::ChainExpression(chain) => match &chain.expression {
            ChainElement::StaticMemberExpression(member) => Ok(Expr::OptionalField {
                object: Box::new(convert_expression(&member.object)?),
                field: Name {
                    span: span(member.property.span),
                },
                span: span(chain.span),
            }),
            ChainElement::CallExpression(call) => {
                let Expression::StaticMemberExpression(member) = &call.callee else {
                    return Err("OXC adapter supports optional static-member calls only".into());
                };
                let mut args = Vec::with_capacity(call.arguments.len());
                for argument in &call.arguments {
                    args.push(convert_argument(argument)?);
                }
                Ok(Expr::OptionalMethodCall {
                    object: Box::new(convert_expression(&member.object)?),
                    method: Name {
                        span: span(member.property.span),
                    },
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
                _ => return Err("OXC adapter does not support this binary operator yet".into()),
            };
            Ok(Expr::Binary {
                op,
                lhs: Box::new(convert_expression(&binary.left)?),
                rhs: Box::new(convert_expression(&binary.right)?),
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
                lhs: Box::new(convert_expression(&logical.left)?),
                rhs: Box::new(convert_expression(&logical.right)?),
                span: span(logical.span),
            })
        }
        Expression::ConditionalExpression(conditional) => Ok(Expr::Conditional {
            condition: Box::new(convert_expression(&conditional.test)?),
            when_true: Box::new(convert_expression(&conditional.consequent)?),
            when_false: Box::new(convert_expression(&conditional.alternate)?),
            span: span(conditional.span),
        }),
        Expression::UnaryExpression(unary) => {
            let op = match unary.operator {
                UnaryOperator::UnaryNegation => crate::ast::UnaryOp::Neg,
                UnaryOperator::LogicalNot => crate::ast::UnaryOp::Not,
                _ => return Err("OXC adapter does not support this unary operator yet".into()),
            };
            Ok(Expr::Unary {
                op,
                operand: Box::new(convert_expression(&unary.argument)?),
                span: span(unary.span),
            })
        }
        Expression::AwaitExpression(await_expression) => Ok(Expr::Await {
            operand: Box::new(convert_expression(&await_expression.argument)?),
            span: span(await_expression.span),
        }),
        Expression::YieldExpression(yield_expression) => {
            if yield_expression.delegate {
                return Err("OXC adapter does not support delegated yield yet".into());
            }
            Ok(Expr::Yield {
                value: Box::new(match yield_expression.argument.as_ref() {
                    Some(argument) => convert_expression(argument)?,
                    None => Expr::Null {
                        span: span(yield_expression.span),
                    },
                }),
                span: span(yield_expression.span),
            })
        }
        Expression::AssignmentExpression(assignment) => {
            if assignment.operator != AssignmentOperator::Assign {
                return Err("OXC adapter currently supports only simple assignments".into());
            }
            let value = Box::new(convert_expression(&assignment.right)?);
            match &assignment.left {
                AssignmentTarget::AssignmentTargetIdentifier(identifier) => Ok(Expr::Assign {
                    target: Name {
                        span: span(identifier.span),
                    },
                    value,
                    span: span(assignment.span),
                }),
                AssignmentTarget::StaticMemberExpression(member) => Ok(Expr::FieldAssign {
                    object: Box::new(convert_expression(&member.object)?),
                    field: Name {
                        span: span(member.property.span),
                    },
                    value,
                    span: span(assignment.span),
                }),
                AssignmentTarget::ComputedMemberExpression(member) => Ok(Expr::IndexAssign {
                    object: Box::new(convert_expression(&member.object)?),
                    index: Box::new(convert_expression(&member.expression)?),
                    value,
                    span: span(assignment.span),
                }),
                AssignmentTarget::PrivateFieldExpression(member) => Ok(Expr::PrivateFieldAssign {
                    object: Box::new(convert_expression(&member.object)?),
                    field: Name {
                        span: span(member.field.span),
                    },
                    value,
                    span: span(assignment.span),
                }),
                _ => Err("OXC adapter supports identifier, field, and index assignments only".into()),
            }
        }
        Expression::CallExpression(call) => {
            let mut args = Vec::with_capacity(call.arguments.len());
            for argument in &call.arguments {
                args.push(convert_argument(argument)?);
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
                    let receiver = Box::new(convert_expression(&member.object)?);
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
                _ => Err("OXC adapter currently supports identifier and static-member callees only".into()),
            }
        }
        Expression::NewExpression(new_expression) => {
            let Expression::Identifier(identifier) = &new_expression.callee else {
                return Err("OXC adapter supports identifier constructors only".into());
            };
            let mut args = Vec::with_capacity(new_expression.arguments.len());
            for argument in &new_expression.arguments {
                args.push(convert_argument(argument)?);
            }
            Ok(Expr::New {
                class: Name {
                    span: span(identifier.span),
                },
                args,
                span: span(new_expression.span),
            })
        }
        _ => Err("OXC adapter currently supports literal, identifier, and binary expressions".into()),
    }
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
    for statement in parsed.program.body {
        if let Statement::ImportDeclaration(import_declaration) = statement {
            let Some(module) = import_declaration.source.value.as_str() else {
                return Err(vec!["OXC adapter cannot represent non-UTF-8 import sources".into()]);
            };
            let mut names = Vec::new();
            if let Some(specifiers) = import_declaration.specifiers.as_deref() {
                for specifier in specifiers {
                match specifier {
                    ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                        let ModuleExportName::IdentifierName(imported) = &specifier.imported else {
                            return Err(vec![
                                "OXC adapter supports identifier named imports only".into(),
                            ]);
                        };
                        names.push(crate::ast::ImportName {
                            imported: Name {
                                span: Span::new(imported.span.start, imported.span.end),
                            },
                            local: Name {
                                span: Span::new(specifier.local.span.start, specifier.local.span.end),
                            },
                            is_default: false,
                        });
                    }
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                        names.push(crate::ast::ImportName {
                            imported: Name {
                                span: Span::new(specifier.local.span.start, specifier.local.span.end),
                            },
                            local: Name {
                                span: Span::new(specifier.local.span.start, specifier.local.span.end),
                            },
                            is_default: true,
                        });
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {
                        return Err(vec![
                            "OXC adapter does not support namespace imports yet".into(),
                        ]);
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
            let base = class_declaration.heritage.as_ref().map(|heritage| {
                let Expression::Identifier(identifier) = &heritage.expression else {
                    return Err("OXC adapter supports identifier class bases only".into());
                };
                Ok(Name {
                    span: Span::new(identifier.span.start, identifier.span.end),
                })
            }).transpose().map_err(|error| vec![error])?;
            let mut fields = Vec::new();
            let mut methods = Vec::new();
            for element in &class_declaration.body.body {
                match element {
                    ClassElement::PropertyDefinition(property) => {
                        if property.value.is_some() {
                            return Err(vec![
                                "OXC adapter does not support initialized class fields yet".into(),
                            ]);
                        }
                        let (field_span, is_private) = match &property.key {
                            PropertyKey::StaticIdentifier(identifier) => {
                                (Span::new(identifier.span.start, identifier.span.end), false)
                            }
                            PropertyKey::PrivateIdentifier(identifier) => {
                                (Span::new(identifier.span.start, identifier.span.end), true)
                            }
                            _ => return Err(vec![
                                "OXC adapter supports identifier class fields only".into(),
                            ]),
                        };
                        let Some(type_annotation) = convert_type_annotation(
                            source,
                            property.type_annotation.as_deref(),
                        )
                        .map_err(|error| vec![error])?
                        else {
                            return Err(vec![
                                "OXC adapter requires class fields to have named types".into(),
                            ]);
                        };
                        fields.push(crate::ast::ClassField {
                            name: Name { span: field_span },
                            ty: type_annotation,
                            is_static: property.r#static,
                            is_private,
                        });
                    }
                    ClassElement::MethodDefinition(method) => {
                        let PropertyKey::StaticIdentifier(identifier) = &method.key else {
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
                                Statement::ReturnStatement(return_statement) => {
                                    method_statements.push(Stmt::Return {
                                        value: return_statement
                                            .argument
                                            .as_ref()
                                            .map(convert_expression)
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
                                        convert_expression(&expression_statement.expression)
                                            .map_err(|error| vec![error])?,
                                    ));
                                }
                                _ => return Err(vec![
                                    "OXC adapter class methods currently support return and expression statements".into(),
                                ]),
                            }
                        }
                        let mut params = Vec::with_capacity(method.value.params.items.len());
                        for parameter in &method.value.params.items {
                            let (identifier_span, is_rest) = match &parameter.pattern {
                                BindingPattern::BindingIdentifier(identifier) => {
                                    (identifier.span, false)
                                }
                                _ => return Err(vec![
                                    "OXC adapter supports identifier class method parameters only".into(),
                                ]),
                            };
                            params.push(crate::ast::Param {
                                name: Name {
                                    span: Span::new(identifier_span.start, identifier_span.end),
                                },
                                ty: convert_type_annotation(source, parameter.type_annotation.as_deref())
                                .map_err(|error| vec![error])?,
                                default_value: parameter
                                    .initializer
                                    .as_ref()
                                    .map(|initializer| convert_expression(initializer))
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
                        methods.push(Function {
                            name: Name {
                                span: Span::new(identifier.span.start, identifier.span.end),
                            },
                            params,
                            return_type: convert_type_annotation(
                                source,
                                method.value.return_type.as_deref(),
                            )
                            .map_err(|error| vec![error])?,
                            is_extern: false,
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
                        });
                    }
                    _ => return Err(vec!["OXC adapter does not support this class element yet".into()]),
                }
            }
            classes.push(crate::ast::Class {
                name: Name {
                    span: Span::new(id.span.start, id.span.end),
                },
                base,
                fields,
                methods,
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
            return Err(vec!["OXC adapter does not support this top-level declaration yet".into()]);
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
                Statement::VariableDeclaration(declaration) => {
                    for declarator in &declaration.declarations {
                        let BindingPattern::BindingIdentifier(identifier) = &declarator.id else {
                            return Err(vec![
                                "OXC adapter currently supports identifier local bindings".into(),
                            ]);
                        };
                        let Some(initializer) = declarator.init.as_ref() else {
                            return Err(vec![
                                "OXC adapter requires an initializer for local bindings".into(),
                            ]);
                        };
                        statements.push(Stmt::Let {
                            name: Name {
                                span: span(identifier.span),
                            },
                            ty: convert_type_annotation(source, declarator.type_annotation.as_deref())
                                .map_err(|error| vec![error])?,
                            init: convert_expression(initializer).map_err(|error| vec![error])?,
                            span: span(declarator.span),
                        });
                    }
                }
                Statement::ReturnStatement(return_statement) => {
                    let value = return_statement
                        .argument
                        .as_ref()
                        .map(convert_expression)
                        .transpose()
                        .map_err(|error| vec![error])?;
                    statements.push(Stmt::Return {
                        value,
                        span: span(return_statement.span),
                    });
                }
                Statement::ExpressionStatement(expression_statement) => {
                    statements.push(Stmt::Expr(
                        convert_expression(&expression_statement.expression)
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
                        value: convert_expression(&throw_statement.argument)
                            .map_err(|error| vec![error])?,
                        span: span(throw_statement.span),
                    });
                }
                Statement::IfStatement(if_statement) => {
                    let then_block = convert_block_statement(&if_statement.consequent)
                        .map_err(|error| vec![error])?;
                    let else_block = if_statement
                        .alternate
                        .as_ref()
                        .map(convert_block_statement)
                        .transpose()
                        .map_err(|error| vec![error])?;
                    statements.push(Stmt::If {
                        cond: convert_expression(&if_statement.test)
                            .map_err(|error| vec![error])?,
                        then_block,
                        else_block,
                        span: span(if_statement.span),
                    });
                }
                Statement::WhileStatement(while_statement) => {
                    let body = convert_block_statement(&while_statement.body)
                        .map_err(|error| vec![error])?;
                    statements.push(Stmt::While {
                        cond: convert_expression(&while_statement.test)
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
                                init: convert_expression(initializer)
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
                                convert_expression(expression).map_err(|error| vec![error])?,
                            )))
                        }
                    };
                    let body = convert_block_statement(&for_statement.body)
                        .map_err(|error| vec![error])?;
                    statements.push(Stmt::For {
                        init,
                        cond: for_statement
                            .test
                            .as_ref()
                            .map(convert_expression)
                            .transpose()
                            .map_err(|error| vec![error])?,
                        update: for_statement
                            .update
                            .as_ref()
                            .map(convert_expression)
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
                    let BindingPattern::BindingIdentifier(identifier) = &declarator.id else {
                        return Err(vec![
                            "OXC adapter supports identifier for-of bindings only".into(),
                        ]);
                    };
                    let body = convert_block_statement(&for_of_statement.body)
                        .map_err(|error| vec![error])?;
                    statements.push(Stmt::ForOf {
                        name: Name {
                            span: span(identifier.span),
                        },
                        iterable: convert_expression(&for_of_statement.right)
                            .map_err(|error| vec![error])?,
                        body,
                        span: span(for_of_statement.span),
                    });
                }
                Statement::DoWhileStatement(do_while_statement) => {
                    let body = convert_block_statement(&do_while_statement.body)
                        .map_err(|error| vec![error])?;
                    statements.push(Stmt::DoWhile {
                        body,
                        cond: convert_expression(&do_while_statement.test)
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
                _ => return Err(vec![
                    "OXC adapter currently supports identifier parameters".into(),
                ]),
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
                    .map(|initializer| convert_expression(initializer))
                    .transpose()
                    .map_err(|error| vec![error])?
                    .map(Box::new),
                is_rest,
            });
        }
        if let Some(rest) = function.params.rest.as_ref() {
            let BindingPattern::BindingIdentifier(identifier) = &rest.rest.argument else {
                return Err(vec!["OXC adapter supports identifier rest parameters only".into()]);
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
            name: Name { span: span(id.span) },
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
            let crate::ast::Stmt::Return { value: Some(value), .. } =
                &program.functions[0].body.statements[0]
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
    fn converts_logical_and_unary_return_expressions() {
        let program = parse_subset("function choose(value) { return !value && true; }", false).unwrap();
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
        let program = parse_subset("function main() { let value = 42; return value; }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Let { .. }
        ));
    }

    #[test]
    fn preserves_typed_local_declarations() {
        let program = parse_subset("function main() { let value: int = 42; return value; }", true)
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
        let program = parse_subset("function set(value) { value = 42; return value; }", false).unwrap();
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
        let program = parse_subset("function greet(name) { return `hello ${name}`; }", false).unwrap();
        assert!(matches!(
            program.functions[0].body.statements[0],
            crate::ast::Stmt::Return {
                value: Some(crate::ast::Expr::Template { ref parts, .. }),
                ..
            } if parts.len() == 3
        ));
    }

    #[test]
    fn converts_null_and_conditional_expressions() {
        let program = parse_subset(
            "function choose(value) { return value ? null : 0; }",
            false,
        )
        .unwrap();
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
        let program = parse_subset("function read(values, index) { return values[index]; }", false)
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
        let program = parse_subset("function call(values) { return merge(...values); }", false).unwrap();
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
        let async_program = parse_subset("async function wait(value) { return await value; }", false)
            .unwrap();
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
        let program = parse_subset("function collect(...values) { return values; }", false).unwrap();
        assert!(program.functions[0].params[0].is_rest);
    }

    #[test]
    fn converts_typed_rest_parameters() {
        let program = parse_subset("function collect(...values: Values) { return values; }", true)
            .unwrap();
        assert!(program.functions[0].params[0].ty.is_some());
    }

    #[test]
    fn converts_named_typescript_function_types() {
        let source = "function add(value: int): int { return value; }";
        let program = parse_subset(source, true).unwrap();
        let function = &program.functions[0];
        assert_eq!(function.params[0].ty.as_ref().unwrap().name.span, Span::new(20, 23));
        assert_eq!(function.return_type.as_ref().unwrap().name.span, Span::new(26, 29));
    }

    #[test]
    fn preserves_typescript_class_field_types() {
        let program = parse_subset("class Box { value: number; } function main() {}", true)
            .unwrap();
        assert_eq!(program.classes[0].fields[0].ty.name.span, Span::new(19, 25));
    }

    #[test]
    fn converts_optional_method_calls() {
        let program = parse_subset("function read(value) { return value?.trim(); }", false)
            .unwrap();
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
        let function = parse_subset("export default function main(): int { return 7; }", true)
            .unwrap();
        let class = parse_subset("export default class Box {}", true).unwrap();
        assert_eq!(function.functions.len(), 1);
        assert_eq!(class.classes.len(), 1);
    }

    #[test]
    fn accepts_unchanged_export_lists_and_rejects_aliases() {
        let program = parse_subset("function main() {} export { main };", false).unwrap();
        assert_eq!(program.functions.len(), 1);
        let error = parse_subset("function main() {} export { main as entry };", false)
            .unwrap_err();
        assert!(error
            .iter()
            .any(|message| message.contains("unchanged local export names")));
        assert!(parse_subset("export { main } from \"other\";", false).is_err());
    }
}
