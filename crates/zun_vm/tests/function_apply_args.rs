//! Function values inside `apply` argument arrays.

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
fn apply_passes_a_function_in_the_args_array() {
    let out = run_source(
        "function g(t: any, f: Function, x: number): number { return (f(x)) as number; }\n\
         function dbl(n: number): number { return n * 2.0; }\n\
         function main() {\n\
            print((g.apply(null, [dbl, 4.0])) as number);\n\
         }",
    );
    assert_eq!(out, ["8"]);
}

#[test]
fn apply_round_trips_a_closure_through_the_args_array() {
    let out = run_source(
        "function g(a: number, f: Function, x: number): number { return a + ((f(x)) as number); }\n\
         function main() {\n\
            let k: number = 10.0;\n\
            let cb = (n) => (n as number) + k;\n\
            print((g.apply(1.0, [cb, 2.0])) as number);\n\
         }",
    );
    assert_eq!(out, ["13"]);
}
