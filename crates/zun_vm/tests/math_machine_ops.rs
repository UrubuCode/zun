//! The float math machine ops observed end to end, through the real `.zm`
//! round-trip, as a running program sees them.
//!
//! The unit and equivalence suites work on synthesized bodies; these go
//! through source, sema, lowering and serialization, so they are what catches
//! a break anywhere along the way.

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
fn negating_zero_produces_negative_zero() {
    // The reason unary minus moved off `0 - x`: that computes `+0.0` here, so
    // the two divisions used to print the same thing. The sign of the result
    // is the only way to observe the sign of a zero.
    //
    // The spelling is Rust's (`inf`), not JS's (`Infinity`) — a pre-existing
    // gap in `format_number`, unrelated to this change and left to the Number
    // surface work.
    let out = run_source(
        r#"function main() {
            let zero = 0.0;
            print(1.0 / -zero);
            print(1.0 / zero);
        }"#,
    );
    assert_eq!(out, vec!["-inf", "inf"]);
}

#[test]
fn negation_still_works_for_ordinary_values() {
    let out = run_source(
        r#"function main() {
            let value = 3.5;
            print(-value);
            print(-(-value));
        }"#,
    );
    assert_eq!(out, vec!["-3.5", "3.5"]);
}

#[test]
fn math_intrinsics_run_through_the_machine_ops() {
    let out = run_source(
        r#"import { math } from "zun:core";
        function main() {
            print(math.sqrtNumber(16.0));
            print(math.minNumber(2.0, 1.0));
            print(math.maxNumber(2.0, 1.0));
            print(math.floorNumber(1.9));
            print(math.ceilNumber(1.1));
            print(math.truncNumber(-1.9));
        }"#,
    );
    assert_eq!(out, vec!["4", "1", "2", "1", "2", "-1"]);
}

#[test]
fn min_and_max_propagate_nan_end_to_end() {
    let out = run_source(
        r#"import { math } from "zun:core";
        function main() {
            let nan = 0.0 / 0.0;
            print(math.minNumber(nan, 1.0));
            print(math.maxNumber(1.0, nan));
        }"#,
    );
    assert_eq!(out, vec!["NaN", "NaN"]);
}
