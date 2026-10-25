//! Behavioral dispatch of `Symbol.match/replace/search/split` on String
//! methods when the argument is a dynamic object (NUCLEUS #5).

use zun_interp::CaptureOutput;

fn run_source(src: &str) -> Vec<String> {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&module.write(), &mut out).expect("program should run");
    out.lines
}

#[test]
fn string_match_delegates_to_symbol_match() {
    let out = run_source(
        r#"function main() {
            let m = { [Symbol.match](s: string) { return "matched:" + s; } };
            let r: any = "abc".match(m);
            print(r as string);
        }"#,
    );
    assert_eq!(out, ["matched:abc"]);
}

#[test]
fn string_search_delegates_to_symbol_search() {
    let out = run_source(
        r#"function main() {
            let m = { [Symbol.search](s: string) { return 7.0; } };
            let r: any = "abc".search(m);
            print(r as number);
        }"#,
    );
    assert_eq!(out, ["7"]);
}

#[test]
fn string_replace_delegates_to_symbol_replace() {
    let out = run_source(
        r#"function main() {
            let m = { [Symbol.replace](s: string, rep: string) { return s + "->" + rep; } };
            let r: any = "abc".replace(m, "X");
            print(r as string);
        }"#,
    );
    assert_eq!(out, ["abc->X"]);
}

#[test]
fn string_split_delegates_to_symbol_split() {
    let out = run_source(
        r#"function main() {
            let m = { [Symbol.split](s: string) { return "split:" + s; } };
            let r: any = "abc".split(m);
            print(r as string);
        }"#,
    );
    assert_eq!(out, ["split:abc"]);
}
