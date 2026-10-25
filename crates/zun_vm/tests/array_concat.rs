//! `Array.prototype.concat` + `Symbol.isConcatSpreadable` (NUCLEUS #7/#5).

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
fn concat_spreads_arrays() {
    let out = run_source(
        r#"function main() {
            let r = [1.0, 2.0].concat([3.0, 4.0]);
            print(r.length);
            print(r[0]); print(r[1]); print(r[2]); print(r[3]);
        }"#,
    );
    assert_eq!(out, ["4", "1", "2", "3", "4"]);
}

#[test]
fn concat_appends_scalars_and_spreads_arrays() {
    let out = run_source(
        r#"function main() {
            let r = [1.0].concat(2.0, [3.0, 4.0]);
            print(r.length);
            print(r[0]); print(r[1]); print(r[2]); print(r[3]);
        }"#,
    );
    assert_eq!(out, ["4", "1", "2", "3", "4"]);
}

#[test]
fn concat_spreadable_false_object_is_one_element() {
    let out = run_source(
        r#"function main() {
            let o = { length: 2.0, [Symbol.isConcatSpreadable]: false };
            let r = [1.0].concat(o);
            print(r.length);
        }"#,
    );
    assert_eq!(out, ["2"]);
}

#[test]
fn concat_spreadable_true_object_is_spread() {
    let out = run_source(
        r#"function main() {
            let o = { length: 2.0, [Symbol.isConcatSpreadable]: true };
            o["0"] = 7.0;
            o["1"] = 8.0;
            let r = [1.0].concat(o);
            print(r.length);
        }"#,
    );
    assert_eq!(out, ["3"]);
}
