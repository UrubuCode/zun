//! Operator-position object->primitive coercion (NUCLEUS #5): `+` and template
//! interpolation apply ToPrimitive to object operands.

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
fn to_primitive_number_participates_in_add() {
    let out = run(r#"function main() {
        let o = { [Symbol.toPrimitive](h: string) { return 10.0; } };
        print((o + 5.0) as number);
    }"#);
    assert_eq!(out, vec!["15"]);
}

#[test]
fn to_primitive_string_in_template() {
    let out = run(r#"function main() {
        let o = { [Symbol.toPrimitive](h: string) { return "hi"; } };
        print(`v=${o}`);
    }"#);
    assert_eq!(out, vec!["v=hi"]);
}

#[test]
fn static_paths_unchanged() {
    let out = run(r#"function main() {
        print(1.0 + 2.0);
        print("a" + "b");
    }"#);
    assert_eq!(out, vec!["3", "ab"]);
}
