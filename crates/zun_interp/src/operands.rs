//! Operand decoding helpers for the interpreter dispatch loop.

use zun_il::Operand;

pub(crate) fn branch_target(instr_end: usize, operand: &Operand) -> usize {
    let displacement = match operand {
        Operand::Branch(value) => *value,
        _ => 0,
    };
    (instr_end as i64 + displacement as i64) as usize
}

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

pub(crate) fn operand_local(o: &Operand) -> usize {
    match o {
        Operand::Local(value) => *value as usize,
        _ => 0,
    }
}

pub(crate) fn operand_strref(o: &Operand) -> u32 {
    match o {
        Operand::StringRef(value) => *value,
        _ => 0,
    }
}

pub(crate) fn operand_token(o: &Operand) -> zun_metadata::Token {
    match o {
        Operand::Token(token) => *token,
        _ => zun_metadata::Token::from_raw(0),
    }
}

pub(crate) fn operand_token_imm32(o: &Operand) -> (zun_metadata::Token, i32) {
    match o {
        Operand::TokenImm32(token, value) => (*token, *value),
        _ => (zun_metadata::Token::from_raw(0), -1),
    }
}
