//! End-to-end pipeline tests: TypeScript source all the way to program output,
//! through the real `.zm` serialization round-trip.

use zun_interp::CaptureOutput;

/// Compile `src` with the std prelude, run it, and return the output lines.
fn run_with_std(user: &str) -> Vec<String> {
    run_source(&zun_prelude::with_prelude(user))
}

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
    assert_eq!(
        run_source(r#"function main() { print("hello, zun"); }"#),
        ["hello, zun"]
    );
}

#[test]
fn bigint_constructor_arithmetic_and_formatting_use_the_managed_floor() {
    let out = run_source(
        r#"function main(): int {
            let left: BigInt = new BigInt("18446744073709551616");
            let right: BigInt = new BigInt("2");
            let sum: BigInt = left.add(right);
            if (sum.toString() == "18446744073709551618") { print(sum.compare(left)); return 0; }
            print(0); return 0;
        }"#,
    );
    assert_eq!(out, ["1"]);
}

#[test]
fn bigint_subtraction_preserves_negative_values() {
    let out = run_source(
        r#"function main(): int {
            let left: BigInt = new BigInt("2");
            let right: BigInt = new BigInt("5");
            print(left.sub(right).toString());
            return 0;
        }"#,
    );
    assert_eq!(out, ["-3"]);
}

#[test]
fn bigint_arithmetic_operators_use_the_managed_floor() {
    let out = run_source(
        r#"function main(): int {
            let left: BigInt = new BigInt("7");
            let right: BigInt = new BigInt("3");
            print((left + right).toString());
            print((left - right).toString());
            print((left * right).toString());
            return 0;
        }"#,
    );
    assert_eq!(out, ["10", "4", "21"]);
}

#[test]
fn bigint_comparison_operators_use_the_managed_floor() {
    let out = run_source(
        r#"function main(): int {
            let left: BigInt = new BigInt("7");
            let right: BigInt = new BigInt("3");
            if (left == right) { print(0); }
            if (left != right) { print(1); }
            if (left > right) { print(2); }
            if (right <= left) { print(3); }
            return 0;
        }"#,
    );
    assert_eq!(out, ["1", "2", "3"]);
}

#[test]
fn bigint_unary_negation_uses_the_managed_floor() {
    let out = run_source(
        r#"function main(): int {
            let value: BigInt = new BigInt("7");
            print((-value).toString());
            return 0;
        }"#,
    );
    assert_eq!(out, ["-7"]);
}

