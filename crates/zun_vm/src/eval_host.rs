//! Compiler-backed [`EvalHost`] for `new Function(...)` and indirect `eval`
//! (feature `eval`).
//!
//! This is the one place the runtime side links the compiler. It exists only in
//! driver builds: the AOT entry points never inject it, so a trimmed or AOT
//! program throws "code generation from strings disallowed" instead.

use zun_interp::{EvalError, EvalHost, Generated};
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

/// Compile `source` and collect its diagnostics into one [`EvalError`].
fn compile_source(source: &str) -> Result<Module, EvalError> {
    zun_compiler::compile_without_prelude(source, &[]).map_err(|diagnostics| {
        EvalError(
            diagnostics
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("; "),
        )
    })
}

/// The method a compiled source was placed at, and whether it declares a
/// receiver (its body mentions `this`, which the compiler marks with an
/// `@this` name suffix and a leading hidden parameter).
fn compile_entry(source: &str) -> Result<(Module, u32, bool), EvalError> {
    let module = compile_source(source)?;
    let with_receiver = format!("{EVAL_FN}@this");
    let (entry, takes_this) = module
        .tables
        .method_defs
        .iter()
        .enumerate()
        .find_map(|(index, method)| {
            let name = module.strings.get(method.name);
            if name == EVAL_FN {
                Some((index, false))
            } else if name == with_receiver {
                Some((index, true))
            } else {
                None
            }
        })
        .ok_or_else(|| EvalError("compiled module lost the function".into()))?;
    Ok((module, entry as u32 + 1, takes_this))
}

impl EvalHost for CompilerEvalHost {
    fn compile_function(&self, params: &[String], body: &str) -> Result<Generated, EvalError> {
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
        let (module, entry, takes_this) = compile_entry(&source)?;
        if !takes_this {
            return Ok(Generated { module, entry, factory: false });
        }
        // The body mentions `this`. A `Function` body is sloppy code: called
        // with a receiver `this` is that receiver, called without one it is
        // the global object. Only the parameterless form can be made exact: a
        // bare call of a receiver-taking function with arguments puts its first
        // argument in the receiver slot, so a body with both would answer
        // wrongly instead of failing. It is refused here.
        if !params.is_empty() {
            return Err(EvalError(
                "unsupported Function body: `this` together with parameters".into(),
            ));
        }
        // The function is built as a function *expression*, the shape every
        // call path already hands its receiver to, returned by a factory the
        // interpreter calls; the body moves one level in so a missing
        // receiver can be replaced by the global object.
        let source = format!(
            "function {EVAL_FN}(): any {{\n\
             return function (this: any): any {{\n\
             return (function (this: any): any {{\n{body}\n}}).call(this ?? globalThis);\n\
             }};\n}}"
        );
        let (module, entry, _) = compile_entry(&source)?;
        Ok(Generated { module, entry, factory: true })
    }

    fn compile_eval(&self, source: &str) -> Result<Generated, EvalError> {
        let body = zun_compiler::eval_function_body(source).map_err(EvalError)?;
        // No sloppy-`this` wrapper here: the interpreter always calls global
        // code with the global object as its receiver.
        let (module, entry, _) =
            compile_entry(&format!("function {EVAL_FN}(): any {{\n{body}\n}}"))?;
        Ok(Generated { module, entry, factory: false })
    }

    fn verify_linked(&self, module: &Module, first_rid: u32, count: u32) -> Result<(), EvalError> {
        crate::verify_module::verify_method_range(module, first_rid, count)
            .map_err(|e| EvalError(e.to_string()))
    }
}
