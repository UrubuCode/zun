//! Verifier fixtures.
//!
//! Each `fixture_Valid_*` builds a body the verifier must accept; each
//! `fixture_Invalid_*` builds one it must reject with a specific [`ErrorKind`].
//! Bodies are assembled from real instructions so the tests track the actual
//! encoding.

use super::*;
use zun_il::{encode, Instruction, Opcode, Operand};
use zun_metadata::{TableKind, Token};

/// Assemble a sequence of instructions into bytes.
fn asm(instrs: &[Instruction]) -> Vec<u8> {
    let mut out = Vec::new();
    for i in instrs {
        encode(i, &mut out);
    }
    out
}

/// A resolver where `call` pops 1 / pushes 1 and `call.builtin` pops 1 / pushes
/// 1 (the phase-1 `print` returns a void value that still occupies a slot).
fn resolver(opcode: Opcode, operand: &Operand) -> Option<(u8, u8)> {
    match opcode {
        Opcode::Call | Opcode::CallVirt => Some((1, 1)),
        Opcode::CallVirtRest => match operand {
            Operand::TokenImm32(_, count) if *count >= 0 => Some((*count as u8, 1)),
            _ => None,
        },
        Opcode::CallVirtRestArray => Some((2, 1)),
        Opcode::CallVirtRestArrayFixed => match operand {
            Operand::TokenImm32(_, count) if *count >= 0 => Some((*count as u8 + 2, 1)),
            _ => None,
        },
        Opcode::CallIndirect => match operand {
            Operand::Token(token) if token.table() == Some(TableKind::MemberRef) => Some((3, 1)),
            _ => Some((2, 1)),
        },
        Opcode::CallIndirectRest => match operand {
            Operand::TokenImm32(_, count) if *count >= 0 => Some((*count as u8 + 1, 1)),
            _ => None,
        },
        Opcode::CallRest => match operand {
            Operand::TokenImm32(_, count) if *count >= 0 => Some((*count as u8, 1)),
            _ => None,
        },
        Opcode::CallRestArray => Some((1, 1)),
        Opcode::CallRestArrayFixed => match operand {
            Operand::TokenImm32(_, count) if *count >= 0 => Some((*count as u8 + 1, 1)),
            _ => None,
        },
        Opcode::CallBuiltin => Some((1, 1)),
        Opcode::NewObj => Some((0, 1)),
        _ => None,
    }
}

fn check(code: &[u8], info: MethodInfo) -> Result<(), VerifyError> {
    verify(code, info, &resolver)
}

const VOID: MethodInfo = MethodInfo {
    local_count: 2,
    returns_value: false,
};
const RETURNS: MethodInfo = MethodInfo {
    local_count: 2,
    returns_value: true,
};

