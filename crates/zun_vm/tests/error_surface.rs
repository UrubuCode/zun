//! The shipped `builtin/error.zun` hierarchy, exercised as a running program.
//!
//! The source is included from the real file rather than restated, so these
//! test what actually ships. `Error` is not yet a realm global (the registry
//! cannot name a class-backed base type — see NUCLEUS §22), so the hierarchy
//! is compiled alongside the test program instead of imported.

use zun_interp::CaptureOutput;

/// The shipped error hierarchy.
const ERROR_SOURCE: &str = include_str!("../../../builtin/error.zun");

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
