//! Generic syntax support. Zun erases generics (PLAN pragmatism, like untyped
//! arrays): type parameters resolve to `any` and type arguments are parsed and
//! discarded. These tests pin that the full TS generic surface parses and runs,
//! and — crucially — that the `<`/`>` disambiguation leaves ordinary
//! comparisons intact.

use zun_interp::CaptureOutput;

fn run_source(user: &str) -> Vec<String> {
    let src = zun_prelude::with_prelude(user);
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(&src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(&src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let bytes = zun_ilgen::lower(&hir).write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

#[test]
fn generic_function_identity() {
    let out = run_source(
        r#"function identity<T>(x: T): T { return x; }
        function main() {
            print(identity(42.0) as number);
            print(identity("hi") as string);
        }"#,
    );
    assert_eq!(out, ["42", "hi"]);
}

#[test]
fn generic_function_with_explicit_type_arguments() {
    // `f<number>(x)` at the call site: the type argument is erased, the `(`
    // commits the generic call.
    let out = run_source(
        r#"function wrap<T>(x: T): T { return x; }
        function main() {
            let n = wrap<number>(10.0) as number;
            print(n + 1.0);
        }"#,
    );
    assert_eq!(out, ["11"]);
}

#[test]
fn generic_class_with_constructor() {
    let out = run_source(
        r#"class Box<T> {
            value: T;
            constructor(v: T) { this.value = v; }
            get(): T { return this.value; }
        }
        function main() {
            let b = new Box<number>(42.0);
            print(b.get() as number);
        }"#,
    );
    assert_eq!(out, ["42"]);
}

#[test]
fn generic_method_and_multiple_params() {
    let out = run_source(
        r#"class Pair<K, V> {
            k: K;
            v: V;
            constructor(k: K, v: V) { this.k = k; this.v = v; }
            first<R>(fallback: R): K { return this.k; }
        }
        function main() {
            let p = new Pair<string, number>("x", 7.0);
            print(p.first<boolean>(false) as string);
            print(p.v as number);
        }"#,
    );
    assert_eq!(out, ["x", "7"]);
}

#[test]
fn nested_generic_annotation_closes_glued_angles() {
    // `Map<string, Array<number>>` ends in a glued `>>` token that must split
    // to close both generic lists.
    let out = run_source(
        r#"function main() {
            let m: Map<string, Array<number>> = new Map();
            print(m.size);
        }"#,
    );
    assert_eq!(out, ["0"]);
}

#[test]
fn comparison_is_not_mistaken_for_type_arguments() {
    // The `<`/`>` disambiguation must leave real comparisons alone. `a < b`
    // with no following `(` stays a comparison; `a < b > c` is `(a < b) > c`.
    let out = run_source(
        r#"function main() {
            let a = 3.0; let b = 5.0; let c = 1.0;
            print(a < b);
            print(a > b);
            print((a < b) == true);
            print(b > c);
        }"#,
    );
    assert_eq!(out, ["true", "false", "true", "true"]);
}

#[test]
fn generic_call_in_comparison_context() {
    // `size<number>(xs) < 10` — a generic call whose result is then compared.
    let out = run_source(
        r#"function count<T>(xs: T[]): number { return xs.length as number; }
        function main() {
            let xs = [1.0, 2.0, 3.0];
            print(count<number>(xs) < 10.0);
        }"#,
    );
    assert_eq!(out, ["true"]);
}
