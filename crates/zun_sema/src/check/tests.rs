    use crate::check::{check, check_with_externals, ExternalFunction};
    use crate::hir;
    use crate::Ty;
    use zun_common::Diagnostic;
    use zun_syntax::parse;
    use zun_syntax::oxc::parse_subset;

    fn check_src(src: &str) -> (hir::Program, Vec<Diagnostic>) {
        let mut diags = Vec::new();
        let ast = parse(src, &mut diags);
        assert!(diags.is_empty(), "parse errors: {diags:?}");
        let hir = check(src, &ast, &mut diags);
        (hir, diags)
    }

    fn check_ok(src: &str) -> hir::Program {
        let (hir, diags) = check_src(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        hir
    }

    fn first_error(src: &str) -> String {
        let (_, diags) = check_src(src);
        assert!(!diags.is_empty(), "expected a diagnostic");
        diags[0].message.clone()
    }

    #[test]
    fn checks_typed_function_and_finds_main() {
        let hir = check_ok(
            "function add(a: number, b: number): number { return a + b; }\n\
             function main() { print(add(2, 3)); }",
        );
        // `add` and `main`, plus the synthetic module initializer appended last
        // for any executable module (PLAN §3.1).
        assert_eq!(hir.functions.len(), 3);
        assert_eq!(hir.main, Some(1));
        assert_eq!(hir.module_init, Some(2));
        let add = &hir.functions[0];
        assert_eq!(add.param_count, 2);
        assert_eq!(add.return_ty, Ty::F64);
    }

    #[test]
    fn default_extended_string_import_rebinds_the_frozen_string_type() {
        let src = "import String from \"extended-string\"; function main(value: String): String { return value.toUpperCase().reverse(); }";
        let mut diagnostics = Vec::new();
        let ast = parse(src, &mut diagnostics);
        let hir = check_with_externals(src, &ast, &mut diagnostics, &[]);
        assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
        assert_eq!(hir.functions[0].locals[0], Ty::String);
        assert_eq!(hir.functions[0].return_ty, Ty::String);
    }

    #[test]
    fn rebound_string_exposes_frozen_floor_extensions() {
        let src = "import String from \"extended-string\"; function main(value: String, needle: String): boolean { let size: u64 = value.length(); let upper: String = value.toUpperCase(); let lower: String = value.toLowerCase(); let trimmed: String = value.trim(); if (size == 0) { return upper == lower; } return value.includes(needle) || trimmed == value; }";
        let mut diagnostics = Vec::new();
        let ast = parse(src, &mut diagnostics);
        let hir = check_with_externals(src, &ast, &mut diagnostics, &[]);
        assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
        assert!(hir.functions.iter().any(|function| function.name == "main"));
    }

    #[test]
    fn rejects_static_method_calls_through_instances() {
        let message = first_error(
            "class Math { static add(left: number, right: number): number { return left + right; } } function main(): number { let value = new Math(); return value.add(1.0, 2.0); }",
        );
        assert!(message.contains("must be called through its class"), "{message}");
    }

    #[test]
    fn accepts_static_fields() {
        let (_, diagnostics) = check_src(
            "class Math { static answer: number; } function main(): number { return Math.answer; }",
        );
        assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
    }

    #[test]
    fn rejects_private_field_access_from_a_derived_type() {
        let (_, diagnostics) = check_src(
            "class Base { #value: number; } class Derived extends Base { read(): number { return this.#value; } } function main(): number { return 0.0; }",
        );
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic.message.contains("private field")),
            "expected private-field diagnostic, got {diagnostics:?}"
        );
    }

    #[test]
    fn async_source_with_await_builds_a_state_machine_plan() {
        let (hir, diagnostics) = check_src(
            "async function main(): Promise { let p = new Promise(); await p; return p; }",
        );
        assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
        assert_eq!(hir.functions[0].async_plan.as_ref().unwrap().await_count, 1);
    }

    #[test]
    fn async_without_await_uses_the_synchronous_promise_path() {
        let hir = check_ok("async function main(): Promise { return new Promise(); }");
        assert!(hir.functions[0].is_async);
        assert_eq!(hir.functions[0].async_plan.as_ref().unwrap().await_count, 0);
    }

    #[test]
    fn import_call_is_a_promise_and_counts_as_no_await_by_itself() {
        let hir = check_ok("async function main(): Promise { return import(\"lib\"); }");
        assert_eq!(hir.functions[0].async_plan.as_ref().unwrap().await_count, 0);
    }

    #[test]
    fn import_call_requires_a_string_specifier() {
        let message = first_error("async function main(): Promise { return import(1); }");
        assert!(message.contains("specifier must be a string"), "{message}");
    }

    #[test]
    fn async_functions_require_a_promise_return_type() {
        let message = first_error("async function main(): int { return 1; }");
        assert!(message.contains("Promise return type"), "{message}");
    }

    #[test]
    fn async_plan_counts_await_states_before_lowering() {
        let (hir, diagnostics) = check_src(
            "async function main(): Promise { let first = await new Promise(); let second = await new Promise(); return new Promise(); }",
        );
        assert!(diagnostics.is_empty(), "unexpected diagnostics: {diagnostics:?}");
        let plan = hir.functions[0]
            .async_plan
            .as_ref()
            .expect("async plan");
        assert_eq!(plan.await_count, 2);
        assert_eq!(plan.state_count, 3);
        assert_eq!(plan.hoisted_locals, vec![0, 1]);
    }

    #[test]
    fn generator_plan_counts_yields_and_hoists_locals() {
        let (hir, diagnostics) = check_src(
            "function* main(): int { let value: int = 1; yield value; yield value + 1; return value; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let plan = hir.functions[0].generator_plan.as_ref().expect("generator plan");
        assert_eq!(plan.yield_count, 2);
        assert_eq!(plan.state_count, 3);
        assert_eq!(plan.hoisted_locals, vec![0]);
    }

    #[test]
    fn async_await_is_rejected_inside_handlers() {
        let catch_error = first_error(
            "async function main(): Promise { try { return new Promise(); } catch (e: any) { await new Promise(); return new Promise(); } }",
        );
        assert!(catch_error.contains("catch handler"), "{catch_error}");
        let finally_error = first_error(
            "async function main(): Promise { try { return new Promise(); } finally { await new Promise(); } }",
        );
        assert!(finally_error.contains("finally handler"), "{finally_error}");
    }

    #[test]
    fn yield_is_rejected_outside_generator_functions() {
        let (_, diagnostics) = check_src("function main(): int { return yield 1; }");
        assert!(diagnostics.iter().any(|diagnostic| diagnostic.message.contains("yield is only valid inside a generator function")), "{diagnostics:?}");
    }

    #[test]
    fn generator_declarations_are_accepted_by_the_checker() {
        let (_, diagnostics) = check_src("function* main(): int { yield 1; return 0; }");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn generator_call_lowers_to_iterator_creation() {
        let (hir, diagnostics) = check_src(
            "function* values(): int { yield 1; return 0; } function main(): any { return values(); }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(matches!(
            hir.functions[1].body.last(),
            Some(hir::Stmt::Return(Some(expr)))
                if matches!(expr.kind, hir::ExprKind::GeneratorCall { .. })
        ));
    }

    #[test]
    fn await_requires_a_promise_operand() {
        let message = first_error(
            "async function main(): Promise { await 1; return new Promise(); }",
        );
        assert!(message.contains("Promise type"), "{message}");
    }

    #[test]
    fn let_infers_type_from_initializer() {
        let hir = check_ok("function f() { let x = 1; let s = \"hi\"; }");
        assert_eq!(hir.functions[0].locals, vec![Ty::F64, Ty::String]);
    }

    #[test]
    fn shadowing_gets_a_fresh_slot() {
        let hir =
            check_ok("function f() { let x = 1; if (x > 0) { let x = \"inner\"; print(x); } }");
        assert_eq!(hir.functions[0].locals, vec![Ty::F64, Ty::String]);
    }

    #[test]
    fn forward_calls_resolve() {
        check_ok("function main() { helper(); } function helper() {}");
    }

    #[test]
    fn plus_concatenates_strings() {
        let hir = check_ok("function f(): string { return \"a\" + \"b\"; }");
        assert_eq!(hir.functions[0].return_ty, Ty::String);
    }

    #[test]
    fn mixed_add_is_an_error() {
        assert!(first_error("function f() { let x = 1 + \"a\"; }").contains("cannot add"));
    }

    #[test]
    fn rest_parameters_remain_gated_until_variadic_lowering_exists() {
        assert!(first_error("function main() { let collect = function(...values: any) { return values; }; }")
            .contains("rest parameters are not lowered yet"));
    }

    #[test]
    fn condition_must_be_boolean() {
        assert!(first_error("function f() { if (1) {} }").contains("must be boolean"));
    }

    #[test]
    fn unknown_variable_diagnoses() {
        assert!(first_error("function f() { print(y); }").contains("unknown variable"));
    }

    #[test]
    fn wrong_argument_type_diagnoses() {
        let msg = first_error("function g(n: number) {} function f() { g(\"s\"); }");
        assert!(msg.contains("does not match"), "{msg}");
    }

    #[test]
    fn wrong_return_type_diagnoses() {
        assert!(first_error("function f(): number { return \"s\"; }").contains("does not match"));
    }

    #[test]
    fn default_parameter_type_is_checked() {
        let msg = first_error("function f(value: number = \"bad\"): number { return value; }");
        assert!(msg.contains("default expression type"), "{msg}");
    }

    #[test]
    fn required_parameter_cannot_follow_default_parameter() {
        let msg = first_error(
            "function f(first: number = 1.0, second: number): number { return second; }",
        );
        assert!(msg.contains("follow a default parameter"), "{msg}");
    }

    #[test]
    fn spread_arguments_are_rejected_until_lowering_exists() {
        let msg = first_error("function main() { print(...values); }");
        assert!(
            msg.contains("spread arguments are not lowered yet"),
            "{msg}"
        );
    }

    #[test]
    fn inner_scope_names_do_not_leak() {
        let msg = first_error("function f(x: boolean) { if (x) { let inner = 1; } print(inner); }");
        assert!(msg.contains("unknown variable"), "{msg}");
    }

    #[test]
    fn class_fields_and_new_check() {
        let hir = check_ok(
            "class Point { x: number; y: number; }\n\
             function main() { let p = new Point(1, 2); print(p.x); }",
        );
        assert_eq!(hir.classes.len(), 1);
        assert_eq!(hir.classes[0].fields.len(), 2);
    }

    #[test]
    fn field_type_flows_through_access() {
        // `p.x` is a number, so it can feed arithmetic.
        check_ok(
            "class Box { v: number; }\n\
             function main() { let b = new Box(3); let y = b.v + 1; print(y); }",
        );
    }

    #[test]
    fn wrong_new_arity_diagnoses() {
        let msg = first_error(
            "class Point { x: number; y: number; }\n\
             function main() { let p = new Point(1); }",
        );
        assert!(msg.contains("field(s)"), "{msg}");
    }

    #[test]
    fn unknown_field_diagnoses() {
        let msg = first_error(
            "class Point { x: number; }\n\
             function main() { let p = new Point(1); print(p.z); }",
        );
        assert!(msg.contains("no field"), "{msg}");
    }

    #[test]
    fn field_assignment_type_checked() {
        let msg = first_error(
            "class Box { v: number; }\n\
             function main() { let b = new Box(1); b.v = \"s\"; }",
        );
        assert!(msg.contains("cannot assign"), "{msg}");
    }

    #[test]
    fn methods_check_and_this_resolves_fields() {
        let hir = check_ok(
            "class Counter {\n\
                n: number;\n\
                inc() { this.n = this.n + 1; }\n\
                get(): number { return this.n; }\n\
             }\n\
             function main() {\n\
                let c = new Counter(0);\n\
                c.inc(); c.inc();\n\
                print(c.get());\n\
             }",
        );
        // 1 top-level function + 2 methods + the synthetic module initializer
        // appended last for any executable module (PLAN §3.1).
        assert_eq!(hir.functions.len(), 4);
        assert_eq!(hir.module_init, Some(3));
    }

    #[test]
    fn method_with_params_checks_arity() {
        let msg = first_error(
            "class Adder { base: number; add(x: number): number { return this.base + x; } }\n\
             function main() { let a = new Adder(10); print(a.add()); }",
        );
        assert!(msg.contains("argument(s)"), "{msg}");
    }

    #[test]
    fn unknown_method_diagnoses() {
        let msg = first_error(
            "class C { x: number; }\n\
             function main() { let c = new C(1); c.nope(); }",
        );
        assert!(msg.contains("no method"), "{msg}");
    }

    #[test]
    fn inherited_fields_and_methods_resolve() {
        check_ok(
            "class Animal { legs: number; describe(): number { return this.legs; } }\n\
             class Dog extends Animal { tailLen: number; }\n\
             function main() {\n\
                let d = new Dog(4, 20);\n\
                print(d.describe());\n\
                print(d.tailLen);\n\
             }",
        );
    }

    #[test]
    fn subclass_is_assignable_to_base() {
        check_ok(
            "class Shape { sides: number; }\n\
             class Square extends Shape { size: number; }\n\
             function area(s: Shape): number { return s.sides; }\n\
             function main() { let sq = new Square(4, 10); print(area(sq)); }",
        );
    }

    #[test]
    fn base_is_not_assignable_to_subclass() {
        let msg = first_error(
            "class Shape { sides: number; }\n\
             class Square extends Shape { size: number; }\n\
             function f(sq: Square) {}\n\
             function main() { let s = new Shape(4); f(s); }",
        );
        assert!(msg.contains("does not match"), "{msg}");
    }

    #[test]
    fn unknown_base_class_diagnoses() {
        let msg = first_error("class Dog extends Missing { n: number; }");
        assert!(msg.contains("unknown base class"), "{msg}");
    }

    #[test]
    fn class_typed_field_references_another_class() {
        check_ok(
            "class Inner { n: number; }\n\
             class Outer { inner: Inner; }\n\
             function main() { let i = new Inner(5); let o = new Outer(i); print(o.inner.n); }",
        );
    }

    #[test]
    fn int_and_float_are_distinct() {
        let hir = check_ok("function main() { let a: int = 1; let b: float = 1.0; let c = 2; }");
        // a:int, b:float(Number), c defaults to Number.
        assert_eq!(hir.functions[0].locals, vec![Ty::INT, Ty::F64, Ty::F64]);
    }

    #[test]
    fn int_arithmetic_stays_int() {
        let hir = check_ok("function f(): int { let x: int = 10; return x * 2 + 5; }");
        assert_eq!(hir.functions[0].return_ty, Ty::INT);
    }

    #[test]
    fn mixing_int_and_float_is_an_error() {
        let msg =
            first_error("function f() { let x: int = 1; let y: float = 2.0; let z = x + y; }");
        assert!(
            msg.contains("cannot add") || msg.contains("matching"),
            "{msg}"
        );
    }

    #[test]
    fn non_integral_literal_not_coerced_to_int() {
        let msg = first_error("function f() { let x: int = 1.5; }");
        assert!(msg.contains("does not match"), "{msg}");
    }

    #[test]
    fn int_param_accepts_integer_literal() {
        check_ok(
            "function inc(n: int): int { return n + 1; }\n\
             function main() { print(inc(41)); }",
        );
    }

    #[test]
    fn any_is_an_explicit_dynamic_type() {
        let hir = check_ok("function identity(value: any): any { return value; }");
        assert_eq!(hir.functions[0].param_count, 1);
        assert_eq!(hir.functions[0].return_ty, Ty::Any);
    }

    #[test]
    fn any_can_be_explicitly_converted_to_string() {
        let hir = check_ok("function main(): string { let value: any = box_any(1); return value as string; }");
        assert_eq!(hir.functions[0].return_ty, Ty::String);
    }

    #[test]
    fn array_is_a_managed_core_type() {
        let hir = check_ok("function identity(values: Array): Array { return values; }");
        assert_eq!(hir.functions[0].locals, vec![Ty::Array]);
        assert_eq!(hir.functions[0].return_ty, Ty::Array);
    }

    #[test]
    fn array_constructor_checks_integer_length() {
        let hir = check_ok("function main(): Array { return new Array(2); }");
        assert_eq!(hir.functions[0].return_ty, Ty::Array);
    }

    #[test]
    fn rejects_incompatible_named_function_callback() {
        let msg = first_error(
            "function takes_text(value: string): number { return 1.0; }\n\
             function apply(f: takes_text, value: number): number { return f(value); }",
        );
        assert!(msg.contains("does not match"), "{msg}");
    }

    #[test]
    fn rejects_named_function_callback_with_incompatible_return() {
        let msg = first_error(
            "function returns_text(value: number): string { return \"value\"; }\n\
             function apply(f: returns_text, value: number): number { return f(value); }",
        );
        assert!(
            msg.contains("return type") || msg.contains("does not match"),
            "{msg}"
        );
    }

    #[test]
    fn any_does_not_implicitly_enable_arithmetic() {
        let msg = first_error("function f(value: any) { let result = value - 1; }");
        assert!(msg.contains("arithmetic requires"), "{msg}");
    }

    #[test]
    fn dynamic_property_builtins_are_type_checked() {
        let hir = check_ok(
            "function main(): any {\n\
                let object = dynamic_new();\n\
                dynamic_set(object, \"answer\", box_any(41));\n\
                return dynamic_get(object, \"answer\");\n\
             }",
        );
        assert_eq!(hir.functions[0].return_ty, Ty::Any);
        assert_eq!(hir.functions[0].locals, vec![Ty::DynamicObject]);
    }

    #[test]
    fn checks_external_function_call_from_linker_signature() {
        let source = "import { answer } from \"core\"; function main(): int { return answer(41); }";
        let mut diagnostics = Vec::new();
        let ast = parse(source, &mut diagnostics);
        let hir = check_with_externals(
            source,
            &ast,
            &mut diagnostics,
            &[ExternalFunction {
                module: "core".into(),
                name: "answer".into(),
                params: vec![Ty::INT],
                return_ty: Ty::INT,
            }],
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let hir::Stmt::Return(Some(expr)) = &hir.functions[0].body[0] else {
            panic!("expected return statement")
        };
        assert!(matches!(
            expr.kind,
            hir::ExprKind::ExternalCall { ref module, ref name, .. }
                if module == "core" && name == "answer"
        ));
        assert_eq!(expr.ty, Ty::INT);
    }

    #[test]
    fn oxc_array_expression_lowers_to_an_array_literal() {
        let source = "function main() { let a = [1, 2, 3]; }";
        let ast = parse_subset(source, true).unwrap();
        let mut diagnostics = Vec::new();
        let hir = check(source, &ast, &mut diagnostics);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let hir::Stmt::Let { init, .. } = &hir.functions[0].body[0] else {
            panic!("expected let statement");
        };
        assert_eq!(init.ty, Ty::Array);
        assert!(matches!(
            &init.kind,
            hir::ExprKind::ArrayLiteral { elements } if elements.len() == 3
        ));
    }

    #[test]
    fn oxc_array_spread_and_holes_now_parse() {
        use zun_syntax::ast::{ArrayElement, Expr, Stmt};
        let program = parse_subset("function main() { let b = [0]; let a = [...b, , 3]; }", true)
            .expect("spread and holes parse");
        let Stmt::Let { init: Expr::ArrayLiteral { elements, .. }, .. } =
            &program.functions[0].body.statements[1]
        else {
            panic!("expected array literal");
        };
        assert_eq!(elements.len(), 3);
        assert!(matches!(elements[0], ArrayElement::Spread(_)));
        assert!(matches!(elements[1], ArrayElement::Hole));
        assert!(matches!(elements[2], ArrayElement::Expr(_)));
    }

    #[test]
    fn namespace_import_member_call_lowers_to_static_native_call() {
        let source = "import { math } from 'zun:core'; function main(): number { return math.sqrt(9.0); }";
        let mut diagnostics = Vec::new();
        let ast = parse_subset(source, true).unwrap();
        let hir = check_with_externals(source, &ast, &mut diagnostics, &[]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let hir::Stmt::Return(Some(expr)) = &hir.functions[0].body[0] else {
            panic!("expected return statement")
        };
        assert!(matches!(expr.kind, hir::ExprKind::CallBuiltin { id, .. } if id == zun_natives::resolve("math", "sqrt").unwrap().0));
        assert_eq!(expr.ty, Ty::F64);
    }

    #[test]
    fn namespace_import_member_call_lowers_to_static_external_call() {
        let source = "import * as math from 'pkg'; function main(): int { return math.answer(41); }";
        let mut diagnostics = Vec::new();
        let ast = parse_subset(source, true).unwrap();
        let hir = check_with_externals(
            source,
            &ast,
            &mut diagnostics,
            &[ExternalFunction {
                module: "pkg".into(),
                name: "answer".into(),
                params: vec![Ty::INT],
                return_ty: Ty::INT,
            }],
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let hir::Stmt::Return(Some(expr)) = &hir.functions[0].body[0] else {
            panic!("expected return statement")
        };
        assert!(matches!(expr.kind, hir::ExprKind::ExternalCall { ref module, ref name, .. } if module == "pkg" && name == "answer"));
    }

    #[test]
    fn computed_literal_field_uses_the_frozen_instance_slot() {
        let hir = check_ok(
            r#"class Point { x: int; } function main(): int { let p = new Point(7); return p["x"]; }"#,
        );
        let hir::Stmt::Return(Some(expr)) = &hir.functions[0].body[1] else {
            panic!("expected return statement")
        };
        assert!(matches!(expr.kind, hir::ExprKind::FieldGet { .. }));
        assert_eq!(expr.ty, Ty::INT);
    }

    #[test]
    fn computed_any_member_uses_dynamic_value_lookup() {
        let hir = check_ok(
            r#"function main(): any { let object = dynamic_new(); let key = "x"; return object[key]; }"#,
        );
        let hir::Stmt::Return(Some(expr)) = &hir.functions[0].body[2] else {
            panic!("expected return statement")
        };
        assert!(matches!(expr.kind, hir::ExprKind::DynamicGetValue { .. }));
    }

    #[test]
    fn initialized_class_field_fills_an_omitted_constructor_slot() {
        let source = "class Point { x: number = 5; } function main(): number { return new Point().x; }";
        let mut diagnostics = Vec::new();
        let ast = parse_subset(source, true).unwrap();
        let hir = check_with_externals(source, &ast, &mut diagnostics, &[]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let hir::Stmt::Return(Some(expr)) = &hir.functions[0].body[0] else {
            panic!("expected return statement")
        };
        let hir::ExprKind::FieldGet { object, .. } = &expr.kind else {
            panic!("expected field access")
        };
        assert!(matches!(object.kind, hir::ExprKind::New { ref args, .. } if args.len() == 1));
    }

    #[test]
    fn error_does_not_cascade() {
        // `y` is unknown; the addition and the assignment must not pile on
        // extra diagnostics.
        let (_, diags) = check_src("function f() { let x = y + 1; }");
        assert_eq!(diags.len(), 1, "{diags:?}");
    }

    #[test]
    fn extern_method_requires_an_explicit_intrinsic_binding() {
        let (_, diagnostics) = check_src(
            "class Service { extern missing(): int; }\nfunction main(): int { return 0; }",
        );
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic
                .message
                .contains("must be bound to an imported intrinsic")),
            "{diagnostics:?}"
        );
    }

