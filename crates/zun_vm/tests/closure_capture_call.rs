//! Calling a captured callable by its bare name inside a closure.

use zun_interp::CaptureOutput;

/// Compile `src` to a module, serialize it, run it, and return output lines.
fn run_source(src: &str) -> Vec<String> {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

#[test]
fn closure_calls_captured_arrow_by_bare_name() {
    let out = run_source(
        "function main() { \
            let res = (v) => { print(v); }; \
            let g = (u) => res(1.0); \
            g(0.0); }",
    );
    assert_eq!(out, ["1"]);
}

#[test]
fn nested_arrow_calls_outer_function_parameter_by_bare_name() {
    let out = run_source(
        "function main() { \
            let f = (res: Function) => ((u) => res(2.0)); \
            let k = f((v) => { print(v); }); \
            k(0.0); }",
    );
    assert_eq!(out, ["2"]);
}

#[test]
fn set_timeout_callback_calls_captured_function() {
    let out = run_source(
        "function main() { \
            let res = (v) => { print(v); }; \
            setTimeout(() => res(3.0), 0.0); \
            print(\"sync\"); }",
    );
    assert_eq!(out, ["sync", "3"]);
}
