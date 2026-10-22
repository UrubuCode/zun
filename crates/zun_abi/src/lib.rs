//! Stable, host-facing ABI for native Zun extension modules.
//!
//! This crate deliberately contains no VM, Node, allocator, or interpreter
//! dependency. An extension exports `zun_register_v1` and receives a
//! [`ZunRegistry`] supplied by the host. The host owns all strings and
//! descriptor storage for the duration of registration; extensions must not
//! retain those pointers unless a future ABI version explicitly permits it.

use core::ffi::{c_char, c_void};
use std::collections::HashMap;
use std::ffi::CStr;

/// ABI contract version encoded in the exported registration symbol.
pub const ZUN_ABI_V1: u32 = 1;

/// Scalar and opaque values understood by the first extension contract.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ZunTy {
    Void = 0,
    Bool = 1,
    I32 = 2,
    U32 = 3,
    I64 = 4,
    U64 = 5,
    F64 = 6,
    Str = 7,
    Opaque = 8,
}

/// Two-word value carrier passed across the extension boundary.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ZunValue {
    pub ty: ZunTyRepr,
    /// Reserved for future carrier flags; must be zero in ABI v1.
    pub reserved: u32,
    pub payload: u64,
}

/// ABI-safe representation of [`ZunTy`] for callers written in C.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ZunTyRepr(pub u32);

impl From<ZunTy> for ZunTyRepr {
    fn from(value: ZunTy) -> Self { Self(value as u32) }
}

/// Native function implementation. Return zero for success and non-zero for
/// an extension-defined error. The host initializes `out` for non-void calls.
pub type ZunInvoke = unsafe extern "C" fn(
    args: *const ZunValue,
    arg_count: usize,
    out: *mut ZunValue,
) -> i32;

/// A function descriptor valid only during the registration call.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ZunFunction {
    pub key: *const c_char,
    pub params: *const ZunTyRepr,
    pub param_count: usize,
    pub ret: ZunTyRepr,
    pub invoke: Option<ZunInvoke>,
}

impl ZunFunction {
    /// Validate the descriptor before the host stores or invokes it.
    ///
    /// # Safety
    /// `self.params` must point to at least `param_count` entries when
    /// `param_count` is non-zero.
    pub unsafe fn validate(&self) -> Result<(), i32> {
        if self.key.is_null() || self.invoke.is_none() {
            return Err(-2);
        }
        if self.param_count != 0 && self.params.is_null() {
            return Err(-3);
        }
        Ok(())
    }
}

/// Host callback used by an extension to publish one function.
pub type ZunRegisterFunction = unsafe extern "C" fn(
    context: *mut c_void,
    module: *const c_char,
    function: *const ZunFunction,
) -> i32;

/// Registration context supplied by the Zun host.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ZunRegistry {
    pub abi_version: u32,
    pub context: *mut c_void,
    pub register_function: Option<ZunRegisterFunction>,
}

impl ZunRegistry {
    /// Validate the host contract before accepting registrations.
    pub fn validate(&self) -> Result<(), i32> {
        if self.abi_version != ZUN_ABI_V1 || self.register_function.is_none() {
            Err(-1)
        } else {
            Ok(())
        }
    }

    /// Publish a descriptor through the host callback.
    ///
    /// # Safety
    /// `module`, `function`, and the descriptor's pointed-to arrays must stay
    /// valid until the callback returns.
    pub unsafe fn register_function(
        &mut self,
        module: *const c_char,
        function: *const ZunFunction,
    ) -> Result<(), i32> {
        self.validate()?;
        if module.is_null() {
            return Err(-5);
        }
        if function.is_null() {
            return Err(-4);
        }
        unsafe { (*function).validate()? };
        let callback = self.register_function.ok_or(-1)?;
        let status = unsafe { callback(self.context, module, function) };
        if status == 0 { Ok(()) } else { Err(status) }
    }
}

/// Signature every version-1 extension registration function must expose.
pub type ZunRegisterV1 = unsafe extern "C" fn(registry: *mut ZunRegistry) -> i32;

/// Owned copy of a descriptor published by a native extension.
#[derive(Debug)]
pub struct OwnedFunction {
    pub module: String,
    pub key: String,
    pub params: Vec<ZunTyRepr>,
    pub ret: ZunTyRepr,
    pub invoke: ZunInvoke,
}

/// A namespace-scoped view over registered extension functions.
#[derive(Debug, Clone)]
pub struct ZunModule<'a> {
    registry: &'a OwnedRegistry,
    name: String,
}

impl<'a> ZunModule<'a> {
    /// Resolve a function by its module-local key.
    pub fn func(&self, key: &str) -> Option<&'a OwnedFunction> {
        self.registry.find(&self.name, key)
    }
}

/// Registration storage that copies descriptor data before the callback ends.
///
/// The callback API is intended for a loader that owns the backing strings and
/// parameter vectors for the lifetime of the loaded library. `OwnedRegistry`
/// itself owns every copied string and parameter vector.
#[derive(Default, Debug)]
pub struct OwnedRegistry {
    pub functions: Vec<OwnedFunction>,
    index: HashMap<(String, String), usize>,
}

