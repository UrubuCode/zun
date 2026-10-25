//! `Number.prototype.toLocaleString()` (NUCLEUS #2), backed by baked ICU4X data.

use zun_interp::CaptureOutput;

fn run_source(src: &str) -> Vec<String> {
    run_with(src)
}

fn run_with(user: &str) -> Vec<String> {
       let src = &zun_prelude::with_prelude(user);
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
fn groups_thousands() {
    let out = run_source("function main() { print((1234.5).toLocaleString()); }");
    assert_eq!(out, ["1,234.5"]);
}

#[test]
fn integer_has_no_fraction() {
    let out = run_source("function main() { print((1.0).toLocaleString()); }");
    assert_eq!(out, ["1"]);
}

#[test]
fn date_locale_date_string_contains_year() {
    let out = run_with(
        "function main() { let d = Date.fromMillis(Date.UTC(2020, 0, 15, 13, 45, 30, 0)); \
            print(d.toLocaleDateString()); print(d.toLocaleTimeString()); print(d.toLocaleString()); }",
    );
    assert!(out[0].contains("2020"), "{out:?}");
    assert!(!out[1].is_empty() && out[2].contains("2020"), "{out:?}");
}
