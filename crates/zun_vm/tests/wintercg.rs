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

#[test]
fn url_parses_components() {
    let out = run(r#"function main() {
        let u = new URL("https://user@host.example:8080/a/b?x=1&y=two#frag");
        console.log(u.protocol);
        console.log(u.hostname);
        console.log(u.host);
        console.log(u.port);
        console.log(u.pathname);
        console.log(u.search);
        console.log(u.hash);
        console.log(u.href);
        console.log(u.searchParams.get("x"));
        console.log(u.searchParams.get("y"));
        console.log(u.searchParams.has("x"));
        console.log(u.searchParams.has("z"));
    }"#);
    assert_eq!(
        out.lines,
        [
            "https:", "host.example", "host.example:8080", "8080", "/a/b", "?x=1&y=two", "#frag",
            "https://user@host.example:8080/a/b?x=1&y=two#frag", "1", "two", "true", "false"
        ]
    );
}

#[test]
fn url_invalid_input_throws() {
    let out = run(r#"function main() {
        try {
            let bad = new URL("not a url");
            console.log(bad.href);
            console.log("no throw");
        } catch (e: TypeError) {
            console.log("TypeError");
        }
    }"#);
    assert_eq!(out.lines, ["TypeError"]);
}

#[test]
fn url_search_params_round_trip_and_encoding() {
    let out = run(r#"function main() {
        let p = new URLSearchParams("?a=1&b=x+y&a=2&c=%26%20z");
        console.log(p.get("b"));
        console.log(p.get("c"));
        console.log(p.getAll("a").length);
        console.log(p.toString());
        p.append("d", "hello world&more");
        console.log(p.toString());
        p.set("a", "9");
        p.delete("b");
        console.log(p.toString());
        let q = new URLSearchParams(p.toString());
        console.log(q.get("d"));
    }"#);
    assert_eq!(
        out.lines,
        [
            "x y", "& z", "2", "a=1&b=x+y&a=2&c=%26+z",
            "a=1&b=x+y&a=2&c=%26+z&d=hello+world%26more",
            "a=9&c=%26+z&d=hello+world%26more", "hello world&more"
        ]
    );
}
