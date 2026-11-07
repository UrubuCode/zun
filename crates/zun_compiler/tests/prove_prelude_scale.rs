//! Reproducible prelude-scale baseline for the phase-3 monomorphization metric.
//!
//! The in-session "653 any-argument sites / 231 candidates / 66 specializations"
//! figure could not be reproduced: no committed corpus produces it, and the
//! prelude (the natural large corpus) yields a much smaller, honest number (see
//! the structural assertions below and `docs/refactor-findings.md`). This test
//! measures the whole prelude deterministically and asserts the invariants that
//! must hold regardless of how the standard library grows, so the metric is
//! reproducible and regression-gated without being brittle to unrelated std
//! edits. The exact snapshot is printed (run with `--nocapture`) and recorded in
//! the findings doc.

#[test]
fn prelude_scale_metric_is_well_formed() {
    let combined = zun_prelude::with_prelude("");
    let ast = zun_syntax::oxc::parse_subset(&combined, true).expect("prelude parses");
    let total = ast.functions.len();
    let mut diags = Vec::new();
    let hir = zun_sema::check(&combined, &ast, &mut diags);
    let report = zun_sema::prove_program(&hir, total);

    eprintln!(
        "PRELUDE-SCALE (partial OFF): total={} candidates={} generic={} any_call_sites={}",
        report.functions.len(),
        report.candidate_count,
        report.generic_count,
        report.any_call_sites
    );
    for (reason, count) in &report.bail_histogram {
        eprintln!("  bail {:>4} {}", count, reason.label());
    }

    // Partial specialization before/after: the same corpus analyzed with the
    // flag on must never lose a candidate, and converts some `AnyArgument` bails
    // into candidates (the dynamic sites stay on the generic).
    let partial = zun_sema::prove_program_with(&hir, total, true);
    eprintln!(
        "PRELUDE-SCALE (partial ON):  total={} candidates={} generic={} any_call_sites={}",
        partial.functions.len(),
        partial.candidate_count,
        partial.generic_count,
        partial.any_call_sites
    );
    for (reason, count) in &partial.bail_histogram {
        eprintln!("  bail {:>4} {}", count, reason.label());
    }
    assert!(
        partial.candidate_count >= report.candidate_count,
        "partial specialization must never reduce the candidate count",
    );

    // Structural invariants (stable across std growth):
    assert_eq!(
        report.functions.len(),
        total,
        "every top-level function is analyzed"
    );
    assert_eq!(
        report.candidate_count + report.generic_count,
        total,
        "every function is either a candidate or generic"
    );
    assert_eq!(
        report.bail_histogram.values().sum::<usize>(),
        report.generic_count,
        "the bail histogram accounts for exactly the generic functions"
    );
    // The prelude must contain at least one monomorphization candidate, else the
    // typed floor has no footing at all.
    assert!(
        report.candidate_count >= 1,
        "expected at least one prelude candidate, got {}",
        report.candidate_count
    );
}
