//! The Zun virtual machine.
//!
//! The VM is what turns an inert `.zm` into a running program: it loads the
//! module and executes its entry point. In phase 1 execution is the Tier 0
//! interpreter only; the Cranelift JIT (Tier 1) and GC plug in here later
//! behind the same entry points, without changing this surface.

mod jit_hook;
mod verify_module;

use zun_interp::value::Value;
use zun_interp::{run_with, Output, RunError, StdoutOutput};
use zun_module::{Module, ReadError};

pub use jit_hook::JitHook;
pub use verify_module::{verify_module, ModuleVerifyError};

/// Errors from loading or running a module.
#[derive(Debug)]
pub enum VmError {
    /// The module bytes could not be parsed.
    Load(ReadError),
    /// The module failed verification.
    Verify(ModuleVerifyError),
    /// Execution failed.
    Run(RunError),
}

impl core::fmt::Display for VmError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            VmError::Load(e) => write!(f, "failed to load module: {e}"),
            VmError::Verify(e) => write!(f, "verification failed: {e}"),
            VmError::Run(e) => write!(f, "runtime error: {e}"),
        }
    }
}

impl std::error::Error for VmError {}

/// Load a `.zm` module from its bytes and verify every method body.
pub fn load(bytes: &[u8]) -> Result<Module, VmError> {
    let module = Module::read(bytes).map_err(VmError::Load)?;
    verify_module(&module).map_err(VmError::Verify)?;
    Ok(module)
}

/// Load and run a `.zm` module, printing to stdout.
pub fn execute(bytes: &[u8]) -> Result<Value, VmError> {
    let mut out = StdoutOutput;
    execute_with(bytes, &mut out)
}

/// Load and run a `.zm` module, directing output to a custom sink.
///
/// Tiering: the module's JIT-eligible methods are compiled up front, and the
/// interpreter routes calls to them into native code via a [`JitHook`]. If the
/// JIT cannot be built, execution falls back to the interpreter alone.
pub fn execute_with(bytes: &[u8], output: &mut dyn Output) -> Result<Value, VmError> {
    let module = load(bytes)?;
    match zun_codegen::Jit::compile(&module) {
        Ok(jit) => {
            let hook = JitHook::new(jit);
            run_with(&module, output, Some(&hook)).map_err(VmError::Run)
        }
        Err(_) => run_with(&module, output, None).map_err(VmError::Run),
    }
}
