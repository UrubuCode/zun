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

    /// Register a signature while rejecting ABI shapes that need generated
    /// marshalling. The infallible `register` remains for host compatibility.
    pub fn register_checked(
        &mut self,
        module: impl Into<String>,
        export: impl Into<String>,
        signature: FfiSignature,
    ) -> Result<(), String> {
        signature.validate()?;
        self.register(module, export, signature);
        Ok(())
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

    fn try_call_external_typed(
        &self,
        module: &str,
        name: &str,
        encoded: &[u8],
        args: &[Value],
    ) -> Option<Result<Value, RunError>> {
        let signature = self.signatures.get(&(module.to_owned(), name.to_owned()))?;
        if encoded.len() >= 2 {
            let arity = usize::from(encoded[0]);
            let expected_len = arity.checked_add(2)?;
            if arity != args.len() || encoded.len() != expected_len {
                return Some(Err(RunError::Native("native metadata signature arity mismatch".into())));
            }
            let expected = encoded[1..]
                .iter()
                .map(|tag| match tag {
                    1 => Some(FfiScalar::I64),
                    2 => Some(FfiScalar::F64),
                    3 => Some(FfiScalar::Bool),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>();
            let Some(expected) = expected else {
                return Some(Err(RunError::Native("unsupported native metadata type".into())));
            };
            let result = expected.last().copied();
            let params = &expected[..expected.len() - 1];
            if result != Some(signature.result()) || params != signature.params() {
                return Some(Err(RunError::Native("native metadata signature mismatch".into())));
            }
        }
        Some(self.invoke(module, name, signature, args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn calls_a_system_library_through_the_registered_scalar_abi() {
        let mut hook = NativeCallHook::new();
        hook.register("kernel32", "GetCurrentProcessId", FfiSignature::new([], FfiScalar::I64));
        let result = hook
            .try_call_external("kernel32", "GetCurrentProcessId", &[])
            .expect("registered native export")
            .expect("system call succeeds");
        assert!(matches!(result, Value::Int(pid, NumKind::I64) if pid > 0));
    }

    #[test]
    fn unregistered_exports_are_not_intercepted() {
        let hook = NativeCallHook::new();
        assert!(hook.try_call_external("kernel32", "missing", &[]).is_none());
    }

    #[test]
    fn rejects_a_native_call_when_metadata_types_do_not_match() {
        let mut hook = NativeCallHook::new();
        hook.register("test", "answer", FfiSignature::new([], FfiScalar::I64));
        let result = hook
            .try_call_external_typed("test", "answer", &[0, 2], &[])
            .expect("registered export must be intercepted")
            .expect_err("metadata must reject an f64 result for an i64 registration");
        assert!(result.to_string().contains("signature mismatch"));
    }

    #[test]
    fn checked_registration_rejects_mixed_scalar_shapes() {
        let mut hook = NativeCallHook::new();
        let error = hook
            .register_checked("test", "mixed", FfiSignature::new([FfiScalar::I64], FfiScalar::F64))
            .expect_err("mixed direct ABI must require a marshalling stub");
        assert!(error.contains("unsupported direct FFI signature"));
    }
}
