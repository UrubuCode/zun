//! Native OXC parsing boundary for standard JavaScript and TypeScript.
//!
//! The existing parser remains authoritative for Zun-specific syntax while the
//! HIR adapter is built incrementally. This boundary lets callers validate
//! standard JS/TS with OXC without coupling the rest of the compiler to OXC's
//! arena-owned AST types.

use oxc_allocator::Allocator;
use oxc_ast::ast::{Expression, Statement};
use oxc_parser::Parser;
use oxc_span::SourceType;
use zun_common::Span;

use crate::ast::{Block, Expr, Function, Name, Program, Stmt};

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
/// This deliberately starts with function declarations whose body is a single
/// numeric return. Unsupported constructs return a diagnostic instead of being
/// silently approximated; additional statement and expression adapters can be
/// added without changing the public frontend boundary.
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
        let [Statement::ReturnStatement(return_statement)] = body.statements.as_slice() else {
            return Err(vec![
                "OXC adapter currently supports one return statement per function".into(),
            ]);
        };
        let Some(Expression::NumericLiteral(number)) = return_statement.argument.as_ref() else {
            return Err(vec![
                "OXC adapter currently supports numeric return expressions".into(),
            ]);
        };
        let span = |span: oxc_span::Span| Span::new(span.start, span.end);
        let function_span = span(function.span);
        let return_span = span(return_statement.span);
        let value_span = span(number.span);
        functions.push(Function {
            name: Name { span: span(id.span) },
            params: Vec::new(),
            return_type: None,
            is_extern: false,
            is_static: false,
            accessor: None,
            external_module: None,
            is_async: function.r#async,
            is_generator: function.generator,
            body: Block {
                statements: vec![Stmt::Return {
                    value: Some(Expr::Number {
                        value: number.value,
                        span: value_span,
                    }),
                    span: return_span,
                }],
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
}
