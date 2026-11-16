//! `super(...)` constructor chaining and `super.method(...)` direct base calls.

use zun_interp::CaptureOutput;

fn run(src: &str) -> Vec<String> {
    let module = zun_compiler::compile_with_default_std(src).expect("compile");
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&module.write(), &mut out).expect("program should run");
    out.lines
}

#[test]
fn super_method_calls_base_implementation() {
    let out = run(r#"class Animal {
            speak(): string { return "base"; }
        }
        class Dog extends Animal {
            speak(): string { return "woof+" + super.speak(); }
        }
        function main() {
            let d = new Dog();
            print(d.speak());
            let a: Animal = d;
            print(a.speak());
        }"#);
    assert_eq!(out, ["woof+base", "woof+base"]);
}

#[test]
fn super_constructor_sets_inherited_field() {
    let out = run(r#"class Base {
            n: number;
            constructor(n: number) { this.n = n; }
        }
        class Derived extends Base {
            m: number;
            constructor(n: number, m: number) { super(n * 2); this.m = m; }
        }
        function main() {
            let d = new Derived(5, 7);
            print(d.n);
            print(d.m);
        }"#);
    assert_eq!(out, ["10", "7"]);
}

#[test]
fn super_constructor_three_level_chain() {
    let out = run(r#"class A {
            a: number;
            constructor() { this.a = 1; }
        }
        class B extends A {
            b: number;
            constructor() { super(); this.b = this.a + 10; }
        }
        class C extends B {
            c: number;
            constructor() { super(); this.c = this.b + 100; }
        }
        function main() {
            let c = new C();
            print(c.a);
            print(c.b);
            print(c.c);
        }"#);
    assert_eq!(out, ["1", "11", "111"]);
}

#[test]
fn super_method_is_non_virtual() {
    let out = run(r#"class A {
            who(): string { return "A"; }
            describe(): string { return this.who(); }
        }
        class B extends A {
            who(): string { return "B"; }
            describe(): string { return "B>" + super.describe(); }
        }
        function main() {
            let b = new B();
            print(b.describe());
        }"#);
    // `super.describe()` is direct, but the inner `this.who()` is still virtual.
    assert_eq!(out, ["B>B"]);
}

#[test]
fn super_method_does_not_redispatch_to_override() {
    let out = run(r#"class A {
            f(): number { return 1; }
        }
        class B extends A {
            f(): number { return super.f() + 1; }
        }
        class C extends B {
            f(): number { return super.f() + 1; }
        }
        function main() {
            print(new C().f());
        }"#);
    assert_eq!(out, ["3"]);
}

#[test]
fn super_without_base_or_outside_constructor_is_an_error() {
    let outside = r#"class A { constructor() {} }
        class B extends A {
            constructor() { super(); }
            m(): number { super(); return 1; }
        }
        function main() {}"#;
    assert!(zun_compiler::compile_with_default_std(outside).is_err());

    let no_base = r#"class A {
            f(): number { return super.f(); }
        }
        function main() {}"#;
    assert!(zun_compiler::compile_with_default_std(no_base).is_err());

    let bad_args = r#"class A {}
        class B extends A { constructor() { super(1); } }
        function main() {}"#;
    assert!(zun_compiler::compile_with_default_std(bad_args).is_err());
}

#[test]
fn super_with_no_base_constructor_and_no_args_is_a_noop() {
    let out = run(r#"class A { x: number = 4; }
        class B extends A {
            constructor() { super(); }
        }
        function main() { print(new B().x); }"#);
    assert_eq!(out, ["4"]);
}

#[test]
fn super_constructor_forwards_rest_and_spread_arguments() {
    let out = run(r#"class A {
            first: any;
            items: any[];
            constructor(first: any, ...rest: any[]) { this.first = first; this.items = rest; }
            show(): string { return this.first + "[" + this.items.join(",") + "]"; }
        }
        class B extends A { constructor(...xs: any[]) { super("b", ...xs); } }
        class C extends A { constructor(a: any, b: any) { super(a, b, b + 1, ...[7, 8]); } }
        class D extends A { constructor() { super("only"); } }
        class E extends B { constructor(x: any, ...more: any[]) { super(x, ...more, "end"); } }
        function main() {
            print(new B(1, 2, 3).show());
            print(new B().show());
            print(new C("c", 4).show());
            print(new D().show());
            print(new E(1, 2).show());
        }"#);
    assert_eq!(out, ["b[1,2,3]", "b[]", "c[4,5,7,8]", "only[]", "b[1,2,end]"]);
}

#[test]
fn static_method_with_a_rest_parameter_gathers_its_arguments() {
    let out = run(r#"class S {
            static sum(...ns: number[]): number { let s = 0; for (const n of ns) s += n; return s; }
            static tagged(tag: string, ...rest: any[]): string { return tag + rest.length; }
        }
        class S2 extends S {}
        function main() {
            print(String(S.sum() + ":" + S.sum(1) + ":" + S.sum(1, 2, 3) + ":" + S.sum(...[4, 5], 6)));
            print(S.tagged("t") + ":" + S.tagged("t", 1, 2) + ":" + S2.tagged("u", ...[1]));
        }"#);
    assert_eq!(out, ["0:1:6:15", "t0:t2:u1"]);
}
