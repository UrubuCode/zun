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
fn class_field_read() {
    let out = run_source(
        "class Point { x: number; y: number; }\n\
         function main() { let p = new Point(3, 4); print(p.x); print(p.y); }",
    );
    assert_eq!(out, ["3", "4"]);
}

#[test]
fn class_field_write_and_reference_semantics() {
    // Writing through one binding is visible through an alias (reference type).
    let out = run_source(
        "class Counter { n: number; }\n\
         function bump(c: Counter) { c.n = c.n + 1; }\n\
         function main() {\n\
            let c = new Counter(0);\n\
            bump(c); bump(c); bump(c);\n\
            print(c.n);\n\
         }",
    );
    assert_eq!(out, ["3"]);
}

#[test]
fn nested_objects() {
    let out = run_source(
        "class Inner { v: number; }\n\
         class Outer { inner: Inner; }\n\
         function main() {\n\
            let o = new Outer(new Inner(42));\n\
            print(o.inner.v);\n\
         }",
    );
    assert_eq!(out, ["42"]);
}

#[test]
fn field_assignment_as_expression() {
    let out = run_source(
        "class Box { v: number; }\n\
         function main() { let b = new Box(0); let y = (b.v = 7); print(y); print(b.v); }",
    );
    assert_eq!(out, ["7", "7"]);
}

#[test]
fn instance_methods_with_this() {
    let out = run_source(
        "class Counter {\n\
            n: number;\n\
            inc() { this.n = this.n + 1; }\n\
            get(): number { return this.n; }\n\
         }\n\
         function main() {\n\
            let c = new Counter(0);\n\
            c.inc(); c.inc(); c.inc();\n\
            print(c.get());\n\
         }",
    );
    assert_eq!(out, ["3"]);
}

#[test]
fn method_with_parameters() {
    let out = run_source(
        "class Adder {\n\
            base: number;\n\
            add(x: number): number { return this.base + x; }\n\
         }\n\
         function main() { let a = new Adder(10); print(a.add(5)); }",
    );
    assert_eq!(out, ["15"]);
}

#[test]
fn method_calling_another_method() {
    let out = run_source(
        "class Rect {\n\
            w: number;\n\
            h: number;\n\
            area(): number { return this.w * this.h; }\n\
            describe(): number { return this.area() + this.area(); }\n\
         }\n\
         function main() { let r = new Rect(3, 4); print(r.describe()); }",
    );
    assert_eq!(out, ["24"]); // 12 + 12
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
