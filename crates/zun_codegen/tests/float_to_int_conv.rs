//! `number` -> integer-element conversions: ECMAScript `ToUint8`/`ToInt8`/...
//! (wrap modulo 2^N, NaN and the infinities to 0), on Tier 0 and Tier 1.
//!
//! Saturating alone is wrong for these: `Infinity` would become the type's
//! maximum and `1e10` would lose its low bits. Both tiers must agree with JS.

mod common;
use common::*;

use zun_codegen::RetVal;
use zun_il::{encode, Instruction, JitKind, NumKind, Opcode, Operand};
use zun_interp::value::Value;
use zun_interp::CaptureOutput;
use zun_metadata::{MethodDef, TableKind};
use zun_module::{write_body, Module};

fn convert_module(kind: NumKind, value: f64) -> Module {
    let mut module = Module::new();
    let mut code = Vec::new();
    for instruction in [
        Instruction::new(Opcode::ConstF64, Operand::Float64(value)),
        Instruction::new(Opcode::Conv, Operand::Imm32(kind.to_imm())),
        Instruction::simple(Opcode::Ret),
    ] {
        encode(&instruction, &mut code);
    }
    let offset = write_body(&mut module.code, 0, 0, true, JitKind::Num(kind).to_byte(), &[], &code);
    module.tables.add_method_def(MethodDef {
        name: module.strings.intern("main"),
        signature: zun_metadata::BlobIndex(0),
        body_offset: offset,
        is_static: false,
    });
    module.entry_point = Some(zun_metadata::Token::new(TableKind::MethodDef, 1));
    module
}

/// Convert on both tiers, require agreement, and return the shared result.
fn both(kind: NumKind, value: f64) -> i64 {
    let module = convert_module(kind, value);
    let mut output = CaptureOutput::default();
    let tier0 = match zun_interp::run(&module, &mut output).expect("interpreter runs") {
        Value::Int(n, _) => n,
        other => panic!("expected an int from Tier 0, got {other:?}"),
    };
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(1), "float -> int conv must be JIT-eligible");
    let tier1 = match jit.call_entry(1).expect("jit call") {
        RetVal::Int(n) => n,
        other => panic!("expected an int from Tier 1, got {other:?}"),
    };
    assert_eq!(tier0, tier1, "{value} as {kind:?}: Tier 0 gave {tier0}, Tier 1 gave {tier1}");
    tier0
}

#[test]
fn non_finite_numbers_convert_to_zero_in_every_narrow_kind() {
    for kind in [NumKind::U8, NumKind::I8, NumKind::U16, NumKind::I16, NumKind::U32, NumKind::I32] {
        for value in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            assert_eq!(both(kind, value), 0, "{value} as {kind:?}");
        }
    }
}

#[test]
fn finite_numbers_truncate_and_wrap() {
    assert_eq!(both(NumKind::U8, 300.0), 44);
    assert_eq!(both(NumKind::U8, -1.0), 255);
    assert_eq!(both(NumKind::U8, 3.7), 3);
    assert_eq!(both(NumKind::U8, 1e10), 0);
    assert_eq!(both(NumKind::U8, 1e20), 0);
    assert_eq!(both(NumKind::U8, -1e20), 0);
    assert_eq!(both(NumKind::I8, 200.0), -56);
    assert_eq!(both(NumKind::I8, -200.0), 56);
    assert_eq!(both(NumKind::U16, 70000.0), 4464);
    assert_eq!(both(NumKind::U32, 1e10), 1410065408);
    assert_eq!(both(NumKind::I32, 3e9), -1294967296);
}

/// Store `Infinity` into element 0 of `new <ctor>(1)` and read it back.
fn stored(ctor: &str) -> f64 {
    let module = compile(&format!(
        "function main(): number {{ const t = new {ctor}(1); t[0] = Infinity; return Number(t[0]); }}"
    ));
    match interp_entry(&module) {
        Value::Float(n, _) => n,
        other => panic!("expected a float, got {other:?}"),
    }
}

#[test]
fn typed_element_store_of_infinity_follows_the_element_kind() {
    for ctor in ["Uint8Array", "Int8Array", "Uint16Array", "Int16Array", "Uint32Array", "Int32Array"] {
        assert_eq!(stored(ctor), 0.0, "{ctor}[0] = Infinity");
    }
    // Uint8ClampedArray really clamps, so Infinity is 255 there.
    assert_eq!(stored("Uint8ClampedArray"), 255.0);
    assert_eq!(stored("Float32Array"), f64::INFINITY);
    assert_eq!(stored("Float64Array"), f64::INFINITY);
}
