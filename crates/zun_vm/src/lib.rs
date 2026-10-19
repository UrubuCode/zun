//! The Zun virtual machine.
//!
//! The VM is what turns an inert `.zm` into a running program: it loads the
//! module and executes its entry point. In phase 1 execution is the Tier 0
//! interpreter only; the Cranelift JIT (Tier 1) and GC plug in here later
//! behind the same entry points, without changing this surface.

use zun_interp::value::Value;
use zun_interp::{run, Output, RunError, StdoutOutput};
use zun_module::{Module, ReadError};

/// Errors from loading or running a module.
#[derive(Debug)]
pub enum VmError {
    /// The module bytes could not be parsed.
    Load(ReadError),
    /// Execution failed.
    Run(RunError),
}

impl core::fmt::Display for VmError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            VmError::Load(e) => write!(f, "failed to load module: {e}"),
            VmError::Run(e) => write!(f, "runtime error: {e}"),
        }
    }
}

impl std::error::Error for VmError {}

/// Load a `.zm` module from its bytes.
pub fn load(bytes: &[u8]) -> Result<Module, VmError> {
    Module::read(bytes).map_err(VmError::Load)
}

/// Load and run a `.zm` module, printing to stdout.
pub fn execute(bytes: &[u8]) -> Result<Value, VmError> {
    let module = load(bytes)?;
    let mut out = StdoutOutput;
    run(&module, &mut out).map_err(VmError::Run)
}

/// Load and run a `.zm` module, directing output to a custom sink.
pub fn execute_with(bytes: &[u8], output: &mut dyn Output) -> Result<Value, VmError> {
    let module = load(bytes)?;
    run(&module, output).map_err(VmError::Run)
}
