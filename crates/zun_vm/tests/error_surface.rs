//! The shipped `builtin/error.zun` hierarchy, exercised as a running program.
//!
//! The source is included from the real file rather than restated, so these
//! test what actually ships. These cases compile the hierarchy alongside the
//! test program; `Error` is also a realm global now (NUCLEUS §22), exercised
//! end-to-end in `end_to_end.rs`.

use zun_interp::CaptureOutput;

/// The shipped error hierarchy.
const ERROR_SOURCE: &str = zun_prelude::ERRORS;

/// Compile `main` against the real error classes and return its output.
fn run_with_errors(main: &str) -> Vec<String> {
    let src = format!("{ERROR_SOURCE}\n{main}");
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
fn construction_takes_the_message_alone() {
    // The JS shape. The name belongs to the class, so making it a field would
    // force `new Error("Error", "boom")` on every caller.
    let out = run_with_errors(
        r#"function main() {
            let e = new Error("boom");
            print(e.message);
            print(e.name);
        }"#,
    );
    assert_eq!(out, vec!["boom", "Error"]);
}

#[test]
fn each_subclass_reports_its_own_name() {
    let out = run_with_errors(
        r#"function main() {
            print(new EvalError("x").name);
            print(new RangeError("x").name);
            print(new ReferenceError("x").name);
            print(new SyntaxError("x").name);
            print(new TypeError("x").name);
            print(new URIError("x").name);
        }"#,
    );
    assert_eq!(
        out,
        vec![
            "EvalError",
            "RangeError",
            "ReferenceError",
            "SyntaxError",
            "TypeError",
            "URIError"
        ]
    );
}

#[test]
fn subclasses_inherit_the_message_field() {
    let out = run_with_errors(
        r#"function main() {
            let e = new TypeError("bad value");
            print(e.message);
            print(e.toString());
        }"#,
    );
    assert_eq!(out, vec!["bad value", "TypeError: bad value"]);
}

#[test]
fn to_string_omits_the_separator_when_the_message_is_empty() {
    // `new Error().toString()` is "Error" in JS, not "Error: ".
    let out = run_with_errors(
        r#"function main() {
            print(new Error("").toString());
            print(new TypeError("").toString());
            print(new Error("boom").toString());
        }"#,
    );
    assert_eq!(out, vec!["Error", "TypeError", "Error: boom"]);
}

#[test]
fn the_name_getter_dispatches_virtually_through_a_base_reference() {
    // The property that makes the getter design work: code holding an `Error`
    // still sees the subclass name, which is what `catch (e) { e.name }` and
    // any logging helper depends on.
    let out = run_with_errors(
        r#"function describe(e: Error): string { return e.toString(); }
        function main() {
            print(describe(new RangeError("out of range")));
            print(describe(new Error("plain")));
        }"#,
    );
    assert_eq!(out, vec!["RangeError: out of range", "Error: plain"]);
}

#[test]
fn errors_survive_being_thrown_and_caught() {
    let out = run_with_errors(
        r#"function main() {
            try {
                throw new TypeError("thrown");
            } catch (e: Error) {
                print(e.toString());
            }
        }"#,
    );
    assert_eq!(out, vec!["TypeError: thrown"]);
}

#[test]
fn stack_starts_with_the_header_and_has_a_frame() {
    // `.stack` is the `name: message` header followed by at least one captured
    // frame line. The trace is rendered from the live interpreter frames and
    // the `.zm` sequence points (byte spans, since the VM has no source text).
    let out = run_with_errors(
        r#"function main() {
            let e = new Error("boom");
            print(e.stack);
        }"#,
    );
    assert_eq!(out.len(), 1);
    let stack = &out[0];
    assert!(stack.starts_with("Error: boom"), "stack was: {stack:?}");
    assert!(stack.contains("\n    at "), "no frame line in: {stack:?}");
    assert!(stack.contains("main"), "no main frame in: {stack:?}");
}

#[test]
fn subclass_stack_uses_the_subclass_header() {
    let out = run_with_errors(
        r#"function main() {
            print(new TypeError("x").stack);
        }"#,
    );
    assert_eq!(out.len(), 1);
    assert!(out[0].starts_with("TypeError: x"), "stack was: {:?}", out[0]);
    assert!(out[0].contains("    at "), "no frame line in: {:?}", out[0]);
}

#[test]
fn nested_calls_each_appear_as_a_frame() {
    // Innermost frame first: the helper that constructs the error, then main.
    let out = run_with_errors(
        r#"function makeIt(): Error {
            return new Error("deep");
        }
        function main() {
            print(makeIt().stack);
        }"#,
    );
    assert_eq!(out.len(), 1);
    let stack = &out[0];
    assert!(stack.starts_with("Error: deep"), "stack was: {stack:?}");
    assert!(stack.contains("makeIt"), "no makeIt frame in: {stack:?}");
    assert!(stack.contains("main"), "no main frame in: {stack:?}");
    // makeIt (innermost) is listed before main.
    let make_at = stack.find("makeIt").unwrap();
    let main_at = stack.find("main").unwrap();
    assert!(make_at < main_at, "frames out of order: {stack:?}");
}

#[test]
fn thrown_and_caught_error_keeps_its_stack() {
    let out = run_with_errors(
        r#"function main() {
            try {
                throw new RangeError("oops");
            } catch (e: Error) {
                print(e.stack);
            }
        }"#,
    );
    assert_eq!(out.len(), 1);
    assert!(out[0].starts_with("RangeError: oops"), "stack was: {:?}", out[0]);
    assert!(out[0].contains("    at "), "no frame line in: {:?}", out[0]);
}

#[test]
fn aggregate_error_carries_its_errors_and_shape() {
    let out = run_with_errors(
        r#"function main() {
            let errs = new Array(2);
            errs[0] = new Error("a");
            errs[1] = new Error("b");
            let agg = new AggregateError(errs, "several");
            print(agg.name);
            print(agg.message);
            print(agg.errors.length);
            print(agg.stack);
        }"#,
    );
    assert_eq!(out.len(), 4);
    assert_eq!(out[0], "AggregateError");
    assert_eq!(out[1], "several");
    assert_eq!(out[2], "2");
    assert!(out[3].starts_with("AggregateError: several"), "stack was: {:?}", out[3]);
}
