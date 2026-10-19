//! ZIL — the Zun intermediate language.
//!
//! ZIL is a stack-machine bytecode: the unit the compiler emits and the VM
//! executes. This crate owns the format's single source of truth, the master
//! opcode table in [`opcode`], and the codec and disassembler that are generated
//! around it.
//!
//! - [`opcode`]: the [`Opcode`] enum, byte values, mnemonics, and operand shapes.
//! - [`instr`]: [`Instruction`]/[`Operand`] plus the binary [`encode`]/[`decode`]
//!   codec.
//! - [`disasm`]: a textual disassembler for tooling and tests.
//!
//! [`encode`]: instr::encode
//! [`decode`]: instr::decode_one

pub mod disasm;
pub mod instr;
pub mod opcode;

pub use disasm::disassemble;
pub use instr::{decode_all, decode_one, encode, DecodeError, Instruction, Operand};
pub use opcode::{Opcode, OperandKind};
