//! A small throughput benchmark: compile a tight integer loop, run it through
//! the interpreter (Tier 0) and the Cranelift JIT (Tier 1), and report the time
//! per loop iteration in nanoseconds.
//!
//! Run with:  cargo run --release --example bench -p zun_codegen

use std::time::Instant;

use zun_codegen::{Jit, RetVal};
use zun_interp::CaptureOutput;

/// Iterations the benchmarked function performs internally.
const N: i64 = 5_000_000;
const COLLECTION_N: i64 = 10_000;

fn compile(src: &str) -> zun_module::Module {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema: {diags:?}");
    zun_ilgen::lower(&hir)
}

fn interp_ns_per_iteration(src: &str, iterations: usize) -> f64 {
    let module = compile(src);
    let mut out = CaptureOutput::default();
    std::hint::black_box(zun_interp::run(&module, &mut out).unwrap());
    let t = Instant::now();
    for _ in 0..3 {
        let mut out = CaptureOutput::default();
        std::hint::black_box(zun_interp::run(&module, &mut out).unwrap());
    }
    t.elapsed().as_nanos() as f64 / (3.0 * iterations as f64)
}

fn benchmark_i32_map_loop() {
    let src = format!(
        "import {{ mapNew, mapSetI32, mapGetI32Or }} from \"zun:collections\";\n\
         function fill(value: Map, n: i32): i32 {{ let i: i32 = 0; while (i < n) {{ mapSetI32(value, i, i); i = i + 1; }} return mapGetI32Or(value, n - 1, -1); }}\n\
         function main(): i32 {{ let value = mapNew(); return fill(value, {COLLECTION_N}); }}"
    );
    let module = compile(&src);
    let fill = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "fill")
        .map(|index| index as u32 + 1)
        .expect("map loop method");
    let jit = Jit::compile(&module).expect("jit compiles i32 map loop");
    assert!(jit.is_compiled(fill));
    let mut heap = zun_gc::Heap::new();
    let map = heap.alloc_map();
    let jit_warmup = jit
        .call_packed_with_heap(fill, &[map.addr() as u64, COLLECTION_N as u64], &mut heap)
        .expect("jit map loop");
    let jit_reps = 10;
    let t = Instant::now();
    for _ in 0..jit_reps {
        std::hint::black_box(
            jit.call_packed_with_heap(fill, &[map.addr() as u64, COLLECTION_N as u64], &mut heap)
                .expect("jit map loop"),
        );
    }
    let jit_ns = t.elapsed().as_nanos() as f64 / (jit_reps as f64 * COLLECTION_N as f64);

    let mut out = CaptureOutput::default();
    let interp_warmup = zun_interp::run(&module, &mut out).unwrap();
    let interp_reps = 3;
    let t = Instant::now();
    for _ in 0..interp_reps {
        let mut out = CaptureOutput::default();
        std::hint::black_box(zun_interp::run(&module, &mut out).unwrap());
    }
    let interp_ns = t.elapsed().as_nanos() as f64 / (interp_reps as f64 * COLLECTION_N as f64);
    assert_eq!(jit_warmup, COLLECTION_N as u64 - 1);
    assert!(
        matches!(interp_warmup, zun_interp::value::Value::Int(value, _) if value == COLLECTION_N - 1)
    );
    println!();
    println!("i32 Map mutation loop ({COLLECTION_N} iterations/call, interp == jit: true)");
    println!("interpreter (Tier 0): {interp_ns:8.3} ns/iteration");
    println!("JIT (Tier 1):         {jit_ns:8.3} ns/iteration");
    println!("speedup:              {:8.2}x", interp_ns / jit_ns);
}

