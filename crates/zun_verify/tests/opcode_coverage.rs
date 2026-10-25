//! Every opcode in the master table gets `_Valid` / `_Invalid_*` coverage.
//!
//! CLAUDE.md and GOAL.md require fixtures with each new opcode, but that was
//! enforced by memory and 22 of the 88 opcodes had none — among them
//! `add.i32`, `sub.f64`, `cmp.eq`, `conv`, `concat`, `ldfld` and `stfld`.
//!
//! ## Why the arities are restated here
//!
//! The obvious fix — generate the fixtures from `stack_effect()` — is
//! worthless, and measurably so: with that version in place, changing
//! `concat` to pop one operand instead of two left the whole suite green. Of
//! course it did. The fixtures and the verifier both read the same table, so
//! the test only proved the table agrees with itself.
//!
//! So [`SHAPES`] states each opcode's arity independently and the fixtures
//! are built from *that*. It is a second copy, deliberately: a test oracle,
//! the same reasoning that pins byte values literally in
//! `zun_il/tests/math_opcodes.rs`. The entries were read against each
//! opcode's documented meaning rather than trusted wholesale — `dup` really
//! is one in and two out, and `stelem` really does leave a value behind for
//! the caller to drop. Changing an arity now takes an edit in both places,
//! which is the point.
//!
//! ## What this does and does not cover
//!
//! It covers the arity contract, which nothing was checking. Opcode-specific
//! rules — branch targets, protected regions, `rethrow` placement — stay in
//! the hand-written fixtures in `src/tests.rs` and `tests/math_opcodes.rs`.

use zun_il::{encode, Instruction, NumKind, Opcode, Operand, OperandKind, StackEffect};
use zun_metadata::{TableKind, Token};
use zun_verify::{ErrorKind, MethodInfo, VerifyError};

/// What an opcode does to the stack, stated independently of the table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    /// Consumes `.0` values and produces `.1`.
    Fixed(usize, usize),
    /// Arity comes from the callee signature; see [`resolver`].
    Call,
    /// Transfers control and does not fall through.
    Terminator,
}

