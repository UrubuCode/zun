//! The Zun virtual machine.
//!
//! The VM is what turns an inert `.zm` into a running program: it loads the
//! module and executes its entry point. In phase 1 execution is the Tier 0
//! interpreter only; the Cranelift JIT (Tier 1) and GC plug in here later
//! behind the same entry points, without changing this surface.

mod ffi;
mod extension;
mod jit_hook;
mod native_hook;
mod promise;
mod verify_module;

use zun_interp::value::Value;
use zun_interp::{run_with, CallHook, Output, RunError, StdoutOutput};
use zun_module::Module;

/// AOT runtime startup hook exposed by the static runtime library.
#[no_mangle]
pub extern "C" fn zun_rt_init() {}

/// AOT runtime shutdown hook paired with [`zun_rt_init`].
#[no_mangle]
pub extern "C" fn zun_rt_shutdown() {}

/// Execute an embedded module through the Tier 0 fallback and return a scalar
/// integer/boolean result for a generated AOT launcher. A negative result
/// reports load, verification, execution, or unsupported-result failure.
#[no_mangle]
pub unsafe extern "C" fn zun_rt_run_embedded_i64(ptr: *const u8, len: u64) -> i64 {
    if ptr.is_null() {
        return -1;
    }
    let Ok(len) = usize::try_from(len) else {
        return -1;
    };
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    match execute(bytes) {
        Ok(Value::Int(value, _)) => value,
        Ok(Value::Bool(value)) => i64::from(value),
        _ => -1,
    }
}

/// Unsigned counterpart to [`zun_rt_run_embedded_i64`] for AOT entrypoints
/// whose declared result is `u8`/`u16`/`u32`/`u64`.
#[no_mangle]
pub unsafe extern "C" fn zun_rt_run_embedded_u64(ptr: *const u8, len: u64) -> u64 {
    if ptr.is_null() {
        return u64::MAX;
    }
    let Ok(len) = usize::try_from(len) else {
        return u64::MAX;
    };
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    match execute(bytes) {
        Ok(Value::Int(value, kind))
            if kind.int_parts().is_some_and(|(_, signed)| !signed) =>
        {
            value as u64
        }
        _ => u64::MAX,
    }
}

pub use ffi::{
    library_name_variants, FfiError, FfiScalar, FfiSignature, FfiStringArgument, FfiStringLength,
    FfiStringReturn, NativeLibrary,
    NativeLibraryResolver,
};
pub use extension::{ExtensionResolver, LoadedExtension};
pub use jit_hook::{JitHook, OsrRequest, OsrSnapshot};
pub use native_hook::NativeCallHook;
pub use promise::{
    AsyncStateMachine, GeneratorIteratorResult, GeneratorStateMachine, GeneratorStep, MicrotaskQueue, Poll, Promise,
    PromiseAwaiter, PromiseState, Scheduler,
};
pub use verify_module::{verify_loaded_module, verify_module, ModuleVerifyError};

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

/// Load and run a module with explicit scalar native bindings while retaining
/// the normal JIT tiering hook. Native bindings are consulted only for
/// registered external `MemberRef` calls.
pub fn execute_with_native(
    bytes: &[u8],
    output: &mut dyn Output,
    native: &NativeCallHook,
) -> Result<Value, VmError> {
    let module = load(bytes)?;
    let jit = zun_codegen::Jit::new(&module).ok().map(|jit| JitHook::new(jit, &module));
    let hook = CombinedCallHook { jit, native };
    run_with(&module, output, Some(&hook)).map_err(VmError::Run)
}

struct CombinedCallHook<'a> {
    jit: Option<JitHook<'a>>,
    native: &'a NativeCallHook,
}

