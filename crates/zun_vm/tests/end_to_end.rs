//! End-to-end pipeline tests: TypeScript source all the way to program output,
//! through the real `.zm` serialization round-trip.

use zun_interp::CaptureOutput;

/// Compile `src` to a module, serialize it, and run the bytes, returning the
/// captured output lines.
fn run_source(src: &str) -> Vec<String> {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);

    // Go through the on-disk format so the test also exercises serialization.
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

#[test]
fn hello_world() {
    assert_eq!(run_source(r#"function main() { print("hello, zun"); }"#), ["hello, zun"]);
}

#[test]
fn arithmetic_and_locals() {
    let out = run_source(
        "function main() { let a = 6; let b = 7; print(a * b); }",
    );
    assert_eq!(out, ["42"]);
}

#[test]
fn user_function_call_with_args() {
    let out = run_source(
        "function add(a: number, b: number): number { return a + b; }\n\
         function main() { print(add(2, 40)); }",
    );
    assert_eq!(out, ["42"]);
}

#[test]
fn while_loop_sums() {
    let out = run_source(
        "function main() {\n\
            let n = 10; let acc = 0;\n\
            while (n > 0) { acc = acc + n; n = n - 1; }\n\
            print(acc);\n\
         }",
    );
    assert_eq!(out, ["55"]); // 10+9+...+1
}

#[test]
fn if_else_picks_branch() {
    let out = run_source(
        "function main() {\n\
            let x = 3;\n\
            if (x > 5) { print(\"big\"); } else { print(\"small\"); }\n\
         }",
    );
    assert_eq!(out, ["small"]);
}

#[test]
fn recursion_factorial() {
    let out = run_source(
        "function fact(n: number): number {\n\
            if (n < 2) { return 1; }\n\
            return n * fact(n - 1);\n\
         }\n\
         function main() { print(fact(5)); }",
    );
    assert_eq!(out, ["120"]);
}

#[test]
fn string_concatenation() {
    let out = run_source(
        "function greet(who: string): string { return \"hi, \" + who; }\n\
         function main() { print(greet(\"zun\")); }",
    );
    assert_eq!(out, ["hi, zun"]);
}

#[test]
fn booleans_and_not() {
    let out = run_source(
        "function main() {\n\
            let ready = false;\n\
            if (!ready) { print(\"waiting\"); }\n\
         }",
    );
    assert_eq!(out, ["waiting"]);
}
