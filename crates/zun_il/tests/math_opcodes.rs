//! The float math machine ops as the master table declares them.
//!
//! Byte values are part of the on-disk format and must never be renumbered, so
//! they are pinned here literally rather than derived from the table: a test
//! that reads the value it is checking would accept a silent renumbering.

use zun_il::{decode_all, encode, Instruction, Opcode, OperandKind, StackEffect};

/// Every math op, with the byte and mnemonic the format fixes.
const MATH_OPS: &[(Opcode, u8, &str)] = &[
    (Opcode::SqrtF64, 0x96, "sqrt.f64"),
    (Opcode::AbsF64, 0x97, "abs.f64"),
    (Opcode::NegF64, 0x98, "neg.f64"),
    (Opcode::FloorF64, 0x99, "floor.f64"),
    (Opcode::CeilF64, 0x9A, "ceil.f64"),
    (Opcode::TruncF64, 0x9B, "trunc.f64"),
    (Opcode::MinF64, 0x9C, "min.f64"),
    (Opcode::MaxF64, 0x9D, "max.f64"),
    (Opcode::RoundF64, 0xA3, "round.f64"),
    (Opcode::SignF64, 0xA4, "sign.f64"),
];

#[test]
fn byte_values_and_mnemonics_are_pinned() {
    for &(op, byte, mnemonic) in MATH_OPS {
        assert_eq!(op.to_byte(), byte, "{mnemonic} changed its byte value");
        assert_eq!(op.mnemonic(), mnemonic, "{op:?} changed its mnemonic");
        assert_eq!(Opcode::from_byte(byte), Some(op));
    }
}

#[test]
fn all_math_ops_take_no_inline_operand() {
    for &(op, _, mnemonic) in MATH_OPS {
        assert_eq!(
            op.operand(),
            OperandKind::None,
            "{mnemonic} must read its operands from the stack"
        );
    }
}

#[test]
fn unary_ops_pop_one_and_binary_ops_pop_two() {
    for &(op, _, mnemonic) in MATH_OPS {
        let expected_pops = match op {
            Opcode::MinF64 | Opcode::MaxF64 => 2,
            _ => 1,
        };
        assert_eq!(
            op.stack_effect(),
            StackEffect::Fixed {
                pops: expected_pops,
                pushes: 1
            },
            "{mnemonic} has the wrong stack effect"
        );
    }
}

#[test]
fn math_ops_round_trip_through_the_encoder() {
    let instrs: Vec<Instruction> = MATH_OPS
        .iter()
        .map(|&(op, _, _)| Instruction::simple(op))
        .collect();
    let mut code = Vec::new();
    for instruction in &instrs {
        encode(instruction, &mut code);
    }

    // Operandless opcodes are one byte each.
    assert_eq!(code.len(), MATH_OPS.len());

    let decoded = decode_all(&code).expect("math ops decode");
    let decoded_ops: Vec<Opcode> = decoded.iter().map(|i| i.opcode).collect();
    let expected: Vec<Opcode> = MATH_OPS.iter().map(|&(op, _, _)| op).collect();
    assert_eq!(decoded_ops, expected);
}

#[test]
fn round_and_sign_are_ops_but_nearest_is_not() {
    // `round.f64` and `sign.f64` exist as machine ops even though neither is
    // a single instruction: both lower to a short branchless sequence.
    //
    // `nearest` must never appear. It is the machine's rounding mode, which
    // sends halves to even, so `nearest(2.5)` is 2 where `Math.round(2.5)` is
    // 3. Offering it under a math name would be a silent semantic change.
    let mnemonics: Vec<&str> = Opcode::ALL.iter().map(|op| op.mnemonic()).collect();
    assert!(mnemonics.contains(&"round.f64"));
    assert!(mnemonics.contains(&"sign.f64"));
    assert!(
        !mnemonics.contains(&"nearest.f64"),
        "machine `nearest` rounds halves to even and is not `Math.round`"
    );
}

/// The integer machine ops, pinned the same way.
const INT_MATH_OPS: &[(Opcode, u8, &str)] = &[
    (Opcode::MinI32, 0x9E, "min.i32"),
    (Opcode::MaxI32, 0x9F, "max.i32"),
    (Opcode::ClzI32, 0xA0, "clz.i32"),
    (Opcode::CtzI32, 0xA1, "ctz.i32"),
    (Opcode::PopcntI32, 0xA2, "popcnt.i32"),
];

#[test]
fn int_math_byte_values_and_mnemonics_are_pinned() {
    for &(op, byte, mnemonic) in INT_MATH_OPS {
        assert_eq!(op.to_byte(), byte, "{mnemonic} changed its byte value");
        assert_eq!(op.mnemonic(), mnemonic, "{op:?} changed its mnemonic");
        assert_eq!(Opcode::from_byte(byte), Some(op));
        assert_eq!(op.operand(), OperandKind::None);
    }
}

#[test]
fn int_math_stack_effects() {
    for &(op, _, mnemonic) in INT_MATH_OPS {
        let expected_pops = match op {
            Opcode::MinI32 | Opcode::MaxI32 => 2,
            _ => 1,
        };
        assert_eq!(
            op.stack_effect(),
            StackEffect::Fixed {
                pops: expected_pops,
                pushes: 1
            },
            "{mnemonic} has the wrong stack effect"
        );
    }
}

#[test]
fn int_math_ops_round_trip_through_the_encoder() {
    let mut code = Vec::new();
    for &(op, _, _) in INT_MATH_OPS {
        encode(&Instruction::simple(op), &mut code);
    }
    assert_eq!(code.len(), INT_MATH_OPS.len());
    let decoded: Vec<Opcode> = decode_all(&code)
        .expect("int math ops decode")
        .iter()
        .map(|i| i.opcode)
        .collect();
    let expected: Vec<Opcode> = INT_MATH_OPS.iter().map(|&(op, _, _)| op).collect();
    assert_eq!(decoded, expected);
}
