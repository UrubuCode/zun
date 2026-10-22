//! Native OXC parsing boundary for standard JavaScript and TypeScript.
//!
//! The existing parser remains authoritative for Zun-specific syntax while the
//! HIR adapter is built incrementally. This boundary lets callers validate
//! standard JS/TS with OXC without coupling the rest of the compiler to OXC's
//! arena-owned AST types.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, BinaryOperator, BindingPattern, Expression, ForStatementInit, ForStatementLeft,
    LogicalOperator, Statement,
    UnaryOperator,
};
use oxc_parser::Parser;
use oxc_span::SourceType;
use zun_common::Span;

use crate::ast::{BinaryOp, Block, Expr, Function, Name, Program, Stmt};

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
        _ => Err("OXC adapter currently supports literal and identifier arguments".into()),
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
        Expression::Identifier(identifier) => Ok(Expr::Ident(Name {
            span: span(identifier.span),
        })),
        Expression::StaticMemberExpression(member) => Ok(Expr::Field {
            object: Box::new(convert_expression(&member.object)?),
            field: Name {
                span: span(member.property.span),
            },
            span: span(member.span),
        }),
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
                Expression::StaticMemberExpression(member) => Ok(Expr::MethodCall {
                    receiver: Box::new(convert_expression(&member.object)?),
                    method: Name {
                        span: span(member.property.span),
                    },
                    args,
                    span: span(call.span),
                }),
                _ => Err("OXC adapter currently supports identifier and static-member callees only".into()),
            }
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
    for statement in parsed.program.body {
        let Statement::FunctionDeclaration(function) = statement else {
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
                            ty: None,
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
                                ty: None,
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
                _ => {
                    return Err(vec![
                        "OXC adapter currently supports local declarations, conditionals, while loops, for loops, for-of loops, return, and expression statements".into(),
                    ])
                }
            }
        }
        let mut params = Vec::with_capacity(function.params.items.len());
        for parameter in &function.params.items {
            let BindingPattern::BindingIdentifier(identifier) = &parameter.pattern else {
                return Err(vec![
                    "OXC adapter currently supports identifier parameters".into(),
                ]);
            };
            if parameter.initializer.is_some() {
                return Err(vec![
                    "OXC adapter does not support default parameters yet".into(),
                ]);
            }
            params.push(crate::ast::Param {
                name: Name {
                    span: span(identifier.span),
                },
                ty: None,
                default_value: None,
                is_rest: false,
            });
        }
        functions.push(Function {
            name: Name { span: span(id.span) },
            params,
            return_type: None,
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
        classes: Vec::new(),
        imports: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::{diagnostics, parse_subset, parses};

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
}