/// The expected shape of every opcode, read against the table's own
/// documentation rather than copied from its `StackEffect` at test time.
const SHAPES: &[(Opcode, Shape)] = &[
    (Opcode::Nop, Shape::Fixed(0, 0)),
    (Opcode::ConstI32, Shape::Fixed(0, 1)),
    (Opcode::ConstI64, Shape::Fixed(0, 1)),
    (Opcode::ConstF64, Shape::Fixed(0, 1)),
    (Opcode::ConstNull, Shape::Fixed(0, 1)),
    (Opcode::ConstTrue, Shape::Fixed(0, 1)),
    (Opcode::ConstFalse, Shape::Fixed(0, 1)),
    (Opcode::ConstStr, Shape::Fixed(0, 1)),
    (Opcode::LoadLocal, Shape::Fixed(0, 1)),
    (Opcode::StoreLocal, Shape::Fixed(1, 0)),
    (Opcode::Pop, Shape::Fixed(1, 0)),
    // Reads the top value and leaves two: one in, two out.
    (Opcode::Dup, Shape::Fixed(1, 2)),
    (Opcode::AddI32, Shape::Fixed(2, 1)),
    (Opcode::SubI32, Shape::Fixed(2, 1)),
    (Opcode::MulI32, Shape::Fixed(2, 1)),
    (Opcode::DivI32, Shape::Fixed(2, 1)),
    (Opcode::RemI32, Shape::Fixed(2, 1)),
    (Opcode::NegI32, Shape::Fixed(1, 1)),
    (Opcode::AddF64, Shape::Fixed(2, 1)),
    (Opcode::SubF64, Shape::Fixed(2, 1)),
    (Opcode::MulF64, Shape::Fixed(2, 1)),
    (Opcode::DivF64, Shape::Fixed(2, 1)),
    (Opcode::RemF64, Shape::Fixed(2, 1)),
    (Opcode::Conv, Shape::Fixed(1, 1)),
    (Opcode::Concat, Shape::Fixed(2, 1)),
    (Opcode::CmpEq, Shape::Fixed(2, 1)),
    (Opcode::CmpNe, Shape::Fixed(2, 1)),
    (Opcode::CmpLt, Shape::Fixed(2, 1)),
    (Opcode::CmpLe, Shape::Fixed(2, 1)),
    (Opcode::CmpGt, Shape::Fixed(2, 1)),
    (Opcode::CmpGe, Shape::Fixed(2, 1)),
    (Opcode::Branch, Shape::Terminator),
    (Opcode::BranchTrue, Shape::Fixed(1, 0)),
    (Opcode::BranchFalse, Shape::Fixed(1, 0)),
    (Opcode::Ret, Shape::Terminator),
    (Opcode::Throw, Shape::Terminator),
    (Opcode::Rethrow, Shape::Terminator),
    (Opcode::NewObj, Shape::Call),
    (Opcode::LoadField, Shape::Fixed(1, 1)),
    // Field assignment is an expression, so the stored value stays.
    (Opcode::StoreField, Shape::Fixed(2, 1)),
    (Opcode::LoadStatic, Shape::Fixed(0, 1)),
    (Opcode::StoreStatic, Shape::Fixed(1, 1)),
    (Opcode::Call, Shape::Call),
    (Opcode::CallVirt, Shape::Call),
    (Opcode::CallBuiltin, Shape::Call),
    (Opcode::BoxAny, Shape::Fixed(1, 1)),
    (Opcode::UnboxAny, Shape::Fixed(1, 1)),
    (Opcode::NewDynamic, Shape::Fixed(0, 1)),
    (Opcode::RealmGlobal, Shape::Fixed(0, 1)),
    (Opcode::LoadDynamic, Shape::Fixed(1, 1)),
    (Opcode::StoreDynamic, Shape::Fixed(2, 1)),
    (Opcode::LoadDynamicValue, Shape::Fixed(2, 1)),
    (Opcode::StoreDynamicValue, Shape::Fixed(3, 1)),
    (Opcode::CastClass, Shape::Fixed(1, 1)),
    (Opcode::NewArray, Shape::Fixed(1, 1)),
    (Opcode::LoadArray, Shape::Fixed(2, 1)),
    // array, index, value in; the stored value stays, as `stfld` does.
    (Opcode::StoreArray, Shape::Fixed(3, 1)),
    (Opcode::ArrayLength, Shape::Fixed(1, 1)),
    (Opcode::ArrayPop, Shape::Fixed(1, 1)),
    (Opcode::ArrayPush, Shape::Fixed(2, 1)),
    (Opcode::LoadFunction, Shape::Fixed(0, 1)),
    (Opcode::MakeClosure, Shape::Fixed(1, 1)),
    (Opcode::CallIndirect, Shape::Call),
    (Opcode::CallRest, Shape::Call),
    (Opcode::CallRestArray, Shape::Call),
    (Opcode::CallRestArrayFixed, Shape::Call),
    (Opcode::CallVirtRest, Shape::Call),
    (Opcode::CallVirtRestArray, Shape::Call),
    (Opcode::CallVirtRestArrayFixed, Shape::Call),
    (Opcode::CallIndirectRest, Shape::Call),
    (Opcode::Await, Shape::Fixed(1, 1)),
    // Publishes the value on the iterator result rather than returning it.
    (Opcode::Yield, Shape::Fixed(1, 0)),
    (Opcode::ImportDynamic, Shape::Fixed(1, 1)),
    (Opcode::SqrtF64, Shape::Fixed(1, 1)),
    (Opcode::AbsF64, Shape::Fixed(1, 1)),
    (Opcode::NegF64, Shape::Fixed(1, 1)),
    (Opcode::FloorF64, Shape::Fixed(1, 1)),
    (Opcode::CeilF64, Shape::Fixed(1, 1)),
    (Opcode::TruncF64, Shape::Fixed(1, 1)),
    (Opcode::MinF64, Shape::Fixed(2, 1)),
    (Opcode::MaxF64, Shape::Fixed(2, 1)),
    (Opcode::MinI32, Shape::Fixed(2, 1)),
    (Opcode::MaxI32, Shape::Fixed(2, 1)),
    (Opcode::ClzI32, Shape::Fixed(1, 1)),
    (Opcode::CtzI32, Shape::Fixed(1, 1)),
    (Opcode::PopcntI32, Shape::Fixed(1, 1)),
    (Opcode::RoundF64, Shape::Fixed(1, 1)),
    (Opcode::SignF64, Shape::Fixed(1, 1)),
    (Opcode::AndI32, Shape::Fixed(2, 1)),
    (Opcode::OrI32, Shape::Fixed(2, 1)),
    (Opcode::XorI32, Shape::Fixed(2, 1)),
    (Opcode::NotI32, Shape::Fixed(1, 1)),
    (Opcode::ShlI32, Shape::Fixed(2, 1)),
    (Opcode::ShrI32, Shape::Fixed(2, 1)),
    (Opcode::UShrI32, Shape::Fixed(2, 1)),
    (Opcode::PowI32, Shape::Fixed(2, 1)),
    (Opcode::ConstUndef, Shape::Fixed(0, 1)),
];

