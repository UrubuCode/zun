//! Operand decoding helpers for the interpreter dispatch loop.

use zun_il::Operand;

pub(crate) fn operand_f64(o: &Operand) -> f64 {
    match o {
        Operand::Float64(value) => *value,
        _ => 0.0,
    }
}

pub(crate) fn operand_i64(o: &Operand) -> i64 {
    match o {
        Operand::Imm64(value) => *value,
        _ => 0,
    }
}

pub(crate) fn operand_i32(o: &Operand) -> i32 {
    match o {
        Operand::Imm32(value) => *value,
        _ => 0,
    }
}
