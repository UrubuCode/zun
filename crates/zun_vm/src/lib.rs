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
use zun_module::Module;

pub use jit_hook::JitHook;
pub use verify_module::{verify_module, ModuleVerifyError};

/// Errors from loading or running a module.
#[derive(Debug)]
pub enum VmError {
    /// The module bytes could not be parsed.
    Load(zun_loader::LoadError),
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
    let module = zun_loader::ModuleScope::from_bytes("<memory>", bytes)
        .map_err(VmError::Load)?
        .into_module();
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
    match zun_codegen::Jit::new(&module) {
        Ok(jit) => {
            let hook = JitHook::new(jit, &module);
            run_with(&module, output, Some(&hook)).map_err(VmError::Run)
        }
        Err(_) => run_with(&module, output, None).map_err(VmError::Run),
    }
}

/// Execute the entry module selected by a validated flattened manifest.
///
/// The loader owns all dependency scopes for the lifetime of the call. The
/// current execution engine starts at the entry scope; imported member calls
/// will use this same hosting boundary when multi-module dispatch is enabled.
pub fn execute_manifest(
    loader: &zun_loader::ModuleLoader,
    manifest: &zun_loader::ModuleManifest,
    output: &mut dyn Output,
) -> Result<Value, VmError> {
    verify_manifest(loader, manifest)?;
    let scope = loader.get(&manifest.entry).ok_or_else(|| {
        VmError::Load(zun_loader::LoadError::UnresolvedModule(
            manifest.entry.clone(),
        ))
    })?;
    let module = scope.module();
    verify_module(module).map_err(VmError::Verify)?;
    match zun_codegen::Jit::new(module) {
        Ok(jit) => {
            let hook = JitHook::new(jit, module);
            run_with(module, output, Some(&hook)).map_err(VmError::Run)
        }
        Err(_) => run_with(module, output, None).map_err(VmError::Run),
    }
}

/// Validate every module selected by a manifest before any code executes.
pub fn verify_manifest(
    loader: &zun_loader::ModuleLoader,
    manifest: &zun_loader::ModuleManifest,
) -> Result<(), VmError> {
    loader
        .validate_member_refs(manifest)
        .map_err(VmError::Load)?;
    for name in manifest.load_order(loader).map_err(VmError::Load)? {
        let scope = loader
            .get(name)
            .ok_or_else(|| VmError::Load(zun_loader::LoadError::UnresolvedModule(name.into())))?;
        verify_module(scope.module()).map_err(VmError::Verify)?;
    }
    Ok(())
}

/// Invoke an exported method from a loaded module by source-level name.
///
/// This is the host-side cross-module call path. Compiler-emitted `MemberRef`
/// calls will be routed through the same resolution contract once module-aware
/// frames are enabled in the interpreter.
pub fn invoke_export(
    loader: &zun_loader::ModuleLoader,
    manifest: &zun_loader::ModuleManifest,
    module: &str,
    export: &str,
    args: Vec<Value>,
    output: &mut dyn Output,
) -> Result<Value, VmError> {
    verify_manifest(loader, manifest)?;
    let (scope, token) = loader
        .resolve_export(module, export)
        .map_err(VmError::Load)?;
    let zun_loader::Export::Method(token) = token else {
        return Err(VmError::Load(zun_loader::LoadError::UnresolvedModule(
            format!("{module}::{export} is not a method"),
        )));
    };
    if let Ok(jit) = zun_codegen::Jit::new(scope.module()) {
        let hook = JitHook::with_threshold(jit, scope.module(), 0);
        if let Some(result) = zun_interp::CallHook::try_call(&hook, token.rid(), &args) {
            return Ok(result);
        }
    }
    zun_interp::run_method(scope.module(), token.rid(), args, output).map_err(VmError::Run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executes_manifest_entry_scope() {
        let module = zun_compiler::compile("function main(): int { return 23; }").unwrap();
        let scope = zun_loader::ModuleScope::from_bytes("app", &module.write()).unwrap();
        let mut loader = zun_loader::ModuleLoader::new();
        loader.insert(scope).unwrap();
        let manifest = zun_loader::ModuleManifest::new("app", Vec::<String>::new());
        let mut output = zun_interp::CaptureOutput::default();
        assert!(matches!(
            execute_manifest(&loader, &manifest, &mut output).unwrap(),
            zun_interp::value::Value::Int(23, zun_il::NumKind::I32)
        ));
    }

    #[test]
    fn verifies_all_manifest_scopes_before_execution() {
        let dependency = zun_module::Module::new();
        let entry = zun_compiler::compile("function main(): int { return 23; }").unwrap();
        let dependency = zun_loader::ModuleScope::from_bytes("core", &dependency.write()).unwrap();
        let entry = zun_loader::ModuleScope::from_bytes("app", &entry.write()).unwrap();
        let mut loader = zun_loader::ModuleLoader::new();
        loader.insert(dependency).unwrap();
        loader.insert(entry).unwrap();
        let manifest = zun_loader::ModuleManifest::new("app", vec!["core".into()]);
        verify_manifest(&loader, &manifest).expect("all modules verify");
    }

    #[test]
    fn invokes_exported_method_from_loaded_module() {
        let module = zun_compiler::compile(
            "function answer(value: int): int { return value + 1; } function main(): int { return 0; }",
        )
        .unwrap();
        let scope = zun_loader::ModuleScope::from_bytes("core", &module.write()).unwrap();
        let mut loader = zun_loader::ModuleLoader::new();
        loader.insert(scope).unwrap();
        let manifest = zun_loader::ModuleManifest::new("core", Vec::<String>::new());
        let mut output = zun_interp::CaptureOutput::default();
        let result = invoke_export(
            &loader,
            &manifest,
            "core",
            "answer",
            vec![zun_interp::value::Value::Int(41, zun_il::NumKind::I32)],
            &mut output,
        )
        .unwrap();
        assert!(matches!(
            result,
            zun_interp::value::Value::Int(42, zun_il::NumKind::I32)
        ));
    }
}