#[test]
fn arithmetic_and_locals() {
    let out = run_source("function main() { let a = 6; let b = 7; print(a * b); }");
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
fn tiering_calls_a_heap_eligible_array_reader() {
    let out = run_source(
        "function read(values: Array): u64 { return values[0] as u64; }\n\
         function main() { let values = new Array(1); values[0] = 41 as u64; print(read(values)); print(read(values)); }",
    );
    assert_eq!(out, ["41", "41"]);
}

#[test]
fn tiering_calls_a_heap_eligible_f64_array_reader() {
    let out = run_source(
        "function read(values: Array): number { return values[0] as number; }\n\
         function main() { let values = new Array(1); values[0] = 41.0; print(read(values)); print(read(values)); }",
    );
    assert_eq!(out, ["41", "41"]);
}

#[test]
fn tiering_calls_a_heap_eligible_f64_array_writer() {
    let out = run_source(
        "function write(values: Array, value: number): number { values[0] = value; return value; }\n\
         function main() { let values = new Array(1); print(write(values, 41.0)); print(write(values, 42.0)); print(values[0]); }",
    );
    assert_eq!(out, ["41", "42", "42"]);
}

#[test]
fn tiering_calls_a_heap_eligible_u64_array_writer() {
    let out = run_source(
        "function write(values: Array, value: u64): u64 { values[0] = value; return value; }\n\
         function main() { let values = new Array(1); print(write(values, 41 as u64)); print(write(values, 42 as u64)); print(values[0] as u64); }",
    );
    assert_eq!(out, ["41", "42", "42"]);
}

#[test]
fn tiering_calls_a_heap_eligible_f64_array_push() {
    let out = run_source(
        "function push(values: Array, value: number): number { return values.push(value) as number; }\n\
         function main() { let values = new Array(0); print(push(values, 41.0)); print(push(values, 42.0)); print(values.length); }",
    );
    assert_eq!(out, ["1", "2", "2"]);
}

#[test]
fn tiering_calls_a_heap_eligible_f64_array_pop() {
    let out = run_source(
        "function pop(values: Array): number { return values.pop() as number; }\n\
         function main() { let values = new Array(2); values[0] = 41.0; values[1] = 42.0; print(pop(values)); print(pop(values)); print(values.length); }",
    );
    assert_eq!(out, ["42", "41", "0"]);
}

#[test]
fn tiering_calls_heap_eligible_u64_array_push_and_pop() {
    let out = run_source(
        "function push(values: Array, value: u64): u64 { return values.push(value) as u64; }\n\
         function pop(values: Array): u64 { return values.pop() as u64; }\n\
         function main() { let values = new Array(0); print(push(values, 41 as u64)); print(push(values, 42 as u64)); print(pop(values)); print(pop(values)); }",
    );
    assert_eq!(out, ["1", "2", "42", "41"]);
}

#[test]
fn tiering_calls_a_heap_eligible_object_field_reader() {
    let out = run_source(
        "class Box { value: number; get(): number { return this.value; } }\n\
         function main() { let box = new Box(41.0); print(box.get()); print(box.get()); }",
    );
    assert_eq!(out, ["41", "41"]);
}

#[test]
fn tiering_calls_a_heap_eligible_object_field_writer() {
    let out = run_source(
        "class Box { value: number; set(value: number): number { this.value = value; return this.value; } }\n\
         function main() { let box = new Box(0.0); print(box.set(41.0)); print(box.set(42.0)); print(box.value); }",
    );
    assert_eq!(out, ["41", "42", "42"]);
}

#[test]
fn tiering_calls_a_heap_eligible_non_overridden_virtual_method() {
    let out = run_source(
        "class Box { value: number; get(): number { return this.value; } }\n\
         function main() { let box = new Box(41.0); print(box.get()); print(box.get()); }",
    );
    assert_eq!(out, ["41", "41"]);
}

#[test]
fn tiering_returns_a_jit_allocated_object_through_the_vm() {
    let out = run_source(
        "class Box { value: number; }\n\
         function make(): Box { return new Box(41.0); }\n\
         function main() { let first = make(); let second = make(); print(first.value); print(second.value); }",
    );
    assert_eq!(out, ["41", "41"]);
}

#[test]
fn tiering_returns_a_managed_object_field_through_the_vm() {
    let out = run_source(
        "class Box { value: number; }\n\
         class Holder { box: Box; get(): Box { return this.box; } }\n\
         function main() { let box = new Box(41.0); let holder = new Holder(box); let first = holder.get(); let second = holder.get(); print(first.value); print(second.value); }",
    );
    assert_eq!(out, ["41", "41"]);
}

#[test]
fn tiering_returns_jit_allocated_arrays_through_the_vm() {
    let out = run_source(
        "function make(length: u64): Array { return new Array(length); }\n\
         function main() { let first = make(2 as u64); let second = make(3 as u64); print(first.length); print(second.length); }",
    );
    assert_eq!(out, ["2", "3"]);
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
fn inherited_field_and_method() {
    let out = run_source(
        "class Animal { legs: number; legCount(): number { return this.legs; } }\n\
         class Dog extends Animal { goodBoy: number; }\n\
         function main() { let d = new Dog(4, 1); print(d.legCount()); print(d.goodBoy); }",
    );
    assert_eq!(out, ["4", "1"]);
}

#[test]
fn virtual_dispatch_calls_the_override() {
    // speak() is overridden in Dog; calling through a method that only knows
    // the base type must still reach Dog's version at runtime.
    let out = run_source(
        "class Animal {\n\
            id: number;\n\
            speak(): number { return 0; }\n\
         }\n\
         class Dog extends Animal {\n\
            speak(): number { return 1; }\n\
         }\n\
         class Cat extends Animal {\n\
            speak(): number { return 2; }\n\
         }\n\
         function hear(a: Animal): number { return a.speak(); }\n\
         function main() {\n\
            print(hear(new Animal(0)));\n\
            print(hear(new Dog(1)));\n\
            print(hear(new Cat(2)));\n\
         }",
    );
    assert_eq!(out, ["0", "1", "2"]);
}

#[test]
fn override_can_extend_via_shared_field() {
    // A base method reads a field; a subclass overrides another method. Field
    // layout is inherited-first, so the base field slot is stable.
    let out = run_source(
        "class Shape {\n\
            kind: number;\n\
            area(): number { return 0; }\n\
         }\n\
         class Square extends Shape {\n\
            side: number;\n\
            area(): number { return this.side * this.side; }\n\
         }\n\
         function main() {\n\
            let s = new Square(1, 5);\n\
            print(s.area());\n\
            print(s.kind);\n\
         }",
    );
    assert_eq!(out, ["25", "1"]);
}

#[test]
fn int_type_arithmetic() {
    let out = run_source(
        "function main() {\n\
            let a: int = 10;\n\
            let b: int = 3;\n\
            print(a * b);\n\
            print(a / b);\n\
            print(a % b);\n\
         }",
    );
    // Integer division truncates (10/3 = 3), unlike float.
    assert_eq!(out, ["30", "3", "1"]);
}

#[test]
fn int_and_float_division_differ() {
    let out = run_source(
        "function main() {\n\
            let i: int = 7;\n\
            let f: float = 7.0;\n\
            print(i / 2);\n\
            print(f / 2.0);\n\
         }",
    );
    assert_eq!(out, ["3", "3.5"]);
}

#[test]
fn int_through_function_and_recursion() {
    let out = run_source(
        "function sumTo(n: int): int {\n\
            if (n == 0) { return 0; }\n\
            return n + sumTo(n - 1);\n\
         }\n\
         function main() { print(sumTo(5)); }",
    );
    assert_eq!(out, ["15"]);
}

#[test]
fn std_prelude_numeric_helpers() {
    let out = run_with_std(
        "function main() {\n\
            print(abs(-7));\n\
            print(clamp(42, 0, 10));\n\
            print(powi(2, 10));\n\
            print(max(3, 9));\n\
            print(fabs(-2.5));\n\
            print(sign(-100));\n\
         }",
    );
    assert_eq!(out, ["7", "10", "1024", "9", "2.5", "-1"]);
}

#[test]
fn fs_intrinsics_round_trip() {
    let dir = std::env::temp_dir();
    let path = dir.join("zun_fs_test.txt");
    let path_str = path.to_string_lossy().replace('\\', "/");
    let _ = std::fs::remove_file(&path);

    let src = format!(
        "import {{ writeText, readText, exists }} from \"zun:fs\";\n\
         function main() {{\n\
            let p: string = \"{path_str}\";\n\
            writeText(p, \"zun io works\");\n\
            print(readText(p));\n\
            if (exists(p)) {{ print(\"yes\"); }}\n\
         }}"
    );
    let out = run_source(&src);
    assert_eq!(out, ["zun io works", "yes"]);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn unknown_import_is_a_sema_error() {
    let src = "import { nope } from \"zun:ghost\";\nfunction main() {}";
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    zun_sema::check(src, &ast, &mut diags);
    assert!(diags.iter().any(|d| d.message.contains("unknown module")));
}

#[test]
fn gc_runs_without_corrupting_live_objects() {
    // Allocate far more objects than the GC interval (256) inside a loop while
    // keeping a live accumulator object referenced from a local. Collection
    // fires repeatedly; if roots were imprecise, the live object (or the
    // garbage churn feeding it) would be corrupted and the sum would be wrong.
    let out = run_source(
        "class Cell { v: int; }\n\
         class Acc { total: int; }\n\
         function main() {\n\
            let acc: Acc = new Acc(0);\n\
            let i: int = 0;\n\
            while (i < 5000) {\n\
                let c: Cell = new Cell(i);\n\
                acc.total = acc.total + c.v;\n\
                i = i + 1;\n\
            }\n\
            print(acc.total);\n\
         }",
    );
    // Sum of 0..4999 = 4999*5000/2 = 12497500.
    assert_eq!(out, ["12497500"]);
}

#[test]
fn adaptive_tiering_compiles_hot_methods() {
    use zun_interp::{run_with, CaptureOutput};
    // A numeric function called repeatedly from main; with a low threshold it
    // becomes hot and is compiled, and the output must stay correct whether a
    // given call was interpreted (cold) or native (hot).
    let src = "function sq(n: int): int { return n * n; }\n\
               function main() {\n\
                   let i: int = 0;\n\
                   while (i < 5) { print(sq(i)); i = i + 1; }\n\
               }";
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty());
    let module = zun_ilgen::lower(&hir);
    let bytes = module.write();

    let loaded = zun_vm::load(&bytes).unwrap();
    let jit = zun_codegen::Jit::new(&loaded).unwrap();
    let sq_rid = 1; // sq is the first function
    let hook = zun_vm::JitHook::with_threshold(jit, &loaded, 2);
    let mut out = CaptureOutput::default();
    run_with(&loaded, &mut out, Some(&hook)).unwrap();

    assert_eq!(out.lines, ["0", "1", "4", "9", "16"]);
    // After 5 calls with threshold 2, `sq` must have been compiled.
    assert!(hook.jit_is_defined(sq_rid));
    // The interpreted loop reports its backedge before the hot call path
    // takes over, providing the observation point required by future OSR.
    assert!(hook.total_loop_backedges(2) > 0);
    let hot_headers = hook.hot_loop_headers(2);
    assert!(!hot_headers.is_empty());
    assert!(hot_headers
        .iter()
        .all(|&target| hook.is_loop_hot(2, target)));
}

#[test]
fn hot_interpreted_backedge_compiles_eligible_method() {
    use zun_interp::{run_with, CaptureOutput};
    let src = "function spin(n: int): int { let i: int = 0; let total: int = 0; while (i < n) { total = total + i; i = i + 1; } return total; }\n\
               function main(): int { return spin(5); }";
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty());
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty());
    let module = zun_ilgen::lower(&hir);
    let loaded = zun_vm::load(&module.write()).unwrap();
    let spin_rid = 1;
    let jit = zun_codegen::Jit::new(&loaded).unwrap();
    let hook = zun_vm::JitHook::with_threshold(jit, &loaded, 2);
    let mut out = CaptureOutput::default();
    let result = run_with(&loaded, &mut out, Some(&hook)).unwrap();
    assert!(matches!(result, zun_interp::value::Value::Int(10, _)));
    // Eligible loop methods are compiled at their normal entry point, so no
    // interpreted backedge or mid-frame OSR request is needed.
    assert_eq!(hook.total_loop_backedges(spin_rid), 0);
    assert!(hook.jit_is_defined(spin_rid));
    assert!(hook.take_osr_requests().is_empty());
    assert!(hook.take_valid_osr_snapshots().is_empty());
    assert!(hook.take_osr_snapshots().is_empty());
}

