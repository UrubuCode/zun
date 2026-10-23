//! Runtime module namespaces for dynamic `import()`.
//!
//! The static `import * as ns` form is resolved at compile time and never
//! builds an object. The dynamic path is the explicit, costly exception: a
//! loaded module's exports are mirrored into a `DynamicObject` on the managed
//! heap so they can be read and called without static knowledge of the module.

use crate::{run_method_loaded, Output, RunError, Value};
use zun_loader::{Export, ModuleLoader, ModuleScope};

/// Materialize a loaded module's namespace as a managed `DynamicObject`.
///
/// Only function exports are mirrored, each as a [`zun_core::ModuleBoundFunction`]
/// naming the defining module by loader identity. Type-only exports do not exist
/// at runtime. Value (const/let) and class exports are out of scope for v1 and
/// read as `undefined` until the compiler emits value-export metadata.
///
/// Must run with a current heap installed.
pub fn materialize_namespace(scope: &ModuleScope) -> Result<Value, RunError> {
    Ok(zun_gc::with_current_heap_mut(|heap| {
        let namespace = heap.alloc_dynamic();
        for (name, export) in scope.exports() {
            if let Export::Method(token) = export {
                let function = zun_core::ModuleBoundFunction::new(scope.name(), token.rid());
                heap.dynamic_set_module_function(namespace, name, function);
            }
        }
        Value::Object(namespace)
    }))
}

/// Call a function export of a materialized namespace by name.
///
/// The bound function is resolved through `loader` against its named module,
/// the same cross-module mechanism `MemberRef` calls use.
pub fn call_namespace_member(
    loader: &ModuleLoader,
    namespace: &Value,
    name: &str,
    args: Vec<Value>,
    output: &mut dyn Output,
) -> Result<Value, RunError> {
    let Value::Object(namespace) = namespace else {
        return Err(RunError::BadToken);
    };
    let function = zun_gc::with_current_heap(|heap| heap.dynamic_get_module_function(*namespace, name))
        .ok_or(RunError::BadToken)?;
    let scope = loader.get(function.module()).ok_or(RunError::BadToken)?;
    run_method_loaded(scope, loader, function.method_rid(), args, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CaptureOutput;

    #[test]
    fn namespace_exposes_function_exports_bound_to_their_module() {
        let module = zun_compiler::compile(
            "function answer(value: int): int { return value + 1; } function main(): int { return 0; }",
        )
        .expect("compile module");
        let scope = ModuleScope::from_bytes("lib", &module.write()).unwrap();
        let mut loader = ModuleLoader::new();
        loader.insert(scope).unwrap();

        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        let namespace = materialize_namespace(loader.get("lib").unwrap()).unwrap();
        let mut output = CaptureOutput::default();
        let result = call_namespace_member(
            &loader,
            &namespace,
            "answer",
            vec![Value::Int(41, zun_il::NumKind::I32)],
            &mut output,
        )
        .unwrap();
        assert!(matches!(result, Value::Int(42, zun_il::NumKind::I32)));
        // v1 scope: value exports and unknown members are not callable yet.
        assert!(call_namespace_member(&loader, &namespace, "missing", vec![], &mut output).is_err());
    }
}
