//! Integer math machine ops: Tier 0 semantics and Tier 0 / Tier 1 agreement.
//!
//! These are width- and signedness-generic, which is where they can go wrong.
//! `clz` of `1u8` is 7, not 63, and `umax(1u64, 2^63)` is the large value
//! while `smax` of the same bits is 1. Tier 0 reaches those answers by masking
//! an i64 container; Tier 1 gets them from the native width. Both are checked
//! against hand-computed expectations, and against each other.

use zun_codegen::RetVal;
use zun_il::{encode, Instruction, JitKind, NumKind, Opcode, Operand};
use zun_interp::value::Value;
use zun_interp::CaptureOutput;
use zun_metadata::{MethodDef, TableKind};
use zun_module::{write_body, Module};

/// Build a nullary `main` returning `kind`, from a literal body.
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

/// Push a constant already narrowed to `kind`, then convert the stack slot to
/// that kind so the op sees the right width and signedness.
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
    assert!(jit.is_compiled(1), "integer math ops must be JIT-eligible");
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
        a,
        b,
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
fn clz_counts_within_the_operand_width() {
    // The whole point of width-generic bit counting: the same value gives a
    // different answer per kind.
    assert_eq!(both(NumKind::U8, &[1], Opcode::ClzI32), 7);
    assert_eq!(both(NumKind::U16, &[1], Opcode::ClzI32), 15);
    assert_eq!(both(NumKind::U32, &[1], Opcode::ClzI32), 31);
    assert_eq!(both(NumKind::U64, &[1], Opcode::ClzI32), 63);

    assert_eq!(both(NumKind::U8, &[0x80], Opcode::ClzI32), 0);
    assert_eq!(both(NumKind::U32, &[0xFF], Opcode::ClzI32), 24);
}

#[test]
fn clz_of_zero_is_the_full_width() {
    for &kind in KINDS {
        let bits = kind.int_parts().expect("int kind").0 as i64;
        assert_eq!(both(kind, &[0], Opcode::ClzI32), bits, "{kind:?}");
    }
}

#[test]
fn ctz_counts_within_the_operand_width() {
    assert_eq!(both(NumKind::U8, &[1], Opcode::CtzI32), 0);
    assert_eq!(both(NumKind::U8, &[0x80], Opcode::CtzI32), 7);
    assert_eq!(both(NumKind::U32, &[0x100], Opcode::CtzI32), 8);
}

#[test]
fn ctz_of_zero_is_the_full_width() {
    // Not 64, and not undefined: a zero operand has `bits` trailing zeros.
    for &kind in KINDS {
        let bits = kind.int_parts().expect("int kind").0 as i64;
        assert_eq!(both(kind, &[0], Opcode::CtzI32), bits, "{kind:?}");
    }
}

#[test]
fn popcnt_counts_set_bits_within_the_width() {
    assert_eq!(both(NumKind::U8, &[0xFF], Opcode::PopcntI32), 8);
    assert_eq!(both(NumKind::U16, &[0xFF], Opcode::PopcntI32), 8);
    assert_eq!(both(NumKind::U32, &[0], Opcode::PopcntI32), 0);
    assert_eq!(both(NumKind::U32, &[0b1011], Opcode::PopcntI32), 3);
    // A negative i8 is all-ones in the high bits *of its width only*.
    assert_eq!(both(NumKind::I8, &[-1], Opcode::PopcntI32), 8);
    assert_eq!(both(NumKind::I64, &[-1], Opcode::PopcntI32), 64);
}

#[test]
fn min_and_max_respect_signedness() {
    // Same bit pattern, opposite answers: as i8 the operands are -1 and 1, as
    // u8 they are 255 and 1.
    assert_eq!(both(NumKind::I8, &[-1, 1], Opcode::MinI32), -1);
    assert_eq!(both(NumKind::U8, &[0xFF, 1], Opcode::MinI32), 1);
    assert_eq!(both(NumKind::I8, &[-1, 1], Opcode::MaxI32), 1);
    assert_eq!(both(NumKind::U8, &[0xFF, 1], Opcode::MaxI32), 0xFF);
}

#[test]
fn unsigned_min_and_max_handle_values_above_the_signed_range() {
    // A u64 above 2^63 reads as negative in the i64 container, so comparing
    // it as signed would invert the result.
    let big = i64::MIN; // 0x8000_0000_0000_0000 as u64 == 2^63
    assert_eq!(both(NumKind::U64, &[big, 1], Opcode::MaxI32), big);
    assert_eq!(both(NumKind::U64, &[big, 1], Opcode::MinI32), 1);
    // The same bits as i64 are negative, so the answers swap.
    assert_eq!(both(NumKind::I64, &[big, 1], Opcode::MaxI32), 1);
    assert_eq!(both(NumKind::I64, &[big, 1], Opcode::MinI32), big);
}

#[test]
fn min_and_max_agree_across_tiers_over_a_matrix() {
    let operands: &[i64] = &[0, 1, -1, 2, 127, -128, 255, i32::MAX as i64, i64::MIN];
    for &kind in KINDS {
        for &a in operands {
            for &b in operands {
                both(kind, &[a, b], Opcode::MinI32);
                both(kind, &[a, b], Opcode::MaxI32);
            }
        }
    }
}

#[test]
fn bit_counting_agrees_across_tiers_over_a_matrix() {
    let operands: &[i64] = &[0, 1, -1, 2, 0x80, 0xFF, 0x1234, i64::MIN, i64::MAX];
    for &kind in KINDS {
        for &operand in operands {
            for op in [Opcode::ClzI32, Opcode::CtzI32, Opcode::PopcntI32] {
                both(kind, &[operand], op);
            }
        }
    }
}
