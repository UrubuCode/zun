//! Does lowering `round`/`sign` to an instruction sequence actually beat the
//! libm call it replaces?
//!
//! `sqrt`, `abs`, `min` and friends are one instruction, so there is nothing
//! to argue about. `round.f64` and `sign.f64` are not: each expands to a short
//! branchless sequence, which is only worth it if a handful of ALU ops cost
//! less than a call. PLAN.md §1 forbids asserting that without ns/op, so this
//! measures it.
//!
//! ## Measured, 2026-10-23 (x86-64, release, 4 runs)
//!
//! | op      | instructions | libm call | ratio        |
//! |---------|--------------|-----------|--------------|
//! | `round` | ~2.1 ns/op   | ~2.8      | 1.21 - 1.54x |
//! | `sign`  | ~1.1 ns/op   | ~1.1      | 0.79 - 1.08x |
//! | `sqrt`  | ~3.9 ns/op   | ~3.9      | 0.99 - 1.00x |
//!
//! Read that conservatively. `round` is consistently ahead, so lowering it is
//! a real if modest win. `sign` straddles 1.0 — sometimes the call wins — so
//! there is no measured gain there.
//!
//! The `sqrt` row is the control and the reason not to over-read the others:
//! a single `sqrtsd` measures the same as a function call that performs a
//! `sqrtsd`, steady at 0.99x. Whatever the microarchitectural cause (serial
//! sqrt latency of ~15-20 cycles swamping the call, most likely), it means
//! this harness does not reliably isolate call overhead at this scale. So
//! these opcodes are justified by PLAN.md §3 and by what the IL carries, not
//! by a performance claim; the measurement says only that nothing regressed.
//!
//! Ignored by default — timings do not belong in the correctness suite. Run
//! with:
//!
//! ```text
//! cargo test -p zun_codegen --release --test math_opcode_bench -- --ignored --nocapture
//! ```

use std::time::Instant;

use zun_codegen::RetVal;
use zun_il::{encode, Instruction, JitKind, NumKind, Opcode, Operand};
use zun_metadata::{MethodDef, TableKind};
use zun_module::{write_body, Module};

const ITERATIONS: i32 = 5_000_000;
/// Applications of the operation per loop iteration.
///
/// With one, the loop's own load/store/add/branch dominates and even a single
/// instruction measures the same as a call — the first run of this benchmark
/// reported sqrt at exactly 1.00x, which says the harness, not the opcode.
/// Chaining the operation keeps a serial dependency while amortizing the loop.
const CHAIN: usize = 16;

