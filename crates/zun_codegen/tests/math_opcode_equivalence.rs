//! Tier 0 / Tier 1 equivalence for the float math machine ops.
//!
//! These opcodes are semantic intrinsics: the JIT replaces them with a single
//! Cranelift instruction while the interpreter computes the same result in
//! Rust. Two implementations of one operation is exactly the arrangement that
//! .docs-dotnet/07-actions-operations/05-jit-intrinsics.md says to pin down —
//! "garanta equivalência de resultados entre expandido e fallback",  with NaN,
//! overflow and rounding named as where they drift apart. So the operand table
//! below is mostly edge cases, and every operand is run through both tiers and
//! compared bit for bit.

use zun_codegen::RetVal;
use zun_il::{encode, Instruction, JitKind, NumKind, Opcode, Operand};
use zun_interp::value::Value;
use zun_interp::CaptureOutput;
use zun_metadata::{MethodDef, TableKind};
use zun_module::{write_body, Module};

/// Operands chosen to hit the places the two paths can disagree: the signed
/// zeros, both infinities, NaN, exact halves (rounding ties), negatives
/// (sqrt's NaN case) and a couple of ordinary values.
const OPERANDS: &[f64] = &[
    0.0,
    -0.0,
    1.0,
    -1.0,
    0.5,
    -0.5,
    1.5,
    -1.5,
    2.5,
    -2.5,
    16.0,
    -16.0,
    3.7,
    -3.7,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::NAN,
    f64::MAX,
    f64::MIN_POSITIVE,
];

const UNARY: &[Opcode] = &[
    Opcode::SqrtF64,
    Opcode::AbsF64,
    Opcode::NegF64,
    Opcode::FloorF64,
    Opcode::CeilF64,
    Opcode::TruncF64,
];
const BINARY: &[Opcode] = &[Opcode::MinF64, Opcode::MaxF64];

fn konst(value: f64) -> Instruction {
    Instruction::new(Opcode::ConstF64, Operand::Float64(value))
}

/// Build a nullary `main` returning f64.
fn module_with(body: &[Instruction]) -> Module {
    let mut module = Module::new();
    let mut code = Vec::new();
    for instruction in body {
        encode(instruction, &mut code);
    }
    let offset = write_body(
        &mut module.code,
        0,
        0,
        true,
        JitKind::Num(NumKind::F64).to_byte(),
        &[],
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

fn tier0(module: &Module) -> f64 {
    let mut output = CaptureOutput::default();
    match zun_interp::run(module, &mut output).expect("interpreter runs the body") {
        Value::Float(value, _) => value,
        other => panic!("expected a float from Tier 0, got {other:?}"),
    }
}

fn tier1(module: &Module) -> f64 {
    let jit = zun_codegen::Jit::compile(module).expect("JIT compiles the body");
    assert!(
        jit.is_compiled(1),
        "math ops must be JIT-eligible, not silently left on Tier 0"
    );
    match jit.call_entry(1).expect("JIT runs the body") {
        RetVal::Float(value) => value,
        other => panic!("expected a float from Tier 1, got {other:?}"),
    }
}

/// Compare by bits so `+0.0` and `-0.0` are distinguished; NaN compares equal
/// to NaN as a class, since the payload is not part of the contract.
fn assert_same(label: String, a: f64, b: f64) {
    if a.is_nan() && b.is_nan() {
        return;
    }
    assert_eq!(
        a.to_bits(),
        b.to_bits(),
        "{label}: Tier 0 produced {a:?} but Tier 1 produced {b:?}"
    );
}

#[test]
fn unary_math_ops_agree_across_tiers() {
    for &op in UNARY {
        for &operand in OPERANDS {
            let module = module_with(&[
                konst(operand),
                Instruction::simple(op),
                Instruction::simple(Opcode::Ret),
            ]);
            assert_same(
                format!("{}({operand:?})", op.mnemonic()),
                tier0(&module),
                tier1(&module),
            );
        }
    }
}

#[test]
fn binary_math_ops_agree_across_tiers() {
    for &op in BINARY {
        for &left in OPERANDS {
            for &right in OPERANDS {
                let module = module_with(&[
                    konst(left),
                    konst(right),
                    Instruction::simple(op),
                    Instruction::simple(Opcode::Ret),
                ]);
                assert_same(
                    format!("{}({left:?}, {right:?})", op.mnemonic()),
                    tier0(&module),
                    tier1(&module),
                );
            }
        }
    }
}

#[test]
fn composed_math_ops_agree_across_tiers() {
    // Chains keep values in Cranelift registers between instructions, which is
    // a different path through the translator than a single op does.
    for &a in OPERANDS {
        for &b in OPERANDS {
            // max(floor(|a|), trunc(sqrt(|b|)))
            let module = module_with(&[
                konst(a),
                Instruction::simple(Opcode::AbsF64),
                Instruction::simple(Opcode::FloorF64),
                konst(b),
                Instruction::simple(Opcode::AbsF64),
                Instruction::simple(Opcode::SqrtF64),
                Instruction::simple(Opcode::TruncF64),
                Instruction::simple(Opcode::MaxF64),
                Instruction::simple(Opcode::Ret),
            ]);
            assert_same(
                format!("composed({a:?}, {b:?})"),
                tier0(&module),
                tier1(&module),
            );
        }
    }
}

#[test]
fn math_ops_do_not_force_a_tier0_fallback() {
    // The point of the opcodes is that the JIT handles them; if eligibility
    // ever stops listing one, the equivalence tests above would still pass
    // (both tiers would be the interpreter) while the optimization is gone.
    for &op in UNARY {
        let module = module_with(&[
            konst(4.0),
            Instruction::simple(op),
            Instruction::simple(Opcode::Ret),
        ]);
        let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
        assert!(jit.is_compiled(1), "{} fell back to Tier 0", op.mnemonic());
    }
    for &op in BINARY {
        let module = module_with(&[
            konst(1.0),
            konst(2.0),
            Instruction::simple(op),
            Instruction::simple(Opcode::Ret),
        ]);
        let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
        assert!(jit.is_compiled(1), "{} fell back to Tier 0", op.mnemonic());
    }
}