impl CallHook for CombinedCallHook<'_> {
    fn try_call(&self, method_rid: u32, args: &[Value]) -> Option<Value> {
        self.jit.as_ref()?.try_call(method_rid, args)
    }

    fn try_call_external(
        &self,
        module: &str,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, RunError>> {
        self.native.try_call_external(module, name, args)
    }

    fn append_roots(&self, roots: &mut Vec<zun_gc::Obj>) {
        if let Some(jit) = &self.jit {
            jit.append_roots(roots);
        }
    }

    fn rewrite_managed_roots(&self, relocation: &zun_gc::RelocationMap) {
        if let Some(jit) = &self.jit {
            jit.rewrite_managed_roots(relocation);
        }
    }

    fn on_backedge(&self, method_rid: u32, target_pc: usize) {
        if let Some(jit) = &self.jit {
            jit.on_backedge(method_rid, target_pc);
        }
    }

    fn on_backedge_state(
        &self,
        method_rid: u32,
        target_pc: usize,
        pc: usize,
        stack: &[Value],
        locals: &[Value],
    ) {
        if let Some(jit) = &self.jit {
            jit.on_backedge_state(method_rid, target_pc, pc, stack, locals);
        }
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
    let entry = module
        .entry_point
        .ok_or(RunError::NoEntryPoint)
        .map_err(VmError::Run)?;
    zun_interp::run_method_loaded(scope, loader, entry.rid(), Vec::new(), output)
        .map_err(VmError::Run)
}

/// Execute a manifest after validating the prelude identity used to compile
/// it. This is the cache-safe entry point for packaged modules; callers that
/// intentionally support legacy manifests may continue using
/// [`execute_manifest`].
pub fn execute_manifest_with_prelude(
    loader: &zun_loader::ModuleLoader,
    manifest: &zun_loader::ModuleManifest,
    expected_prelude: u64,
    output: &mut dyn Output,
) -> Result<Value, VmError> {
    manifest
        .validate_prelude_fingerprint(expected_prelude)
        .map_err(VmError::Load)?;
    execute_manifest(loader, manifest, output)
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
        verify_loaded_module(scope, loader).map_err(VmError::Verify)?;
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
    fn embedded_runtime_hook_returns_scalar_tier0_results() {
        let module = zun_compiler::compile("function main(): int { return 23; }").unwrap();
        let bytes = module.write();
        let result = unsafe { zun_rt_run_embedded_i64(bytes.as_ptr(), bytes.len() as u64) };
        assert_eq!(result, 23);
        assert_eq!(unsafe { zun_rt_run_embedded_i64(std::ptr::null(), 0) }, -1);
    }

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
    fn cache_safe_manifest_execution_rejects_a_stale_prelude() {
        let manifest = zun_loader::ModuleManifest::new("app", Vec::<String>::new())
            .with_prelude_fingerprint(1);
        let loader = zun_loader::ModuleLoader::new();
        let mut output = zun_interp::CaptureOutput::default();
        let error = execute_manifest_with_prelude(&loader, &manifest, 2, &mut output)
            .expect_err("stale prelude must fail before execution");
        assert!(matches!(error, VmError::Load(zun_loader::LoadError::InvalidManifest(detail)) if detail.contains("fingerprint")));
    }

    #[test]
    fn native_execution_keeps_the_jit_hook_when_no_native_call_is_used() {
        let bytes = zun_compiler::compile("function main(): int { return 31; }").unwrap().write();
        let native = NativeCallHook::new();
        let mut output = zun_interp::CaptureOutput::default();
        assert!(matches!(
            execute_with_native(&bytes, &mut output, &native).unwrap(),
            zun_interp::value::Value::Int(31, zun_il::NumKind::I32)
        ));
    }

    #[test]
    fn collection_size_builtins_match_interpreter_and_jit() {
        let source = "import { mapNew, mapSet, mapSize, setNew, setAdd, setSize } from \"zun:collections\"; function map_size(map: Map): u64 { return mapSize(map); } function set_size(set: Set): u64 { return setSize(set); } function main(): u64 { let map = mapNew(); mapSet(map, 1.0, 2.0); let set = setNew(); setAdd(set, 3.0); return map_size(map) + set_size(set); }";
        let bytes = zun_compiler::compile(source).unwrap().write();
        let module = load(&bytes).expect("load collection-size module");
        let map_size_rid = module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == "map_size")
            .map(|index| index as u32 + 1)
            .expect("map_size method");
        let set_size_rid = module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == "set_size")
            .map(|index| index as u32 + 1)
            .expect("set_size method");
        let jit = zun_codegen::Jit::new(&module).expect("compile collection-size methods");
        assert!(jit.is_compiled(map_size_rid));
        assert!(jit.is_compiled(set_size_rid));
        let mut interpreter_output = zun_interp::CaptureOutput::default();
        let interpreter = zun_interp::run(&module, &mut interpreter_output).expect("interpreter run");
        let mut jit_output = zun_interp::CaptureOutput::default();
        let jit = execute_with(&bytes, &mut jit_output).expect("JIT run");
        assert!(matches!(interpreter, Value::Int(2, zun_il::NumKind::U64)));
        assert!(matches!(jit, Value::Int(2, zun_il::NumKind::U64)));
    }

    #[test]
    fn jit_allocation_safepoint_preserves_interpreter_jit_equivalence() {
        let source = "class Box { value: int; } function allocate(): int { let index: int = 0; while (index < 300) { let item = new Box(0); item.value = index; index = index + 1; } return index; } function main(): int { return allocate(); }";
        let bytes = zun_compiler::compile(source).unwrap().write();
        let module = load(&bytes).expect("load allocation module");

        let mut interpreter_output = zun_interp::CaptureOutput::default();
        let interpreted = zun_interp::run(&module, &mut interpreter_output)
            .expect("interpreter allocation loop");
        let mut jit_output = zun_interp::CaptureOutput::default();
        let jitted = execute_with(&bytes, &mut jit_output).expect("JIT allocation loop");

        assert!(matches!(interpreted, Value::Int(300, zun_il::NumKind::I32)));
        assert!(matches!(jitted, Value::Int(300, zun_il::NumKind::I32)));
    }

    #[test]
    fn jit_allocation_safepoint_keeps_live_object_local() {
        let source = "class Box { value: int; } function allocate(): int { let root = new Box(7); let index: int = 0; while (index < 300) { let temporary = new Box(index); index = index + 1; } return root.value; } function main(): int { return allocate(); }";
        let bytes = zun_compiler::compile(source).unwrap().write();
        let module = load(&bytes).expect("load live-root module");

        let mut interpreter_output = zun_interp::CaptureOutput::default();
        let interpreted = zun_interp::run(&module, &mut interpreter_output)
            .expect("interpreter live-root loop");
        let mut jit_output = zun_interp::CaptureOutput::default();
        let jitted = execute_with(&bytes, &mut jit_output).expect("JIT live-root loop");

        assert!(matches!(interpreted, Value::Int(7, zun_il::NumKind::I32)));
        assert!(matches!(jitted, Value::Int(7, zun_il::NumKind::I32)));
    }

    #[test]
    fn jit_allocation_safepoint_keeps_interpreter_argument_rooted() {
        let source = "class Box { value: int; } function consume(root: Box): int { let index: int = 0; while (index < 300) { let temporary = new Box(index); index = index + 1; } return root.value; } function main(): int { let root = new Box(9); return consume(root); }";
        let bytes = zun_compiler::compile(source).unwrap().write();
        let module = load(&bytes).expect("load interpreter-root module");

        let mut interpreter_output = zun_interp::CaptureOutput::default();
        let interpreted = zun_interp::run(&module, &mut interpreter_output)
            .expect("interpreter argument-root loop");
        let mut jit_output = zun_interp::CaptureOutput::default();
        let jitted = execute_with(&bytes, &mut jit_output).expect("JIT argument-root loop");

        assert!(matches!(interpreted, Value::Int(9, zun_il::NumKind::I32)));
        assert!(matches!(jitted, Value::Int(9, zun_il::NumKind::I32)));
    }

    #[test]
    fn jit_managed_object_return_matches_interpreter() {
        let source = "class Box { value: int; } function make(): Box { return new Box(42); } function main(): int { let item = make(); return item.value; }";
        let bytes = zun_compiler::compile(source).unwrap().write();
        let module = load(&bytes).expect("load managed-return module");

        let mut interpreter_output = zun_interp::CaptureOutput::default();
        let interpreted = zun_interp::run(&module, &mut interpreter_output)
            .expect("interpreter managed return");
        let mut jit_output = zun_interp::CaptureOutput::default();
        let jitted = execute_with(&bytes, &mut jit_output).expect("JIT managed return");

        assert!(matches!(interpreted, Value::Int(42, zun_il::NumKind::I32)));
        assert!(matches!(jitted, Value::Int(42, zun_il::NumKind::I32)));
    }

    #[test]
    fn jit_managed_reference_field_matches_interpreter() {
        let source = "class Leaf { value: int; } class Node { child: Leaf; value: int; } function make(): Node { let leaf = new Leaf(7); return new Node(leaf, 0); } function main(): int { let root = make(); return root.child.value; }";
        let bytes = zun_compiler::compile(source).unwrap().write();
        let module = load(&bytes).expect("load managed-reference module");

        let mut interpreter_output = zun_interp::CaptureOutput::default();
        let interpreted = zun_interp::run(&module, &mut interpreter_output)
            .expect("interpreter managed reference");
        let mut jit_output = zun_interp::CaptureOutput::default();
        let jitted = execute_with(&bytes, &mut jit_output).expect("JIT managed reference");

        assert!(matches!(interpreted, Value::Int(7, zun_il::NumKind::I32)));
        assert!(matches!(jitted, Value::Int(7, zun_il::NumKind::I32)));
    }

    #[cfg(windows)]
    #[test]
    fn native_execution_routes_external_member_ref_to_the_registered_hook() {
        let bytes = zun_compiler::compile_with_externals(
            "import { GetCurrentProcessId } from \"kernel32\"; function main(): i64 { return GetCurrentProcessId(); }",
            &[zun_sema::check::ExternalFunction {
                module: "kernel32".into(),
                name: "GetCurrentProcessId".into(),
                params: Vec::new(),
                return_ty: zun_sema::Ty::Int(zun_sema::hir::IntKind::I64),
            }],
        )
        .expect("compile native member reference")
        .write();
        let mut native = NativeCallHook::new();
        native.register("kernel32", "GetCurrentProcessId", FfiSignature::new([], FfiScalar::I64));
        let mut output = zun_interp::CaptureOutput::default();
        let result = execute_with_native(&bytes, &mut output, &native).expect("run native member reference");
        assert!(matches!(result, Value::Int(pid, zun_il::NumKind::I64) if pid > 0));
    }

    #[cfg(windows)]
    #[test]
    fn native_execution_routes_source_extern_function_to_the_registered_hook() {
        let bytes = zun_compiler::compile_with_externals(
            "extern function GetCurrentProcessId(): i64 from \"kernel32\"; function main(): i64 { return GetCurrentProcessId(); }",
            &[zun_sema::check::ExternalFunction {
                module: "kernel32".into(),
                name: "GetCurrentProcessId".into(),
                params: Vec::new(),
                return_ty: zun_sema::Ty::Int(zun_sema::hir::IntKind::I64),
            }],
        )
        .expect("compile source extern function")
        .write();
        let mut native = NativeCallHook::new();
        native.register("kernel32", "GetCurrentProcessId", FfiSignature::new([], FfiScalar::I64));
        let mut output = zun_interp::CaptureOutput::default();
        let result = execute_with_native(&bytes, &mut output, &native)
            .expect("run source extern function");
        assert!(matches!(result, Value::Int(pid, zun_il::NumKind::I64) if pid > 0));
    }

    #[cfg(windows)]
    #[test]
    fn native_execution_routes_f64_external_member_ref() {
        let bytes = zun_compiler::compile_with_externals(
            "import { sqrt } from \"msvcrt\"; function main(): number { return sqrt(9.0); }",
            &[zun_sema::check::ExternalFunction {
                module: "msvcrt".into(),
                name: "sqrt".into(),
                params: vec![zun_sema::Ty::F64],
                return_ty: zun_sema::Ty::F64,
            }],
        )
        .expect("compile f64 native member reference")
        .write();
        let mut native = NativeCallHook::new();
        native.register("msvcrt", "sqrt", FfiSignature::new([FfiScalar::F64], FfiScalar::F64));
        let mut output = zun_interp::CaptureOutput::default();
        let result = execute_with_native(&bytes, &mut output, &native).expect("run f64 native member reference");
        assert!(matches!(result, Value::Float(value, _) if (value - 3.0).abs() < f64::EPSILON));
    }

    #[cfg(windows)]
    #[test]
    fn native_execution_routes_bool_external_member_ref() {
        let bytes = zun_compiler::compile_with_externals(
            "import { IsDebuggerPresent } from \"kernel32\"; function main(): boolean { return IsDebuggerPresent(); }",
            &[zun_sema::check::ExternalFunction {
                module: "kernel32".into(),
                name: "IsDebuggerPresent".into(),
                params: Vec::new(),
                return_ty: zun_sema::Ty::Boolean,
            }],
        )
        .expect("compile bool native member reference")
        .write();
        let mut native = NativeCallHook::new();
        native.register("kernel32", "IsDebuggerPresent", FfiSignature::new([], FfiScalar::Bool));
        let mut output = zun_interp::CaptureOutput::default();
        let result = execute_with_native(&bytes, &mut output, &native).expect("run bool native member reference");
        assert!(matches!(result, Value::Bool(_)));
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
    fn verifies_loaded_binary_callback_tokens() {
        let module = zun_compiler::compile(
            "class Array { reduce(callback: Reducer, initial: any): any { let accumulator: any = initial; let index: u64 = 0; while (index < this.length) { accumulator = callback(accumulator, this[index]); index = index + 1; } return accumulator; } }\n\
             function add(accumulator: any, value: any): any { return (accumulator as number) + (value as number); }\n\
             function main(): any { let values = new Array(1); values[0] = 41.0; return values.reduce(add, 1.0); }",
        )
        .expect("compile binary callback module");
        let scope = zun_loader::ModuleScope::from_bytes("app", &module.write()).unwrap();
        let mut loader = zun_loader::ModuleLoader::new();
        loader.insert(scope).unwrap();
        let manifest = zun_loader::ModuleManifest::new("app", Vec::<String>::new());
        verify_manifest(&loader, &manifest).expect("binary callback token verifies");
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

    #[test]
    fn executes_linked_member_reference_call() {
        let loader = zun_compiler::compile_linked_modules([
            (
                "core",
                "function answer(value: int): int { return value + 1; }",
            ),
            (
                "app",
                "import { answer } from \"core\"; function main(): int { return answer(41); }",
            ),
        ])
        .expect("linked modules compile");
        let manifest = zun_loader::ModuleManifest::new("app", vec!["core".into()]);
        let mut output = zun_interp::CaptureOutput::default();
        let result = execute_manifest(&loader, &manifest, &mut output).unwrap();
        assert!(matches!(
            result,
            zun_interp::value::Value::Int(42, zun_il::NumKind::I32)
        ));
    }

    #[test]
    fn executes_a_module_linked_against_the_cached_prelude() {
        let (fingerprint, prelude) = zun_compiler::compile_prelude().expect("compile prelude");
        let loader = zun_compiler::compile_with_cached_prelude(
            "function main(): int { return abs(-7); }",
            &prelude,
        )
        .expect("link cached prelude");
        let manifest = zun_loader::ModuleManifest::new("app", vec!["zun:prelude".into()])
            .with_prelude_fingerprint(fingerprint);
        let mut output = zun_interp::CaptureOutput::default();
        let result = execute_manifest_with_prelude(&loader, &manifest, fingerprint, &mut output)
            .expect("execute linked prelude");
        assert!(matches!(
            result,
            zun_interp::value::Value::Int(7, zun_il::NumKind::I32)
        ));
    }
}
