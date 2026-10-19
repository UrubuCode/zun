//! Instructions and their binary encoding.
//!
//! An [`Instruction`] pairs an [`Opcode`] with its decoded [`Operand`]. The
//! [`encode`] and [`decode_one`] functions are the assembler and disassembler
//! halves of the same table-driven format: both consult [`Opcode::operand`] to
//! know how many inline bytes to write or read, so they can never disagree.

use crate::opcode::{Opcode, OperandKind};
use zun_metadata::Token;

/// A decoded inline operand. The variant always matches the opcode's
/// [`OperandKind`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Operand {
    /// No operand.
    None,
    /// A 32-bit signed immediate.
    Imm32(i32),
    /// A 64-bit signed immediate.
    Imm64(i64),
    /// A 64-bit float immediate.
    Float64(f64),
    /// A metadata token.
    Token(Token),
    /// A local-variable slot index.
    Local(u16),
    /// An index into the module's string heap.
    StringRef(u32),
    /// A signed branch displacement relative to the end of the instruction.
    Branch(i32),
}

impl Operand {
    /// The operand kind this value represents.
    pub fn kind(&self) -> OperandKind {
        match self {
            Operand::None => OperandKind::None,
            Operand::Imm32(_) => OperandKind::Imm32,
            Operand::Imm64(_) => OperandKind::Imm64,
            Operand::Float64(_) => OperandKind::Float64,
            Operand::Token(_) => OperandKind::Token,
            Operand::Local(_) => OperandKind::Local,
            Operand::StringRef(_) => OperandKind::StringRef,
            Operand::Branch(_) => OperandKind::Branch,
        }
    }
}

/// A single ZIL instruction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instruction {
    /// The operation.
    pub opcode: Opcode,
    /// Its inline operand.
    pub operand: Operand,
}

impl Instruction {
    /// Build an instruction, checking that the operand matches the opcode.
    ///
    /// Panics on a mismatch, which is always a bug in the producer.
    pub fn new(opcode: Opcode, operand: Operand) -> Self {
        assert_eq!(
            opcode.operand(),
            operand.kind(),
            "operand {:?} does not match opcode {:?}",
            operand.kind(),
            opcode
        );
        Self { opcode, operand }
    }

    /// A no-operand instruction.
    pub fn simple(opcode: Opcode) -> Self {
        Self::new(opcode, Operand::None)
    }

    /// The total encoded size in bytes (opcode byte + operand).
    pub fn encoded_len(&self) -> usize {
        1 + self.opcode.operand().size()
    }
}

/// An error encountered while decoding an instruction stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The opcode byte is not a known opcode.
    UnknownOpcode(u8),
    /// The stream ended in the middle of an instruction.
    UnexpectedEof,
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DecodeError::UnknownOpcode(b) => write!(f, "unknown opcode byte 0x{b:02X}"),
            DecodeError::UnexpectedEof => write!(f, "unexpected end of instruction stream"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Append the binary encoding of `instr` to `out`.
pub fn encode(instr: &Instruction, out: &mut Vec<u8>) {
    out.push(instr.opcode.to_byte());
    match instr.operand {
        Operand::None => {}
        Operand::Imm32(v) => out.extend_from_slice(&v.to_le_bytes()),
        Operand::Imm64(v) => out.extend_from_slice(&v.to_le_bytes()),
        Operand::Float64(v) => out.extend_from_slice(&v.to_bits().to_le_bytes()),
        Operand::Token(t) => out.extend_from_slice(&t.to_raw().to_le_bytes()),
        Operand::Local(v) => out.extend_from_slice(&v.to_le_bytes()),
        Operand::StringRef(v) => out.extend_from_slice(&v.to_le_bytes()),
        Operand::Branch(v) => out.extend_from_slice(&v.to_le_bytes()),
    }
}

/// Decode one instruction from the front of `bytes`, returning it and the number
/// of bytes consumed.
pub fn decode_one(bytes: &[u8]) -> Result<(Instruction, usize), DecodeError> {
    let &first = bytes.first().ok_or(DecodeError::UnexpectedEof)?;
    let opcode = Opcode::from_byte(first).ok_or(DecodeError::UnknownOpcode(first))?;
    let kind = opcode.operand();
    let body = &bytes[1..];
    if body.len() < kind.size() {
        return Err(DecodeError::UnexpectedEof);
    }
    let operand = match kind {
        OperandKind::None => Operand::None,
        OperandKind::Imm32 => Operand::Imm32(i32::from_le_bytes(body[..4].try_into().unwrap())),
        OperandKind::Imm64 => Operand::Imm64(i64::from_le_bytes(body[..8].try_into().unwrap())),
        OperandKind::Float64 => {
            Operand::Float64(f64::from_bits(u64::from_le_bytes(body[..8].try_into().unwrap())))
        }
        OperandKind::Token => {
            Operand::Token(Token::from_raw(u32::from_le_bytes(body[..4].try_into().unwrap())))
        }
        OperandKind::Local => Operand::Local(u16::from_le_bytes(body[..2].try_into().unwrap())),
        OperandKind::StringRef => {
            Operand::StringRef(u32::from_le_bytes(body[..4].try_into().unwrap()))
        }
        OperandKind::Branch => Operand::Branch(i32::from_le_bytes(body[..4].try_into().unwrap())),
    };
    Ok((Instruction { opcode, operand }, 1 + kind.size()))
}

/// Decode an entire instruction stream into a vector.
pub fn decode_all(mut bytes: &[u8]) -> Result<Vec<Instruction>, DecodeError> {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        let (instr, n) = decode_one(bytes)?;
        out.push(instr);
        bytes = &bytes[n..];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_mixed_stream() {
        let program = vec![
            Instruction::new(Opcode::ConstI32, Operand::Imm32(7)),
            Instruction::new(Opcode::ConstI32, Operand::Imm32(35)),
            Instruction::simple(Opcode::AddI32),
            Instruction::new(Opcode::StoreLocal, Operand::Local(0)),
            Instruction::new(Opcode::Call, Operand::Token(Token::from_raw(0x0300_0001))),
            Instruction::new(Opcode::Branch, Operand::Branch(-12)),
            Instruction::simple(Opcode::Ret),
        ];

        let mut bytes = Vec::new();
        for i in &program {
            encode(i, &mut bytes);
        }
        let decoded = decode_all(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    #[test]
    fn float_roundtrip() {
        let i = Instruction::new(Opcode::ConstF64, Operand::Float64(3.141592653589793));
        let mut bytes = Vec::new();
        encode(&i, &mut bytes);
        let (back, n) = decode_one(&bytes).unwrap();
        assert_eq!(back, i);
        assert_eq!(n, 9);
    }

    #[test]
    fn unknown_opcode_errors() {
        assert_eq!(decode_one(&[0xFF]), Err(DecodeError::UnknownOpcode(0xFF)));
    }

    #[test]
    fn truncated_operand_errors() {
        // ConstI32 wants 4 operand bytes; give it 2.
        assert_eq!(
            decode_one(&[Opcode::ConstI32.to_byte(), 1, 2]),
            Err(DecodeError::UnexpectedEof)
        );
    }

    #[test]
    #[should_panic]
    fn operand_mismatch_panics() {
        Instruction::new(Opcode::AddI32, Operand::Imm32(1));
    }
}