impl OwnedRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn as_registry(&mut self) -> ZunRegistry {
        ZunRegistry {
            abi_version: ZUN_ABI_V1,
            context: self as *mut Self as *mut c_void,
            register_function: Some(Self::capture),
        }
    }

    /// Resolve a copied function descriptor by its stable module/key pair.
    pub fn find(&self, module: &str, key: &str) -> Option<&OwnedFunction> {
        let index = self.index.get(&(module.to_owned(), key.to_owned()))?;
        self.functions.get(*index)
    }

    /// Open a namespace view for a registered module.
    pub fn module(&self, name: &str) -> ZunModule<'_> {
        ZunModule { registry: self, name: name.to_owned() }
    }

    unsafe extern "C" fn capture(
        context: *mut c_void,
        module: *const c_char,
        function: *const ZunFunction,
    ) -> i32 {
        let Some(registry) = (unsafe { context.cast::<Self>().as_mut() }) else { return -10 };
        if module.is_null() || function.is_null() { return -11; }
        let function = unsafe { &*function };
        if unsafe { function.validate() }.is_err() { return -12; }
        let Ok(module) = (unsafe { CStr::from_ptr(module) }).to_str() else { return -13 };
        let Ok(key) = (unsafe { CStr::from_ptr(function.key) }).to_str() else { return -14 };
        let params = if function.param_count == 0 { Vec::new() } else {
            unsafe { std::slice::from_raw_parts(function.params, function.param_count) }.to_vec()
        };
        let module = module.to_owned();
        let key = key.to_owned();
        if registry.index.contains_key(&(module.clone(), key.clone())) {
            return -15;
        }
        let index = registry.functions.len();
        registry.functions.push(OwnedFunction {
            module: module.clone(),
            key: key.clone(),
            params,
            ret: function.ret,
            invoke: function.invoke.expect("validated invoke"),
        });
        registry.index.insert((module, key), index);
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, size_of};

    #[test]
    fn abi_carriers_have_stable_word_layouts() {
        assert_eq!(size_of::<ZunValue>(), 16);
        assert_eq!(align_of::<ZunValue>(), 8);
        assert_eq!(size_of::<ZunTyRepr>(), 4);
        assert_eq!(ZunTy::Str as u32, 7);
    }

    #[test]
    fn registration_reports_host_callback_errors() {
        unsafe extern "C" fn reject(_: *mut c_void, _: *const c_char, _: *const ZunFunction) -> i32 { 17 }
        unsafe extern "C" fn invoke(_: *const ZunValue, _: usize, _: *mut ZunValue) -> i32 { 0 }
        static KEY: &[u8] = b"test\0";
        let function = ZunFunction {
            key: KEY.as_ptr() as *const c_char,
            params: core::ptr::null(),
            param_count: 0,
            ret: ZunTy::Void.into(),
            invoke: Some(invoke),
        };
        let mut registry = ZunRegistry { abi_version: ZUN_ABI_V1, context: core::ptr::null_mut(), register_function: Some(reject) };
        static MODULE: &[u8] = b"zun:test\0";
        let result = unsafe { registry.register_function(MODULE.as_ptr() as *const c_char, &function) };
        assert_eq!(result, Err(17));
    }

    #[test]
    fn registration_rejects_null_module_name() {
        unsafe extern "C" fn invoke(_: *const ZunValue, _: usize, _: *mut ZunValue) -> i32 { 0 }
        static KEY: &[u8] = b"answer\0";
        let function = ZunFunction {
            key: KEY.as_ptr() as *const c_char,
            params: core::ptr::null(),
            param_count: 0,
            ret: ZunTy::I32.into(),
            invoke: Some(invoke),
        };
        let mut owned = OwnedRegistry::new();
        let mut registry = owned.as_registry();
        assert_eq!(unsafe { registry.register_function(core::ptr::null(), &function) }, Err(-5));
    }

    #[test]
    fn registry_rejects_wrong_abi_version() {
        let registry = ZunRegistry { abi_version: 99, context: core::ptr::null_mut(), register_function: None };
        assert_eq!(registry.validate(), Err(-1));
    }

    #[test]
    fn owned_registry_copies_descriptor_storage() {
        unsafe extern "C" fn invoke(_: *const ZunValue, _: usize, _: *mut ZunValue) -> i32 { 0 }
        static MODULE: &[u8] = b"zun:test\0";
        static KEY: &[u8] = b"answer\0";
        let function = ZunFunction { key: KEY.as_ptr() as *const c_char, params: core::ptr::null(), param_count: 0, ret: ZunTy::I32.into(), invoke: Some(invoke) };
        let mut owned = OwnedRegistry::new();
        let mut registry = owned.as_registry();
        assert_eq!(unsafe { registry.register_function(MODULE.as_ptr() as *const c_char, &function) }, Ok(()));
        assert_eq!(owned.functions[0].module, "zun:test");
        assert_eq!(owned.functions[0].key, "answer");
        assert_eq!(owned.functions[0].params, Vec::<ZunTyRepr>::new());
        assert!(owned.find("zun:test", "answer").is_some());
        assert!(owned.module("zun:test").func("answer").is_some());
        assert!(owned.module("zun:other").func("answer").is_none());
        assert!(owned.find("zun:test", "missing").is_none());
        assert_eq!(unsafe { registry.register_function(MODULE.as_ptr() as *const c_char, &function) }, Err(-15));
    }
}
