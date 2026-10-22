//! Stable, host-facing ABI for native Zun extension modules.
//!
//! This crate deliberately contains no VM, Node, allocator, or interpreter
//! dependency. An extension exports `zun_register_v1` and receives a
//! [`ZunRegistry`] supplied by the host. The host owns all strings and
//! descriptor storage for the duration of registration; extensions must not
//! retain those pointers unless a future ABI version explicitly permits it.

use core::ffi::{c_char, c_void};

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
        let callback = self.register_function.ok_or(-1)?;
        let status = unsafe { callback(self.context, module, function) };
        if status == 0 { Ok(()) } else { Err(status) }
    }
}

/// Signature every version-1 extension registration function must expose.
pub type ZunRegisterV1 = unsafe extern "C" fn(registry: *mut ZunRegistry) -> i32;

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
        let mut registry = ZunRegistry { abi_version: ZUN_ABI_V1, context: core::ptr::null_mut(), register_function: Some(reject) };
        let result = unsafe { registry.register_function(core::ptr::null(), core::ptr::null()) };
        assert_eq!(result, Err(17));
    }
}
