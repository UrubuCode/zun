//! `new Function(...)` / `Function(...)`: runtime code generation from strings.
//!
//! The interpreter never links a compiler (PLAN section 2). It instead calls an
//! injected [`EvalHost`], which turns `(parameter names, body)` into an
//! ephemeral [`Module`]. The interpreter appends that module's methods to its
//! live module ([`Module::link`]: token and string rewrite), has the host
//! re-verify the appended methods, and returns a callable function value that
//! runs on Tier 0. With no host injected (AOT, `--jitless`-style builds) the
//! intrinsic throws the ECMAScript "code generation from strings disallowed"
//! error, exactly like V8 under a CSP.

use super::{Interp, RunError, Value};
use std::rc::Rc;
use zun_module::Module;

/// A failure reported by an [`EvalHost`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalError(pub String);

impl core::fmt::Display for EvalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for EvalError {}

/// The message thrown when `new Function` runs without an [`EvalHost`].
pub const CODEGEN_DISALLOWED: &str = "EvalError: code generation from strings disallowed";

/// Compiles source strings on behalf of the interpreter.
///
/// Implemented by a crate that links the compiler (the dev/JIT driver); the
/// interpreter itself only sees this trait.
pub trait EvalHost {
    /// Compile `function(params...) { body }` into a self-contained module and
    /// return it with the 1-based method index of the function itself (it takes
    /// the parameters in order; nested closures may add further methods).
    /// The module must be free of types and external references (see
    /// [`Module::link`]).
    fn compile_function(&self, params: &[String], body: &str) -> Result<(Module, u32), EvalError>;

    /// Verify the methods `first_rid..first_rid + count` of the freshly linked
    /// `module` before they can run.
    fn verify_linked(&self, module: &Module, first_rid: u32, count: u32) -> Result<(), EvalError>;
}

impl Interp<'_> {
    /// `functionConstruct(argsArray)`: all elements are strings; the last is the
    /// body, the preceding ones are parameter names (each may itself be a
    /// comma-separated list, as in ECMAScript).
    pub(super) fn function_construct(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let Some(host) = self.eval_host else {
            return Err(RunError::Thrown(Value::Str(Rc::from(CODEGEN_DISALLOWED))));
        };
        let array = match args.first() {
            Some(Value::Object(array)) => *array,
            _ => return Err(RunError::BadToken),
        };
        let mut strings = Vec::new();
        for value in self.read_array_values(array)? {
            match value {
                Value::Str(s) => strings.push(s.to_string()),
                other => strings.push(other.display()),
            }
        }
        let body = strings.pop().unwrap_or_default();
        let params: Vec<String> = strings
            .iter()
            .flat_map(|p| p.split(','))
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();

        let cache_key = format!("{}\u{0}{}", params.join(","), body);
        if let Some(handle) = self.eval_cache.get(&cache_key) {
            return Ok(Value::Function(*handle));
        }

        let thrown = |message: String| RunError::Thrown(Value::Str(Rc::from(message.as_str())));
        let (ephemeral, entry) = host
            .compile_function(&params, &body)
            .map_err(|e| thrown(format!("SyntaxError: {e}")))?;
        let (linked, placed) = self
            .module
            .link(&ephemeral)
            .map_err(|e| thrown(format!("EvalError: {e}")))?;
        if entry == 0 || entry > placed.count {
            return Err(thrown("EvalError: compiled function is empty".into()));
        }
        host.verify_linked(&linked, placed.first_rid, placed.count)
            .map_err(|e| thrown(format!("EvalError: {e}")))?;

        // Publish the grown module. The previous module stays alive (leaked) so
        // every borrow of it held by running frames stays valid; the leak is
        // bounded by the number of distinct sources (see `eval_cache`).
        let linked: &'static Module = Box::leak(Box::new(linked));
        self.module = linked;
        let handle = zun_core::FunctionHandle::new(placed.first_rid + entry - 1, None);
        self.eval_cache.insert(cache_key, handle);
        Ok(Value::Function(handle))
    }
}