#[test]
fn fixture_valid_straight_line() {
    let code = asm(&[
        Instruction::new(Opcode::ConstF64, Operand::Float64(1.0)),
        Instruction::new(Opcode::ConstF64, Operand::Float64(2.0)),
        Instruction::simple(Opcode::AddF64),
        Instruction::new(Opcode::StoreLocal, Operand::Local(0)),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_valid_any_box_and_unbox() {
    let code = asm(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(7)),
        Instruction::simple(Opcode::BoxAny),
        Instruction::simple(Opcode::UnboxAny),
        Instruction::simple(Opcode::Pop),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_valid_cast_class_stack_effect() {
    let code = asm(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
        Instruction::new(
            Opcode::CastClass,
            Operand::Token(Token::new(TableKind::TypeDef, 1)),
        ),
        Instruction::simple(Opcode::Pop),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_invalid_cast_class_underflow() {
    let code = asm(&[
        Instruction::new(
            Opcode::CastClass,
            Operand::Token(Token::new(TableKind::TypeDef, 1)),
        ),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(
        check(&code, VOID).unwrap_err().kind,
        ErrorKind::StackUnderflow
    );
}

#[test]
fn fixture_valid_load_function() {
    let code = asm(&[
        Instruction::new(
            Opcode::LoadFunction,
            Operand::Token(Token::new(TableKind::MethodDef, 1)),
        ),
        Instruction::simple(Opcode::Pop),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_valid_make_closure() {
    let code = asm(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
        Instruction::new(
            Opcode::MakeClosure,
            Operand::Token(Token::new(TableKind::MethodDef, 1)),
        ),
        Instruction::simple(Opcode::Pop),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_valid_indirect_call() {
    let code = asm(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(7)),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(35)),
        Instruction::new(
            Opcode::CallIndirect,
            Operand::Token(Token::new(TableKind::MethodDef, 1)),
        ),
        Instruction::simple(Opcode::Pop),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_valid_rest_call_with_explicit_argument_count() {
    let code = asm(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(1)),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(2)),
        Instruction::new(
            Opcode::CallRest,
            Operand::TokenImm32(Token::new(TableKind::MethodDef, 1), 2),
        ),
        Instruction::simple(Opcode::Ret),
    ]);
    check(&code, RETURNS).expect("valid rest call");
}

#[test]
fn fixture_invalid_rest_call_with_negative_argument_count() {
    let code = asm(&[Instruction::new(
        Opcode::CallRest,
        Operand::TokenImm32(Token::new(TableKind::MethodDef, 1), -1),
    )]);
    assert!(matches!(
        check(&code, VOID),
        Err(VerifyError {
            kind: ErrorKind::UnknownCallArity,
            ..
        })
    ));
}

#[test]
fn fixture_valid_virtual_rest_call_with_explicit_argument_count() {
    let code = asm(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(1)),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(2)),
        Instruction::new(
            Opcode::CallVirtRest,
            Operand::TokenImm32(Token::new(TableKind::MethodDef, 1), 2),
        ),
        Instruction::simple(Opcode::Ret),
    ]);
    check(&code, RETURNS).expect("valid virtual rest call");
}

#[test]
fn fixture_invalid_virtual_rest_call_with_negative_argument_count() {
    let code = asm(&[Instruction::new(
        Opcode::CallVirtRest,
        Operand::TokenImm32(Token::new(TableKind::MethodDef, 1), -1),
    )]);
    assert!(matches!(
        check(&code, VOID),
        Err(VerifyError {
            kind: ErrorKind::UnknownCallArity,
            ..
        })
    ));
}

#[test]
fn fixture_valid_virtual_rest_array_calls() {
    let code = asm(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(1)),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(2)),
        Instruction::new(Opcode::CallVirtRestArray, Operand::Token(Token::new(TableKind::MethodDef, 1))),
        Instruction::simple(Opcode::Ret),
    ]);
    check(&code, RETURNS).expect("valid virtual rest array call");
}

#[test]
fn fixture_invalid_virtual_rest_array_fixed_call_with_negative_count() {
    let code = asm(&[Instruction::new(
        Opcode::CallVirtRestArrayFixed,
        Operand::TokenImm32(Token::new(TableKind::MethodDef, 1), -1),
    )]);
    assert!(matches!(
        check(&code, VOID),
        Err(VerifyError { kind: ErrorKind::UnknownCallArity, .. })
    ));
}

#[test]
fn fixture_valid_indirect_rest_call_with_explicit_argument_count() {
    let code = asm(&[
        Instruction::new(Opcode::LoadFunction, Operand::Token(Token::new(TableKind::MethodDef, 1))),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(1)),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(2)),
        Instruction::new(Opcode::CallIndirectRest, Operand::TokenImm32(Token::new(TableKind::MethodDef, 1), 2)),
        Instruction::simple(Opcode::Ret),
    ]);
    check(&code, RETURNS).expect("valid indirect rest call");
}

#[test]
fn fixture_invalid_indirect_rest_call_with_negative_argument_count() {
    let code = asm(&[Instruction::new(
        Opcode::CallIndirectRest,
        Operand::TokenImm32(Token::new(TableKind::MethodDef, 1), -1),
    )]);
    assert!(matches!(
        check(&code, VOID),
        Err(VerifyError { kind: ErrorKind::UnknownCallArity, .. })
    ));
}

#[test]
fn fixture_valid_fixed_rest_array_call() {
    let code = asm(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(7)),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(3)),
        Instruction::new(Opcode::NewArray, Operand::None),
        Instruction::new(
            Opcode::CallRestArrayFixed,
            Operand::TokenImm32(Token::new(TableKind::MethodDef, 1), 1),
        ),
        Instruction::simple(Opcode::Ret),
    ]);
    check(&code, RETURNS).expect("valid fixed rest array call");
}

