//! Routing external module calls to a loaded `zun_abi` native extension.
//!
//! A program that imports from a native module (e.g. `node:fs`, provided by the
//! `zun-node` extension) compiles those calls to `MemberRef` externals. At run
//! time the interpreter asks its [`CallHook`] to resolve them; this hook looks
//! the `(module, name)` up in the loaded extension(s), marshals the arguments
//! into the stable [`ZunValue`] carrier, invokes through the checked ABI path,
//! and marshals the result back.
//!
//! Marshalling follows the extension ABI v1 string/error convention (see
//! `zun-node`'s `lib.rs`): a `Str` argument is a borrowed NUL-terminated UTF-8
//! pointer valid for the call; a `Str` result is a pointer the host copies out
//! immediately. A non-zero status is turned into a thrown value by reading the
//! extension's `lastErrorMessage` (a Node-accurate message), so a failed `fs`
//! call surfaces as a catchable exception rather than a hard fault.

use std::ffi::{CStr, CString};
use std::rc::Rc;

use zun_abi::{OwnedFunction, OwnedRegistry, ZunTy, ZunTyRepr, ZunValue};
use zun_il::NumKind;
use zun_interp::value::Value;
use zun_interp::{CallHook, RunError};

use crate::extension::ExtensionResolver;

/// A source of registered extension functions, keyed by `(module, key)`.
pub trait ExtensionLookup {
    fn lookup(&self, module: &str, key: &str) -> Option<&OwnedFunction>;
}

impl ExtensionLookup for ExtensionResolver {
    fn lookup(&self, module: &str, key: &str) -> Option<&OwnedFunction> {
        self.find_function(module, key)
    }
}

impl ExtensionLookup for OwnedRegistry {
    fn lookup(&self, module: &str, key: &str) -> Option<&OwnedFunction> {
        self.find(module, key)
    }
}

/// A [`CallHook`] that resolves external module calls against a loaded
/// `zun_abi` extension surface.
pub struct ExtensionCallHook<L: ExtensionLookup> {
    lookup: L,
}

impl<L: ExtensionLookup> core::fmt::Debug for ExtensionCallHook<L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExtensionCallHook").finish_non_exhaustive()
    }
}

impl<L: ExtensionLookup> ExtensionCallHook<L> {
    pub fn new(lookup: L) -> Self {
        Self { lookup }
    }

    pub fn lookup(&self) -> &L {
        &self.lookup
    }

    /// Marshal one runtime value into the carrier shape the function's declared
    /// parameter expects. A `Str` parameter borrows `owned` (a NUL-terminated
    /// copy kept alive by the caller for the duration of the call).
    fn marshal_arg(
        value: &Value,
        expected: ZunTyRepr,
        owned: &mut Vec<CString>,
    ) -> Result<ZunValue, RunError> {
        let as_i64 = |value: &Value| -> Option<i64> {
            match value {
                Value::Int(number, _) => Some(*number),
                Value::Float(number, _) => Some(*number as i64),
                Value::Bool(flag) => Some(*flag as i64),
                _ => None,
            }
        };
        let ty = expected.0;
        let payload = if ty == ZunTy::Str as u32 {
            let Value::Str(text) = value else {
                return Err(RunError::Native("external call expected a string argument".into()));
            };
            let encoded = CString::new(text.as_bytes())
                .map_err(|_| RunError::Native("string argument contains an interior NUL".into()))?;
            let ptr = encoded.as_ptr() as u64;
            owned.push(encoded);
            ptr
        } else if ty == ZunTy::Bool as u32 {
            match value {
                Value::Bool(flag) => *flag as u64,
                _ => as_i64(value).map(|n| (n != 0) as u64).ok_or_else(|| {
                    RunError::Native("external call expected a boolean argument".into())
                })?,
            }
        } else if ty == ZunTy::F64 as u32 {
            let number = match value {
                Value::Float(number, _) => *number,
                Value::Int(number, _) => *number as f64,
                _ => return Err(RunError::Native("external call expected a number argument".into())),
            };
            number.to_bits()
        } else if matches!(
            ty,
            x if x == ZunTy::I32 as u32
                || x == ZunTy::U32 as u32
                || x == ZunTy::I64 as u32
                || x == ZunTy::U64 as u32
        ) {
            as_i64(value)
                .ok_or_else(|| RunError::Native("external call expected an integer argument".into()))?
                as u64
        } else {
            return Err(RunError::Native(
                "external call has an unsupported parameter type".into(),
            ));
        };
        Ok(ZunValue { ty: expected, reserved: 0, payload })
    }

