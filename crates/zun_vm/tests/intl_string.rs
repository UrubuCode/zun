//! ECMA-402 / Unicode surface on `String` (NUCLEUS #1), backed by baked ICU4X
//! data: `normalize`, `localeCompare`, `toLocaleLowerCase`/`toLocaleUpperCase`.

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
fn normalize_decomposes_and_composes() {
    // U+00E9 (é) decomposes (NFD) to "e" + U+0301 (two UTF-16 units)…
    let out = run_source(
        "function main() { let s = String.fromCharCode(233.0); \
            print(s.normalize(\"NFD\").length()); }",
    );
    assert_eq!(out, ["2"]);
    // …and "e" + U+0301 composes (NFC) back to the single U+00E9.
    let out = run_source(
        "function main() { let s = String.fromCharCode(101.0, 769.0); \
            print(s.normalize(\"NFC\").length()); }",
    );
    assert_eq!(out, ["1"]);
}

#[test]
fn normalize_defaults_to_nfc() {
    let out = run_source(
        "function main() { let s = String.fromCharCode(101.0, 769.0); \
            print(s.normalize().length()); }",
    );
    assert_eq!(out, ["1"]);
}

#[test]
fn normalize_rejects_unknown_form() {
    let mut diags = Vec::new();
    let src = "function main() { print(\"x\".normalize(\"BOGUS\").length()); }";
    let ast = zun_syntax::parse(src, &mut diags);
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    let mut out = CaptureOutput::default();
    // An invalid normalization form is a runtime error (RangeError-equivalent).
    assert!(zun_vm::execute_with(&module.write(), &mut out).is_err());
}

#[test]
fn locale_compare_returns_sign() {
    assert_eq!(run_source("function main() { print(\"a\".localeCompare(\"b\")); }"), ["-1"]);
    assert_eq!(run_source("function main() { print(\"b\".localeCompare(\"a\")); }"), ["1"]);
    assert_eq!(run_source("function main() { print(\"a\".localeCompare(\"a\")); }"), ["0"]);
}

#[test]
fn locale_case_mapping() {
    assert_eq!(
        run_source("function main() { print(\"HeLLo\".toLocaleLowerCase()); }"),
        ["hello"]
    );
    assert_eq!(
        run_source("function main() { print(\"HeLLo\".toLocaleUpperCase()); }"),
        ["HELLO"]
    );
}
