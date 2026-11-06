use crate::oxc::{diagnostics, parse_subset, parses};
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
    // `var` is function-scoped: the declaration hoists to the function head
    // (as `undefined`), and the branch keeps the initializing assignment.
    let statements = &program.functions[0].body.statements;
    assert!(matches!(statements.first(), Some(crate::ast::Stmt::Let { .. })), "{statements:?}");
    let Some(crate::ast::Stmt::If { then_block, .. }) = statements.iter().find(|s| matches!(s, crate::ast::Stmt::If { .. })) else {
        panic!("expected an if statement");
    };
    assert!(
        matches!(then_block.statements.as_slice(), [crate::ast::Stmt::Expr(crate::ast::Expr::Assign { .. })]),
        "{then_block:?}"
    );
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

