//! Node `Buffer` surface as a running program sees it (NUCLEUS #21).
//!
//! `Buffer` is a `Uint8Array` byte view here (Node's Buffer IS a Uint8Array),
//! so these exercise the statics (`from`/`alloc`/`concat`/…), the encodings,
//! indexed byte get/set, `toString`/`slice`/`write`, and the fixed-width
//! readers/writers — all built on the existing byte-view floor.

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
fn from_string_length_and_roundtrip() {
    let out = run_source(
        r#"function main() {
            let b = Buffer.from("hi");
            print(b.length);
            print(b.toString());
            print(b.toString("hex"));
        }"#,
    );
    assert_eq!(out, vec!["2", "hi", "6869"]);
}

#[test]
fn from_base64_decodes() {
    let out = run_source(
        r#"function main() {
            print(Buffer.from("aGk=", "base64").toString());
        }"#,
    );
    assert_eq!(out, vec!["hi"]);
}

#[test]
fn alloc_is_zero_filled_and_indexable() {
    let out = run_source(
        r#"function main() {
            let b = Buffer.alloc(4);
            print(b.length);
            print(b[0]);
            b[0] = 65;
            print(b[0]);
            print(b.toString("hex"));
        }"#,
    );
    assert_eq!(out, vec!["4", "0", "65", "41000000"]);
}

#[test]
fn concat_slice_and_write() {
    let out = run_source(
        r#"function main() {
            let a = Buffer.from("ab");
            let b = Buffer.from("cd");
            let j = Buffer.concat([a, b]);
            print(j.toString());
            print(j.slice(1, 3).toString());
            let w = Buffer.alloc(3);
            w.write("x", 0);
            print(w[0]);
        }"#,
    );
    assert_eq!(out, vec!["abcd", "bc", "120"]);
}

#[test]
fn uint16_le_roundtrip() {
    let out = run_source(
        r#"function main() {
            let b = Buffer.alloc(4);
            b.writeUInt16LE(513, 0);
            print(b[0]);
            print(b[1]);
            print(b.readUInt16LE(0));
            print(b.readUInt16BE(0));
        }"#,
    );
    // 513 = 0x0201 -> LE bytes [1, 2]; BE read of those bytes = 0x0102 = 258.
    assert_eq!(out, vec!["1", "2", "513", "258"]);
}

#[test]
fn is_buffer_and_equals() {
    let out = run_source(
        r#"function main() {
            let a = Buffer.from("hi");
            print(Buffer.isBuffer(a));
            print(Buffer.isBuffer(5));
            print(a.equals(Buffer.from("hi")));
            print(a.equals(Buffer.from("ho")));
            print(Buffer.byteLength("hi"));
        }"#,
    );
    assert_eq!(out, vec!["true", "false", "true", "false", "2"]);
}
