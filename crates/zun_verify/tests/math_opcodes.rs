//! Verifier fixtures for the float math machine ops.
//!
//! Every opcode lands with `_Valid` / `_Invalid_<Error>` fixtures (CLAUDE.md).
//! The verifier is height-only and reads its arities straight from the master
//! table, so these fixtures are what proves the table rows are right: a wrong
//! `pops` would let an underflowing body through.

use zun_il::{encode, Instruction, Opcode, Operand};
use zun_verify::{ErrorKind, MethodInfo, VerifyError};

fn asm(instrs: &[Instruction]) -> Vec<u8> {
    let mut out = Vec::new();
    for i in instrs {
        encode(i, &mut out);
    }
    out
}

/// No math op is a call, so the resolver is never consulted.
fn resolver(_: Opcode, _: &Operand) -> Option<(u8, u8)> {
    None
}

fn check(code: &[u8], info: MethodInfo) -> Result<(), VerifyError> {
    zun_verify::verify(code, info, &resolver)
}

const VOID: MethodInfo = MethodInfo {
    local_count: 1,
    returns_value: false,
};
const RETURNS: MethodInfo = MethodInfo {
    local_count: 1,
    returns_value: true,
};

const UNARY: &[Opcode] = &[
    Opcode::SqrtF64,
    Opcode::AbsF64,
    Opcode::NegF64,
    Opcode::FloorF64,
    Opcode::CeilF64,
    Opcode::TruncF64,
    // Not single instructions, but the same shape to the verifier: one
    // operand in, one out.
    Opcode::RoundF64,
    Opcode::SignF64,
];
const BINARY: &[Opcode] = &[Opcode::MinF64, Opcode::MaxF64];

fn konst(value: f64) -> Instruction {
    Instruction::new(Opcode::ConstF64, Operand::Float64(value))
}

#[test]
fn fixture_valid_unary_math_consumes_one_operand() {
    for &op in UNARY {
        let code = asm(&[
            konst(2.0),
            Instruction::simple(op),
            Instruction::simple(Opcode::Ret),
        ]);
        assert_eq!(check(&code, RETURNS), Ok(()), "{}", op.mnemonic());
    }
}

#[test]
fn fixture_valid_binary_math_consumes_two_operands() {
    for &op in BINARY {
        let code = asm(&[
            konst(1.0),
            konst(2.0),
            Instruction::simple(op),
            Instruction::simple(Opcode::Ret),
        ]);
        assert_eq!(check(&code, RETURNS), Ok(()), "{}", op.mnemonic());
    }
}

#[test]
fn fixture_valid_math_ops_compose() {
    // min(|a|, sqrt(b)) — the result of one math op feeds the next.
    let code = asm(&[
        konst(-9.0),
        Instruction::simple(Opcode::AbsF64),
        konst(16.0),
        Instruction::simple(Opcode::SqrtF64),
        Instruction::simple(Opcode::MinF64),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, RETURNS), Ok(()));
}

#[test]
fn fixture_invalid_stack_underflow_on_empty_unary() {
    for &op in UNARY {
        let code = asm(&[Instruction::simple(op), Instruction::simple(Opcode::Ret)]);
        assert_eq!(
            check(&code, RETURNS).map_err(|e| e.kind),
            Err(ErrorKind::StackUnderflow),
            "{}",
            op.mnemonic()
        );
    }
}

#[test]
fn fixture_invalid_stack_underflow_on_single_operand_binary() {
    for &op in BINARY {
        let code = asm(&[
            konst(1.0),
            Instruction::simple(op),
            Instruction::simple(Opcode::Ret),
        ]);
        assert_eq!(
            check(&code, RETURNS).map_err(|e| e.kind),
            Err(ErrorKind::StackUnderflow),
            "{}",
            op.mnemonic()
        );
    }
}

