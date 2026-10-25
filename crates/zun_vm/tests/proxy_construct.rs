//! `Reflect.construct` and the Proxy `construct` trap for CLASS targets.
//! Arbitrary-function constructors are out of scope (no function.prototype /
//! `this` construction model in Zun).

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
fn reflect_construct_builds_instance_from_args() {
    let out = run_source(
        r#"class C { n: number; }
        function main() {
            let o: any = Reflect.construct(C, [7.0]);
            let c = o as C;
            print(c.n);
        }"#,
    );
    assert_eq!(out, ["7"]);
}

#[test]
fn proxy_construct_trap_fires() {
    let out = run_source(
        r#"import { dynamicSetFunction } from "zun:core";
        class C { n: number; }
        function ctor(target: any, args: any, newTarget: any): any {
            print("trap");
            return Reflect.construct(target, [9.0]);
        }
        function main() {
            let handler = dynamic_new();
            dynamicSetFunction(handler, "construct", ctor);
            let p = new Proxy(C, handler);
            let o: any = new p(1.0);
            let c = o as C;
            print(c.n);
        }"#,
    );
    assert_eq!(out, ["trap", "9"]);
}

#[test]
fn proxy_construct_defaults_to_target_class() {
    let out = run_source(
        r#"class C { n: number; }
        function main() {
            let p = new Proxy(C, dynamic_new());
            let o: any = new p(5.0);
            let c = o as C;
            print(c.n);
        }"#,
    );
    assert_eq!(out, ["5"]);
}

#[test]
fn new_on_proxy_from_function_call() {
    let out = run_source(
        r#"class C { n: number; }
        function getProxy(): Proxy { return new Proxy(C, dynamic_new()); }
        function main() {
            let o: any = new (getProxy())(4.0);
            let c = o as C;
            print(c.n);
        }"#,
    );
    assert_eq!(out, ["4"]);
}

#[test]
fn reflect_construct_applies_field_default() {
    let out = run_source(
        r#"class C { a: number; b: number = 7.0; }
        function main() {
            let o: any = Reflect.construct(C, [1.0]);
            let c = o as C;
            print(c.a);
            print(c.b);
        }"#,
    );
    assert_eq!(out, ["1", "7"]);
}
