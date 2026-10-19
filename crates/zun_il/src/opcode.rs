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
macro_rules! define_opcodes {
    ($(
        $(#[$meta:meta])*
        $name:ident = $byte:literal, $operand:ident, $mnemonic:literal;
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

            /// Every opcode, in table order. Used by tests and tooling.
            pub const ALL: &'static [Opcode] = &[ $( Opcode::$name, )* ];
        }
    };
}

define_opcodes! {
    /// Do nothing.
    Nop = 0x00, None, "nop";

    // --- constants ---
    /// Push a 32-bit integer constant.
    ConstI32 = 0x10, Imm32, "const.i32";
    /// Push a 64-bit integer constant.
    ConstI64 = 0x11, Imm64, "const.i64";
    /// Push a 64-bit float constant.
    ConstF64 = 0x12, Float64, "const.f64";
    /// Push the null reference.
    ConstNull = 0x13, None, "const.null";
    /// Push boolean true (as i32 1).
    ConstTrue = 0x14, None, "const.true";
    /// Push boolean false (as i32 0).
    ConstFalse = 0x15, None, "const.false";
    /// Push a string literal from the module string heap.
    ConstStr = 0x16, StringRef, "const.str";

    // --- locals ---
    /// Push the value of a local slot.
    LoadLocal = 0x20, Local, "ldloc";
    /// Pop and store into a local slot.
    StoreLocal = 0x21, Local, "stloc";

    // --- stack ---
    /// Remove the top stack value.
    Pop = 0x28, None, "pop";
    /// Duplicate the top stack value.
    Dup = 0x29, None, "dup";

    // --- integer arithmetic (pop two, push one) ---
    /// Integer add.
    AddI32 = 0x30, None, "add.i32";
    /// Integer subtract.
    SubI32 = 0x31, None, "sub.i32";
    /// Integer multiply.
    MulI32 = 0x32, None, "mul.i32";
    /// Signed integer divide.
    DivI32 = 0x33, None, "div.i32";
    /// Signed integer remainder.
    RemI32 = 0x34, None, "rem.i32";
    /// Integer negate (pop one, push one).
    NegI32 = 0x35, None, "neg.i32";

    // --- float arithmetic ---
    /// Float add.
    AddF64 = 0x40, None, "add.f64";
    /// Float subtract.
    SubF64 = 0x41, None, "sub.f64";
    /// Float multiply.
    MulF64 = 0x42, None, "mul.f64";
    /// Float divide.
    DivF64 = 0x43, None, "div.f64";
    /// Float remainder (JS `%` semantics).
    RemF64 = 0x44, None, "rem.f64";

    // --- strings ---
    /// String concatenation (pop two strings, push one).
    Concat = 0x48, None, "concat";

    // --- comparison (pop two, push i32 0/1) ---
    /// Equal.
    CmpEq = 0x50, None, "cmp.eq";
    /// Not equal.
    CmpNe = 0x51, None, "cmp.ne";
    /// Signed less-than.
    CmpLt = 0x52, None, "cmp.lt";
    /// Signed less-than-or-equal.
    CmpLe = 0x53, None, "cmp.le";
    /// Signed greater-than.
    CmpGt = 0x54, None, "cmp.gt";
    /// Signed greater-than-or-equal.
    CmpGe = 0x55, None, "cmp.ge";

    // --- control flow ---
    /// Unconditional branch.
    Branch = 0x60, Branch, "br";
    /// Branch if the popped value is non-zero.
    BranchTrue = 0x61, Branch, "br.true";
    /// Branch if the popped value is zero.
    BranchFalse = 0x62, Branch, "br.false";
    /// Return, popping the return value if the method has one.
    Ret = 0x63, None, "ret";

    // --- objects and calls (operand is a metadata token) ---
    /// Allocate an object and run its constructor.
    NewObj = 0x70, Token, "newobj";
    /// Load an object field.
    LoadField = 0x71, Token, "ldfld";
    /// Store an object field.
    StoreField = 0x72, Token, "stfld";
    /// Direct (static / non-virtual) call.
    Call = 0x73, Token, "call";
    /// Virtual / interface call.
    CallVirt = 0x74, Token, "callvirt";
    /// Call a runtime builtin by id. Phase-1 scaffolding: builtins move to
    /// `zun_core` MemberRefs once the std exists.
    CallBuiltin = 0x75, Imm32, "call.builtin";
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
