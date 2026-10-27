//! `console` and `JSON` are nucleus prelude classes: compiled with every
//! program and run through the real VM.

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
fn console_streams() {
    let out = run(r#"function main() {
        console.log("a"); console.info("b"); console.debug("c");
        console.error("d"); console.warn("e");
    }"#);
    assert_eq!(out.lines, ["a", "b", "c"]);
    assert_eq!(out.errors, ["d", "e"]);
}

#[test]
fn console_renders_values() {
    let out = run(r#"function main() {
        console.log(42);
        console.log([1, "x", true]);
        console.log({ a: 1, b: { c: "y" }, d: [1, 2] });
        console.log(null);
    }"#);
    assert_eq!(out.lines, ["42", "[ 1, 'x', true ]", "{ a: 1, b: { c: 'y' }, d: [ 1, 2 ] }", "null"]);
}

#[test]
fn json_stringify() {
    let out = run(r#"function main() {
        console.log(JSON.stringify(null));
        console.log(JSON.stringify(true));
        console.log(JSON.stringify(1.5));
        console.log(JSON.stringify("a\"b\n"));
        console.log(JSON.stringify("c" + String.fromCharCode(92) + "d" + String.fromCharCode(1)));
        console.log(JSON.stringify([1, [2, 3], "x"]));
        console.log(JSON.stringify({ a: 1, b: { c: [true, null] } }));
        let u: any = new Error("x").cause;
        console.log(JSON.stringify({ a: 1, u: u }));
        console.log(JSON.stringify([u, 1]));
        console.log(JSON.stringify(1.0 / 0.0));
    }"#);
    assert_eq!(out.lines, [
        "null",
        "true",
        "1.5",
        r#""a\"b\n""#,
        r#""c\\d\u0001""#,
        "[1,[2,3],\"x\"]",
        r#"{"a":1,"b":{"c":[true,null]}}"#,
        r#"{"a":1}"#,
        "[null,1]",
        "null",
    ]);
}

#[test]
fn json_parse() {
    let out = run(r#"function main() {
        console.log(JSON.parse("12"));
        let bs = String.fromCharCode(92);
        console.log(JSON.parse("\"hi" + bs + "u0041" + bs + "n" + bs + bs + bs + "/\""));
        console.log(JSON.parse("[1e2, -0.5, 2E-1]"));
        console.log(JSON.parse("true"));
        console.log(JSON.parse("null"));
        console.log(JSON.parse("[1, [2], {\"k\": \"v\"}]"));
        let x = { a: [1, 2, { b: "z" }], n: null, t: true };
        console.log(JSON.stringify(JSON.parse(JSON.stringify(x))));
    }"#);
    assert_eq!(out.lines, [
        "12",
        "hiA\n\\/",
        "[ 100, -0.5, 0.2 ]",
        "true",
        "null",
        "[ 1, [ 2 ], { k: 'v' } ]",
        r#"{"a":[1,2,{"b":"z"}],"n":null,"t":true}"#,
    ]);
}

#[test]
fn json_parse_rejects_malformed() {
    let out = run(r#"function main() {
        try {
            JSON.parse("{\"a\": ");
            console.log("no error");
        } catch (e: SyntaxError) {
            console.log("syntax error");
        }
    }"#);
    assert_eq!(out.lines, ["syntax error"]);
}

