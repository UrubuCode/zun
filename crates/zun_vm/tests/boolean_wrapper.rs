//! `new Boolean(x)` wrapper object (NUCLEUS #3). The bare call `Boolean(x)`
//! keeps returning a primitive.

use zun_interp::CaptureOutput;

fn run(user: &str) -> Vec<String> {
    let src = zun_prelude::with_prelude(user);
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(&src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(&src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let bytes = zun_ilgen::lower(&hir).write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

#[test]
fn wrapper_value_of_is_the_boxed_primitive() {
    let out = run(r#"function main() {
        print(new Boolean(true).valueOf());
        print(new Boolean(0.0).valueOf());
        print(new Boolean(1.0).valueOf());
        print(new Boolean("").valueOf());
    }"#);
    assert_eq!(out, vec!["true", "false", "true", "false"]);
}

#[test]
fn wrapper_to_string_renders_the_flag() {
    let out = run(r#"function main() {
        print(new Boolean(true).toString());
        print(new Boolean(0.0).toString());
    }"#);
    assert_eq!(out, vec!["true", "false"]);
}

#[test]
fn bare_call_still_returns_a_primitive() {
    let out = run(r#"function main() {
        let p = Boolean(1.0);
        print(p);
        print(p.toString());
    }"#);
    assert_eq!(out, vec!["true", "true"]);
}
