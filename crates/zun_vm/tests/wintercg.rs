//! WinterCG minimum-common globals as nucleus prelude items: `performance`,
//! `TextEncoder`, `TextDecoder`, `structuredClone`, `btoa`, `atob`.

use zun_interp::CaptureOutput;

fn run(user: &str) -> CaptureOutput {
    let src = &zun_prelude::with_prelude(user);
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&module.write(), &mut out).expect("program should run");
    out
}

#[test]
fn performance_now_is_a_number() {
    let out = run(r#"function main() {
        let a = performance.now();
        let b = performance.now();
        console.log(typeof a);
        console.log(b >= a);
    }"#);
    assert_eq!(out.lines, ["number", "true"]);
}

#[test]
fn text_encoder_encodes_utf8() {
    let out = run(r#"function main() {
        let enc = new TextEncoder().encode("AB");
        console.log(enc.length);
        console.log(enc[0]);
        console.log(enc[1]);
        console.log(new TextEncoder().encode("é").length);
    }"#);
    assert_eq!(out.lines, ["2", "65", "66", "2"]);
}

#[test]
fn text_decoder_round_trips() {
    let out = run(r#"function main() {
        let s = "héllo €";
        let bytes = new TextEncoder().encode(s);
        console.log(bytes.length);
        let back = new TextDecoder().decode(bytes);
        console.log(back == s);
        console.log(back);
    }"#);
    assert_eq!(out.lines, ["10", "true", "h\u{e9}llo \u{20ac}"]);
}

#[test]
fn structured_clone_is_independent() {
    let out = run(r#"function main() {
        let inner = [1, 2, 3];
        let original: any = [inner, ["x"], 7];
        let copy: any = structuredClone(original);
        inner[0] = 99;
        console.log(JSON.stringify(original));
        console.log(JSON.stringify(copy));
    }"#);
    assert_eq!(out.lines, ["[[99,2,3],[\"x\"],7]", "[[1,2,3],[\"x\"],7]"]);
}

#[test]
fn base64_btoa_atob() {
    let out = run(r#"function main() {
        console.log(btoa("Man"));
        console.log(atob("TWFu"));
        console.log(btoa("Ma"));
        console.log(atob(btoa("hello")));
    }"#);
    assert_eq!(out.lines, ["TWFu", "Man", "TWE=", "hello"]);
}

#[test]
fn structured_clone_copies_objects() {
    let out = run(r#"function main() {
        let list = [1, 2];
        let original: any = { a: 1, list: list, nested: { s: "x" } };
        let copy: any = structuredClone(original);
        list[0] = 50;
        console.log(JSON.stringify(original));
        console.log(JSON.stringify(copy));
    }"#);
    assert_eq!(out.lines, [r#"{"a":1,"list":[50,2],"nested":{"s":"x"}}"#, r#"{"a":1,"list":[1,2],"nested":{"s":"x"}}"#]);
}
