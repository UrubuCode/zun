use std::cell::RefCell;
use std::collections::HashMap;

use zun_interp::value::Value;
use zun_interp::{CallHook, RunError};
use zun_il::NumKind;

use crate::{FfiScalar, FfiSignature, NativeLibraryResolver};

/// Host call hook for the supported blittable native ABI.
///
/// Bindings are explicit: a logical Zun module and export name must be
/// registered with its validated signature before execution. Values outside
/// that signature are rejected instead of being implicitly marshalled.
#[derive(Debug)]
pub struct NativeCallHook {
    resolver: RefCell<NativeLibraryResolver>,
    signatures: HashMap<(String, String), FfiSignature>,
}

impl NativeCallHook {
    pub fn new() -> Self {
        Self {
            resolver: RefCell::new(NativeLibraryResolver::new()),
            signatures: HashMap::new(),
        }
    }

    pub fn register(
        &mut self,
        module: impl Into<String>,
        export: impl Into<String>,
        signature: FfiSignature,
    ) {
        self.signatures
            .insert((module.into(), export.into()), signature);
    }

    pub fn resolver(&self) -> &RefCell<NativeLibraryResolver> {
        &self.resolver
    }

    fn invoke(
        &self,
        module: &str,
        export: &str,
        signature: &FfiSignature,
        args: &[Value],
    ) -> Result<Value, RunError> {
        let mut resolver = self.resolver.borrow_mut();
        let library = unsafe {
            resolver
                .load(module)
                .map_err(|error| RunError::Native(error.to_string()))?
        };
        if signature
            .params()
            .iter()
            .all(|param| *param == FfiScalar::I64)
            && signature.result() == FfiScalar::I64
        {
            let values: Option<Vec<i64>> = args
                .iter()
                .map(|value| match value {
                    Value::Int(number, _) => Some(*number),
                    _ => None,
                })
                .collect();
            let values = values.ok_or_else(|| RunError::Native("native i64 argument type mismatch".into()))?;
            let result = unsafe {
                library
                    .call_i64_signature(export.as_bytes(), signature, &values)
                    .map_err(|error| RunError::Native(error.to_string()))?
            };
            return Ok(Value::Int(result, NumKind::I64));
        }
        if signature
            .params()
            .iter()
            .all(|param| *param == FfiScalar::F64)
            && signature.result() == FfiScalar::F64
        {
            let values: Option<Vec<f64>> = args
                .iter()
                .map(|value| match value {
                    Value::Float(number, _) => Some(*number),
                    _ => None,
                })
                .collect();
            let values = values.ok_or_else(|| RunError::Native("native f64 argument type mismatch".into()))?;
            let result = unsafe {
                library
                    .call_f64_signature(export.as_bytes(), signature, &values)
                    .map_err(|error| RunError::Native(error.to_string()))?
            };
            return Ok(Value::Float(result, NumKind::F64));
        }
        if signature.result() == FfiScalar::Bool && signature.params().is_empty() && args.is_empty() {
            let result = unsafe {
                library
                    .call_bool_signature(export.as_bytes(), signature)
                    .map_err(|error| RunError::Native(error.to_string()))?
            };
            return Ok(Value::Bool(result));
        }
        Err(RunError::Native("unsupported native call signature".into()))
    }
}

impl Default for NativeCallHook {
    fn default() -> Self {
        Self::new()
    }
}

impl CallHook for NativeCallHook {
    fn try_call(&self, _method_rid: u32, _args: &[Value]) -> Option<Value> {
        None
    }

    fn try_call_external(
        &self,
        module: &str,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, RunError>> {
        let signature = self.signatures.get(&(module.to_owned(), name.to_owned()))?;
        Some(self.invoke(module, name, signature, args))
    }
}
