//! Tier 0 semantics for the float math machine ops.
//!
//! Tier 0 is the always-correct fallback, so these are the reference results
//! the JIT is later held to. The cases are weighted toward the edges —  NaN,
//! signed zero, infinities — because that is where an IEEE-754 primitive and a
//! hand-written fallback actually diverge; agreeing on `sqrt(4)` proves little.

use zun_il::{encode, Instruction, JitKind, NumKind, Opcode, Operand};
use zun_interp::value::Value;
use zun_interp::CaptureOutput;
use zun_metadata::{MethodDef, TableKind};
use zun_module::{write_body, Module};

/// Build a nullary `main` returning f64 from a literal instruction sequence.
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

fn konst(value: f64) -> Instruction {
    Instruction::new(Opcode::ConstF64, Operand::Float64(value))
}

/// Run `op` over one operand and return the f64 result.
fn unary(op: Opcode, a: f64) -> f64 {
    run(&module_with(&[
        konst(a),
        Instruction::simple(op),
        Instruction::simple(Opcode::Ret),
    ]))
}

/// Run `op` over two operands and return the f64 result.
fn binary(op: Opcode, a: f64, b: f64) -> f64 {
    run(&module_with(&[
        konst(a),
        konst(b),
        Instruction::simple(op),
        Instruction::simple(Opcode::Ret),
    ]))
}

fn run(module: &Module) -> f64 {
    let mut output = CaptureOutput::default();
    match zun_interp::run(module, &mut output).expect("interpreter runs the body") {
        Value::Float(value, _) => value,
        other => panic!("expected a float result, got {other:?}"),
    }
}

/// Distinguishes `+0.0` from `-0.0`, which `==` does not.
fn is_negative_zero(value: f64) -> bool {
    value == 0.0 && value.is_sign_negative()
}

#[test]
fn sqrt_matches_ieee754() {
    assert_eq!(unary(Opcode::SqrtF64, 16.0), 4.0);
    assert_eq!(unary(Opcode::SqrtF64, 0.0), 0.0);
    assert_eq!(unary(Opcode::SqrtF64, f64::INFINITY), f64::INFINITY);
    assert!(unary(Opcode::SqrtF64, -1.0).is_nan());
    assert!(unary(Opcode::SqrtF64, f64::NAN).is_nan());
    // sqrt(-0) is -0, not NaN.
    assert!(is_negative_zero(unary(Opcode::SqrtF64, -0.0)));
}

#[test]
fn abs_clears_the_sign_bit() {
    assert_eq!(unary(Opcode::AbsF64, -3.5), 3.5);
    assert_eq!(unary(Opcode::AbsF64, 3.5), 3.5);
    assert_eq!(unary(Opcode::AbsF64, f64::NEG_INFINITY), f64::INFINITY);
    assert!(unary(Opcode::AbsF64, f64::NAN).is_nan());
    // abs(-0) is +0: the distinction abs() is expected to erase.
    assert!(!is_negative_zero(unary(Opcode::AbsF64, -0.0)));
}

#[test]
fn neg_flips_the_sign_bit() {
    assert_eq!(unary(Opcode::NegF64, 3.5), -3.5);
    assert_eq!(unary(Opcode::NegF64, -3.5), 3.5);
    assert_eq!(unary(Opcode::NegF64, f64::INFINITY), f64::NEG_INFINITY);
    // Unlike abs, neg preserves the zero distinction and swaps it.
    assert!(is_negative_zero(unary(Opcode::NegF64, 0.0)));
    assert!(!is_negative_zero(unary(Opcode::NegF64, -0.0)));
    assert!(unary(Opcode::NegF64, f64::NAN).is_nan());
}

#[test]
fn floor_rounds_toward_negative_infinity() {
    assert_eq!(unary(Opcode::FloorF64, 1.9), 1.0);
    assert_eq!(unary(Opcode::FloorF64, -1.1), -2.0);
    assert_eq!(unary(Opcode::FloorF64, 2.0), 2.0);
    assert!(unary(Opcode::FloorF64, f64::NAN).is_nan());
}

#[test]
fn ceil_rounds_toward_positive_infinity() {
    assert_eq!(unary(Opcode::CeilF64, 1.1), 2.0);
    assert_eq!(unary(Opcode::CeilF64, -1.9), -1.0);
    assert_eq!(unary(Opcode::CeilF64, 2.0), 2.0);
    // ceil(-0.5) is -0, a case that trips implementations that go via i64.
    assert!(is_negative_zero(unary(Opcode::CeilF64, -0.5)));
    assert!(unary(Opcode::CeilF64, f64::NAN).is_nan());
}

#[test]
fn trunc_rounds_toward_zero() {
    assert_eq!(unary(Opcode::TruncF64, 1.9), 1.0);
    assert_eq!(unary(Opcode::TruncF64, -1.9), -1.0);
    assert!(is_negative_zero(unary(Opcode::TruncF64, -0.5)));
    assert!(unary(Opcode::TruncF64, f64::NAN).is_nan());
}

#[test]
fn min_and_max_pick_the_expected_operand() {
    assert_eq!(binary(Opcode::MinF64, 1.0, 2.0), 1.0);
    assert_eq!(binary(Opcode::MinF64, 2.0, 1.0), 1.0);
    assert_eq!(binary(Opcode::MaxF64, 1.0, 2.0), 2.0);
    assert_eq!(binary(Opcode::MaxF64, 2.0, 1.0), 2.0);
    assert_eq!(binary(Opcode::MinF64, -1.0, f64::INFINITY), -1.0);
    assert_eq!(binary(Opcode::MaxF64, -1.0, f64::NEG_INFINITY), -1.0);
}

#[test]
fn min_and_max_propagate_nan() {
    // IEEE-754 2019 `minimum`/`maximum` and `Math.min`/`Math.max` agree that
    // NaN wins. Rust's `f64::min` does the opposite (it returns the non-NaN
    // operand), so this is the case that catches a lazy implementation.
    for &op in &[Opcode::MinF64, Opcode::MaxF64] {
        assert!(binary(op, f64::NAN, 1.0).is_nan(), "{}", op.mnemonic());
        assert!(binary(op, 1.0, f64::NAN).is_nan(), "{}", op.mnemonic());
        assert!(
            binary(op, f64::NAN, f64::NAN).is_nan(),
            "{}",
            op.mnemonic()
        );
    }
}

#[test]
fn min_and_max_order_signed_zeros() {
    // -0.0 == +0.0, so an implementation that only compares with `<` returns
    // whichever operand it happens to prefer. JS requires -0 to be the minimum
    // and +0 the maximum, in both argument orders.
    assert!(is_negative_zero(binary(Opcode::MinF64, 0.0, -0.0)));
    assert!(is_negative_zero(binary(Opcode::MinF64, -0.0, 0.0)));
    assert!(!is_negative_zero(binary(Opcode::MaxF64, 0.0, -0.0)));
    assert!(!is_negative_zero(binary(Opcode::MaxF64, -0.0, 0.0)));
}

#[test]
fn math_ops_compose_on_the_evaluation_stack() {
    // min(|-9|, sqrt(16)) == 4
    let module = module_with(&[
        konst(-9.0),
        Instruction::simple(Opcode::AbsF64),
        konst(16.0),
        Instruction::simple(Opcode::SqrtF64),
        Instruction::simple(Opcode::MinF64),
        Instruction::simple(Opcode::Ret),
    ]);
    assert_eq!(run(&module), 4.0);
}
