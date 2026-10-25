//! OrdinaryToPrimitive completeness (NUCLEUS #5): valueOf/toString fallback and
//! class-instance `[Symbol.toPrimitive]`.

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
fn value_of_participates_in_add() {
    let out = run(r#"function main() {
        let o = { valueOf() { return 10.0; } };
        print((o + 5.0) as number);
    }"#);
    assert_eq!(out, vec!["15"]);
}

#[test]
fn to_string_in_template() {
    let out = run(r#"function main() {
        let o = { toString() { return "hi"; } };
        print(`v=${o}`);
    }"#);
    assert_eq!(out, vec!["v=hi"]);
}

#[test]
fn class_instance_symbol_to_primitive() {
    let out = run(r#"class Money {
        [Symbol.toPrimitive](hint: string): number { return 7.0; }
    }
    function main() {
        let m: any = new Money();
        print((m + 1.0) as number);
        print(String(m));
    }"#);
    assert_eq!(out, vec!["8", "7"]);
}