    /// Marshal a returned carrier back into a runtime value.
    fn marshal_result(ret: ZunTyRepr, out: Option<ZunValue>) -> Value {
        let Some(out) = out else {
            return Value::Void;
        };
        let ty = ret.0;
        if ty == ZunTy::Str as u32 {
            if out.payload == 0 {
                return Value::Str(Rc::from(""));
            }
            let text = unsafe { CStr::from_ptr(out.payload as *const std::ffi::c_char) }
                .to_string_lossy()
                .into_owned();
            Value::Str(Rc::from(text.as_str()))
        } else if ty == ZunTy::Bool as u32 {
            Value::Bool(out.payload != 0)
        } else if ty == ZunTy::F64 as u32 {
            Value::Float(f64::from_bits(out.payload), NumKind::F64)
        } else if ty == ZunTy::I32 as u32 {
            Value::Int(out.payload as i32 as i64, NumKind::I32)
        } else if ty == ZunTy::U32 as u32 {
            Value::Int(out.payload as u32 as i64, NumKind::U32)
        } else if ty == ZunTy::U64 as u32 {
            Value::Int(out.payload as i64, NumKind::U64)
        } else if ty == ZunTy::I64 as u32 {
            Value::Int(out.payload as i64, NumKind::I64)
        } else {
            Value::Void
        }
    }

    /// Read the extension's last Node-accurate error message for `module`, if it
    /// exposes `lastErrorMessage`; otherwise a generic status message.
    fn error_message(&self, module: &str, status: i32) -> String {
        if let Some(function) = self.lookup.lookup(module, "lastErrorMessage") {
            if let Ok(Some(out)) = function.invoke_checked(&[]) {
                if out.payload != 0 {
                    let message = unsafe { CStr::from_ptr(out.payload as *const std::ffi::c_char) }
                        .to_string_lossy()
                        .into_owned();
                    if !message.is_empty() {
                        return message;
                    }
                }
            }
        }
        format!("external call to `{module}` failed with status {status}")
    }

    fn dispatch(&self, module: &str, name: &str, args: &[Value]) -> Result<Value, RunError> {
        let function = self
            .lookup
            .lookup(module, name)
            .ok_or_else(|| RunError::Native(format!("external function `{module}.{name}` not found")))?;
        if function.params.len() != args.len() {
            return Err(RunError::Native(format!(
                "external function `{module}.{name}` expects {} args, got {}",
                function.params.len(),
                args.len()
            )));
        }
        let mut owned: Vec<CString> = Vec::new();
        let mut marshalled: Vec<ZunValue> = Vec::with_capacity(args.len());
        for (value, &expected) in args.iter().zip(&function.params) {
            marshalled.push(Self::marshal_arg(value, expected, &mut owned)?);
        }
        match function.invoke_checked(&marshalled) {
            Ok(out) => Ok(Self::marshal_result(function.ret, out)),
            // A Node-style failure becomes a catchable thrown value carrying the
            // extension's message (a proper Error object with `.code` is a
            // follow-up once a host error-constructor is available here).
            Err(status) => Err(RunError::Thrown(Value::Str(Rc::from(
                self.error_message(module, status).as_str(),
            )))),
        }
    }
}

impl<L: ExtensionLookup> CallHook for ExtensionCallHook<L> {
    fn try_call(&self, _method_rid: u32, _args: &[Value]) -> Option<Value> {
        None
    }

