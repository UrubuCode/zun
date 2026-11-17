//! `new Function(...)` / `Function(...)` via runtime module linking.

use zun_interp::CaptureOutput;

fn module_bytes(src: &str) -> Vec<u8> {
    let mut diags = Vec::new();
    let ast = zun_syntax::oxc::parse_subset(src, true)
        .unwrap_or_else(|errors| panic!("parse errors: {errors:?}"));
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    zun_ilgen::lower(&hir).write()
}

fn run_source(src: &str) -> Vec<String> {
    let bytes = module_bytes(src);
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

#[test]
fn new_function_with_parameter_and_body() {
    let out = run_source(
        "function main() {\n\
            let f = new Function(\"a\", \"return a * 2.0;\");\n\
            print((f(21.0)) as number);\n\
         }",
    );
    assert_eq!(out, ["42"]);
}

#[test]
fn function_call_without_new_behaves_identically() {
    let out = run_source(
        "function main() {\n\
            let f = Function(\"return 7.0;\");\n\
            print((f()) as number);\n\
         }",
    );
    assert_eq!(out, ["7"]);
}

#[test]
fn multiple_parameters_and_repeated_construction() {
    let out = run_source(
        "function main() {\n\
            let add = new Function(\"a\", \"b\", \"return a + b;\");\n\
            let again = new Function(\"a\", \"b\", \"return a + b;\");\n\
            let comma = new Function(\"x, y\", \"return x * y;\");\n\
            print((add(2.0, 3.0)) as number);\n\
            print((again(4.0, 5.0)) as number);\n\
            print((comma(6.0, 7.0)) as number);\n\
         }",
    );
    assert_eq!(out, ["5", "9", "42"]);
}

#[test]
fn constructed_function_runs_a_loop_and_coexists_with_jitted_code() {
    let out = run_source(
        "function sq(n: number): number { return n * n; }\n\
         function main() {\n\
            let sum = new Function(\"n\", \"let t = 0.0; let i = 0.0; while (i < n) { t = t + i; i = i + 1.0; } return t;\");\n\
            print((sum(5.0)) as number);\n\
            print(sq(3.0));\n\
            print(sq(4.0));\n\
            print((sum(4.0)) as number);\n\
         }",
    );
    assert_eq!(out, ["10", "9", "16", "6"]);
}

#[test]
fn invalid_source_throws_instead_of_linking() {
    let bytes = module_bytes("function main() { let f = new Function(\"return (\"); print(1.0); }");
    let mut out = CaptureOutput::default();
    let error = zun_vm::execute_with(&bytes, &mut out).expect_err("syntax error must throw");
    assert!(error.to_string().contains("SyntaxError"), "{error}");
    assert!(out.lines.is_empty());
}

#[test]
fn no_host_throws_code_generation_disallowed() {
    let bytes =
        module_bytes("function main() { let f = new Function(\"return 1.0;\"); print(2.0); }");
    // AOT runtime path: never injects a host.
    let mut out = CaptureOutput::default();
    let error =
        zun_vm::execute_with_eval_host(&bytes, &mut out, None).expect_err("no host must throw");
    assert!(
        error
            .to_string()
            .contains("code generation from strings disallowed"),
        "{error}"
    );
    assert!(out.lines.is_empty());
}

/// Compile `user` with the standard prelude (which defines the `eval` value).
fn prelude_module(user: &str) -> zun_module::Module {
    let src = &zun_prelude::with_prelude(user);
    let mut diags = Vec::new();
    let ast = zun_syntax::oxc::parse_subset(src, true)
        .unwrap_or_else(|errors| panic!("parse errors: {errors:?}"));
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    zun_ilgen::lower(&hir)
}

fn run_with_prelude(user: &str) -> Vec<String> {
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&prelude_module(user).write(), &mut out).expect("program should run");
    out.lines
}

