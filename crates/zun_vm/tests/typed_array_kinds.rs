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
