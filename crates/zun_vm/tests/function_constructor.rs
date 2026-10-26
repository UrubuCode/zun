//! `new Function(...)` / `Function(...)` via runtime module linking.

use zun_interp::CaptureOutput;

fn module_bytes(src: &str) -> Vec<u8> {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
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
    let bytes = module_bytes(
        "function main() { let f = new Function(\"return (\"); print(1.0); }",
    );
    let mut out = CaptureOutput::default();
    let error = zun_vm::execute_with(&bytes, &mut out).expect_err("syntax error must throw");
    assert!(error.to_string().contains("SyntaxError"), "{error}");
    assert!(out.lines.is_empty());
}

#[test]
fn no_host_throws_code_generation_disallowed() {
    let bytes = module_bytes(
        "function main() { let f = new Function(\"return 1.0;\"); print(2.0); }",
    );
    // AOT runtime path: never injects a host.
    let mut out = CaptureOutput::default();
    let error = zun_vm::execute_with_eval_host(&bytes, &mut out, None)
        .expect_err("no host must throw");
    assert!(
        error.to_string().contains("code generation from strings disallowed"),
        "{error}"
    );
    assert!(out.lines.is_empty());
}
