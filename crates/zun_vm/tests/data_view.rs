//! `DataView`'s element accessors.
//!
//! Only six existed — `Uint16`, `Uint32` and `Float64` — so there was no way
//! to read a signed value out of a binary buffer. The same byte is `-1` as
//! `Int8` and `255` as `Uint8`, and reading the unsigned one where the format
//! says signed is a wrong answer rather than a missing feature.
//!
//! Endianness gets the most attention here. It is a boolean argument that is
//! easy to thread through incorrectly, and a mistake only shows on
//! multi-byte values whose bytes are not palindromic.

use zun_interp::CaptureOutput;

fn run_source(user: &str) -> Vec<String> {
    let src = &zun_prelude::with_prelude(user);
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

/// Wrap a body in a program with a 16-byte view in scope as `v`.
fn with_view(body: &str) -> Vec<String> {
    run_source(&format!(
        "function main() {{
            let b = new ArrayBuffer(16);
            let v = new DataView(b, 0, 16);
            {body}
        }}"
    ))
}

#[test]
fn the_same_byte_reads_signed_or_unsigned() {
    // The distinction that could not be expressed before.
    let out = with_view(
        "v.setUint8(0, 255);
         print(v.getInt8(0));
         print(v.getUint8(0));
         v.setInt8(1, -128);
         print(v.getInt8(1));
         print(v.getUint8(1));",
    );
    assert_eq!(out, vec!["-1", "255", "-128", "128"]);
}

#[test]
fn signed_16_and_32_bit_round_trip() {
    let out = with_view(
        "v.setInt16(0, -2, true);
         print(v.getInt16(0, true));
         v.setInt32(4, -70000, true);
         print(v.getInt32(4, true));",
    );
    assert_eq!(out, vec!["-2", "-70000"]);
}

#[test]
fn endianness_actually_swaps_the_bytes() {
    // 0x0102 little-endian is byte 0x02 then 0x01; big-endian is the reverse.
    // A value whose bytes differ is the only way to see this.
    let out = with_view(
        "v.setInt16(0, 258, true);
         print(v.getUint8(0));
         print(v.getUint8(1));
         v.setInt16(4, 258, false);
         print(v.getUint8(4));
         print(v.getUint8(5));",
    );
    assert_eq!(out, vec!["2", "1", "1", "2"]);
}

#[test]
fn reading_back_with_the_wrong_endianness_gives_the_swapped_value() {
    let out = with_view(
        "v.setInt16(0, 258, true);
         print(v.getInt16(0, true));
         print(v.getInt16(0, false));",
    );
    // 0x0102 read the other way round is 0x0201.
    assert_eq!(out, vec!["258", "513"]);
}

#[test]
fn single_byte_access_has_no_endianness_to_get_wrong() {
    let out = with_view("v.setInt8(3, 42); print(v.getInt8(3));");
    assert_eq!(out, vec!["42"]);
}

#[test]
fn float32_round_trips_and_narrows() {
    let out = with_view(
        "v.setFloat32(0, 1.5, true);
         print(v.getFloat32(0, true));
         v.setFloat32(8, 0.0, true);
         print(v.getFloat32(8, true));",
    );
    assert_eq!(out, vec!["1.5", "0"]);
}

#[test]
fn sixty_four_bit_integers_round_trip_both_signs() {
    // JS names these getBigInt64/getBigUint64 and hands back a BigInt; Zun
    // has native 64-bit integers, so they are plain ints here.
    let out = with_view(
        "v.setInt64(0, -5, true);
         print(v.getInt64(0, true));
         v.setUint64(8, 9, true);
         print(v.getUint64(8, true));",
    );
    assert_eq!(out, vec!["-5", "9"]);
}

#[test]
fn the_existing_unsigned_accessors_still_work() {
    let out = with_view(
        "v.setUint16(0, 65535, true);
         print(v.getUint16(0, true));
         v.setUint32(4, 4294967295, true);
         print(v.getUint32(4, true));
         v.setFloat64(8, 2.5, true);
         print(v.getFloat64(8, true));",
    );
    assert_eq!(out, vec!["65535", "4294967295", "2.5"]);
}

#[test]
fn byte_length_reports_the_view_size() {
    let out = run_source(
        r#"function main() {
            let b = new ArrayBuffer(16);
            print(new DataView(b, 0, 16).byteLength());
            print(new DataView(b, 4, 8).byteLength());
        }"#,
    );
    assert_eq!(out, vec!["16", "8"]);
}

#[test]
fn writes_through_one_width_are_visible_through_another() {
    // The view is a window onto bytes, not onto typed cells.
    let out = with_view(
        "v.setUint8(0, 1);
         v.setUint8(1, 0);
         print(v.getUint16(0, true));
         print(v.getUint16(0, false));",
    );
    assert_eq!(out, vec!["1", "256"]);
}

// --- numeric literals, found while testing the above -------------------

#[test]
fn unsigned_literals_above_the_signed_range_keep_their_value() {
    // Not a DataView matter, but this is where it surfaced: writing
    // `setUint32(_, 4294967295)` read back 2147483647. Lowering built the
    // constant with `f64 as i32`, which saturates in Rust, so every u32
    // literal above i32::MAX silently became i32::MAX.
    let out = run_source(
        r#"function main() {
            let max: u32 = 4294967295;
            let mid: u32 = 3000000000;
            let small: u32 = 7;
            print(max);
            print(mid);
            print(small);
        }"#,
    );
    assert_eq!(out, vec!["4294967295", "3000000000", "7"]);
}

#[test]
fn signed_and_narrower_literals_are_unaffected() {
    let out = run_source(
        r#"function main() {
            let a: i32 = -2147483648;
            let b: i32 = 2147483647;
            let c: u16 = 65535;
            let d: u8 = 255;
            let e: i8 = -128;
            print(a); print(b); print(c); print(d); print(e);
        }"#,
    );
    assert_eq!(out, vec!["-2147483648", "2147483647", "65535", "255", "-128"]);
}

#[test]
fn a_large_unsigned_literal_survives_a_round_trip_through_a_buffer() {
    let out = with_view(
        "v.setUint32(0, 4294967295, true);
         print(v.getUint32(0, true));
         print(v.getInt32(0, true));",
    );
    // The same four bytes: 4294967295 unsigned, -1 signed.
    assert_eq!(out, vec!["4294967295", "-1"]);
}
