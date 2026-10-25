//! ECMA-402 `Intl` v1: Collator, NumberFormat, DateTimeFormat with an explicit locale.

use zun_interp::CaptureOutput;

fn run_source(user: &str) -> Vec<String> {
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
fn collator_compares() {
    let out = run_source(
        r#"function main() {
            let c = new Intl.Collator("en");
            print(c.compare("a", "b"));
            print(c.compare("b", "a"));
            print(c.compare("a", "a"));
        }"#,
    );
    assert_eq!(out, ["-1", "1", "0"]);
}

#[test]
fn number_format_honors_locale() {
    let out = run_source(
        r#"function main() {
            let en = new Intl.NumberFormat("en");
            let de = new Intl.NumberFormat("de");
            print(en.format(1234.5));
            print(de.format(1234.5));
        }"#,
    );
    assert_eq!(out, ["1,234.5", "1.234,5"]);
}

#[test]
fn date_time_format_contains_year() {
    let out = run_source(
        r#"function main() {
            let d = Date.fromMillis(Date.UTC(2020, 0, 15, 13, 45, 30, 0));
            let f = new Intl.DateTimeFormat("en");
            print(f.format(d));
            let g: Intl.DateTimeFormat = new Intl.DateTimeFormat("de");
            print(g.format(d));
        }"#,
    );
    assert!(out[0].contains("2020") && out[1].contains("2020"), "{out:?}");
    assert_ne!(out[0], out[1]);
}
