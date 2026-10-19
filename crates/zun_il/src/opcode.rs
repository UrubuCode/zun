//! The ZIL master opcode table.
//!
//! Every fact the rest of the toolchain needs about an opcode — its byte value,
//! its mnemonic, and the shape of its inline operand — lives in the single
//! [`define_opcodes!`] invocation below. The macro expands to the [`Opcode`]
//! enum plus the lookup functions, so the assembler, disassembler, verifier, and
//! codegen importer all read from one source of truth. Adding an instruction
//! means adding one row here and nowhere else.
//!
//! ZIL is a stack machine: operands are consumed from and results pushed to an
//! evaluation stack. Opcodes reference types and members by metadata [`Token`],
//! never by byte offset, so object and method layout can change freely.
//!
//! [`Token`]: zun_metadata::Token

/// The shape of the bytes that follow an opcode in the instruction stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandKind {
    /// No inline operand.
    None,
    /// A 32-bit signed immediate (little-endian).
    Imm32,
    /// A 64-bit signed immediate (little-endian).
    Imm64,
    /// A 64-bit IEEE-754 float immediate (little-endian bits).
    Float64,
    /// A 4-byte metadata token.
    Token,
    /// A 2-byte local-variable slot index.
    Local,
    /// A 4-byte index into the module's string heap.
    StringRef,
    /// A 4-byte signed branch displacement, relative to the end of the
    /// instruction.
    Branch,
}

impl OperandKind {
    /// Number of inline bytes this operand kind occupies.
    pub const fn size(self) -> usize {
        match self {
            OperandKind::None => 0,
            OperandKind::Local => 2,
            OperandKind::Imm32
            | OperandKind::Token
            | OperandKind::Branch
            | OperandKind::StringRef => 4,
            OperandKind::Imm64 | OperandKind::Float64 => 8,
        }
    }
}

/// Generate the [`Opcode`] enum and its lookup tables from a single list.
/// The net effect of an opcode on the evaluation stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackEffect {
    /// Pops `pops` values and pushes `pushes` values — both fixed.
    Fixed {
        /// Values consumed from the stack.
        pops: u8,
        /// Values produced onto the stack.
        pushes: u8,
    },
    /// Effect depends on a callee signature not known from the opcode alone
    /// (the call family). The verifier resolves arity from metadata.
    Call,
    /// Transfers control and does not fall through (`br`, `ret`).
    Terminator,
}

