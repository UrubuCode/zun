//! Union/intersection types and object/tuple/function type literals in
//! annotation position. Zun has no sum/product types at runtime, so these all
//! erase to `any` — the annotation parses and the value flows dynamically.

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
fn union_parameter_accepts_either_member() {
    let out = run_source(
        r#"function describe(x: number | string): string { return typeof x; }
        function main() {
            print(describe(42.0));
            print(describe("hi"));
        }"#,
    );
    assert_eq!(out, ["number", "string"]);
}

#[test]
fn union_return_type() {
    let out = run_source(
        r#"function pick(b: boolean): number | string {
            if (b) { return 7.0; }
            return "no";
        }
        function main() {
            print(pick(true) as number);
            print(pick(false) as string);
        }"#,
    );
    assert_eq!(out, ["7", "no"]);
}

#[test]
fn object_type_literal_annotation_erases() {
    let out = run_source(
        r#"function main() {
            let o: { a: number; b: string } = { a: 1.0, b: "x" };
            print(o.a as number);
            print(o.b as string);
        }"#,
    );
    assert_eq!(out, ["1", "x"]);
}

#[test]
fn function_type_annotation_erases() {
    let out = run_source(
        r#"function main() {
            let cb: (n: number) => number = (n: number) => n + 1.0;
            print(cb(41.0) as number);
        }"#,
    );
    assert_eq!(out, ["42"]);
}

#[test]
fn intersection_and_union_with_classes() {
    let out = run_source(
        r#"class A { x: number; constructor() { this.x = 1.0; } }
        class B { y: number; constructor() { this.y = 2.0; } }
        function main() {
            let v: A | B = new A();
            print((v as A).x as number);
        }"#,
    );
    assert_eq!(out, ["1"]);
}

#[test]
fn array_of_union() {
    let out = run_source(
        r#"function main() {
            let xs: (number | string)[] = [1.0, "two", 3.0];
            print(xs.length as number);
        }"#,
    );
    assert_eq!(out, ["3"]);
}