/// Locals available to a fixture body; only the slot operand needs one.
const LOCAL_COUNT: u16 = 2;

fn asm(instrs: &[Instruction]) -> Vec<u8> {
    let mut out = Vec::new();
    for i in instrs {
        encode(i, &mut out);
    }
    out
}

/// Arity for the call family, which no table can state on its own.
///
/// Mirrors the shapes `src/tests.rs` uses: the `TokenImm32` forms take their
/// count from the immediate, the rest are fixed.
fn resolver(opcode: Opcode, operand: &Operand) -> Option<(u8, u8)> {
    let count = match operand {
        Operand::TokenImm32(_, count) if *count >= 0 => *count as u8,
        _ => 0,
    };
    match opcode {
        Opcode::NewObj => Some((0, 1)),
        Opcode::Call | Opcode::CallVirt | Opcode::CallBuiltin => Some((1, 1)),
        Opcode::CallIndirect => Some((2, 1)),
        Opcode::CallRestArray => Some((1, 1)),
        Opcode::CallVirtRestArray => Some((2, 1)),
        Opcode::CallRest | Opcode::CallVirtRest => Some((count, 1)),
        Opcode::CallIndirectRest => Some((count + 1, 1)),
        Opcode::CallRestArrayFixed => Some((count + 1, 1)),
        Opcode::CallVirtRestArrayFixed => Some((count + 2, 1)),
        _ => None,
    }
}

fn check(code: &[u8], returns_value: bool) -> Result<(), VerifyError> {
    zun_verify::verify(
        code,
        MethodInfo {
            local_count: LOCAL_COUNT,
            returns_value,
        },
        &resolver,
    )
}

/// A well-formed operand of the shape `op` expects.
///
/// The smallest valid values: slot 0 is in range, a branch displacement of 0
/// targets the next instruction, and the token only has to parse, since the
/// verifier resolves tokens solely for the call family.
fn operand_for(op: Opcode) -> Operand {
    match op.operand() {
        OperandKind::None => Operand::None,
        OperandKind::Imm32 => Operand::Imm32(NumKind::I32.to_imm()),
        OperandKind::Imm64 => Operand::Imm64(0),
        OperandKind::Float64 => Operand::Float64(0.0),
        OperandKind::Token => Operand::Token(Token::new(TableKind::MethodDef, 1)),
        OperandKind::TokenImm32 => Operand::TokenImm32(Token::new(TableKind::MethodDef, 1), 1),
        OperandKind::Local => Operand::Local(0),
        OperandKind::StringRef => Operand::StringRef(0),
        OperandKind::Branch => Operand::Branch(0),
    }
}

/// The expected arity of `op`, from [`SHAPES`] — never from the table.
/// `None` for the terminators, which are built by hand.
fn expected_arity(op: Opcode) -> Option<(usize, usize)> {
    let (_, shape) = SHAPES
        .iter()
        .find(|(candidate, _)| *candidate == op)
        .unwrap_or_else(|| panic!("{} is missing from SHAPES", op.mnemonic()));
    match shape {
        Shape::Fixed(pops, pushes) => Some((*pops, *pushes)),
        Shape::Call => {
            let (pops, pushes) = resolver(op, &operand_for(op))
                .unwrap_or_else(|| panic!("no test arity for {}", op.mnemonic()));
            Some((pops as usize, pushes as usize))
        }
        Shape::Terminator => None,
    }
}

fn push_i32() -> Instruction {
    Instruction::new(Opcode::ConstI32, Operand::Imm32(0))
}

/// A body the verifier must accept: push what the opcode consumes, run it,
/// drain what it produces, return.
fn valid_body(op: Opcode) -> Option<Vec<Instruction>> {
    let (pops, pushes) = expected_arity(op)?;
    let mut body: Vec<Instruction> = std::iter::repeat_with(push_i32).take(pops).collect();
    body.push(Instruction::new(op, operand_for(op)));
    body.extend(std::iter::repeat_with(|| Instruction::simple(Opcode::Pop)).take(pushes));
    body.push(Instruction::simple(Opcode::Ret));
    Some(body)
}

