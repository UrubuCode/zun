//! `type X = ...` aliases and `interface X { ... }` declarations. Both are
//! erased: an interface (and a structural/union alias) resolves to `any`, while
//! a simple alias to a named type keeps that type.

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
fn alias_to_primitive_keeps_its_type() {
    // `type Id = number` resolves to the frozen number type, so arithmetic on a
    // parameter typed `Id` type-checks without any dynamic boxing.
    let out = run_source(
        r#"type Id = number;
        function inc(x: Id): Id { return x + 1.0; }
        function main() { print(inc(41.0)); }"#,
    );
    assert_eq!(out, ["42"]);
}

#[test]
fn alias_to_class_resolves_to_instance() {
    let out = run_source(
        r#"class Point { x: number; constructor(x: number) { this.x = x; } }
        type P = Point;
        function origin(): P { return new Point(0.0); }
        function main() { print(origin().x); }"#,
    );
    assert_eq!(out, ["0"]);
}

#[test]
fn interface_is_erased_to_any() {
    // An interface name is usable as an annotation and resolves to `any`, so a
    // value of any shape flows through it.
    let out = run_source(
        r#"interface Named {
            name: string;
            tag(): string;
        }
        function id(n: Named): Named { return n; }
        function main() {
            let o: any = 7.0;
            print(id(o) as number);
        }"#,
    );
    assert_eq!(out, ["7"]);
}

#[test]
fn generic_and_union_alias_erase() {
    let out = run_source(
        r#"type Box<T> = T;
        type Scalar = number | string;
        function echo(x: Scalar): Scalar { return x; }
        function main() {
            print(echo(5.0) as number);
            print(echo("s") as string);
        }"#,
    );
    assert_eq!(out, ["5", "s"]);
}

#[test]
fn interface_with_extends_and_generics() {
    // `interface D<T> extends B<T> { ... }` parses (heritage + generics + body
    // are erased) and the name resolves to `any`.
    let out = run_source(
        r#"interface Base<T> { value: T; }
        interface Derived<T> extends Base<T> { extra: boolean; }
        function take(d: Derived<number>): number { return 3.0; }
        function main() {
            let obj: any = 3.0;
            print(take(obj));
        }"#,
    );
    assert_eq!(out, ["3"]);
}
