//! Global-code sources handed to an indirect `eval`.
//!
//! An indirect `eval(source)` runs `source` as a script and answers its
//! *completion value*. The runtime compiles strings as function bodies, so the
//! script is rewritten into one: every statement but the last is kept as is and
//! the final expression statement becomes the `return`.
//!
//! Only the shapes whose completion value and effects that rewrite preserves
//! are accepted. Everything else is rejected with a message rather than
//! approximated: a `var` or function declaration would have to create a
//! binding on the global object, and a final non-expression statement (`if`,
//! a loop, a block) has a completion value a function body cannot return.

use oxc_allocator::Allocator;
use oxc_ast::ast::{Statement, VariableDeclarationKind};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

/// Rewrite the script `source` into a function body returning its completion
/// value, or explain why it cannot be evaluated faithfully.
pub fn eval_function_body(source: &str) -> Result<String, String> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::cjs()).parse();
    if let Some(diagnostic) = parsed.diagnostics.first() {
        return Err(diagnostic.to_string());
    }
    let program = &parsed.program;
    let text = |span: oxc_span::Span| &source[span.start as usize..span.end as usize];
    if program
        .directives
        .iter()
        .any(|directive| directive.directive.as_str() == "use strict")
    {
        return Err(unsupported("strict-mode eval code"));
    }
    let Some((last, leading)) = program.body.split_last() else {
        // A script of string-literal statements parses as directives only; the
        // completion value is the last literal. An empty script is `undefined`.
        return Ok(match program.directives.last() {
            Some(directive) => format!("return (\n{}\n);", text(directive.expression.span)),
            None => "return undefined;".to_string(),
        });
    };
    for statement in leading {
        match statement {
            Statement::ExpressionStatement(_) | Statement::EmptyStatement(_) => {}
            Statement::VariableDeclaration(declaration)
                if declaration.kind != VariableDeclarationKind::Var => {}
            Statement::VariableDeclaration(_) => {
                return Err(unsupported("a `var` declaration (it would create a global binding)"));
            }
            Statement::FunctionDeclaration(_) => {
                return Err(unsupported(
                    "a function declaration (it would create a global binding)",
                ));
            }
            _ => return Err(unsupported("a statement other than an expression or `let`/`const`")),
        }
    }
    let Statement::ExpressionStatement(last) = last else {
        return Err(unsupported(match last {
            Statement::VariableDeclaration(declaration)
                if declaration.kind == VariableDeclarationKind::Var =>
            {
                "a `var` declaration (it would create a global binding)"
            }
            Statement::FunctionDeclaration(_) => {
                "a function declaration (it would create a global binding)"
            }
            _ => "a final statement that is not an expression",
        }));
    };
    let expression = last.expression.span();
    // The newline before the closing parenthesis keeps a trailing line comment
    // from swallowing it.
    Ok(format!(
        "{}\nreturn (\n{}\n);",
        &source[..last.span.start as usize],
        text(expression)
    ))
}

fn unsupported(what: &str) -> String {
    format!(
        "unsupported indirect eval source: {what}; only expression statements and `let`/`const` \
         declarations followed by a final expression are evaluated"
    )
}

#[cfg(test)]
mod tests {
    use super::eval_function_body;

    #[test]
    fn a_single_expression_is_returned() {
        assert_eq!(eval_function_body("1 + 2").unwrap(), "\nreturn (\n1 + 2\n);");
        assert_eq!(eval_function_body("this === globalThis;").unwrap(), "\nreturn (\nthis === globalThis\n);");
    }

    #[test]
    fn leading_statements_are_kept_and_the_last_expression_returned() {
        assert_eq!(
            eval_function_body("let a = 1; a + 1 // done").unwrap(),
            "let a = 1; \nreturn (\na + 1\n);"
        );
    }

    #[test]
    fn a_string_literal_script_is_its_value_and_an_empty_one_is_undefined() {
        assert_eq!(eval_function_body("'abc'").unwrap(), "return (\n'abc'\n);");
        assert_eq!(eval_function_body("").unwrap(), "return undefined;");
    }

    #[test]
    fn unfaithful_shapes_are_rejected() {
        for source in [
            "var a = 1; a + 1",
            "var a = 1",
            "function f() {} f()",
            "if (true) { 5 } else { 6 }",
            "{ a: 1 }",
            "'use strict'; this",
            "1; let x = 2;",
        ] {
            let error = eval_function_body(source).unwrap_err();
            assert!(error.starts_with("unsupported indirect eval source"), "{source}: {error}");
        }
    }

    #[test]
    fn a_syntax_error_is_reported() {
        assert!(eval_function_body("1 +").is_err());
    }
}