#[test]
fn the_table_agrees_with_the_expected_shapes() {
    // The check that makes the rest meaningful: a wrong `pops` or `pushes` in
    // the master table fails here, because this side was written separately.
    for &(op, shape) in SHAPES {
        let actual = match op.stack_effect() {
            StackEffect::Fixed { pops, pushes } => Shape::Fixed(pops as usize, pushes as usize),
            StackEffect::Call => Shape::Call,
            StackEffect::Terminator => Shape::Terminator,
        };
        assert_eq!(
            actual,
            shape,
            "{} declares {actual:?} but is expected to be {shape:?}",
            op.mnemonic()
        );
    }
}

#[test]
fn every_opcode_has_an_expected_shape() {
    // A new row in the table without an entry here fails immediately, so the
    // oracle cannot silently fall behind.
    for &op in Opcode::ALL {
        assert!(
            SHAPES.iter().any(|(candidate, _)| *candidate == op),
            "{} is missing from SHAPES; every opcode needs fixture coverage",
            op.mnemonic()
        );
    }
    assert_eq!(SHAPES.len(), Opcode::ALL.len(), "SHAPES has stale entries");
}

#[test]
fn fixture_valid_every_non_terminator_opcode() {
    for &op in Opcode::ALL {
        let Some(body) = valid_body(op) else {
            continue;
        };
        assert_eq!(
            check(&asm(&body), false),
            Ok(()),
            "{} should verify with {} operand(s)",
            op.mnemonic(),
            expected_arity(op).expect("non-terminator").0
        );
    }
}

#[test]
fn fixture_invalid_stack_underflow_for_every_consuming_opcode() {
    for &op in Opcode::ALL {
        let Some((pops, _)) = expected_arity(op) else {
            continue;
        };
        if pops == 0 {
            continue;
        }
        // One operand short of what the opcode consumes.
        let mut body: Vec<Instruction> = std::iter::repeat_with(push_i32).take(pops - 1).collect();
        body.push(Instruction::new(op, operand_for(op)));
        body.push(Instruction::simple(Opcode::Ret));

        assert_eq!(
            check(&asm(&body), false).map_err(|e| e.kind),
            Err(ErrorKind::StackUnderflow),
            "{} accepted {} operand(s) when it pops {pops}",
            op.mnemonic(),
            pops - 1
        );
    }
}

#[test]
fn fixture_invalid_bad_return_height_for_every_producing_opcode() {
    // Pins `pushes`: a void method that leaves the result behind must be
    // rejected, so an opcode producing a different number of values than
    // expected is caught here.
    for &op in Opcode::ALL {
        let Some((pops, pushes)) = expected_arity(op) else {
            continue;
        };
        if pushes == 0 {
            continue;
        }
        let mut body: Vec<Instruction> = std::iter::repeat_with(push_i32).take(pops).collect();
        body.push(Instruction::new(op, operand_for(op)));
        body.push(Instruction::simple(Opcode::Ret));

        assert_eq!(
            check(&asm(&body), false).map_err(|e| e.kind),
            Err(ErrorKind::BadReturnHeight {
                expected: 0,
                found: pushes
            }),
            "{} should leave {pushes} value(s) on the stack",
            op.mnemonic()
        );
    }
}

#[test]
fn fixture_valid_terminators() {
    // `br` to the following instruction.
    assert_eq!(
        check(
            &asm(&[
                Instruction::new(Opcode::Branch, Operand::Branch(0)),
                Instruction::simple(Opcode::Ret),
            ]),
            false
        ),
        Ok(())
    );
    // `ret` with nothing on the stack, in a void method.
    assert_eq!(
        check(&asm(&[Instruction::simple(Opcode::Ret)]), false),
        Ok(())
    );
    // `ret` with the single value a value-returning method owes.
    assert_eq!(
        check(&asm(&[push_i32(), Instruction::simple(Opcode::Ret)]), true),
        Ok(())
    );
    // `throw` and `rethrow` consume the exception on the stack.
    for op in [Opcode::Throw, Opcode::Rethrow] {
        assert_eq!(
            check(&asm(&[push_i32(), Instruction::simple(op)]), false),
            Ok(()),
            "{}",
            op.mnemonic()
        );
    }
}

#[test]
fn fixture_invalid_throw_and_rethrow_on_an_empty_stack() {
    for op in [Opcode::Throw, Opcode::Rethrow] {
        assert_eq!(
            check(&asm(&[Instruction::simple(op)]), false).map_err(|e| e.kind),
            Err(ErrorKind::StackUnderflow),
            "{}",
            op.mnemonic()
        );
    }
}