fn main() {
    // `work` does N iterations of a small integer expression; `main` returns it.
    let src = format!(
        "function work(n: int): int {{\n\
            let acc: int = 0;\n\
            let i: int = 0;\n\
            while (i < n) {{ acc = acc + i * 2 - 1; i = i + 1; }}\n\
            return acc;\n\
         }}\n\
         function main(): int {{ return work({N}); }}"
    );
    let module = compile(&src);
    let entry = module.entry_point.unwrap().rid();

    // --- JIT (Tier 1) ---
    let jit = Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry), "entry should be JIT-eligible");
    let jit_value = jit.call_entry(entry).expect("jit call"); // warm up
    let jit_reps = 50;
    let t = Instant::now();
    for _ in 0..jit_reps {
        std::hint::black_box(jit.call_entry(entry).unwrap());
    }
    let jit_ns = t.elapsed().as_nanos() as f64 / (jit_reps as f64 * N as f64);

    // --- Interpreter (Tier 0) ---
    let interp_reps = 3;
    let mut out = CaptureOutput::default();
    let interp_value = zun_interp::run(&module, &mut out).unwrap(); // warm up
    let t = Instant::now();
    for _ in 0..interp_reps {
        let mut out = CaptureOutput::default();
        std::hint::black_box(zun_interp::run(&module, &mut out).unwrap());
    }
    let interp_ns = t.elapsed().as_nanos() as f64 / (interp_reps as f64 * N as f64);

    // Sanity: both engines must agree on the result.
    let jit_result = match jit_value {
        RetVal::Int(n) => n,
        other => panic!("unexpected jit result {other:?}"),
    };
    let interp_result = match interp_value {
        zun_interp::value::Value::Int(n, _) => n,
        other => panic!("unexpected interp result {other:?}"),
    };
    assert_eq!(jit_result, interp_result);

    println!("loop body: acc = acc + i*2 - 1   ({N} iterations/call)");
    println!(
        "result: {jit_result}  (interp == jit: {})",
        jit_result == interp_result
    );
    println!();
    println!("interpreter (Tier 0): {interp_ns:8.3} ns/iteration");
    println!("JIT (Tier 1):         {jit_ns:8.3} ns/iteration");
    println!("speedup:              {:8.2}x", interp_ns / jit_ns);
    println!();
    println!(
        "per iteration there are ~4 int ops (mul, add, sub, cmp) + a store + loop,\n\
         so divide by ~4 for a rough per-op figure."
    );

    benchmark_i32_map_loop();

    // These workloads stay in Tier 0 because object operations are not yet
    // JIT-eligible. They specifically exercise the interpreter metadata
    // lookups optimized by the runtime benchmark branch.
    let field_src = format!(
        "class Counter {{ n: int; }}\n\
         function main(): int {{ let c = new Counter(0); let i: int = 0;\n\
         while (i < {N}) {{ c.n = c.n + 1; i = i + 1; }} return c.n; }}"
    );
    let virtual_src = format!(
        "class Base {{ tick(): int {{ return 1; }} }}\n\
         class Derived extends Base {{ tick(): int {{ return 1; }} }}\n\
         function run(b: Base): int {{ let acc: int = 0; let i: int = 0;\n\
         while (i < {N}) {{ acc = acc + b.tick(); i = i + 1; }} return acc; }}\n\
         function main(): int {{ return run(new Derived()); }}"
    );
    println!();
    println!(
        "field load/store loop (Tier 0): {:8.3} ns/iteration",
        interp_ns_per_iteration(&field_src, N as usize)
    );
    println!(
        "virtual dispatch loop (Tier 0):  {:8.3} ns/iteration",
        interp_ns_per_iteration(&virtual_src, N as usize)
    );

    let array_src = format!(
        "function main(): i32 {{ let values = new Array(1); values[0] = 0; let i: i32 = 0;\n\
         while (i < {N}) {{ values[0] = (values[0] as i32) + 1; i = i + 1; }}\n\
         return values[0] as i32; }}"
    );
    let array_module = compile(&array_src);
    let array_entry = array_module.entry_point.unwrap().rid();
    let array_jit = Jit::compile(&array_module).expect("jit compiles i32 array loop");
    assert!(array_jit.is_compiled(array_entry));
    let mut array_heap = zun_gc::Heap::new();
    let array_jit_warmup = array_jit
        .call_entry_with_heap(array_entry, &mut array_heap)
        .expect("jit array warmup");
    let array_reps = 20;
    let t = Instant::now();
    for _ in 0..array_reps {
        std::hint::black_box(
            array_jit
                .call_entry_with_heap(array_entry, &mut array_heap)
                .expect("jit array call"),
        );
    }
    let array_jit_ns = t.elapsed().as_nanos() as f64 / (array_reps as f64 * N as f64);
    let mut array_out = CaptureOutput::default();
    let array_interp_warmup = zun_interp::run(&array_module, &mut array_out).unwrap();
    let t = Instant::now();
    for _ in 0..3 {
        let mut out = CaptureOutput::default();
        std::hint::black_box(zun_interp::run(&array_module, &mut out).unwrap());
    }
    let array_interp_ns = t.elapsed().as_nanos() as f64 / (3.0 * N as f64);
    let RetVal::Int(array_jit_result) = array_jit_warmup else {
        panic!("unexpected array JIT result")
    };
    let zun_interp::value::Value::Int(array_interp_result, _) = array_interp_warmup else {
        panic!("unexpected array interpreter result")
    };
    assert_eq!(array_jit_result, array_interp_result);
    println!();
    println!(
        "i32 array increment loop (interp == jit: {})",
        array_jit_result == array_interp_result
    );
    println!("interpreter (Tier 0): {:8.3} ns/iteration", array_interp_ns);
    println!("JIT (Tier 1):         {:8.3} ns/iteration", array_jit_ns);
    println!(
        "speedup:              {:8.2}x",
        array_interp_ns / array_jit_ns
    );
}
