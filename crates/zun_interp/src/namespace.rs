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

/// Runtime helper behind `import.dynamic`: resolve `specifier` through the
/// module loader, materialize the namespace, and settle a fresh Promise.
///
/// Resolution is by exact loader identity, falling back to the specifier with a
/// leading `./` removed. Modules must already be registered with the loader
/// (the interpreter only holds a shared reference); anything else rejects the
/// Promise rather than throwing, matching `import()` semantics.
pub fn zun_dynamic_import(
    loader: Option<&ModuleLoader>,
    specifier: &str,
) -> Result<Value, RunError> {
    let promise = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
    let scope = loader.and_then(|loader| {
        loader
            .get(specifier)
            .or_else(|| specifier.strip_prefix("./").and_then(|name| loader.get(name)))
    });
    match scope {
        Some(scope) => {
            let Value::Object(namespace) = materialize_namespace(scope)? else {
                return Err(RunError::BadToken);
            };
            let any = zun_core::Any::object(namespace.addr() as usize);
            zun_gc::with_current_heap_mut(|heap| heap.promise_resolve(promise, any));
        }
        None => {
            let message = format!("cannot find module `{specifier}`");
            zun_gc::with_current_heap_mut(|heap| heap.promise_reject(promise, message));
        }
    }
    Ok(Value::Object(promise))
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

    fn app_module(specifier: &str, await_result: bool) -> zun_module::Module {
        use zun_il::{encode, Instruction, Operand};
        use zun_metadata::{MethodDef, TableKind, Token};
        let mut module = zun_module::Module::new();
        let name = module.strings.intern(specifier);
        let mut code = Vec::new();
        let mut body = vec![
            Instruction::new(zun_il::Opcode::ConstStr, Operand::StringRef(name.0)),
            Instruction::simple(zun_il::Opcode::ImportDynamic),
        ];
        if await_result {
            body.push(Instruction::simple(zun_il::Opcode::Await));
        }
        body.extend([
            Instruction::simple(zun_il::Opcode::Pop),
            Instruction::new(zun_il::Opcode::ConstI32, Operand::Imm32(7)),
            Instruction::simple(zun_il::Opcode::Ret),
        ]);
        for instruction in &body {
            encode(instruction, &mut code);
        }
        let offset = zun_module::write_body(
            &mut module.code,
            0,
            0,
            true,
            zun_il::JitKind::Num(zun_il::NumKind::I32).to_byte(),
            &[],
            &code,
        );
        module.tables.add_method_def(MethodDef {
            name: module.strings.intern("main"),
            signature: zun_metadata::BlobIndex(0),
            body_offset: offset,
            is_static: false,
        });
        module.entry_point = Some(Token::new(TableKind::MethodDef, 1));
        module
    }

    fn loader_with_lib() -> ModuleLoader {
        let lib = zun_compiler::compile(
            "function answer(value: int): int { return value + 1; } function main(): int { return 0; }",
        )
        .expect("compile module");
        let mut loader = ModuleLoader::new();
        loader.load_bytes("lib", &lib.write()).unwrap();
        loader
    }

    #[test]
    fn import_dynamic_il_awaits_a_resolved_namespace() {
        let mut loader = loader_with_lib();
        loader.load_bytes("app", &app_module("lib", true).write()).unwrap();
        let app = loader.get("app").unwrap();
        let mut output = CaptureOutput::default();
        let result = run_method_loaded(app, &loader, 1, vec![], &mut output).unwrap();
        assert!(matches!(result, Value::Int(7, zun_il::NumKind::I32)));
    }

    #[test]
    fn import_dynamic_il_rejects_unknown_modules() {
        let mut loader = loader_with_lib();
        loader.load_bytes("app", &app_module("missing", true).write()).unwrap();
        let app = loader.get("app").unwrap();
        let mut output = CaptureOutput::default();
        let error = run_method_loaded(app, &loader, 1, vec![], &mut output).unwrap_err();
        assert!(matches!(error, RunError::Thrown(Value::Str(message)) if message.contains("missing")));
        // Without an await the rejected Promise is simply discarded.
        let mut loader = loader_with_lib();
        loader.load_bytes("app", &app_module("missing", false).write()).unwrap();
        let app = loader.get("app").unwrap();
        let result = run_method_loaded(app, &loader, 1, vec![], &mut output).unwrap();
        assert!(matches!(result, Value::Int(7, _)));
    }

    const APP_SOURCE: &str = "async function compute(): Promise { let ns: any = await import(\"lib\"); let p = new Promise(); let v: any = ns.answer(41.0); p.resolve(v); return p; }
\n        function main(): any { let r = compute(); return r.result(); }";

    #[test]
    fn import_expression_runs_end_to_end_on_tier_zero() {
        // v1 scope: only function exports are present on the namespace; const/let and
        // class exports read as undefined until the compiler emits value-export metadata.
        let lib = zun_compiler::compile(
            "function answer(value: number): number { return value + 1.0; } function main(): int { return 0; }",
        )
        .expect("compile lib");
        for oxc in [false, true] {
        let app = if oxc {
            zun_compiler::compile_with_oxc(APP_SOURCE, true).expect("compile app with import() via OXC")
        } else {
            zun_compiler::compile(APP_SOURCE).expect("compile app with import()")
        };
        let mut loader = ModuleLoader::new();
        loader.load_bytes("lib", &lib.write()).unwrap();
        loader.load_bytes("app", &app.write()).unwrap();
        let entry = loader.get("app").unwrap().module().entry_point.unwrap().rid();
        let mut output = CaptureOutput::default();
        let imported = run_method_loaded(loader.get("app").unwrap(), &loader, entry, vec![], &mut output).unwrap();
        // The same export called directly (what a static import links to).
        let answer = loader.get("lib").unwrap().export("answer").map(|export| match export {
            Export::Method(token) => token.rid(),
            Export::Type(_) => unreachable!(),
        });
        let direct = run_method_loaded(
            loader.get("lib").unwrap(),
            &loader,
            answer.unwrap(),
            vec![Value::Float(41.0, zun_il::NumKind::F64)],
            &mut output,
        )
        .unwrap();
        let (Value::Any(any), Value::Float(direct, _)) = (imported, direct) else {
            panic!("unexpected result shapes");
        };
        assert_eq!(any.as_float(), Some(direct));
        assert_eq!(direct, 42.0);
        }
    }

    #[test]
    fn dynamic_import_promise_resolves_to_a_callable_namespace() {
        let loader = loader_with_lib();
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        for specifier in ["lib", "./lib"] {
            let Value::Object(promise) = zun_dynamic_import(Some(&loader), specifier).unwrap() else {
                panic!("import.dynamic must produce a Promise object");
            };
            let address = zun_gc::with_current_heap(|heap| heap.promise_result(promise))
                .and_then(|any| any.as_object())
                .expect("promise resolved with the namespace object");
            let namespace = Value::Object(unsafe { zun_gc::Obj::from_addr(address as *mut u64) });
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
        }
    }

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
