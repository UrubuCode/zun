//! The tiering bridge: a [`CallHook`] that routes eligible calls to the JIT.
//!
//! When the interpreter is about to call a method the JIT compiled, this hook
//! marshals the argument [`Value`]s into the shim's packed `u64` buffer, invokes
//! the native code, and unmarshals the packed result back into a `Value`.
//! Ineligible methods return `None` so the interpreter runs them itself.

use zun_codegen::{Jit, JitKind, NumKind};
use zun_interp::value::Value;
use zun_interp::CallHook;

/// Bridges the interpreter to a compiled [`Jit`].
pub struct JitHook {
    jit: Jit,
}

impl core::fmt::Debug for JitHook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("JitHook").finish()
    }
}

impl JitHook {
    /// Wrap a compiled JIT.
    pub fn new(jit: Jit) -> Self {
        Self { jit }
    }
}

impl CallHook for JitHook {
    fn try_call(&self, method_rid: u32, args: &[Value]) -> Option<Value> {
        if !self.jit.is_compiled(method_rid) {
            return None;
        }
        let kinds = self.jit.param_kinds(method_rid)?;
        if kinds.len() != args.len() {
            return None;
        }
        let packed: Vec<u64> = args
            .iter()
            .zip(kinds)
            .map(|(v, &k)| pack_value(v, k))
            .collect();
        let result = self.jit.call_packed(method_rid, &packed).ok()?;
        Some(unpack_value(result, self.jit.return_kind(method_rid)))
    }
}

/// Marshal a `Value` into a packed `u64` for parameter kind `k`.
fn pack_value(v: &Value, k: JitKind) -> u64 {
    match k {
        JitKind::Num(NumKind::F64) => v.as_number().to_bits(),
        JitKind::Num(NumKind::F32) => (v.as_number() as f32).to_bits() as u64,
        JitKind::Bool => matches!(v, Value::Bool(true)) as u64,
        JitKind::Num(_) => v.as_int() as u64,
        JitKind::Other => 0,
    }
}

/// Unmarshal a packed `u64` result back into a `Value` of the return kind.
fn unpack_value(bits: u64, ret: Option<JitKind>) -> Value {
    match ret {
        None => Value::Void,
        Some(JitKind::Bool) => Value::Bool(bits != 0),
        Some(JitKind::Num(NumKind::F64)) => Value::Float(f64::from_bits(bits), NumKind::F64),
        Some(JitKind::Num(NumKind::F32)) => {
            Value::Float(f32::from_bits(bits as u32) as f64, NumKind::F32)
        }
        Some(JitKind::Num(n)) => Value::Int(bits as i64, n),
        Some(JitKind::Other) => Value::Void,
    }
}
