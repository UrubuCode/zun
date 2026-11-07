//! Tier 0 type feedback: the dynamic `any` dispatch path records observed tags
//! per call-site when collection is enabled, and records nothing when it is off.

mod common;

const POLYMORPHIC_SRC: &str = r#"
    function describe(v: any): any {
        return v.toString();
    }
    describe("hello");
    describe(42.0);
"#;

/// A polymorphic dynamic call-site — a method called on an `any` receiver that
/// is a string on one call and a number on the next — records both observed
/// receiver tags; and with collection off the same program records nothing.
///
/// The two phases share one `#[test]` so they run sequentially: the enable flag
/// is process-global, so splitting them into parallel tests would race on it.
#[test]
fn type_feedback_records_only_when_enabled() {
    // Phase 1: feedback on.
    let module = zun_compiler::compile_with_oxc(POLYMORPHIC_SRC, true).expect("compiles");
    let report = zun_vm::execute_and_prove_feedback(&module.write()).expect("runs");

    assert!(
        report.contains("String="),
        "expected a String witness in the feedback report:\n{report}"
    );
    assert!(
        report.contains("Float=") || report.contains("Int="),
        "expected a numeric witness in the feedback report:\n{report}"
    );
    // At least one receiver histogram carries both tags (the polymorphic site).
    let polymorphic = report.lines().any(|line| {
        line.trim_start().starts_with("receiver:")
            && line.contains("String=")
            && (line.contains("Float=") || line.contains("Int="))
    });
    assert!(
        polymorphic,
        "expected one receiver histogram with both String and a numeric tag:\n{report}"
    );

    // Phase 2: feedback off (the default) records nothing.
    let module = zun_compiler::compile_with_oxc(POLYMORPHIC_SRC, true).expect("compiles");
    zun_interp::feedback::reset();
    zun_interp::feedback::set_enabled(false);
    let mut out = zun_interp::CaptureOutput::default();
    zun_vm::execute_with(&module.write(), &mut out).expect("runs");
    assert_eq!(
        zun_interp::feedback::site_count(),
        0,
        "feedback-off run must record no sites"
    );
}