#[test]
fn osr_rejects_snapshots_with_a_live_evaluation_stack() {
    use zun_interp::value::Value;

    let src = "function main(): int { return 1; }";
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    let hir = zun_sema::check(src, &ast, &mut diags);
    let module = zun_ilgen::lower(&hir);
    let loaded = zun_vm::load(&module.write()).unwrap();
    let jit = zun_codegen::Jit::new(&loaded).unwrap();
    let hook = zun_vm::JitHook::with_threshold(jit, &loaded, 1);
    let snapshot = zun_vm::OsrSnapshot {
        method_rid: 1,
        target_pc: 0,
        pc: 0,
        stack: vec![Value::Int(1, zun_il::NumKind::I32)],
        locals: Vec::new(),
    };
    assert!(!hook.is_valid_osr_snapshot(&snapshot));

    let snapshot_without_backedge = zun_vm::OsrSnapshot {
        method_rid: 1,
        target_pc: 0,
        pc: 0,
        stack: Vec::new(),
        locals: Vec::new(),
    };
    assert!(!hook.is_valid_osr_snapshot(&snapshot_without_backedge));
}

#[test]
fn osr_rejects_snapshots_inside_exception_regions() {
    use zun_interp::value::Value;

    let src = "class Error {} function main(): int { try { throw new Error(); } catch (error: Error) { return 42; } }";
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty());
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty());
    let module = zun_ilgen::lower(&hir);
    let loaded = zun_vm::load(&module.write()).unwrap();
    let jit = zun_codegen::Jit::new(&loaded).unwrap();
    let hook = zun_vm::JitHook::with_threshold(jit, &loaded, 1);
    let method = loaded.tables.method_defs.first().unwrap();
    let body = zun_module::read_body(&loaded.code, method.body_offset).unwrap();
    let region = body.handlers.first().expect("catch region");
    let snapshot = zun_vm::OsrSnapshot {
        method_rid: 1,
        target_pc: region.handler_start as usize,
        pc: region.handler_start as usize,
        stack: Vec::new(),
        locals: vec![Value::Void; body.local_count as usize],
    };
    assert!(!hook.is_valid_osr_snapshot(&snapshot));
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