#[test]
fn indirect_eval_runs_as_global_code() {
    let out = run_with_prelude(
        "const evalGlobal = \"global\";\n\
         function probe(): string {\n\
           const evalGlobal = \"local\";\n\
           const direct = eval(\"evalGlobal\");\n\
           const indirect = (0, eval)(\"typeof evalGlobal === 'undefined' ? 'missing' : evalGlobal\");\n\
           return direct + \":\" + indirect;\n\
         }\n\
         console.log(probe());\n\
         console.log((0, eval)(\"this === globalThis\"));\n\
         const e = eval;\n\
         console.log(e(\"1 + 2 * 3\"));\n\
         console.log((globalThis as any).eval(\"let a = 20; a + 1\"));\n\
         console.log(evalGlobal);",
    );
    assert_eq!(out, ["local:missing", "true", "7", "21", "global"]);
}

#[test]
fn eval_value_reflects_like_a_native_function() {
    let out = run_with_prelude(
        "const g: any = globalThis;\n\
         console.log(typeof g.eval, g.eval === eval, eval.name, g.eval.length);\n\
         console.log(String(g.eval));\n\
         console.log(\"eval\" in g, Object.keys(g).includes(\"eval\"));\n\
         console.log(JSON.stringify(Object.getOwnPropertyDescriptor(g, \"eval\")));\n\
         const o = { k: 1 };\n\
         console.log((0, eval)(42 as any), (0, eval)(o as any) === o);\n\
         console.log(eval.call(null, \"3 + 3\"), eval.apply(null, [\"4 + 4\"]), eval.bind(null)(\"5 + 5\"));",
    );
    assert_eq!(
        out,
        [
            "function true eval 1",
            "function eval() { [native code] }",
            "true false",
            "{\"writable\":true,\"enumerable\":false,\"configurable\":true}",
            "42 true",
            "6 8 10",
        ]
    );
}

#[test]
fn indirect_eval_rejects_sources_it_cannot_evaluate_faithfully() {
    let out = run_with_prelude(
        "function attempt(source: string): string {\n\
           try { return String((0, eval)(source)); } catch (e: any) { return String(e).slice(0, 46); }\n\
         }\n\
         console.log(attempt(\"var a = 1; a + 1\"));\n\
         console.log(attempt(\"if (true) { 5 } else { 6 }\"));\n\
         console.log(attempt(\"1 +\").startsWith(\"SyntaxError\"));",
    );
    assert_eq!(
        out,
        [
            "SyntaxError: unsupported indirect eval source:",
            "SyntaxError: unsupported indirect eval source:",
            "true",
        ]
    );
}

#[test]
fn function_constructor_reached_as_a_value() {
    let out = run_with_prelude(
        "const F: any = Function;\n\
         console.log(F(\"return 1 + 2 * 3\")());\n\
         console.log(new F(\"a\", \"b\", \"return a + b\")(2, 3));\n\
         console.log((globalThis as any).Function(\"return 6\")());\n\
         console.log(Reflect.construct(Function, [\"return 10\"])());\n\
         console.log(F.call(null, \"a\", \"return a * 3\")(4), F.apply(null, [\"a\", \"return a * 2\"])(4));\n\
         console.log(Function(\"return this === globalThis\")());\n\
         const holder: any = { x: 9, read: Function(\"return this.x\") };\n\
         console.log(holder.read());\n\
         console.log(Function.length, String(Function));\n\
         let refused = \"\";\n\
         try { Function(\"a\", \"return this.x + a\"); } catch (e: any) { refused = String(e); }\n\
         console.log(refused);",
    );
    assert_eq!(
        out,
        [
            "7",
            "5",
            "6",
            "10",
            "12 8",
            "true",
            "9",
            "1 function Function() { [native code] }",
            "SyntaxError: unsupported Function body: `this` together with parameters",
        ]
    );
}

#[test]
fn eval_value_without_a_host_throws_and_marks_the_module() {
    let module = prelude_module("const e = eval; console.log(e(\"1 + 1\"));");
    assert!(module.requires_codegen, "an `eval` reference is a codegen site");
    let mut out = CaptureOutput::default();
    let error = zun_vm::execute_with_eval_host(&module.write(), &mut out, None)
        .expect_err("no host must throw");
    assert!(
        error
            .to_string()
            .contains("code generation from strings disallowed"),
        "{error}"
    );
    assert!(out.lines.is_empty());
    // A non-string argument generates no code, and a program that never
    // mentions `eval` does not ask for the capability.
    assert!(!prelude_module("console.log(1);").requires_codegen);
}