#[test]
fn fixture_invalid_bad_return_height_from_leftover_math_result() {
    // A math op pushes a value; a void method that never pops it must be
    // rejected, which is what pins `pushes: 1`.
    let code = asm(&[
        konst(4.0),
        Instruction::simple(Opcode::SqrtF64),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(
        check(&code, VOID).map_err(|e| e.kind),
        Err(ErrorKind::BadReturnHeight {
            expected: 0,
            found: 1
        })
    );
}

const INT_UNARY: &[Opcode] = &[
    Opcode::ClzI32,
    Opcode::CtzI32,
    Opcode::PopcntI32,
    // Pre-dates this work and had no fixture anywhere; the coverage guard
    // below found it, and it is the same one-in-one-out shape.
    Opcode::NegI32,
];
const INT_BINARY: &[Opcode] = &[Opcode::MinI32, Opcode::MaxI32];

fn int_konst(value: i32) -> Instruction {
    Instruction::new(Opcode::ConstI32, Operand::Imm32(value))
}

#[test]
fn fixture_valid_unary_int_math_consumes_one_operand() {
    for &op in INT_UNARY {
        let code = asm(&[
            int_konst(1),
            Instruction::simple(op),
            Instruction::simple(Opcode::Ret),
        ]);
        assert_eq!(check(&code, RETURNS), Ok(()), "{}", op.mnemonic());
    }
}

#[test]
fn fixture_valid_binary_int_math_consumes_two_operands() {
    for &op in INT_BINARY {
        let code = asm(&[
            int_konst(1),
            int_konst(2),
            Instruction::simple(op),
            Instruction::simple(Opcode::Ret),
        ]);
        assert_eq!(check(&code, RETURNS), Ok(()), "{}", op.mnemonic());
    }
}

#[test]
fn fixture_invalid_stack_underflow_on_empty_int_unary() {
    for &op in INT_UNARY {
        let code = asm(&[Instruction::simple(op), Instruction::simple(Opcode::Ret)]);
        assert_eq!(
            check(&code, RETURNS).map_err(|e| e.kind),
            Err(ErrorKind::StackUnderflow),
            "{}",
            op.mnemonic()
        );
    }
}

#[test]
fn fixture_invalid_stack_underflow_on_single_operand_int_binary() {
    for &op in INT_BINARY {
        let code = asm(&[
            int_konst(1),
            Instruction::simple(op),
            Instruction::simple(Opcode::Ret),
        ]);
        assert_eq!(
            check(&code, RETURNS).map_err(|e| e.kind),
            Err(ErrorKind::StackUnderflow),
            "{}",
            op.mnemonic()
        );
    }
}

/// Guards the invariant itself: every math machine op must appear in the
/// fixtures above. Adding a row to the master table without a fixture is the
/// failure this catches.
#[test]
fn every_math_machine_op_has_fixtures() {
    let covered: Vec<Opcode> = UNARY
        .iter()
        .chain(BINARY)
        .chain(INT_UNARY)
        .chain(INT_BINARY)
        .copied()
        .collect();
    for &op in Opcode::ALL {
        let mnemonic = op.mnemonic();
        let is_math = mnemonic.starts_with("sqrt.")
            || mnemonic.starts_with("abs.")
            || mnemonic.starts_with("neg.")
            || mnemonic.starts_with("floor.")
            || mnemonic.starts_with("ceil.")
            || mnemonic.starts_with("trunc.")
            || mnemonic.starts_with("round.")
            || mnemonic.starts_with("sign.")
            || mnemonic.starts_with("min.")
            || mnemonic.starts_with("max.")
            || mnemonic.starts_with("clz.")
            || mnemonic.starts_with("ctz.")
            || mnemonic.starts_with("popcnt.")
            || mnemonic.starts_with("neg.");
        if is_math {
            assert!(
                covered.contains(&op),
                "{mnemonic} has no verifier fixture; every opcode ships \
                 _Valid/_Invalid_* coverage"
            );
        }
    }
}
