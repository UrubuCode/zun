//! Whole-module verification.
//!
//! Runs the `zun_verify` body checker over every method in a module, resolving
//! call arities from the callees' body frames (until signatures exist, the
//! frame carries `param_count` and the return shape). A module that passes is
//! safe for the interpreter and, later, the JIT to consume without guards.

use zun_il::{Opcode, Operand};
use zun_loader::{ModuleLoader, ModuleScope};
use zun_module::{read_body, Module};
use zun_verify::{verify, MethodInfo};

/// A verification failure tied to the method it occurred in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleVerifyError {
    /// 1-based RID of the offending method.
    pub method_rid: u32,
    /// The method's name, for the message.
    pub method_name: String,
    /// The underlying body-level error.
    pub error: zun_verify::VerifyError,
}

impl core::fmt::Display for ModuleVerifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "in method `{}` (#{}) at offset {}: {:?}",
            self.method_name, self.method_rid, self.error.offset, self.error.kind
        )
    }
}

/// Verify every method body in `module`.
pub fn verify_module(module: &Module) -> Result<(), ModuleVerifyError> {
    verify_module_with(module, |opcode, operand| {
        resolve_call(module, opcode, operand)
    })
}

/// Verify a loaded scope, resolving external method references through its loader.
pub fn verify_loaded_module(
    scope: &ModuleScope,
    loader: &ModuleLoader,
) -> Result<(), ModuleVerifyError> {
    let module = scope.module();
    verify_module_with(module, |opcode, operand| {
        resolve_loaded_call(scope, loader, opcode, operand)
    })
}

fn verify_module_with(
    module: &Module,
    resolve: impl Fn(Opcode, &Operand) -> Option<(u8, u8)>,
) -> Result<(), ModuleVerifyError> {
    for (i, method) in module.tables.method_defs.iter().enumerate() {
        let rid = i as u32 + 1;
        if method.body_offset == u32::MAX {
            continue; // abstract / external: no body to verify
        }
        let body = read_body(&module.code, method.body_offset).map_err(|_| ModuleVerifyError {
            method_rid: rid,
            method_name: module.strings.get(method.name).to_string(),
            error: zun_verify::VerifyError {
                offset: 0,
                kind: zun_verify::ErrorKind::BadEncoding,
            },
        })?;

        let info = MethodInfo {
            local_count: body.local_count,
            returns_value: body.returns_value,
        };

        verify(body.code, info, &resolve).map_err(|error| ModuleVerifyError {
            method_rid: rid,
            method_name: module.strings.get(method.name).to_string(),
            error,
        })?;
    }
    Ok(())
}

fn resolve_loaded_call(
    requester: &ModuleScope,
    loader: &ModuleLoader,
    opcode: Opcode,
    operand: &Operand,
) -> Option<(u8, u8)> {
    if !matches!(opcode, Opcode::Call | Opcode::CallVirt | Opcode::CallIndirect) {
        return resolve_call(requester.module(), opcode, operand);
    }
    let token = match operand {
        Operand::Token(token) => *token,
        _ => return None,
    };
    if token.table() != Some(zun_metadata::TableKind::MemberRef) {
        return resolve_call(requester.module(), opcode, operand);
    }
    let (scope, method) = loader.resolve_method_ref(requester, token).ok()?;
    let method = scope
        .module()
        .tables
        .method_defs
        .get(method.rid() as usize - 1)?;
    let body = read_body(&scope.module().code, method.body_offset).ok()?;
    Some((body.param_count as u8, 1))
}

/// Resolve a call-family opcode's `(pops, pushes)` from module metadata.
fn resolve_call(module: &Module, opcode: Opcode, operand: &Operand) -> Option<(u8, u8)> {
    match opcode {
        // A direct or virtual call pops the (statically named) callee's
        // parameters and pushes its single result value. For callvirt the
        // static method's arity matches every override's, so it is exact.
        Opcode::Call | Opcode::CallVirt | Opcode::CallIndirect => {
            let token = match operand {
                Operand::Token(t) => *t,
                _ => return None,
            };
            if opcode == Opcode::CallIndirect && token.rid() == 0 {
                return Some((2, 1));
            }
            let callee = module.tables.method_defs.get(token.rid() as usize - 1)?;
            if callee.body_offset == u32::MAX {
                return None;
            }
            let body = read_body(&module.code, callee.body_offset).ok()?;
            let handle = u8::from(opcode == Opcode::CallIndirect);
            Some((body.param_count as u8 + handle, 1))
        }
        // newobj pops one initializer per field and pushes the new reference.
        Opcode::NewObj => {
            let token = match operand {
                Operand::Token(t) => *t,
                _ => return None,
            };
            let ty = module.tables.type_defs.get(token.rid() as usize - 1)?;
            let field_count = (ty.field_end - ty.field_start) as u8;
            Some((field_count, 1))
        }
        // An intrinsic pops its parameters (by id) and pushes one result value
        // (void intrinsics push a void value, discarded by the always-pop rule).
        Opcode::CallBuiltin => {
            let id = match operand {
                Operand::Imm32(v) => *v as u32,
                _ => return None,
            };
            Some((zun_natives::param_count(id) as u8, 1))
        }
        _ => None,
    }
}
