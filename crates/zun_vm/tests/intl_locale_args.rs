//! Optional trailing `locale` argument on the root-locale `toLocale*` methods
//! (ECMA-402), plus `BigInt.prototype.valueOf()`.

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
fn locale_compare_takes_a_locale() {
    // Swedish sorts "ä" after "z"; German sorts it with "a" (before "z").
    let out = run_source(
        "function main() { print(\"ä\".localeCompare(\"z\", \"de\")); \
            print(\"ä\".localeCompare(\"z\", \"sv\")); \
            print(\"a\".localeCompare(\"b\")); }",
    );
    assert_eq!(out, ["-1", "1", "-1"]);
}

#[test]
fn case_mapping_takes_a_locale() {
    let out = run_source(
        "function main() { print(\"I\".toLocaleLowerCase(\"tr\")); print(\"I\".toLocaleLowerCase()); \
            print(\"i\".toLocaleUpperCase(\"tr\")); }",
    );
    assert_eq!(out, ["ı", "i", "İ"]);
}

#[test]
fn number_to_locale_string_takes_a_locale() {
    let out = run_source(
        "function main() { print((1234.5).toLocaleString(\"de\")); print((1234.5).toLocaleString()); }",
    );
    assert_eq!(out, ["1.234,5", "1,234.5"]);
}

#[test]
fn date_to_locale_string_takes_a_locale() {
    let out = run_source(
        "function main() { let d = new Date(0); \
            print(d.toLocaleDateString(\"de\") != d.toLocaleDateString()); \
            print(d.toLocaleString(\"de\") != d.toLocaleString()); \
            print(d.toLocaleTimeString() == d.toLocaleTimeString(\"\")); }",
    );
    assert_eq!(out, ["true", "true", "true"]);
}

#[test]
fn bigint_value_of_is_identity() {
    let out = run_source("function main() { print((5n).valueOf().toString()); print((5n).valueOf() == 5n); }");
    assert_eq!(out, ["5", "true"]);
}
