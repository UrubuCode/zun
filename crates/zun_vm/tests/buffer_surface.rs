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

#[test]
fn utf16le_and_ucs2_roundtrip() {
    let out = run_source(
        r#"function main() {
            let b = Buffer.from("hi", "utf16le");
            print(b.length);
            print(b.toString("hex"));
            print(b.toString("utf16le"));
            print(Buffer.from("hi", "ucs2").toString("ucs2"));
        }"#,
    );
    // 'h'=0x68,'i'=0x69 -> LE 16-bit units [68 00 69 00].
    assert_eq!(out, vec!["4", "68006900", "hi", "hi"]);
}

#[test]
fn base64url_roundtrip() {
    let out = run_source(
        r#"function main() {
            print(Buffer.from("hello").toString("base64url"));
            print(Buffer.from("aGVsbG8", "base64url").toString());
        }"#,
    );
    assert_eq!(out, vec!["aGVsbG8", "hello"]);
}

#[test]
fn float_and_double_roundtrip() {
    let out = run_source(
        r#"function main() {
            let b = Buffer.alloc(8);
            b.writeFloatLE(1.5, 0);
            print(b.readFloatLE(0));
            b.writeFloatBE(0.5, 0);
            print(b.readFloatBE(0));
            b.writeDoubleLE(1.5, 0);
            print(b.readDoubleLE(0));
            b.writeDoubleBE(0.25, 0);
            print(b.readDoubleBE(0));
        }"#,
    );
    assert_eq!(out, vec!["1.5", "0.5", "1.5", "0.25"]);
}

#[test]
fn variable_width_int_roundtrip() {
    let out = run_source(
        r#"function main() {
            let b = Buffer.alloc(6);
            b.writeUIntLE(66051, 0, 3);
            print(b[0]);
            print(b[1]);
            print(b[2]);
            print(b.readUIntLE(0, 3));
            print(b.readUIntBE(0, 3));
            let s = Buffer.alloc(3);
            s.writeIntLE(-2, 0, 3);
            print(s.readIntLE(0, 3));
        }"#,
    );
    // 66051 = 0x010203 -> LE bytes [3, 2, 1]; BE read = 0x030201 = 197121.
    assert_eq!(out, vec!["3", "2", "1", "66051", "197121", "-2"]);
}

#[test]
fn byte_swaps() {
    let out = run_source(
        r#"function main() {
            print(Buffer.from([1, 2, 3, 4]).swap16().toString("hex"));
            print(Buffer.from([1, 2, 3, 4]).swap32().toString("hex"));
            print(Buffer.from([1, 2, 3, 4, 5, 6, 7, 8]).swap64().toString("hex"));
        }"#,
    );
    assert_eq!(out, vec!["02010403", "04030201", "0807060504030201"]);
}

#[test]
fn index_of_includes_and_last() {
    let out = run_source(
        r#"function main() {
            let b = Buffer.from("abcabc");
            print(b.indexOf("bc"));
            print(b.lastIndexOf("bc"));
            print(b.indexOf(99));
            print(b.indexOf("bc", 2));
            print(b.indexOf("zz"));
            print(b.includes("abc"));
            print(b.includes("zz"));
        }"#,
    );
    assert_eq!(out, vec!["1", "4", "2", "4", "-1", "true", "false"]);
}

#[test]
fn negative_slice_indices() {
    let out = run_source(
        r#"function main() {
            let b = Buffer.from("hello");
            print(b.slice(-3).toString());
            print(b.slice(1, -1).toString());
            print(b.slice(-4, -2).toString());
        }"#,
    );
    assert_eq!(out, vec!["llo", "ell", "el"]);
}

#[test]
fn copy_with_negative_source_indices() {
    let out = run_source(
        r#"function main() {
            let src = Buffer.from("hello");
            let dst = Buffer.alloc(5 as u64);
            // src.copy(dst, targetStart, sourceStart): sourceStart = -3 -> index
            // 2 ("llo"), sourceEnd default = length.
            let n = src.copy(dst, 0, -3);
            print(n);
            print(dst.toString("utf8", 0, n));
            let dst2 = Buffer.alloc(5 as u64);
            // sourceStart 1, sourceEnd -1 -> "ell".
            let m = src.copy(dst2, 0, 1, -1);
            print(m);
            print(dst2.toString("utf8", 0, m));
        }"#,
    );
    assert_eq!(out, vec!["3", "llo", "3", "ell"]);
}
