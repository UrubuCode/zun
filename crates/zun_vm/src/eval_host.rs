//! Compiler-backed [`EvalHost`] for `new Function(...)` (feature `eval`).
//!
//! This is the one place the runtime side links the compiler. It exists only in
//! driver builds: the AOT entry points never inject it, so a trimmed or AOT
//! program throws "code generation from strings disallowed" instead.

use zun_interp::{EvalError, EvalHost};
use zun_module::Module;

/// Name given to the synthesized function; the only method the host promises
/// to place at a known position.
const EVAL_FN: &str = "__zun_eval";

/// Compiles `function(params) { body }` with the project's own compiler.
#[derive(Debug, Default, Clone, Copy)]
pub struct CompilerEvalHost;

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

impl EvalHost for CompilerEvalHost {
    fn compile_function(&self, params: &[String], body: &str) -> Result<(Module, u32), EvalError> {
        for param in params {
            if !is_identifier(param) {
                return Err(EvalError(format!("invalid parameter name `{param}`")));
            }
        }
        // v1 typing: untyped parameters are `number`, the result is dynamic.
        let list = params
            .iter()
            .map(|p| format!("{p}: number"))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("function {EVAL_FN}({list}): any {{\n{body}\n}}");
        let module = zun_compiler::compile_without_prelude(&source, &[]).map_err(|diagnostics| {
            EvalError(
                diagnostics
                    .iter()
                    .map(|d| d.message.clone())
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        let entry = module
            .tables
            .method_defs
            .iter()
            .position(|m| module.strings.get(m.name) == EVAL_FN)
            .ok_or_else(|| EvalError("compiled module lost the function".into()))?;
        Ok((module, entry as u32 + 1))
    }

    fn verify_linked(&self, module: &Module, first_rid: u32, count: u32) -> Result<(), EvalError> {
        crate::verify_module::verify_method_range(module, first_rid, count)
            .map_err(|e| EvalError(e.to_string()))
    }
}