/// Build `main` as a loop whose body applies `op` to a loop-carried value.
///
/// The operand depends on the accumulator so the optimizer cannot fold the
/// sequence to a constant — which it could do for the instruction form but
/// not for an opaque call, and that would make the comparison meaningless.
///
/// ```text
/// acc = 0.0; i = 0
/// loop: acc = <op>(acc + 1.1); i += 1; if i < N goto loop
/// return acc
/// ```
fn loop_module(op: &[Instruction]) -> Module {
    let mut body = vec![
        // acc = 0.0
        Instruction::new(Opcode::ConstF64, Operand::Float64(0.0)),
        Instruction::new(Opcode::StoreLocal, Operand::Local(1)),
        // i = 0
        Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
        Instruction::new(Opcode::StoreLocal, Operand::Local(0)),
    ];
    let loop_start = body.len();

    // acc = op(acc + 1.1)
    body.push(Instruction::new(Opcode::LoadLocal, Operand::Local(1)));
    body.push(Instruction::new(Opcode::ConstF64, Operand::Float64(1.1)));
    body.push(Instruction::simple(Opcode::AddF64));
    for _ in 0..CHAIN {
        body.extend_from_slice(op);
    }
    body.push(Instruction::new(Opcode::StoreLocal, Operand::Local(1)));
    // i += 1
    body.push(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
    body.push(Instruction::new(Opcode::ConstI32, Operand::Imm32(1)));
    body.push(Instruction::simple(Opcode::AddI32));
    body.push(Instruction::new(Opcode::StoreLocal, Operand::Local(0)));
    // if i < N goto loop
    body.push(Instruction::new(Opcode::LoadLocal, Operand::Local(0)));
    body.push(Instruction::new(Opcode::ConstI32, Operand::Imm32(ITERATIONS)));
    body.push(Instruction::simple(Opcode::CmpLt));

    // Encode once to learn each instruction's offset, then patch the branch.
    let mut offsets = Vec::new();
    let mut probe = Vec::new();
    for instruction in &body {
        offsets.push(probe.len());
        encode(instruction, &mut probe);
    }
    let branch_end = probe.len() + 5; // br.true is opcode + i32 displacement
    let displacement = offsets[loop_start] as i32 - branch_end as i32;
    body.push(Instruction::new(
        Opcode::BranchTrue,
        Operand::Branch(displacement),
    ));
    body.push(Instruction::new(Opcode::LoadLocal, Operand::Local(1)));
    body.push(Instruction::simple(Opcode::Ret));

    let mut code = Vec::new();
    for instruction in &body {
        encode(instruction, &mut code);
    }

    let mut module = Module::new();
    let offset = write_body(
        &mut module.code,
        0,
        2,
        true,
        JitKind::Num(NumKind::F64).to_byte(),
        &[
            JitKind::Num(NumKind::I32).to_byte(),
            JitKind::Num(NumKind::F64).to_byte(),
        ],
        &code,
    );
    module.tables.add_method_def(MethodDef {
        name: module.strings.intern("main"),
        signature: zun_metadata::BlobIndex(0),
        body_offset: offset,
        is_static: false,
    });
    module.entry_point = Some(zun_metadata::Token::new(TableKind::MethodDef, 1));
    module
}

/// JIT the module, run it once to warm, then time a run. Returns ns/iteration.
fn time_ns_per_op(label: &str, op: &[Instruction]) -> f64 {
    let module = loop_module(op);
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(1), "{label} must be JIT-compiled to compare");

    let warm = jit.call_entry(1).expect("warm-up run");
    let start = Instant::now();
    let measured = jit.call_entry(1).expect("timed run");
    let elapsed = start.elapsed();

    // Keep both results observable so nothing is optimized away.
    assert_eq!(
        format!("{warm:?}"),
        format!("{measured:?}"),
        "{label} is not deterministic"
    );
    if let RetVal::Float(value) = measured {
        assert!(value.is_finite() || value.is_infinite());
    }

    let ops = f64::from(ITERATIONS) * CHAIN as f64;
    let ns = elapsed.as_secs_f64() * 1e9 / ops;
    println!("{label:28} {ns:7.3} ns/op");
    ns
}

/// The native id behind `zun:math`'s `name`, as `call.builtin` takes it.
fn native(name: &str) -> Instruction {
    let (id, _) = zun_natives::resolve("zun:math", name)
        .unwrap_or_else(|| panic!("`zun:math` exports `{name}`"));
    Instruction::new(Opcode::CallBuiltin, Operand::Imm32(id as i32))
}

#[test]
#[ignore = "timing; run explicitly with --release --ignored --nocapture"]
fn round_opcode_versus_native_call() {
    println!("\n{ITERATIONS} iterations each\n");
    let opcode = time_ns_per_op("round.f64 (instructions)", &[Instruction::simple(
        Opcode::RoundF64,
    )]);
    let call = time_ns_per_op("roundNumber (libm call)", &[native("roundNumber")]);
    println!("  ratio: {:.2}x\n", call / opcode);
}

#[test]
#[ignore = "timing; run explicitly with --release --ignored --nocapture"]
fn sign_opcode_versus_native_call() {
    println!("\n{ITERATIONS} iterations each\n");
    let opcode = time_ns_per_op("sign.f64 (instructions)", &[Instruction::simple(
        Opcode::SignF64,
    )]);
    let call = time_ns_per_op("signNumber (libm call)", &[native("signNumber")]);
    println!("  ratio: {:.2}x\n", call / opcode);
}

#[test]
#[ignore = "timing; run explicitly with --release --ignored --nocapture"]
fn sqrt_opcode_versus_native_call() {
    // The uncontroversial one, as a scale reference: a single instruction
    // against a call.
    println!("\n{ITERATIONS} iterations each\n");
    let opcode = time_ns_per_op("sqrt.f64 (instruction)", &[Instruction::simple(
        Opcode::SqrtF64,
    )]);
    let call = time_ns_per_op("sqrtNumber (libm call)", &[native("sqrtNumber")]);
    println!("  ratio: {:.2}x\n", call / opcode);
}
