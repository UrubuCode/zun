//! Whole-module verification.
//!
//! Runs the `zun_verify` body checker over every method in a module, resolving
//! call arities from the callees' body frames (until signatures exist, the
//! frame carries `param_count` and the return shape). A module that passes is
//! safe for the interpreter and, later, the JIT to consume without guards.

use zun_il::{Opcode, Operand};
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

        let resolve = |opcode: Opcode, operand: &Operand| resolve_call(module, opcode, operand);

        verify(body.code, info, &resolve).map_err(|error| ModuleVerifyError {
            method_rid: rid,
            method_name: module.strings.get(method.name).to_string(),
            error,
        })?;
    }
    Ok(())
}

/// Resolve a call-family opcode's `(pops, pushes)` from module metadata.
fn resolve_call(module: &Module, opcode: Opcode, operand: &Operand) -> Option<(u8, u8)> {
    match opcode {
        // A direct/virtual call pops the callee's parameters and pushes its
        // single result value (void results still occupy one slot in phase 1).
        Opcode::Call | Opcode::CallVirt => {
            let token = match operand {
                Operand::Token(t) => *t,
                _ => return None,
            };
            let callee = module.tables.method_defs.get(token.rid() as usize - 1)?;
            if callee.body_offset == u32::MAX {
                return None;
            }
            let body = read_body(&module.code, callee.body_offset).ok()?;
            Some((body.param_count as u8, 1))
        }
        // The phase-1 builtins all take one argument and push one result.
        Opcode::CallBuiltin => Some((1, 1)),
        _ => None,
    }
}
