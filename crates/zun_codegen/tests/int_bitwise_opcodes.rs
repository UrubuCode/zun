//! Integer bitwise, shift, and exponent machine ops: Tier 0 semantics and
//! Tier 0 / Tier 1 agreement.
//!
//! Like the other integer machine ops these are width- and signedness-generic,
//! which is where they can diverge: `shr.i32` is arithmetic for a signed kind
//! and logical for an unsigned one, shift amounts are masked to the operand
//! width, and `pow.i32` wraps to the kind. Tier 0 reaches these answers by
//! masking an i64 container; Tier 1 gets them from the native width. Both are
//! checked against hand-computed expectations and against each other.

use zun_codegen::RetVal;
use zun_il::{encode, Instruction, JitKind, NumKind, Opcode, Operand};
use zun_interp::value::Value;
use zun_interp::CaptureOutput;
use zun_metadata::{MethodDef, TableKind};
use zun_module::{write_body, Module};

fn module_with(kind: NumKind, body: &[Instruction]) -> Module {
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
        JitKind::Num(kind).to_byte(),
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

fn push_as(kind: NumKind, value: i64) -> Vec<Instruction> {
    vec![
        Instruction::new(Opcode::ConstI64, Operand::Imm64(value)),
        Instruction::new(Opcode::Conv, Operand::Imm32(kind.to_imm())),
    ]
}

fn body(kind: NumKind, operands: &[i64], op: Opcode) -> Vec<Instruction> {
    let mut instrs = Vec::new();
    for &operand in operands {
        instrs.extend(push_as(kind, operand));
    }
    instrs.push(Instruction::simple(op));
    instrs.push(Instruction::simple(Opcode::Ret));
    instrs
}

fn tier0(module: &Module) -> i64 {
    let mut output = CaptureOutput::default();
    match zun_interp::run(module, &mut output).expect("interpreter runs") {
        Value::Int(value, _) => value,
        other => panic!("expected an int from Tier 0, got {other:?}"),
    }
}

fn tier1(module: &Module) -> i64 {
    let jit = zun_codegen::Jit::compile(module).expect("jit compiles");
    assert!(jit.is_compiled(1), "integer bitwise ops must be JIT-eligible");
    match jit.call_entry(1).expect("jit call") {
        RetVal::Int(value) => value,
        other => panic!("expected an int from Tier 1, got {other:?}"),
    }
}

/// Run on both tiers, require they agree, and return the shared result.
fn both(kind: NumKind, operands: &[i64], op: Opcode) -> i64 {
    let module = module_with(kind, &body(kind, operands, op));
    let a = tier0(&module);
    let b = tier1(&module);
    assert_eq!(
        a, b,
        "{} over {operands:?} as {kind:?}: Tier 0 gave {a}, Tier 1 gave {b}",
        op.mnemonic()
    );
    a
}

const KINDS: &[NumKind] = &[
    NumKind::I8,
    NumKind::I16,
    NumKind::I32,
    NumKind::I64,
    NumKind::U8,
    NumKind::U16,
    NumKind::U32,
    NumKind::U64,
];

#[test]
fn bitwise_and_or_xor_hand_computed() {
    assert_eq!(both(NumKind::I32, &[5, 3], Opcode::AndI32), 1);
    assert_eq!(both(NumKind::I32, &[5, 2], Opcode::OrI32), 7);
    assert_eq!(both(NumKind::I32, &[6, 3], Opcode::XorI32), 5);
    assert_eq!(both(NumKind::U8, &[0xF0, 0x0F], Opcode::OrI32), 0xFF);
    assert_eq!(both(NumKind::U8, &[0xFF, 0x0F], Opcode::XorI32), 0xF0);
}

#[test]
fn not_is_ones_complement_within_the_width() {
    // ~5 == -6 as i32; as u8, ~5 == 0xFA (250).
    assert_eq!(both(NumKind::I32, &[5], Opcode::NotI32), -6);
    assert_eq!(both(NumKind::U8, &[5], Opcode::NotI32), 0xFA);
    assert_eq!(both(NumKind::U16, &[0], Opcode::NotI32), 0xFFFF);
    assert_eq!(both(NumKind::I64, &[0], Opcode::NotI32), -1);
}

#[test]
fn left_shift_wraps_to_the_width() {
    assert_eq!(both(NumKind::I32, &[1, 4], Opcode::ShlI32), 16);
    // Shift amount is masked to the width: `1 << 8` as u8 is `1 << 0`.
    assert_eq!(both(NumKind::U8, &[1, 8], Opcode::ShlI32), 1);
    assert_eq!(both(NumKind::U8, &[1, 7], Opcode::ShlI32), 0x80);
}

#[test]
fn right_shift_is_arithmetic_for_signed_logical_for_unsigned() {
    // Signed `>>` keeps the sign.
    assert_eq!(both(NumKind::I32, &[-8, 1], Opcode::ShrI32), -4);
    // Unsigned `>>` fills with zeros.
    assert_eq!(both(NumKind::U8, &[0x80, 1], Opcode::ShrI32), 0x40);
    // The same bit pattern as i8 is -128, arithmetic shift gives -64.
    assert_eq!(both(NumKind::I8, &[-128, 1], Opcode::ShrI32), -64);
}

#[test]
fn unsigned_right_shift_is_always_logical() {
    // `-1 >>> 28` over a u32 is 15 (the JS result).
    assert_eq!(both(NumKind::U32, &[-1, 28], Opcode::UShrI32), 15);
    // Even on a signed kind, `>>>` treats the bits as unsigned of the width.
    assert_eq!(both(NumKind::I32, &[-1, 28], Opcode::UShrI32), 15);
    assert_eq!(both(NumKind::U8, &[0x80, 3], Opcode::UShrI32), 0x10);
}

#[test]
fn integer_pow_wraps_to_the_width() {
    assert_eq!(both(NumKind::I32, &[2, 10], Opcode::PowI32), 1024);
    assert_eq!(both(NumKind::I64, &[3, 4], Opcode::PowI32), 81);
    // x ** 0 is 1, x ** 1 is x.
    assert_eq!(both(NumKind::I32, &[7, 0], Opcode::PowI32), 1);
    assert_eq!(both(NumKind::I32, &[7, 1], Opcode::PowI32), 7);
    // A negative exponent is the empty product.
    assert_eq!(both(NumKind::I32, &[2, -3], Opcode::PowI32), 1);
    // Overflow wraps to the kind: 2 ** 8 as u8 is 0.
    assert_eq!(both(NumKind::U8, &[2, 8], Opcode::PowI32), 0);
    assert_eq!(both(NumKind::U8, &[2, 7], Opcode::PowI32), 0x80);
}

#[test]
fn bitwise_and_shift_agree_across_tiers_over_a_matrix() {
    let operands: &[i64] = &[0, 1, -1, 2, 7, 0x80, 0xFF, i32::MAX as i64, i64::MIN];
    for &kind in KINDS {
        for &a in operands {
            for &b in operands {
                for op in [
                    Opcode::AndI32,
                    Opcode::OrI32,
                    Opcode::XorI32,
                    Opcode::ShlI32,
                    Opcode::ShrI32,
                    Opcode::UShrI32,
                ] {
                    both(kind, &[a, b], op);
                }
            }
            both(kind, &[a], Opcode::NotI32);
        }
    }
}

#[test]
fn integer_pow_agrees_across_tiers_over_a_matrix() {
    let bases: &[i64] = &[0, 1, -1, 2, 3, 7, i32::MAX as i64];
    let exponents: &[i64] = &[0, 1, 2, 3, 5, 10, 63, -1];
    for &kind in KINDS {
        for &base in bases {
            for &exp in exponents {
                both(kind, &[base, exp], Opcode::PowI32);
            }
        }
    }
}
