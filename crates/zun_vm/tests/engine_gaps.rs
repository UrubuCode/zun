//! Regression tests for engine gaps fixed as a cleanup pass: numeric computed
//! keys on `any` values, and the length-only typed-array constructor.

use zun_interp::CaptureOutput;

fn out(user: &str) -> Vec<String> {
    let src = zun_prelude::with_prelude(user);
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(&src, &mut diags);
    assert!(diags.is_empty(), "parse: {diags:#?}");
    let hir = zun_sema::check(&src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema: {diags:#?}");
    let bytes = zun_ilgen::lower(&hir).write();
    let mut o = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut o).expect("runs");
    o.lines
}

#[test]
fn any_numeric_index_read_write() {
    // Reading and writing a numeric computed key on an `any` holding an array,
    // and a numeric property on an `any` holding a plain object, used to throw
    // BadToken.
    assert_eq!(
        out(r#"function main(): void {
            let a: any = [10.0, 20.0, 30.0];
            print(a[1] as number);
            a[1] = 99.0;
            print(a[1] as number);
            let o: any = {};
            o[2] = 7.0;
            print(o[2] as number);
            print(typeof a[5]);
        }"#),
        // a[1], a[1] after write, o[2], and an out-of-range array read is undefined.
        vec!["20", "99", "7", "undefined"],
    );
}

#[test]
fn typed_array_length_constructor() {
    assert_eq!(
        out(r#"function main(): void {
            let a = new Uint8Array(3);
            a.set(0, 5);
            a.set(1, 255);
            print(a.length);
            print(a.get(1));
            let b = new Uint32Array(4);
            print(b.length);
            print(b.byteLength());
            let c = new Uint8Array(new ArrayBuffer(8), 0, 8);
            print(c.length);
        }"#),
        vec!["3", "255", "4", "16", "8"],
    );
}