#[test]
fn fixture_invalid_fixed_rest_array_call_with_negative_count() {
    let code = asm(&[Instruction::new(
        Opcode::CallRestArrayFixed,
        Operand::TokenImm32(Token::new(TableKind::MethodDef, 1), -1),
    )]);
    assert!(matches!(
        check(&code, VOID),
        Err(VerifyError {
            kind: ErrorKind::UnknownCallArity,
            ..
        })
    ));
}

#[test]
fn fixture_valid_binary_indirect_call_token() {
    let code = asm(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(7)),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(35)),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
        Instruction::new(
            Opcode::CallIndirect,
            Operand::Token(Token::null(TableKind::MemberRef)),
        ),
        Instruction::simple(Opcode::Pop),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_valid_array_operations() {
    let code = asm(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(1)),
        Instruction::simple(Opcode::NewArray),
        Instruction::simple(Opcode::Dup),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(7)),
        Instruction::simple(Opcode::StoreArray),
        Instruction::simple(Opcode::Pop),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
        Instruction::simple(Opcode::LoadArray),
        Instruction::simple(Opcode::Pop),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(2)),
        Instruction::simple(Opcode::NewArray),
        Instruction::simple(Opcode::ArrayLength),
        Instruction::simple(Opcode::Pop),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(1)),
        Instruction::simple(Opcode::NewArray),
        Instruction::simple(Opcode::ArrayPop),
        Instruction::simple(Opcode::Pop),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(1)),
        Instruction::simple(Opcode::NewArray),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(7)),
        Instruction::simple(Opcode::ArrayPush),
        Instruction::simple(Opcode::Pop),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_invalid_array_load_underflow() {
    let code = asm(&[
        Instruction::simple(Opcode::LoadArray),
        Instruction::simple(Opcode::Ret),
    ]);
    assert!(matches!(
        check(&code, VOID),
        Err(VerifyError {
            kind: ErrorKind::StackUnderflow,
            ..
        })
    ));
}

