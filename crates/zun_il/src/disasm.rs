//! Textual disassembly.
//!
//! A small, deterministic printer used by tooling and tests. It renders each
//! instruction as `offset: mnemonic operand`, reading operand shapes from the
//! same master table as the binary codec.

use crate::instr::{decode_all, DecodeError, Instruction, Operand};

/// Format a single instruction as `mnemonic operand`.
pub fn format_instruction(instr: &Instruction) -> String {
    let m = instr.opcode.mnemonic();
    match instr.operand {
        Operand::None => m.to_string(),
        Operand::Imm32(v) => format!("{m} {v}"),
        Operand::Imm64(v) => format!("{m} {v}"),
        Operand::Float64(v) => format!("{m} {v}"),
        Operand::Token(t) => format!("{m} {t:?}"),
        Operand::TokenImm32(t, v) => format!("{m} {t:?} {v}"),
        Operand::Local(v) => format!("{m} {v}"),
        Operand::StringRef(v) => format!("{m} str:{v}"),
        Operand::Branch(v) => format!("{m} {v:+}"),
    }
}

/// Disassemble a byte stream into a multi-line listing with byte offsets.
pub fn disassemble(bytes: &[u8]) -> Result<String, DecodeError> {
    let mut out = String::new();
    let mut offset = 0usize;
    for instr in decode_all(bytes)? {
        out.push_str(&format!("{offset:04X}: {}\n", format_instruction(&instr)));
        offset += instr.encoded_len();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instr::encode;
    use crate::opcode::Opcode;

    #[test]
    fn listing_has_offsets_and_mnemonics() {
        let program = [
            Instruction::new(Opcode::ConstI32, Operand::Imm32(5)),
            Instruction::simple(Opcode::NegI32),
            Instruction::simple(Opcode::Ret),
        ];
        let mut bytes = Vec::new();
        for i in &program {
            encode(i, &mut bytes);
        }
        let text = disassemble(&bytes).unwrap();
        let expected = "0000: const.i32 5\n0005: neg.i32\n0006: ret\n";
        assert_eq!(text, expected);
    }
}
