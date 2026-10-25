//! Multi-argument calls through a bound function.

use zun_interp::CaptureOutput;

/// Compile `src` to a module, serialize it, run it, and return output lines.
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
fn bound_function_receives_all_three_call_arguments() {
    // `thisArg` is the leading parameter slot (Zun threads `this` positionally),
    // so `bind(null)` fixes `t` and the three call arguments reach `a`, `b`, `c`.
    let out = run_source(
        "function f(t: any, a: number, b: number, c: number): number { return a + b + c; }\n\
         function main() {\n\
            let g = f.bind(null);\n\
            print((g(1.0, 2.0, 3.0)) as number);\n\
         }",
    );
    assert_eq!(out, ["6"]);
}

#[test]
fn bound_call_with_prepended_and_multiple_call_arguments() {
    let out = run_source(
        "function f(a: number, b: number, c: number, d: number): number { return a * 1000.0 + b * 100.0 + c * 10.0 + d; }\n\
         function main() {\n\
            let g = f.bind(1.0, 2.0);\n\
            print((g(3.0, 4.0)) as number);\n\
         }",
    );
    assert_eq!(out, ["1234"]);
}

#[test]
fn single_argument_bound_call_still_works() {
    let out = run_source(
        "function f(a: number, b: number): number { return a + b; }\n\
         function main() {\n\
            let g = f.bind(1.0);\n\
            print((g(2.0)) as number);\n\
         }",
    );
    assert_eq!(out, ["3"]);
}
