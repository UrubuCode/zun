//! Length-tracking `DataView` over a resizable `ArrayBuffer` (ES2024,
//! NUCLEUS #17/#18): no explicit length means `byteLength()` re-resolves to
//! `buffer.byteLength - byteOffset` at access time.

use zun_interp::CaptureOutput;

fn run_source(src: &str) -> Vec<String> {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

#[test]
fn tracking_view_follows_resize() {
    let out = run_source(
        r#"function main() {
            let b = new ArrayBuffer(8, { maxByteLength: 32 });
            let v = new DataView(b, 2);
            print(v.byteLength());
            b.resize(16);
            print(v.byteLength());
            b.resize(4);
            print(v.byteLength());
        }"#,
    );
    assert_eq!(out, vec!["6", "14", "2"]);
}

#[test]
fn fixed_view_does_not_follow_resize() {
    let out = run_source(
        r#"function main() {
            let b = new ArrayBuffer(8, { maxByteLength: 32 });
            let v = new DataView(b, 0, 8);
            b.resize(16);
            print(v.byteLength());
        }"#,
    );
    assert_eq!(out, vec!["8"]);
}

#[test]
fn tracking_view_access_past_the_shrunk_length_fails() {
    let out = run_source(
        r#"function main() {
            let b = new ArrayBuffer(8, { maxByteLength: 32 });
            let v = new DataView(b);
            b.resize(16);
            v.setUint8(12, 7);
            print(v.getUint8(12));
        }"#,
    );
    assert_eq!(out, vec!["7"]);
    let mut diags = Vec::new();
    let src = r#"function main() {
            let b = new ArrayBuffer(8, { maxByteLength: 32 });
            let v = new DataView(b);
            b.resize(4);
            print(v.getUint8(6));
        }"#;
    let ast = zun_syntax::parse(src, &mut diags);
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let bytes = zun_ilgen::lower(&hir).write();
    let mut sink = CaptureOutput::default();
    assert!(zun_vm::execute_with(&bytes, &mut sink).is_err());
}
