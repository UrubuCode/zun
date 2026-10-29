    use super::*;
    fn parse_ok(src: &str) -> Program {
        let mut diags = Vec::new();
        let p = parse(src, &mut diags);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        p
    }
    fn ident_text<'a>(src: &'a str, name: &Name) -> &'a str {
        &src[name.span.start as usize..name.span.end as usize]
    }
    #[test]
    fn parses_operator_tokens_typeof_in_instanceof_delete() {
        // The operator keywords parse in expression position.
        parse_ok("function main(): string { return typeof x; }");
        parse_ok("function main(): boolean { return k in obj; }");
        parse_ok("function main(): boolean { return x instanceof C; }");
        parse_ok("function main(): boolean { return delete obj.k; }");
    }

    #[test]
    fn keyword_as_property_name_still_parses() {
        // Making these operators keywords must not break member access that
        // uses the same words as property names.
        parse_ok("function main(): any { return obj.in; }");
        parse_ok("function main(): any { return obj.typeof; }");
        parse_ok("function main(): any { return obj.instanceof; }");
        parse_ok("function main(): any { return obj.delete; }");
    }

    #[test]
    fn parses_function_with_params_and_return_type() {
        let src = "function add(a: number, b: number): number { return a + b; }";
        let p = parse_ok(src);
        assert_eq!(p.functions.len(), 1);
        let f = &p.functions[0];
        assert_eq!(ident_text(src, &f.name), "add");
        assert_eq!(f.params.len(), 2);
        assert!(f.return_type.is_some());
        assert_eq!(f.body.statements.len(), 1);
    }

    #[test]
    fn parses_async_function_and_await_expression() {
        let src = "async function main(): int { return await load(); }";
        let p = parse_ok(src);
        assert!(p.functions[0].is_async);
        let Stmt::Return { value: Some(Expr::Await { .. }), .. } = &p.functions[0].body.statements[0]
        else {
            panic!("expected await in return expression");
        };
    }

    #[test]
    fn parses_yield_expression_for_generator_frontend() {
        let src = "function main(): int { return yield 1; }";
        let mut diagnostics = Vec::new();
        let program = parse(src, &mut diagnostics);
        assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
        assert!(matches!(
            program.functions[0].body.statements[0],
            Stmt::Return { value: Some(Expr::Yield { .. }), .. }
        ));
    }

    #[test]
    fn parses_generator_function_marker() {
        let mut diagnostics = Vec::new();
        let program = parse("function* main(): int { yield 1; return 0; }", &mut diagnostics);
        assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
        assert!(program.functions[0].is_generator);
    }
    #[test]
    fn parses_anonymous_function_and_arrow_forms() {
        let src = "function main() { let a = x => x + 1; let b = (x: number, y) => { return x + y; }; let c = function(x) { return x; }; }";
        let p = parse_ok(src);
        let body = &p.functions[0].body.statements;
        let Stmt::Let {
            init: Expr::Function(first),
            ..
        } = &body[0]
        else {
            panic!("expected single-parameter arrow");
        };
        assert!(first.is_arrow);
        assert_eq!(first.params.len(), 1);
        let Stmt::Let {
            init: Expr::Function(second),
            ..
        } = &body[1]
        else {
            panic!("expected parenthesized arrow");
        };
        assert!(second.is_arrow);
        assert_eq!(second.params.len(), 2);
        let Stmt::Let {
            init: Expr::Function(third),
            ..
        } = &body[2]
        else {
            panic!("expected anonymous function");
        };
        assert!(!third.is_arrow);
    }
    #[test]
    fn parses_const_declaration_like_let() {
        // `const` is not a lexer keyword; a const declaration (with an optional
        // type annotation) parses to the same `Stmt::Let` as `let`, and `const`
        // is accepted in a for-of header.
        let src = "function main() { const n: int = 3; const xs = [1, 2]; for (const x of xs) { print(x); } }";
        let p = parse_ok(src);
        let body = &p.functions[0].body.statements;
        assert!(
            matches!(&body[0], Stmt::Let { ty: Some(_), .. }),
            "expected a typed const binding as Stmt::Let, got {:?}",
            body[0]
        );
        assert!(matches!(&body[1], Stmt::Let { ty: None, .. }), "const without annotation");
        assert!(matches!(&body[2], Stmt::ForOf { .. }), "const in a for-of header");
    }
    #[test]
    fn parses_template_literal_segments() {
        let src = "function main() { let name: string = \"Zun\"; return `hello ${name}!`; }";
        let p = parse_ok(src);
        let Stmt::Return { value: Some(Expr::Template { parts, .. }), .. } = &p.functions[0].body.statements[1] else {
            panic!("expected template literal");
        };
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], crate::ast::TemplatePart::Text("hello ".into()));
        assert!(matches!(parts[1], crate::ast::TemplatePart::Expr(_)));
        assert_eq!(parts[2], crate::ast::TemplatePart::Text("!".into()));
    }
    #[test]
    fn parses_expression_inside_template_interpolation() {
        let src = "function main() { let value: number = 1.0; return `${value + 2.0}`; }";
        let p = parse_ok(src);
        let Stmt::Return { value: Some(Expr::Template { parts, .. }), .. } = &p.functions[0].body.statements[1] else {
            panic!("expected template literal");
        };
        assert!(matches!(&parts[0], crate::ast::TemplatePart::Expr(expression) if matches!(expression.as_ref(), Expr::Binary { .. })));
    }
    #[test]
    fn keeps_braces_inside_quoted_template_interpolations() {
        let src = "function main() { return `${\"}\"}`; }";
        let p = parse_ok(src);
        let Stmt::Return { value: Some(Expr::Template { parts, .. }), .. } = &p.functions[0].body.statements[0] else {
            panic!("expected template literal");
        };
        assert!(matches!(&parts[0], crate::ast::TemplatePart::Expr(expression) if matches!(expression.as_ref(), Expr::String { value, .. } if value == "}")));
    }
    #[test]
    fn escaped_template_marker_stays_literal_text() {
        let src = r#"function main() { return `literal \${name}`; }"#;
        let p = parse_ok(src);
        let Stmt::Return { value: Some(Expr::Template { parts, .. }), .. } = &p.functions[0].body.statements[0] else {
            panic!("expected template literal");
        };
        assert_eq!(parts.as_slice(), [crate::ast::TemplatePart::Text("literal ${name}".into())]);
    }
    #[test]
    fn parses_default_parameter_expressions() {
        let p = parse_ok("function add(value: number = 2.0): number { return value; }");
        assert!(p.functions[0].params[0].default_value.is_some());
    }
    #[test]
    fn parses_spread_call_arguments() {
        let p = parse_ok("function main() { print(...values); }");
        let Stmt::Expr(Expr::Call { args, .. }) = &p.functions[0].body.statements[0] else {
            panic!("expected call expression");
        };
        assert!(matches!(args[0], Expr::Spread { .. }));
    }
    #[test]
    fn parses_array_destructuring_declaration() {
        let p = parse_ok("function main() { let [first, second] = values; }");
        assert!(matches!(
            p.functions[0].body.statements[0],
            Stmt::Destructure { pattern: Pattern::Array { ref elements, .. }, .. }
                if elements.len() == 2
        ));
    }
    #[test]
    fn parses_object_destructuring_declaration() {
        let p = parse_ok("function main() { let {answer, other} = object; }");
        assert!(matches!(
            p.functions[0].body.statements[0],
            Stmt::Destructure { pattern: Pattern::Object { ref properties, .. }, .. }
                if properties.len() == 2
        ));
    }
    #[test]
    fn parses_rest_parameter_without_enabling_lowering() {
        let p = parse_ok("function collect(...values: Array) { return values; }");
        assert!(p.functions[0].params[0].is_rest);
        assert!(p.functions[0].params[0].ty.is_some());
    }
    #[test]
    fn parses_for_of_statement() {
        let src = "function main() { for (let value of values) { print(value); } }";
        let p = parse_ok(src);
        let Stmt::ForOf {
            pattern,
            iterable,
            body,
            ..
        } = &p.functions[0].body.statements[0]
        else {
            panic!("expected for-of statement");
        };
        let Pattern::Ident { name, .. } = pattern else {
            panic!("expected identifier for-of binding");
        };
        assert_eq!(ident_text(src, name), "value");
        assert!(matches!(iterable, Expr::Ident(_)));
        assert_eq!(body.statements.len(), 1);
    }
    #[test]
    fn parses_labeled_break_and_continue() {
        let mut diagnostics = Vec::new();
        let program = parse(
            "function main(): int { outer: while (true) { continue outer; break outer; } return 0; }",
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let statements = &program.functions[0].body.statements;
        assert!(matches!(
            &statements[0],
            Stmt::Labeled { statement, .. }
                if matches!(statement.as_ref(), Stmt::While { body, .. }
                    if matches!(body.statements[0], Stmt::Continue { label: Some(_), .. }))
        ));
    }
    #[test]
    fn precedence_mul_binds_tighter_than_add() {
        let src = "function f() { return 1 + 2 * 3; }";
        let p = parse_ok(src);
        let Stmt::Return { value: Some(e), .. } = &p.functions[0].body.statements[0] else {
            panic!("expected return");
        };
        let Expr::Binary {
            op: BinaryOp::Add,
            rhs,
            ..
        } = e
        else {
            panic!("expected add at root, got {e:?}");
        };
        assert!(matches!(
            **rhs,
            Expr::Binary {
                op: BinaryOp::Mul,
                ..
            }
        ));
    }
    #[test]
    fn comparison_binds_looser_than_arithmetic() {
        let src = "function f() { return 1 + 2 < 3 * 4; }";
        let p = parse_ok(src);
        let Stmt::Return { value: Some(e), .. } = &p.functions[0].body.statements[0] else {
            panic!("expected return");
        };
        assert!(matches!(
            e,
            Expr::Binary {
                op: BinaryOp::Lt,
                ..
            }
        ));
    }
    #[test]
    fn parses_let_if_while_and_calls() {
        let src = r#"
            function main() {
                let n: number = 10;
                let acc = 0;
                while (n > 0) {
                    acc = acc + n;
                    n = n - 1;
                }
                if (acc == 55) {
                    print("ok");
                } else {
                    print("bad");
                }
            }
        "#;
        let p = parse_ok(src);
        let body = &p.functions[0].body.statements;
        assert!(matches!(body[0], Stmt::Let { .. }));
        assert!(matches!(body[2], Stmt::While { .. }));
        assert!(matches!(
            body[3],
            Stmt::If {
                else_block: Some(_),
                ..
            }
        ));
    }
    #[test]
    fn parses_do_while_statement() {
        let p = parse_ok("function main() { do { print(1); } while (false); }");
        assert!(matches!(
            p.functions[0].body.statements[0],
            Stmt::DoWhile { .. }
        ));
    }
    #[test]
    fn parses_c_style_for_statement() {
        let p =
            parse_ok("function main() { for (let i: int = 0; i < 3; i = i + 1) { print(i); } }");
        assert!(matches!(
            p.functions[0].body.statements[0],
            Stmt::For { .. }
        ));
    }
    #[test]
    fn parses_short_circuit_logical_operators() {
        let p = parse_ok("function main(): boolean { return true || false && false; }");
        assert!(matches!(
            p.functions[0].body.statements[0],
            Stmt::Return { .. }
        ));
    }
    #[test]
    fn parses_conditional_expression() {
        let p = parse_ok("function main(): int { return true ? 1 : 2; }");
        assert!(matches!(
            p.functions[0].body.statements[0],
            Stmt::Return { .. }
        ));
    }
    #[test]
    fn parses_nullish_and_optional_property_expressions() {
        let p = parse_ok(
            "function main(): any { let value: any = null; return value?.answer ?? null; }",
        );
        let Stmt::Return {
            value: Some(expr), ..
        } = &p.functions[0].body.statements[1]
        else {
            panic!("expected return expression");
        };
        assert!(matches!(
            expr,
            Expr::Binary {
                op: BinaryOp::Nullish,
                ..
            }
        ));
    }
    #[test]
    fn string_escapes_decode() {
        let src = r#"function f() { let s = "a\nb\"c"; }"#;
        let p = parse_ok(src);
        let Stmt::Let {
            init: Expr::String { value, .. },
            ..
        } = &p.functions[0].body.statements[0]
        else {
            panic!("expected string let");
        };
        assert_eq!(value, "a\nb\"c");
    }
    #[test]
    fn string_escapes_cover_the_full_set() {
        // `\b \f \v \xHH \uHHHH \u{..}` used to drop the backslash and keep the
        // rest (so `A` became `u0041`); they now decode correctly. `\\`
        // stays a single backslash and an unknown escape (`\q`) is the char.
        let decode = |body: &str| -> String {
            let src = format!("function f() {{ let s = {body}; }}");
            let p = parse_ok(&src);
            let Stmt::Let { init: Expr::String { value, .. }, .. } =
                &p.functions[0].body.statements[0]
            else {
                panic!("expected string let");
            };
            value.clone()
        };
        assert_eq!(decode(r#""A""#), "A");
        assert_eq!(decode(r#""\x41""#), "A");
        assert_eq!(decode(r#""\u{1F600}""#), "\u{1F600}");
        assert_eq!(decode(r#""\b\f\v""#), "\u{08}\u{0C}\u{0B}");
        assert_eq!(decode(r#""a\\b""#), "a\\b");
        assert_eq!(decode(r#""\q""#), "q");
    }
    #[test]
    fn unary_and_parens() {
        let src = "function f() { return -(1 + 2); }";
        let p = parse_ok(src);
        let Stmt::Return { value: Some(e), .. } = &p.functions[0].body.statements[0] else {
            panic!("expected return");
        };
        let Expr::Unary {
            op: UnaryOp::Neg,
            operand,
            ..
        } = e
        else {
            panic!("expected negation, got {e:?}");
        };
        assert!(matches!(
            **operand,
            Expr::Binary {
                op: BinaryOp::Add,
                ..
            }
        ));
    }
    #[test]
    fn missing_semicolon_diagnoses_but_still_parses() {
        let mut diags = Vec::new();
        let p = parse("function f() { let x = 1 }", &mut diags);
        assert_eq!(p.functions.len(), 1);
        assert!(!diags.is_empty());
    }
    #[test]
    fn invalid_assignment_target_diagnoses() {
        let mut diags = Vec::new();
        parse("function f() { 1 = 2; }", &mut diags);
        assert!(diags
            .iter()
            .any(|d| d.message.contains("assignment target")));
    }
    #[test]
    fn stray_top_level_tokens_recover_to_next_function() {
        let mut diags = Vec::new();
        let p = parse("let x = 1; function ok() {}", &mut diags);
        assert_eq!(p.functions.len(), 1);
        assert!(!diags.is_empty());
    }
    #[test]
    fn parses_class_with_fields() {
        let src = "class Point { x: number; y: number; }";
        let p = parse_ok(src);
        assert_eq!(p.classes.len(), 1);
        let c = &p.classes[0];
        assert_eq!(ident_text(src, &c.name), "Point");
        assert_eq!(c.fields.len(), 2);
        assert_eq!(ident_text(src, &c.fields[0].name), "x");
    }

    #[test]
    fn preserves_static_field_modifier_for_sema() {
        let src = "class Config { static answer: number; value: number; }";
        let p = parse_ok(src);
        let fields = &p.classes[0].fields;
        assert!(fields[0].is_static);
        assert!(!fields[1].is_static);
    }
    #[test]
    fn parses_new_and_field_access() {
        let src = "function f() { let p = new Point(1, 2); print(p.x); }";
        let p = parse_ok(src);
        let body = &p.functions[0].body.statements;
        let Stmt::Let {
            init: Expr::New { args, .. },
            ..
        } = &body[0]
        else {
            panic!("expected new");
        };
        assert_eq!(args.len(), 2);
        let Stmt::Expr(Expr::Call { args, .. }) = &body[1] else {
            panic!("expected call");
        };
        assert!(matches!(args[0], Expr::Field { .. }));
    }
    #[test]
    fn parses_computed_symbol_iterator_call() {
        let program =
            parse_ok("function main(): any { let iterator = values[Symbol.iterator](); }");
        assert!(matches!(
            &program.functions[0].body.statements[0],
            Stmt::Let {
                init: Expr::ComputedCall { .. },
                ..
            }
        ));
    }
    #[test]
    fn parses_field_assignment() {
        let src = "function f() { p.x = 5; }";
        let p = parse_ok(src);
        assert!(matches!(
            p.functions[0].body.statements[0],
            Stmt::Expr(Expr::FieldAssign { .. })
        ));
    }
    #[test]
    fn parses_import() {
        let src = "import { print, println } from \"zun:core\";\nfunction main() {}";
        let p = parse_ok(src);
        assert_eq!(p.imports.len(), 1);
        assert_eq!(p.imports[0].module, "zun:core");
        assert_eq!(p.imports[0].names.len(), 2);
        assert_eq!(ident_text(src, &p.imports[0].names[0].imported), "print");
        assert_eq!(ident_text(src, &p.imports[0].names[0].local), "print");
        assert_eq!(p.functions.len(), 1);
    }

    #[test]
    fn parses_import_alias() {
        let src = "import { bigIntAdd as add } from \"zun:core\";\nfunction main() {}";
        let p = parse_ok(src);
        assert_eq!(ident_text(src, &p.imports[0].names[0].imported), "bigIntAdd");
        assert_eq!(ident_text(src, &p.imports[0].names[0].local), "add");
    }

    #[test]
    fn parses_default_import() {
        let src = "import String from \"extended-string\";\nfunction main() {}";
        let p = parse_ok(src);
        assert_eq!(p.imports[0].module, "extended-string");
        assert!(p.imports[0].names[0].is_default);
        assert_eq!(ident_text(src, &p.imports[0].names[0].local), "String");
    }
    #[test]
    fn parses_class_with_methods() {
        let src = "class Counter { n: number; inc() { n = n + 1; } get(): number { return n; } }";
        let p = parse_ok(src);
        let c = &p.classes[0];
        assert_eq!(c.fields.len(), 1);
        assert_eq!(c.methods.len(), 2);
        assert_eq!(ident_text(src, &c.methods[0].name), "inc");
    }
    #[test]
    fn parses_extends() {
        let src = "class Dog extends Animal { bark(): number { return 1; } }";
        let p = parse_ok(src);
        let c = &p.classes[0];
        assert!(c.base.is_some());
        assert_eq!(ident_text(src, c.base.as_ref().unwrap()), "Animal");
    }
    #[test]
    fn parses_method_call() {
        let src = "function f() { let r = c.add(1, 2); }";
        let p = parse_ok(src);
        let Stmt::Let {
            init: Expr::MethodCall { args, .. },
            ..
        } = &p.functions[0].body.statements[0]
        else {
            panic!("expected method call");
        };
        assert_eq!(args.len(), 2);
    }
    #[test]
    fn chained_field_access() {
        let src = "function f() { let z = a.b.c; }";
        let p = parse_ok(src);
        let Stmt::Let {
            init: Expr::Field { object, .. },
            ..
        } = &p.functions[0].body.statements[0]
        else {
            panic!("expected field access");
        };
        assert!(matches!(**object, Expr::Field { .. }));
    }

    #[test]
    fn parses_private_field_marker_and_access() {
        let p = parse_ok("class Box { #value: number; } function f() { let b = new Box(1); print(b.#value); }");
        assert!(p.classes[0].fields[0].is_private);
        let Stmt::Let { init: Expr::New { .. }, .. } = &p.functions[0].body.statements[0] else {
            panic!("expected private-field test setup");
        };
        assert!(matches!(
            &p.functions[0].body.statements[1],
            Stmt::Expr(Expr::Call { args, .. }) if matches!(&args[0], Expr::PrivateField { .. })
        ));
    }

    #[test]
    fn parses_regex_literal_expression() {
        let p = parse_ok(r"function main() { return /a\/b[/]/gi; }");
        let Stmt::Return { value: Some(Expr::RegexLiteral { pattern, flags, .. }), .. } = &p.functions[0].body.statements[0] else {
            panic!("expected regex literal");
        };
        assert_eq!(pattern, r"a\/b[/]");
        assert_eq!(flags, "gi");
    }

    #[test]
    fn division_after_identifier_still_parses_as_binary() {
        let p = parse_ok("function main(a, b) { return a / b; }");
        assert!(matches!(
            &p.functions[0].body.statements[0],
            Stmt::Return { value: Some(Expr::Binary { .. }), .. }
        ));
    }

    #[test]
    fn parses_object_literal_with_shorthand_and_computed_keys() {
        let src = "function main() { let o = { a: 1, b, [\"k\"]: 2, [s]: 3 }; }";
        let p = parse_ok(src);
        let Stmt::Let { init: Expr::ObjectLiteral { properties, .. }, .. } =
            &p.functions[0].body.statements[0]
        else {
            panic!("expected object literal");
        };
        assert_eq!(properties.len(), 4);
        let prop = |index: usize| match &properties[index] {
            ObjectMember::Property(property) => property,
            _ => panic!("expected a keyed property"),
        };
        assert!(matches!(prop(0).key, PropertyKey::Ident(_)));
        // Shorthand `b` desugars to key=value=ident `b`.
        assert!(matches!(prop(1).key, PropertyKey::Ident(_)));
        assert!(matches!(prop(1).value, Expr::Ident(_)));
        assert!(matches!(prop(2).key, PropertyKey::Computed(_)));
        assert!(matches!(prop(3).key, PropertyKey::Computed(_)));
    }

    #[test]
    fn parses_empty_and_nested_object_literals() {
        let p = parse_ok("function main() { let e = {}; let o = { a: { b: 1 } }; }");
        let Stmt::Let { init: Expr::ObjectLiteral { properties, .. }, .. } =
            &p.functions[0].body.statements[0]
        else {
            panic!("expected empty object literal");
        };
        assert!(properties.is_empty());
        let Stmt::Let { init: Expr::ObjectLiteral { properties, .. }, .. } =
            &p.functions[0].body.statements[1]
        else {
            panic!("expected nested object literal");
        };
        let ObjectMember::Property(first) = &properties[0] else {
            panic!("expected a keyed property");
        };
        assert!(matches!(first.value, Expr::ObjectLiteral { .. }));
    }

    #[test]
    fn parses_array_literals_empty_nested_and_trailing_comma() {
        let p = parse_ok(
            "function main() { let e = []; let a = [1, 2, 3,]; let n = [[1], [2]]; }",
        );
        let Stmt::Let { init: Expr::ArrayLiteral { elements, .. }, .. } =
            &p.functions[0].body.statements[0]
        else {
            panic!("expected empty array literal");
        };
        assert!(elements.is_empty());
        // A single trailing comma is accepted and does not add an element.
        let Stmt::Let { init: Expr::ArrayLiteral { elements, .. }, .. } =
            &p.functions[0].body.statements[1]
        else {
            panic!("expected array literal");
        };
        assert_eq!(elements.len(), 3);
        let Stmt::Let { init: Expr::ArrayLiteral { elements, .. }, .. } =
            &p.functions[0].body.statements[2]
        else {
            panic!("expected nested array literal");
        };
        assert!(matches!(elements[0], ArrayElement::Expr(Expr::ArrayLiteral { .. })));
    }

    #[test]
    fn parses_decorator_on_class() {
        let src = "@foo class C {}";
        let p = parse_ok(src);
        let c = &p.classes[0];
        assert_eq!(c.decorators.len(), 1);
        let Expr::Ident(callee) = &c.decorators[0].callee else {
            panic!("expected identifier decorator callee");
        };
        assert_eq!(ident_text(src, callee), "foo");
        assert!(c.decorators[0].args.is_none());
    }

    #[test]
    fn parses_decorator_call_form_with_args() {
        let src = "@foo(1, \"x\") class C {}";
        let p = parse_ok(src);
        let dec = &p.classes[0].decorators[0];
        let args = dec.args.as_ref().expect("call form carries args");
        assert_eq!(args.len(), 2);
        assert!(matches!(args[0], Expr::Number { .. }));
        assert!(matches!(args[1], Expr::String { .. }));
    }

    #[test]
    fn parses_multiple_class_decorators() {
        let src = "@a @b class C {}";
        let p = parse_ok(src);
        let decorators = &p.classes[0].decorators;
        assert_eq!(decorators.len(), 2);
    }

    #[test]
    fn parses_member_decorators() {
        let src = "class C { @ns.dec method() {} @log field: number; }";
        let p = parse_ok(src);
        let c = &p.classes[0];
        assert_eq!(c.methods[0].decorators.len(), 1);
        // The method decorator uses a member-access callee `ns.dec`.
        let Expr::Field { field, .. } = &c.methods[0].decorators[0].callee else {
            panic!("expected member-access decorator callee");
        };
        assert_eq!(ident_text(src, field), "dec");
        assert_eq!(c.fields[0].decorators.len(), 1);
        let Expr::Ident(callee) = &c.fields[0].decorators[0].callee else {
            panic!("expected identifier field decorator callee");
        };
        assert_eq!(ident_text(src, callee), "log");
    }

    #[test]
    fn undecorated_class_has_no_decorators() {
        let p = parse_ok("class C { x: number; m() {} }");
        let c = &p.classes[0];
        assert!(c.decorators.is_empty());
        assert!(c.fields[0].decorators.is_empty());
        assert!(c.methods[0].decorators.is_empty());
    }
