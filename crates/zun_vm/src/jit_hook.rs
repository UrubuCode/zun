//! The tiering bridge: a [`CallHook`] that routes hot calls to the JIT.
//!
//! The hook counts how often each method is called. While a method is cold it
//! returns `None`, so the interpreter runs it. Once its count crosses the
//! threshold it is compiled on demand (together with its eligible-callee
//! closure) and every later call goes to native code. Ineligible methods
//! (anything touching the heap) always return `None`.
//!
//! Marshalling: argument [`Value`]s are packed into the shim's `u64` buffer and
//! the packed result is unpacked by the method's return kind.

use std::cell::RefCell;
use std::collections::HashMap;

use zun_codegen::{Jit, JitKind, NumKind};
use zun_interp::value::Value;
use zun_interp::CallHook;
use zun_module::Module;

/// How many interpreted calls a method takes before it is JIT-compiled.
pub const DEFAULT_THRESHOLD: u32 = 2;

/// Bridges the interpreter to a lazily-compiled [`Jit`].
pub struct JitHook<'a> {
    jit: RefCell<Jit>,
    module: &'a Module,
    counts: RefCell<HashMap<u32, u32>>,
    threshold: u32,
}

impl core::fmt::Debug for JitHook<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("JitHook").field("threshold", &self.threshold).finish()
    }
}

impl<'a> JitHook<'a> {
    /// Wrap a JIT with the default hotness threshold.
    pub fn new(jit: Jit, module: &'a Module) -> Self {
        Self::with_threshold(jit, module, DEFAULT_THRESHOLD)
    }

    /// Whether the given method has been JIT-compiled so far (observability).
    pub fn jit_is_defined(&self, method_rid: u32) -> bool {
        self.jit.borrow().is_defined(method_rid)
    }

    /// Wrap a JIT with a custom threshold (0 compiles on the first call).
    pub fn with_threshold(jit: Jit, module: &'a Module, threshold: u32) -> Self {
        Self {
            jit: RefCell::new(jit),
            module,
            counts: RefCell::new(HashMap::new()),
            threshold,
        }
    }
}

impl CallHook for JitHook<'_> {
    fn try_call(&self, method_rid: u32, args: &[Value]) -> Option<Value> {
        // Ineligible methods never go to the JIT.
        if !self.jit.borrow().is_compiled(method_rid) {
            return None;
        }

        // Already-compiled methods always run native; otherwise count and wait
        // until the method is hot before paying to compile it.
        let defined = self.jit.borrow().is_defined(method_rid);
        if !defined {
            let count = {
                let mut counts = self.counts.borrow_mut();
                let c = counts.entry(method_rid).or_insert(0);
                *c += 1;
                *c
            };
            if count < self.threshold {
                return None; // stay interpreted while cold
            }
            self.jit
                .borrow_mut()
                .ensure_compiled(self.module, method_rid)
                .ok()?;
        }

        let jit = self.jit.borrow();
        let kinds = jit.param_kinds(method_rid)?;
        if kinds.len() != args.len() {
            return None;
        }
        let native_roots: Vec<zun_gc::Obj> = args
            .iter()
            .filter_map(|arg| match arg {
                Value::Object(obj) => Some(*obj),
                _ => None,
            })
            .collect();
        // Keep object arguments visible if a future native safepoint requests
        // collection while this JIT frame is active.
        let _roots = unsafe { zun_gc::NativeRootScope::enter(&native_roots) };
        let packed: Vec<u64> = args.iter().zip(kinds).map(|(v, &k)| pack_value(v, k)).collect();
        let result = jit.call_packed(method_rid, &packed).ok()?;
        Some(unpack_value(result, jit.return_kind(method_rid)))
    }
}

/// Marshal a `Value` into a packed `u64` for parameter kind `k`.
fn pack_value(v: &Value, k: JitKind) -> u64 {
    match k {
        JitKind::Num(NumKind::F64) => v.as_number().to_bits(),
        JitKind::Num(NumKind::F32) => (v.as_number() as f32).to_bits() as u64,
        JitKind::Bool => matches!(v, Value::Bool(true)) as u64,
        JitKind::Object => v.as_object().addr() as u64,
        JitKind::Num(_) => v.as_int() as u64,
        JitKind::Other => 0,
    }
}

/// Unmarshal a packed `u64` result into a `Value` of the return kind.
fn unpack_value(bits: u64, ret: Option<JitKind>) -> Value {
    match ret {
        None => Value::Void,
        Some(JitKind::Bool) => Value::Bool(bits != 0),
        Some(JitKind::Object) => Value::Object(unsafe { zun_gc::Obj::from_addr(bits as *mut u64) }),
        Some(JitKind::Num(NumKind::F64)) => Value::Float(f64::from_bits(bits), NumKind::F64),
        Some(JitKind::Num(NumKind::F32)) => {
            Value::Float(f32::from_bits(bits as u32) as f64, NumKind::F32)
        }
        Some(JitKind::Num(n)) => Value::Int(bits as i64, n),
        Some(JitKind::Other) => Value::Void,
    }
}
