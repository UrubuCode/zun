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
