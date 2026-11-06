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
