//! `ArrayBuffer`'s surface as a running program sees it.
//!
//! `Ty::Buffer` is what the realm registers under the name `ArrayBuffer`, and
//! it had no members at all: `new ArrayBuffer(8).byteLength` reported "type
//! Buffer has no fields".
//!
//! These are methods, not the properties JS has. That matches the rest of this
//! codebase — `String.length()` and `Map.size()` are methods too — and the
//! divergence is tracked in NUCLEUS rather than fixed per type.

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
fn byte_length_reports_the_allocated_size() {
    let out = run_source(
        r#"function main() {
            print(new ArrayBuffer(8).byteLength());
            print(new ArrayBuffer(0).byteLength());
        }"#,
    );
    assert_eq!(out, vec!["8", "0"]);
}

#[test]
fn slice_returns_the_requested_length() {
    let out = run_source(
        r#"function main() {
            let b = new ArrayBuffer(8);
            print(b.slice(2, 6).byteLength());
            print(b.slice(0, 8).byteLength());
        }"#,
    );
    assert_eq!(out, vec!["4", "8"]);
}

#[test]
fn slice_clamps_instead_of_failing() {
    // JS clamps out-of-range and inverted bounds rather than throwing.
    let out = run_source(
        r#"function main() {
            let b = new ArrayBuffer(8);
            print(b.slice(6, 2).byteLength());
            print(b.slice(4, 99).byteLength());
            print(b.slice(50, 99).byteLength());
        }"#,
    );
    assert_eq!(out, vec!["0", "4", "0"]);
}

#[test]
fn slicing_leaves_the_source_intact() {
    let out = run_source(
        r#"function main() {
            let b = new ArrayBuffer(8);
            let piece = b.slice(2, 4);
            print(b.byteLength());
            print(piece.byteLength());
        }"#,
    );
    assert_eq!(out, vec!["8", "2"]);
}

#[test]
fn a_slice_can_be_sliced_again() {
    let out = run_source(
        r#"function main() {
            print(new ArrayBuffer(16).slice(4, 12).slice(2, 6).byteLength());
        }"#,
    );
    assert_eq!(out, vec!["4"]);
}

/// Like the typed-array tests, `isView` needs the realm's view constructors,
/// so its program is compiled with the prelude in place.
fn run_with_prelude(user: &str) -> Vec<String> {
    let src = zun_prelude::with_prelude(user);
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(&src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(&src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

#[test]
fn is_view_is_true_for_views_and_false_for_everything_else() {
    let out = run_with_prelude(
        r#"function main() {
            let b = new ArrayBuffer(16);
            let a = new Uint8Array(b, 0, 8);
            let dv = new DataView(b, 0, 16);
            print(ArrayBuffer.isView(a));
            print(ArrayBuffer.isView(dv));
            // A plain ArrayBuffer is not itself a view.
            print(ArrayBuffer.isView(b));
            // An object that is not a view, and a non-object value.
            let other = new Array(2);
            print(ArrayBuffer.isView(other));
            print(ArrayBuffer.isView(42));
        }"#,
    );
    assert_eq!(out, vec!["true", "true", "false", "false", "false"]);
}

// --- transfer / transferToFixedLength / detached (NUCLEUS #17) ---

#[test]
fn transfer_detaches_the_source_and_keeps_the_length() {
    let out = run_source(
        r#"function main() {
            let b = new ArrayBuffer(8);
            print(b.detached());
            let moved = b.transfer();
            print(moved.byteLength());
            print(b.byteLength());
            print(b.detached());
            print(moved.detached());
        }"#,
    );
    assert_eq!(out, vec!["false", "8", "0", "true", "false"]);
}

#[test]
fn transfer_can_resize_the_backing_store() {
    let out = run_source(
        r#"function main() {
            print(new ArrayBuffer(8).transfer(16).byteLength());
            print(new ArrayBuffer(8).transfer(4).byteLength());
        }"#,
    );
    assert_eq!(out, vec!["16", "4"]);
}

#[test]
fn transfer_preserves_the_surviving_bytes() {
    let out = run_with_prelude(
        r#"function main() {
            let b = new ArrayBuffer(8);
            let src = new Uint8Array(b, 0, 8);
            src.set(0, 42);
            src.set(7, 99);
            let moved = b.transfer();
            let dst = new Uint8Array(moved, 0, 8);
            print(dst.get(0));
            print(dst.get(7));
        }"#,
    );
    assert_eq!(out, vec!["42", "99"]);
}

#[test]
fn transfer_to_fixed_length_yields_a_non_resizable_buffer() {
    let out = run_source(
        r#"function main() {
            let b = new ArrayBuffer(8, { maxByteLength: 16 });
            print(b.resizable());
            let kept = b.transfer();
            print(kept.resizable());
            let fixed = new ArrayBuffer(8, { maxByteLength: 16 }).transferToFixedLength();
            print(fixed.resizable());
        }"#,
    );
    assert_eq!(out, vec!["true", "true", "false"]);
}