    fn try_call_external(
        &self,
        module: &str,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, RunError>> {
        self.lookup.lookup(module, name)?;
        Some(self.dispatch(module, name, args))
    }

    fn try_call_external_typed(
        &self,
        module: &str,
        name: &str,
        _signature: &[u8],
        args: &[Value],
    ) -> Option<Result<Value, RunError>> {
        self.try_call_external(module, name, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ffi::c_char;
    use zun_abi::{ZunFunction, ZunRegistry};

    // A tiny in-process extension: `demo.echoLen(s: Str) -> I64` returns the
    // byte length, and `demo.boom() -> Str` fails, stashing an error the hook
    // reads back through `lastErrorMessage`.
    thread_local! {
        static LAST: std::cell::RefCell<CString> = std::cell::RefCell::new(CString::default());
        static SCRATCH: std::cell::RefCell<CString> = std::cell::RefCell::new(CString::default());
    }

    unsafe extern "C" fn echo_len(args: *const ZunValue, n: usize, out: *mut ZunValue) -> i32 {
        if n != 1 {
            return 2;
        }
        let ptr = unsafe { (*args).payload } as *const c_char;
        let len = unsafe { CStr::from_ptr(ptr) }.to_bytes().len() as i64;
        unsafe {
            (*out).ty = ZunTyRepr::from(ZunTy::I64);
            (*out).reserved = 0;
            (*out).payload = len as u64;
        }
        0
    }

    unsafe extern "C" fn boom(_: *const ZunValue, _: usize, _: *mut ZunValue) -> i32 {
        LAST.with(|slot| *slot.borrow_mut() = CString::new("ENOENT: boom").unwrap());
        7
    }

    unsafe extern "C" fn last_error_message(_: *const ZunValue, _: usize, out: *mut ZunValue) -> i32 {
        let message = LAST.with(|slot| slot.borrow().clone());
        let ptr = SCRATCH.with(|slot| {
            *slot.borrow_mut() = message;
            slot.borrow().as_ptr()
        });
        unsafe {
            (*out).ty = ZunTyRepr::from(ZunTy::Str);
            (*out).reserved = 0;
            (*out).payload = ptr as u64;
        }
        0
    }

    fn registry() -> OwnedRegistry {
        let mut owned = OwnedRegistry::new();
        let mut host: ZunRegistry = owned.as_registry();
        let register = |host: &mut ZunRegistry, key: &[u8], params: &[ZunTyRepr], ret: ZunTy, invoke| {
            let function = ZunFunction {
                key: key.as_ptr() as *const c_char,
                params: params.as_ptr(),
                param_count: params.len(),
                ret: ZunTyRepr::from(ret),
                invoke: Some(invoke),
            };
            unsafe { host.register_function(b"demo\0".as_ptr() as *const c_char, &function) }
                .expect("register");
        };
        register(&mut host, b"echoLen\0", &[ZunTyRepr::from(ZunTy::Str)], ZunTy::I64, echo_len);
        register(&mut host, b"boom\0", &[], ZunTy::Str, boom);
        register(&mut host, b"lastErrorMessage\0", &[], ZunTy::Str, last_error_message);
        owned
    }

    #[test]
    fn dispatches_a_string_argument_and_marshals_the_scalar_result() {
        let hook = ExtensionCallHook::new(registry());
        let result = hook
            .try_call_external("demo", "echoLen", &[Value::Str(Rc::from("zun"))])
            .expect("resolved")
            .expect("ok");
        assert!(matches!(result, Value::Int(3, NumKind::I64)));
    }

    #[test]
    fn unresolved_external_falls_through_to_the_loader() {
        let hook = ExtensionCallHook::new(registry());
        assert!(hook.try_call_external("demo", "missing", &[]).is_none());
    }

    #[test]
    fn a_failing_call_throws_the_extensions_node_message() {
        let hook = ExtensionCallHook::new(registry());
        let outcome = hook.try_call_external("demo", "boom", &[]).expect("resolved");
        match outcome {
            Err(RunError::Thrown(Value::Str(message))) => assert_eq!(&*message, "ENOENT: boom"),
            other => panic!("expected a thrown Node message, got {other:?}"),
        }
    }

    // End to end: a compiled program imports an external function and calls it;
    // the interpreter routes the `MemberRef` through this hook into the
    // in-process extension and marshals the string argument and scalar result.
    #[test]
    fn a_program_calls_an_external_function_through_the_hook() {
        let bytes = zun_compiler::compile_with_externals(
            "import { echoLen } from \"demo\"; \
             function main(): i64 { return echoLen(\"hello\"); }",
            &[zun_sema::check::ExternalFunction {
                module: "demo".into(),
                name: "echoLen".into(),
                params: vec![zun_sema::Ty::String],
                return_ty: zun_sema::Ty::Int(zun_sema::hir::IntKind::I64),
            }],
        )
        .expect("compile external call")
        .write();

        let hook = ExtensionCallHook::new(registry());
        let mut output = zun_interp::CaptureOutput::default();
        let result = crate::execute_with_extensions(&bytes, &mut output, &hook)
            .expect("run external call");
        assert!(matches!(result, Value::Int(5, NumKind::I64)), "got {result:?}");
    }
}
