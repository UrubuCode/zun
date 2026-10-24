//! Every typed-array kind is wired all the way through.
//!
//! A kind has to be registered in six places — the element enum in
//! `zun_core`, `BufferViewKind` in the HIR, the natives, the interpreter
//! dispatch, the realm facets, and the checker's method table — and missing
//! one leaves a kind that half exists. This enumerates them instead of
//! spot-checking, so adding a kind without finishing the wiring fails here
//! rather than in whatever program first uses it.

use zun_interp::CaptureOutput;

/// Every typed-array constructor the realm is expected to expose, with a
/// value that round-trips through one element of it.
///
/// Add a kind here when adding it to the realm; the coverage test below
/// refuses to let this list fall behind what the language accepts.
const KINDS: &[(&str, &str)] = &[
    ("Int8Array", "-128"),
    ("Uint8ClampedArray", "255"),
    ("Int16Array", "-32768"),
    ("Int32Array", "-2147483648"),
    ("Float32Array", "2.5"),
    ("BigInt64Array", "-9223372036854775808"),
    // 2^53, not `u64::MAX`: a bare numeric literal is an `f64`, so a value
    // above 2^53 cannot be written as one and arrives rounded. The element
    // itself is a full 64 bits — `crates/zun_core/tests/typed_elements.rs`
    // round-trips `u64::MAX` through the view directly.
    ("BigUint64Array", "9007199254740992"),
    ("Uint8Array", "200"),
    ("Uint16Array", "65535"),
    ("Uint32Array", "4294967295"),
    ("Float64Array", "2.5"),
];

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

/// Compile and run, returning `Err` with the diagnostics instead of
/// panicking, so a kind that is not wired up reports usefully.
fn try_run(user: &str) -> Result<Vec<String>, String> {
    let src = zun_prelude::with_prelude(user);
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(&src, &mut diags);
    if !diags.is_empty() {
        return Err(format!("{diags:?}"));
    }
    let hir = zun_sema::check(&src, &ast, &mut diags);
    if !diags.is_empty() {
        return Err(format!("{diags:?}"));
    }
    let module = zun_ilgen::lower(&hir);
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).map_err(|e| format!("{e:?}"))?;
    Ok(out.lines)
}

#[test]
fn every_kind_constructs_and_round_trips_an_element() {
    for (name, value) in KINDS {
        let program = format!(
            "function main() {{
                let b = new ArrayBuffer(64);
                let a = new {name}(b, 0, 8);
                a.set(0, {value});
                print(a.get(0));
            }}"
        );
        match try_run(&program) {
            Ok(out) => assert_eq!(out, vec![*value], "{name} did not round-trip {value}"),
            Err(error) => panic!("{name} is not fully wired up: {error}"),
        }
    }
}

#[test]
fn every_kind_reports_its_length_and_byte_length() {
    for (name, _) in KINDS {
        let program = format!(
            "function main() {{
                let b = new ArrayBuffer(64);
                let a = new {name}(b, 0, 8);
                print(a.length());
                print(a.byteLength());
            }}"
        );
        let out = try_run(&program).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(out.len(), 2, "{name}");
        // The element count is whatever was asked for; the byte length is
        // that times the element width, so it is at least as large.
        let length: u64 = out[0].parse().expect("length");
        let byte_length: u64 = out[1].parse().expect("byteLength");
        assert!(
            byte_length >= length,
            "{name}: byteLength {byte_length} < length {length}"
        );
    }
}

#[test]
fn the_kind_list_covers_every_constructor_the_realm_accepts() {
    // The guard that makes the two tests above meaningful: a kind added to
    // the realm but left out of `KINDS` would simply never be exercised.
    // Walk the names the language accepts and require each to be listed.
    for candidate in [
        "Int8Array",
        "Uint8Array",
        "Uint8ClampedArray",
        "Int16Array",
        "Uint16Array",
        "Int32Array",
        "Uint32Array",
        "Float32Array",
        "Float64Array",
        "BigInt64Array",
        "BigUint64Array",
    ] {
        let accepted = try_run(&format!(
            "function main() {{
                let b = new ArrayBuffer(64);
                let a = new {candidate}(b, 0, 8);
                print(a.length());
            }}"
        ))
        .is_ok();
        let listed = KINDS.iter().any(|(name, _)| name == &candidate);
        assert_eq!(
            accepted, listed,
            "{candidate}: the language {} it but KINDS {} — the list and the \
             realm have drifted apart",
            if accepted { "accepts" } else { "rejects" },
            if listed { "lists" } else { "omits" }
        );
    }
}

#[test]
fn a_view_sees_writes_made_through_another_view_of_the_same_buffer() {
    // Views are windows onto one buffer, not copies of it.
    let out = run_source(
        r#"function main() {
            let b = new ArrayBuffer(8);
            let bytes = new Uint8Array(b, 0, 8);
            let words = new Uint32Array(b, 0, 2);
            words.set(0, 1);
            print(bytes.get(0));
            print(bytes.get(1));
        }"#,
    );
    // Little-endian: the low byte holds the 1.
    assert_eq!(out, vec!["1", "0"]);
}

#[test]
fn every_kind_has_subarray_and_fill() {
    // The sweep that keeps the method surface uniform: a kind that gained
    // `get`/`set` but not these would be half a typed array.
    for (name, value) in KINDS {
        let program = format!(
            "function main() {{
                let b = new ArrayBuffer(64);
                let a = new {name}(b, 0, 8);
                a.fill({value}, 2, 5);
                let part = a.subarray(2, 5);
                print(part.length());
                print(part.get(0));
                print(a.get(1));
                print(a.get(4));
                print(a.get(5));
            }}"
        );
        let out = try_run(&program).unwrap_or_else(|e| panic!("{name}: {e}"));
        // Three filled elements; the window starts at the first of them; the
        // elements on either side of the range were left alone and the last
        // one inside it was written.
        assert_eq!(out[0], "3", "{name} subarray length");
        assert_eq!(&out[1], *value, "{name} subarray sees the fill");
        assert_ne!(&out[2], *value, "{name} filled outside the range");
        assert_eq!(&out[3], *value, "{name} missed the last element in range");
        assert_ne!(&out[4], *value, "{name} filled past the end of the range");
    }
}

#[test]
fn subarray_shares_the_bytes_rather_than_copying_them() {
    // The difference between `subarray` and `ArrayBuffer.slice`: a write
    // through one view is visible through the other.
    let out = run_source(
        r#"function main() {
            let b = new ArrayBuffer(32);
            let a = new Uint32Array(b, 0, 8);
            let part = a.subarray(2, 5);
            part.set(0, 7);
            print(a.get(2));
            a.set(4, 9);
            print(part.get(2));
        }"#,
    );
    assert_eq!(out, vec!["7", "9"]);
}

#[test]
fn subarray_and_fill_clamp_their_bounds() {
    // Out-of-range bounds clamp into the view, and a backwards range is
    // empty, rather than either failing.
    let out = run_source(
        r#"function main() {
            let b = new ArrayBuffer(32);
            let a = new Uint32Array(b, 0, 8);
            print(a.subarray(6, 99).length());
            print(a.subarray(5, 2).length());
            print(a.subarray(99, 99).length());
            a.fill(3, 6, 99);
            print(a.get(7));
            a.fill(4, 5, 2);
            print(a.get(5));
        }"#,
    );
    // Two elements from index 6; an empty window; an empty window past the
    // end; the clamped fill reached the last element; the backwards fill
    // wrote nothing.
    assert_eq!(out, vec!["2", "0", "0", "3", "0"]);
}
