//! Stage 2d, slice 1: compile a regex to a synthesized ZIL function.
//!
//! This is the pipeline proof for lowering a regex to ordinary ZIL (no new
//! opcodes). It handles only the trivial subset: a pattern that is a plain
//! sequence of literal characters. The emitted method is
//!
//! ```text
//! matchAt(s: string, pos: u64): boolean
//! ```
//!
//! returning whether the literal occurs in `s` exactly at `pos` (sticky /
//! anchored semantics, like [`crate::Regex::match_at_chars`]).
//!
//! Positions are **UTF-16 code units** here (ZIL strings are indexed through
//! the `length` / `charCodeAt` builtins), whereas the interpreted engine
//! indexes by `char`. The two agree on BMP-only input.
//!
//! The string builtins are addressed by native id. Ids are assigned by
//! `zun_natives`; to keep this crate free of that dependency the caller
//! resolves them (`zun_natives::resolve("zun:string", "length")`, ...) and
//! passes them in via [`StringNatives`].

use zun_il::{encode, Instruction, JitKind, NumKind, Opcode, Operand};
use zun_metadata::{MethodDef, StringIndex, TableKind, Token};
use zun_module::{write_body, Module};

/// Native ids of the string builtins the synthesized code calls.
#[derive(Debug, Clone, Copy)]
pub struct StringNatives {
    /// Id of `zun:string` `length(s): u64`.
    pub length: u32,
    /// Id of `zun:string` `charCodeAt(s, index: u64): u32`.
    pub char_code_at: u32,
}

/// The module produced by [`compile_literal`] and the method to call.
#[derive(Debug)]
pub struct CompiledLiteral {
    /// A module holding exactly one method, `matchAt`.
    pub module: Module,
    /// Method token of `matchAt` (`rid` is the 1-based method index).
    pub method: Token,
}

/// Characters with special meaning in a pattern; their presence means the
/// pattern is not literal-only and is not compilable in this slice.
const META: &[char] = &['\\', '^', '$', '.', '|', '?', '*', '+', '(', ')', '[', ']', '{', '}'];

/// The UTF-16 code units of a literal-only `pattern`, or `None` when the
/// pattern uses any regex syntax (so the caller falls back to the interpreter).
pub fn literal_units(pattern: &str) -> Option<Vec<u16>> {
    if pattern.chars().any(|c| META.contains(&c) || c == '/') {
        return None;
    }
    Some(pattern.encode_utf16().collect())
}

/// Compile a literal-only `pattern` (no flags) to a ZIL `matchAt` method.
///
/// Returns `None` if the pattern is not literal-only.
pub fn compile_literal(pattern: &str, natives: StringNatives) -> Option<CompiledLiteral> {
    let units = literal_units(pattern)?;
    let mut module = Module::new();
    let mut code = Vec::new();
    let mut emit = |op: Opcode, operand: Operand| encode(&Instruction::new(op, operand), &mut code);
    let u64_imm = NumKind::U64.to_imm();
    let u32_imm = NumKind::U32.to_imm();
    // Offset of the `const.false; ret` early-exit tail (two 1-byte opcodes).
    const FAIL_TAIL: i32 = 2;

    // Bounds check: pos + len > s.length() -> false.
    emit(Opcode::LoadLocal, Operand::Local(1));
    emit(Opcode::ConstI64, Operand::Imm64(units.len() as i64));
    emit(Opcode::Conv, Operand::Imm32(u64_imm));
    emit(Opcode::AddI32, Operand::None);
    emit(Opcode::LoadLocal, Operand::Local(0));
    emit(Opcode::CallBuiltin, Operand::Imm32(natives.length as i32));
    emit(Opcode::CmpGt, Operand::None);
    emit(Opcode::BranchFalse, Operand::Branch(FAIL_TAIL));
    emit(Opcode::ConstFalse, Operand::None);
    emit(Opcode::Ret, Operand::None);

    // One compare per code unit: s.charCodeAt(pos + i) != unit -> false.
    for (i, unit) in units.iter().enumerate() {
        emit(Opcode::LoadLocal, Operand::Local(0));
        emit(Opcode::LoadLocal, Operand::Local(1));
        if i > 0 {
            emit(Opcode::ConstI64, Operand::Imm64(i as i64));
            emit(Opcode::Conv, Operand::Imm32(u64_imm));
            emit(Opcode::AddI32, Operand::None);
        }
        emit(Opcode::CallBuiltin, Operand::Imm32(natives.char_code_at as i32));
        emit(Opcode::ConstI32, Operand::Imm32(*unit as i32));
        emit(Opcode::Conv, Operand::Imm32(u32_imm));
        emit(Opcode::CmpNe, Operand::None);
        emit(Opcode::BranchFalse, Operand::Branch(FAIL_TAIL));
        emit(Opcode::ConstFalse, Operand::None);
        emit(Opcode::Ret, Operand::None);
    }
    emit(Opcode::ConstTrue, Operand::None);
    emit(Opcode::Ret, Operand::None);

    let name: StringIndex = module.strings.intern("matchAt");
    let local_kinds = [
        JitKind::String.to_byte(),
        JitKind::Num(NumKind::U64).to_byte(),
    ];
    let body_offset = write_body(
        &mut module.code,
        2,
        2,
        true,
        JitKind::Bool.to_byte(),
        &local_kinds,
        &code,
    );
    let rid = module.tables.add_method_def(MethodDef {
        name,
        signature: zun_metadata::BlobIndex(0),
        body_offset,
        is_static: true,
    });
    let _ = rid;
    let method = Token::new(TableKind::MethodDef, 1);
    Some(CompiledLiteral { module, method })
}
