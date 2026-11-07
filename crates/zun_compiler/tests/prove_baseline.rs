//! Committed, reproducible baseline for the phase-3 monomorphization metric.
//!
//! The goal's primary evidence ("`any` becomes the expensive exception") is the
//! `zun prove` report. Before this harness that number was only ever produced by
//! ad-hoc in-session runs, so it was lost on context clear and could not be
//! regression-gated. This test pins the metric to a fixed, checked-in corpus so
//! each phase can show the counts moving. It asserts nothing about the prelude's
//! absolute size (that drifts as the standard library grows); it asserts the
//! classification of a small, representative user corpus whose every verdict is
//! hand-derivable, which is what actually exercises the analysis.

use zun_sema::{BailReason, Verdict};

/// A representative user corpus that exercises one of each decisive verdict:
/// a clean monomorphic candidate, a polymorphic bail, a read-as-value bail, an
/// any-argument bail, and an exported entry. Every call site is intra-module.
const CORPUS: &str = "\
function mono(value: any): void { print(value); }\n\
function poly(value: any): any { return value; }\n\
function helper(value: number): number { return value + 1; }\n\
function apply(callback: any, x: number): number { return x; }\n\
function tainted(value: any): void { print(value); }\n\
function main(): void {\n\
    const s = \"aaaa\";\n\
    mono(s); mono(\"bbbb\");\n\
    poly(\"a\"); poly(1);\n\
    apply(helper, 2); helper(3);\n\
    const d: any = s;\n\
    tainted(\"x\"); tainted(d);\n\
}";

fn verdict<'a>(report: &'a zun_sema::ProveReport, name: &str) -> &'a Verdict {
    &report
        .functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("function `{name}` not analyzed"))
        .verdict
}

#[test]
fn prove_baseline_corpus_is_classified_as_expected() {
    let report =
        zun_compiler::prove_with_oxc(CORPUS, true).expect("corpus compiles without errors");

    // `mono` receives the same concrete `string` at every site -> candidate.
    assert!(
        matches!(verdict(&report, "mono"), Verdict::Monomorphic(params) if params == &vec![zun_sema::Ty::String]),
        "mono should be monomorphic at string, got {:?}",
        verdict(&report, "mono")
    );

    // `poly` sees string and number -> polymorphic bail.
    assert_eq!(
        verdict(&report, "poly"),
        &Verdict::Generic(BailReason::PolymorphicCallSites)
    );

    // `helper` is passed as a value to `apply` -> read-as-value bail, even
    // though its direct call site agrees on `number`.
    assert_eq!(
        verdict(&report, "helper"),
        &Verdict::Generic(BailReason::ReadAsValue)
    );

    // `tainted` sees one concrete string and one `any` site. Under the current
    // all-or-nothing rule this is an any-argument bail. If partial
    // specialization is ever approved, this expectation flips to Monomorphic
    // and this assertion is the regression gate that documents the change.
    assert_eq!(
        verdict(&report, "tainted"),
        &Verdict::Generic(BailReason::AnyArgument),
        "tainted is the partial-specialization sentinel"
    );

    // `main` is the exported entry point.
    assert_eq!(
        verdict(&report, "main"),
        &Verdict::Generic(BailReason::Exported)
    );

    // `apply` is also a candidate: its one call site `apply(helper, 2)` passes
    // a concrete function value for `callback` and a concrete `number` for `x`,
    // so both parameters agree on a concrete type. (`helper` itself still bails
    // ReadAsValue because its value is read; the two facts are independent.)
    assert!(
        matches!(verdict(&report, "apply"), Verdict::Monomorphic(_)),
        "apply should be monomorphic (concrete function + number), got {:?}",
        verdict(&report, "apply")
    );

    // Exactly two candidates (`mono`, `apply`) in the user corpus.
    let user_candidates = ["mono", "poly", "helper", "apply", "tainted", "main"]
        .iter()
        .filter(|n| matches!(verdict(&report, n), Verdict::Monomorphic(_)))
        .count();
    assert_eq!(
        user_candidates, 2,
        "only `mono` and `apply` should be candidates"
    );

    // At least the two `any`-carrying sites (poly(1) is concrete number, not
    // any; tainted(d) is any; poly/tainted any paths) are counted. The metric
    // is the number of intra-module call sites that pass an `any` argument.
    assert!(
        report.any_call_sites >= 1,
        "expected at least one any-argument call site, got {}",
        report.any_call_sites
    );
}
