//! Class `constructor` methods: `new C(args)` allocates with field defaults,
//! then runs the user `constructor` with `this` = the instance. Classes without
//! a `constructor` keep positional field initialization. `super(...)` chaining
//! to a base constructor is supported (see `super_calls.rs`).

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
fn constructor_derives_fields_and_validates() {
    let out = run_source(
        r#"class Rect {
            w: number;
            h: number;
            area: number;
            constructor(w: number, h: number) {
                if (w < 0) { throw new Error("negative width"); }
                this.w = w;
                this.h = h;
                this.area = w * h;
            }
        }
        function main() {
            let r = new Rect(3, 4);
            print(r.w);
            print(r.h);
            print(r.area);
            try {
                let bad = new Rect(-1, 2);
                print(bad.area);
            } catch (e: Error) {
                print(e.message);
            }
        }"#,
    );
    assert_eq!(out, ["3", "4", "12", "negative width"]);
}

#[test]
fn constructor_with_fewer_params_than_fields_keeps_defaults() {
    let out = run_source(
        r#"class Counter {
            n: number;
            step: number = 5;
            label: string;
            ready: boolean;
            constructor(n: number) {
                this.n = n;
            }
            total(): number { return this.n + this.step; }
        }
        function main() {
            let c = new Counter(10);
            print(c.n);
            print(c.step);
            print(c.label === "");
            print(c.ready);
            print(c.total());
        }"#,
    );
    assert_eq!(out, ["10", "5", "true", "false", "15"]);
}

#[test]
fn constructor_can_call_methods() {
    let out = run_source(
        r#"class Box {
            v: number;
            doubled: number;
            constructor(v: number) {
                this.v = v;
                this.doubled = this.twice();
            }
            twice(): number { return this.v * 2; }
        }
        function main() {
            print(new Box(21).doubled);
        }"#,
    );
    assert_eq!(out, ["42"]);
}

#[test]
fn class_without_constructor_is_positional() {
    let out = run_source(
        r#"class P { x: number; y: number = 9; }
        function main() {
            let a = new P(1, 2);
            print(a.x);
            print(a.y);
            let b = new P(3);
            print(b.x);
            print(b.y);
        }"#,
    );
    assert_eq!(out, ["1", "2", "3", "9"]);
}

#[test]
fn constructor_runs_hot_in_a_loop() {
    // Many constructions drive the callee past the tiering threshold.
    let out = run_source(
        r#"class V {
            a: number;
            b: number;
            constructor(a: number) { this.a = a; this.b = a + 1; }
        }
        function main() {
            let sum = 0;
            for (let i = 0; i < 2000; i = i + 1) {
                let v = new V(i);
                sum = sum + v.b;
            }
            print(sum);
        }"#,
    );
    assert_eq!(out, ["2001000"]);
}
