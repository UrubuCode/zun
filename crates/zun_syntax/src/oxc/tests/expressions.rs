use crate::oxc::parse_subset;
use zun_common::Span;

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