macro_rules! define_opcodes {
    ($(
        $(#[$meta:meta])*
        $name:ident = $byte:literal, $operand:ident, $mnemonic:literal, $effect:expr;
    )*) => {
        /// A ZIL instruction opcode. The discriminant is the on-disk byte value
        /// and is part of the format: never renumber an existing opcode.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum Opcode {
            $( $(#[$meta])* $name = $byte, )*
        }

        impl Opcode {
            /// Decode an opcode from its byte value.
            pub const fn from_byte(byte: u8) -> Option<Opcode> {
                match byte {
                    $( $byte => Some(Opcode::$name), )*
                    _ => None,
                }
            }

            /// The byte value this opcode encodes to.
            pub const fn to_byte(self) -> u8 {
                self as u8
            }

            /// The textual mnemonic used by the disassembler.
            pub const fn mnemonic(self) -> &'static str {
                match self {
                    $( Opcode::$name => $mnemonic, )*
                }
            }

            /// The inline operand shape this opcode expects.
            pub const fn operand(self) -> OperandKind {
                match self {
                    $( Opcode::$name => OperandKind::$operand, )*
                }
            }

            /// This opcode's effect on the evaluation stack.
            pub const fn stack_effect(self) -> StackEffect {
                match self {
                    $( Opcode::$name => $effect, )*
                }
            }

            /// Every opcode, in table order. Used by tests and tooling.
            pub const ALL: &'static [Opcode] = &[ $( Opcode::$name, )* ];
        }
    };
}

define_opcodes! {
    /// Do nothing.
    Nop = 0x00, None, "nop", StackEffect::Fixed { pops: 0, pushes: 0 };

    // --- constants ---
    /// Push a 32-bit integer constant.
    ConstI32 = 0x10, Imm32, "const.i32", StackEffect::Fixed { pops: 0, pushes: 1 };
    /// Push a 64-bit integer constant.
    ConstI64 = 0x11, Imm64, "const.i64", StackEffect::Fixed { pops: 0, pushes: 1 };
    /// Push a 64-bit float constant.
    ConstF64 = 0x12, Float64, "const.f64", StackEffect::Fixed { pops: 0, pushes: 1 };
    /// Push the null reference.
    ConstNull = 0x13, None, "const.null", StackEffect::Fixed { pops: 0, pushes: 1 };
    /// Push boolean true (as i32 1).
    ConstTrue = 0x14, None, "const.true", StackEffect::Fixed { pops: 0, pushes: 1 };
    /// Push boolean false (as i32 0).
    ConstFalse = 0x15, None, "const.false", StackEffect::Fixed { pops: 0, pushes: 1 };
    /// Push a string literal from the module string heap.
    ConstStr = 0x16, StringRef, "const.str", StackEffect::Fixed { pops: 0, pushes: 1 };

    // --- locals ---
    /// Push the value of a local slot.
    LoadLocal = 0x20, Local, "ldloc", StackEffect::Fixed { pops: 0, pushes: 1 };
    /// Pop and store into a local slot.
    StoreLocal = 0x21, Local, "stloc", StackEffect::Fixed { pops: 1, pushes: 0 };

    // --- stack ---
    /// Remove the top stack value.
    Pop = 0x28, None, "pop", StackEffect::Fixed { pops: 1, pushes: 0 };
    /// Duplicate the top stack value.
    Dup = 0x29, None, "dup", StackEffect::Fixed { pops: 1, pushes: 2 };

    // --- integer arithmetic (pop two, push one) ---
    /// Integer add.
    AddI32 = 0x30, None, "add.i32", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Integer subtract.
    SubI32 = 0x31, None, "sub.i32", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Integer multiply.
    MulI32 = 0x32, None, "mul.i32", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Signed integer divide.
    DivI32 = 0x33, None, "div.i32", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Signed integer remainder.
    RemI32 = 0x34, None, "rem.i32", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Integer negate (pop one, push one).
    NegI32 = 0x35, None, "neg.i32", StackEffect::Fixed { pops: 1, pushes: 1 };

    // --- float arithmetic ---
    /// Float add.
    AddF64 = 0x40, None, "add.f64", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Float subtract.
    SubF64 = 0x41, None, "sub.f64", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Float multiply.
    MulF64 = 0x42, None, "mul.f64", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Float divide.
    DivF64 = 0x43, None, "div.f64", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Float remainder (JS `%` semantics).
    RemF64 = 0x44, None, "rem.f64", StackEffect::Fixed { pops: 2, pushes: 1 };

    /// Convert the numeric value on top of the stack to another numeric kind
    /// (width/signedness/float), encoded in the immediate. Covers the `as`
    /// operator and implicit narrowing on store.
    Conv = 0x45, Imm32, "conv", StackEffect::Fixed { pops: 1, pushes: 1 };

    // --- strings ---
    /// String concatenation (pop two strings, push one).
    Concat = 0x48, None, "concat", StackEffect::Fixed { pops: 2, pushes: 1 };

    // --- comparison (pop two, push i32 0/1) ---
    /// Equal.
    CmpEq = 0x50, None, "cmp.eq", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Not equal.
    CmpNe = 0x51, None, "cmp.ne", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Signed less-than.
    CmpLt = 0x52, None, "cmp.lt", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Signed less-than-or-equal.
    CmpLe = 0x53, None, "cmp.le", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Signed greater-than.
    CmpGt = 0x54, None, "cmp.gt", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Signed greater-than-or-equal.
    CmpGe = 0x55, None, "cmp.ge", StackEffect::Fixed { pops: 2, pushes: 1 };

    // --- control flow ---
    /// Unconditional branch.
    Branch = 0x60, Branch, "br", StackEffect::Terminator;
    /// Branch if the popped value is non-zero.
    BranchTrue = 0x61, Branch, "br.true", StackEffect::Fixed { pops: 1, pushes: 0 };
    /// Branch if the popped value is zero.
    BranchFalse = 0x62, Branch, "br.false", StackEffect::Fixed { pops: 1, pushes: 0 };
    /// Return, popping the return value if the method has one.
    Ret = 0x63, None, "ret", StackEffect::Terminator;

    // --- objects and calls (operand is a metadata token) ---
    /// Allocate an object and run its constructor.
    NewObj = 0x70, Token, "newobj", StackEffect::Call;
    /// Load an object field.
    LoadField = 0x71, Token, "ldfld", StackEffect::Fixed { pops: 1, pushes: 1 };
    /// Store an object field, leaving the stored value on the stack so a field
    /// assignment can be used as an expression.
    StoreField = 0x72, Token, "stfld", StackEffect::Fixed { pops: 2, pushes: 1 };
    /// Direct (static / non-virtual) call.
    Call = 0x73, Token, "call", StackEffect::Call;
    /// Virtual / interface call.
    CallVirt = 0x74, Token, "callvirt", StackEffect::Call;
    /// Call a runtime builtin by id. Phase-1 scaffolding: builtins move to
    /// `zun_core` MemberRefs once the std exists.
    CallBuiltin = 0x75, Imm32, "call.builtin", StackEffect::Call;
}

#[cfg(test)]
mod tests {
    use super::{Opcode, OperandKind};

    #[test]
    fn byte_roundtrip_for_all() {
        for &op in Opcode::ALL {
            assert_eq!(Opcode::from_byte(op.to_byte()), Some(op));
        }
    }

    #[test]
    fn byte_values_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for &op in Opcode::ALL {
            assert!(seen.insert(op.to_byte()), "duplicate byte for {op:?}");
        }
    }

    #[test]
    fn mnemonics_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for &op in Opcode::ALL {
            assert!(seen.insert(op.mnemonic()), "duplicate mnemonic {}", op.mnemonic());
        }
    }

    #[test]
    fn unknown_byte_is_none() {
        assert_eq!(Opcode::from_byte(0xFF), None);
    }

    #[test]
    fn operand_sizes() {
        assert_eq!(Opcode::Nop.operand(), OperandKind::None);
        assert_eq!(Opcode::ConstI64.operand().size(), 8);
        assert_eq!(Opcode::Call.operand().size(), 4);
        assert_eq!(Opcode::LoadLocal.operand().size(), 2);
    }
}
