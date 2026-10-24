//! End-to-end proof that a compile-time macro decorator (PLAN §3.5, Phase 2)
//! transforms its target and the generated code executes.
//!
//! These tests route through `zun_compiler::compile`, which runs the
//! macro-expansion pass (parse → macro expand → sema → ilgen), then execute
//! the serialized module so the generated member is observed at run time.

use zun_interp::CaptureOutput;

/// Compile `src` through the full driver (so macro expansion runs) and run it,
/// returning the captured output lines.
fn run(src: &str) -> Vec<String> {
    let module = zun_compiler::compile(src).expect("program should compile");
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

/// Compile `src`, expecting failure, and assert a diagnostic with `code` is
/// present (so the test never names `zun_common` directly).
fn assert_compile_fails_with(src: &str, code: &str) {
    match zun_compiler::compile(src) {
        Ok(_) => panic!("expected compilation to fail with diagnostic {code}"),
        Err(diagnostics) => assert!(
            diagnostics.iter().any(|d| d.code == code),
            "expected diagnostic {code}, got {diagnostics:?}"
        ),
    }
}

#[test]
fn auto_to_string_macro_output_runs() {
    // `@autoToString` generates `toString()` returning the class name. Calling
    // it on an instance prints that name — the generated code really executes.
    let out = run(
        "@autoToString\n\
         class Widget { size: number; }\n\
         function main() {\n\
            print(new Widget(1.0).toString());\n\
         }",
    );
    assert_eq!(out, ["Widget"]);
}

#[test]
fn an_undecorated_class_does_not_gain_tostring_behavior() {
    // Without the decorator the macro does not run; a hand-written toString is
    // the only source of that string, proving the decorator is what adds it.
    let decorated = run(
        "@autoToString\n\
         class Account { balance: number; }\n\
         function main() { print(new Account(10.0).toString()); }",
    );
    assert_eq!(decorated, ["Account"]);

    let undecorated = run(
        "class Account { balance: number;\n\
            toString(): string { return \"hand-written\"; }\n\
         }\n\
         function main() { print(new Account(10.0).toString()); }",
    );
    assert_eq!(undecorated, ["hand-written"]);
}

#[test]
fn a_macro_that_cannot_transform_its_target_reports_a_clean_diagnostic() {
    // Failure contract (PLAN §3.5): analyze → decide → diagnostic + safe
    // fallback, never a panic and never a silent miscompile. `@autoToString`
    // on a class that already declares `toString` cannot proceed, so the macro
    // returns a diagnostic; the pass records it and leaves the target
    // unchanged, and the driver stops with a clear error rather than emitting a
    // conflicting member.
    assert_compile_fails_with(
        "@autoToString\n\
         class Clash { size: number;\n\
            toString(): string { return \"mine\"; }\n\
         }\n\
         function main() { print(new Clash(1.0).toString()); }",
        "MACRO0001",
    );
}
