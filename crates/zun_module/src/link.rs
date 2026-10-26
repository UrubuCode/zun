//! Runtime linking: append an ephemeral module's methods to a live module.
//!
//! Used by `new Function(...)`: the source string is compiled into a
//! self-contained ephemeral [`Module`], then its method bodies are appended to
//! the live module's code section and tables. ZIL references metadata by token
//! and strings by heap index (never by byte offset), so the relocation is
//! mechanical: every `MethodDef` token and every string index in the appended
//! IL is rewritten to the matching entry in the live module.
//!
//! Scope (v1): the ephemeral module may define **methods only**. A type,
//! field, type/member/module reference, exception catch type or module
//! initializer in the ephemeral module is rejected with
//! [`LinkError::Unsupported`], because those would need cross-module identity
//! (and a rebuilt type system) that the single-module interpreter does not have.

use crate::{write_body_with_handlers, read_body, ExceptionHandler, Module};
use zun_il::{decode_all, encode, Instruction, Operand};
use zun_metadata::{MethodDef, StringIndex, TableKind, Token};

/// Why an ephemeral module could not be linked into a live one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// The ephemeral module uses a construct the linker cannot relocate.
    Unsupported(String),
    /// The ephemeral module is structurally invalid.
    Malformed(String),
}

impl core::fmt::Display for LinkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LinkError::Unsupported(m) => write!(f, "cannot link module: {m}"),
            LinkError::Malformed(m) => write!(f, "malformed module: {m}"),
        }
    }
}

impl std::error::Error for LinkError {}

/// Where the ephemeral module's methods landed in the linked module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkedMethods {
    /// RID (1-based) the ephemeral module's first method received; its method
    /// `n` (1-based) is `first_rid + n - 1`.
    pub first_rid: u32,
    /// Number of methods appended.
    pub count: u32,
}

impl Module {
    /// Return a copy of `self` with `ext`'s methods appended, plus where they
    /// landed. `self` is left untouched, so already-running code that borrows
    /// it keeps seeing a stable module.
    pub fn link(&self, ext: &Module) -> Result<(Module, LinkedMethods), LinkError> {
        if !ext.tables.type_defs.is_empty()
            || !ext.tables.field_defs.is_empty()
            || !ext.tables.type_refs.is_empty()
            || !ext.tables.member_refs.is_empty()
            || !ext.tables.module_refs.is_empty()
        {
            return Err(LinkError::Unsupported(
                "dynamically compiled code may not declare or reference types or external members"
                    .into(),
            ));
        }
        if ext.module_initializer.is_some() {
            return Err(LinkError::Unsupported("module initializer".into()));
        }
        let mut merged = self.clone();
        let base = self.tables.method_defs.len() as u32;
        let count = ext.tables.method_defs.len() as u32;
        for method in &ext.tables.method_defs {
            let name = merged.strings.intern(ext.strings.get(method.name));
            let signature = merged.blobs.intern(ext.blobs.get(method.signature));
            let body_offset = if method.body_offset == u32::MAX {
                u32::MAX
            } else {
                let body = read_body(&ext.code, method.body_offset)
                    .map_err(|e| LinkError::Malformed(e.to_string()))?;
                if body.handlers.iter().any(|h| h.catch_type.is_some()) {
                    return Err(LinkError::Unsupported("typed catch handler".into()));
                }
                let handlers: Vec<ExceptionHandler> = body.handlers.clone();
                let code = relocate_code(body.code, ext, &mut merged, base, count)?;
                write_body_with_handlers(
                    &mut merged.code,
                    body.param_count,
                    body.local_count,
                    body.returns_value,
                    body.return_kind,
                    body.local_kinds,
                    &code,
                    &handlers,
                )
            };
            merged.tables.add_method_def(MethodDef {
                name,
                signature,
                body_offset,
                is_static: method.is_static,
            });
        }
        Ok((merged, LinkedMethods { first_rid: base + 1, count }))
    }
}

fn relocate_token(token: Token, base: u32, count: u32) -> Result<Token, LinkError> {
    match token.table() {
        // The null token of any table is a sentinel (e.g. the dynamic
        // `call.indirect` marker); it carries no identity to relocate.
        _ if token.rid() == 0 => Ok(token),
        Some(TableKind::MethodDef) if token.rid() <= count => {
            Ok(Token::new(TableKind::MethodDef, base + token.rid()))
        }
        other => Err(LinkError::Unsupported(format!(
            "token {other:?} rid {} cannot be relocated",
            token.rid()
        ))),
    }
}

fn relocate_code(
    code: &[u8],
    ext: &Module,
    merged: &mut Module,
    base: u32,
    count: u32,
) -> Result<Vec<u8>, LinkError> {
    let instructions = decode_all(code).map_err(|e| LinkError::Malformed(e.to_string()))?;
    let mut out = Vec::with_capacity(code.len());
    for instr in instructions {
        let operand = match instr.operand {
            Operand::Token(t) => Operand::Token(relocate_token(t, base, count)?),
            Operand::TokenImm32(t, imm) => Operand::TokenImm32(relocate_token(t, base, count)?, imm),
            Operand::StringRef(index) => {
                let text = ext.strings.get(StringIndex(index));
                Operand::StringRef(merged.strings.intern(text).0)
            }
            other => other,
        };
        // Operand sizes are fixed per opcode, so branch displacements and
        // exception-handler offsets stay valid after re-encoding.
        encode(&Instruction { opcode: instr.opcode, operand }, &mut out);
    }
    debug_assert_eq!(out.len(), code.len());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write_body;
    use zun_il::Opcode;
    use zun_metadata::{BlobIndex, TypeDef};

    fn module_with_method(string: &str) -> Module {
        let mut m = Module::new();
        let s = m.strings.intern(string);
        let mut code = Vec::new();
        encode(&Instruction::new(Opcode::ConstStr, Operand::StringRef(s.0)), &mut code);
        encode(&Instruction::simple(Opcode::Ret), &mut code);
        let body_offset = write_body(&mut m.code, 0, 0, true, 0, &[], &code);
        let name = m.strings.intern("f");
        m.tables.add_method_def(MethodDef { name, signature: BlobIndex(0), body_offset, is_static: false });
        m
    }

    #[test]
    fn link_appends_methods_and_remaps_strings() {
        let mut base = module_with_method("base-only-string-with-different-index");
        base.strings.intern("pad");
        let ext = module_with_method("hello");
        let (merged, placed) = base.link(&ext).unwrap();
        assert_eq!(placed, LinkedMethods { first_rid: 2, count: 1 });
        assert_eq!(merged.tables.method_defs.len(), 2);
        let body = read_body(&merged.code, merged.tables.method_defs[1].body_offset).unwrap();
        let instrs = decode_all(body.code).unwrap();
        let Operand::StringRef(i) = instrs[0].operand else { panic!("string operand") };
        assert_eq!(merged.strings.get(StringIndex(i)), "hello");
        // The source module is untouched.
        assert_eq!(base.tables.method_defs.len(), 1);
    }

    #[test]
    fn link_rejects_types() {
        let mut ext = module_with_method("x");
        ext.tables.add_type_def(TypeDef {
            name: StringIndex(0),
            namespace: StringIndex(0),
            base: Token::null(TableKind::TypeDef),
            field_start: 1,
            field_end: 1,
            method_start: 1,
            method_end: 1,
        });
        assert!(matches!(Module::new().link(&ext), Err(LinkError::Unsupported(_))));
    }
}