#[test]
fn fixture_valid_await_stack_effect() {
    let code = asm(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
        Instruction::simple(Opcode::Await),
        Instruction::simple(Opcode::Pop),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_invalid_await_underflow() {
    let code = asm(&[Instruction::simple(Opcode::Await), Instruction::simple(Opcode::Ret)]);
    assert!(matches!(
        check(&code, VOID),
        Err(VerifyError {
            kind: ErrorKind::StackUnderflow,
            ..
        })
    ));
}

#[test]
fn fixture_valid_yield_stack_effect() {
    let code = asm(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
        Instruction::simple(Opcode::Yield),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_invalid_yield_underflow() {
    let code = asm(&[Instruction::simple(Opcode::Yield), Instruction::simple(Opcode::Ret)]);
    assert!(matches!(
        check(&code, VOID),
        Err(VerifyError { kind: ErrorKind::StackUnderflow, .. })
    ));
}

#[test]
fn fixture_invalid_any_box_underflow() {
    let code = asm(&[
        Instruction::simple(Opcode::BoxAny),
        Instruction::simple(Opcode::Ret),
    ]);
    assert!(matches!(
        check(&code, VOID),
        Err(VerifyError {
            kind: ErrorKind::StackUnderflow,
            ..
        })
    ));
}

#[test]
fn fixture_valid_dynamic_property_access() {
    let code = asm(&[
        Instruction::simple(Opcode::NewDynamic),
        Instruction::simple(Opcode::Dup),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(7)),
        Instruction::simple(Opcode::BoxAny),
        Instruction::new(Opcode::StoreDynamic, Operand::StringRef(0)),
        Instruction::simple(Opcode::Pop),
        Instruction::new(Opcode::LoadDynamic, Operand::StringRef(0)),
        Instruction::simple(Opcode::UnboxAny),
        Instruction::simple(Opcode::Pop),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_valid_realm_global_load() {
    let code = asm(&[
        Instruction::simple(Opcode::RealmGlobal),
        Instruction::simple(Opcode::Pop),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_invalid_realm_global_return_height() {
    let code = asm(&[
        Instruction::simple(Opcode::RealmGlobal),
        Instruction::simple(Opcode::Ret),
    ]);
    assert!(matches!(check(&code, VOID), Err(VerifyError { .. })));
}

#[test]
fn fixture_invalid_dynamic_store_underflow() {
    let code = asm(&[
        Instruction::new(Opcode::StoreDynamic, Operand::StringRef(0)),
        Instruction::simple(Opcode::Ret),
    ]);
    assert!(matches!(
        check(&code, VOID),
        Err(VerifyError {
            kind: ErrorKind::StackUnderflow,
            ..
        })
    ));
}

#[test]
fn fixture_valid_dynamic_value_access() {
    let code = asm(&[
        Instruction::simple(Opcode::NewDynamic),
        Instruction::simple(Opcode::Dup),
        Instruction::new(Opcode::ConstStr, Operand::StringRef(0)),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(7)),
        Instruction::simple(Opcode::BoxAny),
        Instruction::simple(Opcode::StoreDynamicValue),
        Instruction::simple(Opcode::Pop),
        Instruction::new(Opcode::ConstStr, Operand::StringRef(0)),
        Instruction::simple(Opcode::LoadDynamicValue),
        Instruction::simple(Opcode::UnboxAny),
        Instruction::simple(Opcode::Pop),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_invalid_dynamic_value_load_underflow() {
    let code = asm(&[
        Instruction::simple(Opcode::LoadDynamicValue),
        Instruction::simple(Opcode::Ret),
    ]);
    assert!(matches!(
        check(&code, VOID),
        Err(VerifyError { kind: ErrorKind::StackUnderflow, .. })
    ));
}

#[test]
fn fixture_valid_return_value_leaves_one() {
    let code = asm(&[
        Instruction::new(Opcode::ConstF64, Operand::Float64(42.0)),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, RETURNS), Ok(()));
}

#[test]
fn fixture_valid_throw_consumes_exception() {
    let code = asm(&[
        Instruction::simple(Opcode::NewDynamic),
        Instruction::simple(Opcode::Throw),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_invalid_throw_without_exception() {
    let code = asm(&[Instruction::simple(Opcode::Throw)]);
    assert!(matches!(
        check(&code, VOID),
        Err(VerifyError {
            kind: ErrorKind::StackUnderflow,
            ..
        })
    ));
}

#[test]
fn fixture_valid_if_else_joins_consistently() {
    // const.true; br.false +L; const.f64 1; stloc0; br +M; [L] const.f64 2;
    // stloc0; [M] ret
    let then_branch = asm(&[
        Instruction::new(Opcode::ConstF64, Operand::Float64(1.0)),
        Instruction::new(Opcode::StoreLocal, Operand::Local(0)),
    ]);
    let else_branch = asm(&[
        Instruction::new(Opcode::ConstF64, Operand::Float64(2.0)),
        Instruction::new(Opcode::StoreLocal, Operand::Local(0)),
    ]);
    let br_over_else_len = 5;
    let mut code = Vec::new();
    encode(&Instruction::simple(Opcode::ConstTrue), &mut code);
    // br.false to else: skip the then-branch and the trailing `br`.
    let to_else = (then_branch.len() + br_over_else_len) as i32;
    encode(
        &Instruction::new(Opcode::BranchFalse, Operand::Branch(to_else)),
        &mut code,
    );
    code.extend_from_slice(&then_branch);
    encode(
        &Instruction::new(Opcode::Branch, Operand::Branch(else_branch.len() as i32)),
        &mut code,
    );
    code.extend_from_slice(&else_branch);
    encode(&Instruction::simple(Opcode::Ret), &mut code);

    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_valid_backward_loop() {
    // [start] ldloc0; const.f64 0; cmp.gt; br.false +exit; ldloc0; stloc0;
    // br -start; [exit] ret
    let mut code = Vec::new();
    let start = code.len();
    encode(
        &Instruction::new(Opcode::LoadLocal, Operand::Local(0)),
        &mut code,
    );
    encode(
        &Instruction::new(Opcode::ConstF64, Operand::Float64(0.0)),
        &mut code,
    );
    encode(&Instruction::simple(Opcode::CmpGt), &mut code);
    // Reserve the br.false; patch after we know the body length.
    let brfalse_at = code.len();
    encode(
        &Instruction::new(Opcode::BranchFalse, Operand::Branch(0)),
        &mut code,
    );
    encode(
        &Instruction::new(Opcode::LoadLocal, Operand::Local(0)),
        &mut code,
    );
    encode(
        &Instruction::new(Opcode::StoreLocal, Operand::Local(0)),
        &mut code,
    );
    let back = start as i32 - (code.len() as i32 + 5);
    encode(
        &Instruction::new(Opcode::Branch, Operand::Branch(back)),
        &mut code,
    );
    let exit = code.len();
    encode(&Instruction::simple(Opcode::Ret), &mut code);
    // Patch br.false to jump to `exit`.
    let disp = (exit as i32) - (brfalse_at as i32 + 5);
    code[brfalse_at + 1..brfalse_at + 5].copy_from_slice(&disp.to_le_bytes());

    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_invalid_underflow() {
    // add.f64 with an empty stack.
    let code = asm(&[
        Instruction::simple(Opcode::AddF64),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(
        check(&code, VOID).unwrap_err().kind,
        ErrorKind::StackUnderflow
    );
}

#[test]
fn fixture_invalid_bad_return_height() {
    // Leave a value on the stack in a void method.
    let code = asm(&[
        Instruction::new(Opcode::ConstF64, Operand::Float64(1.0)),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(
        check(&code, VOID).unwrap_err().kind,
        ErrorKind::BadReturnHeight {
            expected: 0,
            found: 1
        }
    );
}

#[test]
fn fixture_invalid_local_out_of_range() {
    let code = asm(&[
        Instruction::new(Opcode::ConstF64, Operand::Float64(1.0)),
        Instruction::new(Opcode::StoreLocal, Operand::Local(9)),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(
        check(&code, VOID).unwrap_err().kind,
        ErrorKind::LocalOutOfRange(9)
    );
}

#[test]
fn fixture_invalid_stack_mismatch_at_join() {
    // const.true; br.false +L; const.f64 1 (fall-through leaves 1 extra);
    // [L] nop; pop?; ret.  The join at `nop` is reached with height 1 (fall
    // through) and height 0 (taken branch): a mismatch at a non-terminator, so
    // it is reported before any return-height check.
    let mut code = Vec::new();
    encode(&Instruction::simple(Opcode::ConstTrue), &mut code);
    let const_len = 9; // const.f64 is 9 bytes; jump over it to the nop
    encode(
        &Instruction::new(Opcode::BranchFalse, Operand::Branch(const_len)),
        &mut code,
    );
    encode(
        &Instruction::new(Opcode::ConstF64, Operand::Float64(1.0)),
        &mut code,
    );
    encode(&Instruction::simple(Opcode::Nop), &mut code);
    encode(&Instruction::simple(Opcode::Ret), &mut code);
    // RETURNS so the tall (height-1) path satisfies `ret`; the short path then
    // rejoins `nop` with height 0, which is the mismatch under test.
    assert!(matches!(
        check(&code, RETURNS).unwrap_err().kind,
        ErrorKind::StackMismatch { .. }
    ));
}

#[test]
fn fixture_invalid_branch_into_middle_of_instruction() {
    // br with a displacement that lands inside the const.f64 operand bytes.
    let mut code = Vec::new();
    // br +1 : target offset = end(5) + 1 = 6, which is inside the const.f64.
    encode(
        &Instruction::new(Opcode::Branch, Operand::Branch(1)),
        &mut code,
    );
    encode(
        &Instruction::new(Opcode::ConstF64, Operand::Float64(1.0)),
        &mut code,
    );
    encode(&Instruction::simple(Opcode::Ret), &mut code);
    assert!(matches!(
        check(&code, VOID).unwrap_err().kind,
        ErrorKind::BadBranchTarget(_)
    ));
}

#[test]
fn fixture_invalid_branch_out_of_range() {
    let mut code = Vec::new();
    encode(
        &Instruction::new(Opcode::Branch, Operand::Branch(9999)),
        &mut code,
    );
    assert!(matches!(
        check(&code, VOID).unwrap_err().kind,
        ErrorKind::BadBranchTarget(_)
    ));
}

#[test]
fn fixture_invalid_falls_off_end() {
    // No terminator: ends after a store.
    let code = asm(&[
        Instruction::new(Opcode::ConstF64, Operand::Float64(1.0)),
        Instruction::new(Opcode::StoreLocal, Operand::Local(0)),
    ]);
    assert_eq!(check(&code, VOID).unwrap_err().kind, ErrorKind::FallsOffEnd);
}

#[test]
fn fixture_invalid_bad_encoding() {
    // 0xFF is not an opcode.
    assert_eq!(
        check(&[0xFF], VOID).unwrap_err().kind,
        ErrorKind::BadEncoding
    );
}

#[test]
fn fixture_invalid_unknown_call_arity() {
    // A call the resolver cannot resolve: use a token for a MethodDef the
    // resolver maps... our resolver resolves Call to (1,1), so instead test a
    // resolver returning None by using LoadField? No — exercise the path with a
    // resolver that declines.
    let decline = |_: Opcode, _: &Operand| None;
    let code = asm(&[
        Instruction::new(Opcode::ConstF64, Operand::Float64(1.0)),
        Instruction::new(
            Opcode::Call,
            Operand::Token(Token::new(TableKind::MethodDef, 1)),
        ),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(
        verify(&code, RETURNS, &decline).unwrap_err().kind,
        ErrorKind::UnknownCallArity
    );
}

#[test]
fn fixture_valid_static_field_stack_effects() {
    let code = asm(&[
        Instruction::new(Opcode::LoadStatic, Operand::Token(Token::new(TableKind::FieldDef, 1))),
        Instruction::simple(Opcode::Pop),
        Instruction::new(Opcode::ConstI64, Operand::Imm64(1)),
        Instruction::new(Opcode::StoreStatic, Operand::Token(Token::new(TableKind::FieldDef, 1))),
        Instruction::simple(Opcode::Pop),
        Instruction::simple(Opcode::Ret),
    ]);
    let resolve = |_: Opcode, _: &Operand| Some((0, 1));
    assert_eq!(verify(&code, VOID, &resolve), Ok(()));
}

#[test]
fn fixture_invalid_static_store_underflow() {
    let code = asm(&[
        Instruction::new(Opcode::StoreStatic, Operand::Token(Token::new(TableKind::FieldDef, 1))),
        Instruction::simple(Opcode::Ret),
    ]);
    let resolve = |_: Opcode, _: &Operand| Some((0, 1));
    assert!(matches!(
        verify(&code, VOID, &resolve),
        Err(VerifyError { kind: ErrorKind::StackUnderflow, .. })
    ));
}

#[test]
fn fixture_invalid_static_load_return_height() {
    let code = asm(&[
        Instruction::new(Opcode::LoadStatic, Operand::Token(Token::new(TableKind::FieldDef, 1))),
        Instruction::simple(Opcode::Ret),
    ]);
    let resolve = |_: Opcode, _: &Operand| Some((0, 1));
    assert!(matches!(
        verify(&code, VOID, &resolve),
        Err(VerifyError { kind: ErrorKind::BadReturnHeight { .. }, .. })
    ));
}

#[test]
fn fixture_valid_handler_region_accepts_error_type() {
    let mut section = Vec::new();
    let body_offset = zun_module::write_body_with_handlers(
        &mut section,
        0,
        0,
        false,
        0,
        &[],
        &asm(&[
            Instruction::simple(Opcode::NewDynamic),
            Instruction::simple(Opcode::Throw),
            Instruction::simple(Opcode::Ret),
        ]),
        &[zun_module::ExceptionHandler {
            try_start: 0,
            try_end: 2,
            handler_start: 2,
            handler_end: 3,
            kind: zun_module::HandlerKind::Catch,
            catch_type: Some(Token::new(TableKind::TypeDef, 1)),
        }],
    );
    let body = zun_module::read_body(&section, body_offset).unwrap();
    assert_eq!(verify_body(&body, VOID, &resolver, &|_| true), Ok(()));
}

#[test]
fn fixture_valid_handler_region_accepts_catch_all() {
    let mut section = Vec::new();
    let body_offset = zun_module::write_body_with_handlers(
        &mut section,
        0,
        0,
        false,
        0,
        &[],
        &asm(&[
            Instruction::simple(Opcode::NewDynamic),
            Instruction::simple(Opcode::Throw),
            Instruction::simple(Opcode::Ret),
        ]),
        &[zun_module::ExceptionHandler {
            try_start: 0,
            try_end: 1,
            handler_start: 1,
            handler_end: 2,
            kind: zun_module::HandlerKind::Catch,
            catch_type: None,
        }],
    );
    let body = zun_module::read_body(&section, body_offset).unwrap();
    assert_eq!(verify_body(&body, VOID, &resolver, &|_| true), Ok(()));
}

#[test]
fn fixture_invalid_handler_crossing_is_rejected() {
    let mut section = Vec::new();
    let body_offset = zun_module::write_body_with_handlers(
        &mut section,
        0,
        0,
        false,
        0,
        &[],
        &[0x63, 0x63, 0x63, 0x63, 0x63, 0x63],
        &[
            zun_module::ExceptionHandler {
                try_start: 0,
                try_end: 3,
                handler_start: 3,
                handler_end: 4,
                kind: zun_module::HandlerKind::Finally,
                catch_type: None,
            },
            zun_module::ExceptionHandler {
                try_start: 2,
                try_end: 5,
                handler_start: 5,
                handler_end: 6,
                kind: zun_module::HandlerKind::Finally,
                catch_type: None,
            },
        ],
    );
    let body = zun_module::read_body(&section, body_offset).unwrap();
    assert!(matches!(
        verify_body(&body, VOID, &resolver, &|_| true),
        Err(VerifyError {
            kind: ErrorKind::InvalidHandlerRegion,
            ..
        })
    ));
}

#[test]
fn fixture_invalid_catch_type_is_rejected() {
    let mut section = Vec::new();
    let body_offset = zun_module::write_body_with_handlers(
        &mut section,
        0,
        0,
        false,
        0,
        &[],
        &[0x63, 0x63],
        &[zun_module::ExceptionHandler {
            try_start: 0,
            try_end: 1,
            handler_start: 1,
            handler_end: 2,
            kind: zun_module::HandlerKind::Catch,
            catch_type: Some(Token::new(TableKind::TypeDef, 9)),
        }],
    );
    let body = zun_module::read_body(&section, body_offset).unwrap();
    assert!(matches!(
        verify_body(&body, VOID, &resolver, &|_| false),
        Err(VerifyError {
            kind: ErrorKind::InvalidCatchType,
            ..
        })
    ));
}

#[test]
fn fixture_valid_import_dynamic_stack_effect() {
    let code = asm(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
        Instruction::simple(Opcode::ImportDynamic),
        Instruction::simple(Opcode::Pop),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(check(&code, VOID), Ok(()));
}

#[test]
fn fixture_invalid_import_dynamic_underflow() {
    let code = asm(&[Instruction::simple(Opcode::ImportDynamic), Instruction::simple(Opcode::Ret)]);
    assert!(matches!(
        check(&code, VOID),
        Err(VerifyError {
            kind: ErrorKind::StackUnderflow,
            ..
        })
    ));
}
