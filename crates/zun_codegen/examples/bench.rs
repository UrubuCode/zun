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

fn compile(src: &str) -> zun_module::Module {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema: {diags:?}");
    zun_ilgen::lower(&hir)
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
    println!("result: {jit_result}  (interp == jit: {})", jit_result == interp_result);
    println!();
    println!("interpreter (Tier 0): {interp_ns:8.3} ns/iteration");
    println!("JIT (Tier 1):         {jit_ns:8.3} ns/iteration");
    println!("speedup:              {:8.2}x", interp_ns / jit_ns);
    println!();
    println!(
        "per iteration there are ~4 int ops (mul, add, sub, cmp) + a store + loop,\n\
         so divide by ~4 for a rough per-op figure."
    );
}
