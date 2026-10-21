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

/// A hot loop boundary ready for a future on-stack replacement transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OsrRequest {
    /// Method containing the active Tier 0 frame.
    pub method_rid: u32,
    /// IL program counter identifying the loop target.
    pub target_pc: usize,
}

/// Tier 0 state captured at a hot loop boundary for a future OSR transfer.
#[derive(Debug, Clone)]
pub struct OsrSnapshot {
    pub method_rid: u32,
    pub target_pc: usize,
    pub pc: usize,
    pub stack: Vec<Value>,
    pub locals: Vec<Value>,
}

/// Bridges the interpreter to a lazily-compiled [`Jit`].
pub struct JitHook<'a> {
    jit: RefCell<Jit>,
    module: &'a Module,
    counts: RefCell<HashMap<u32, u32>>,
    loop_counts: RefCell<HashMap<(u32, usize), u64>>,
    osr_requests: RefCell<Vec<OsrRequest>>,
    osr_snapshots: RefCell<Vec<OsrSnapshot>>,
    threshold: u32,
}

impl core::fmt::Debug for JitHook<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("JitHook")
            .field("threshold", &self.threshold)
            .finish()
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
            loop_counts: RefCell::new(HashMap::new()),
            osr_requests: RefCell::new(Vec::new()),
            osr_snapshots: RefCell::new(Vec::new()),
            threshold,
        }
    }

    /// Return the number of interpreted backedges observed at a loop header.
    pub fn loop_count(&self, method_rid: u32, target_pc: usize) -> u64 {
        self.loop_counts
            .borrow()
            .get(&(method_rid, target_pc))
            .copied()
            .unwrap_or(0)
    }

    /// Return all observed backedges for one method, summed across headers.
    pub fn total_loop_backedges(&self, method_rid: u32) -> u64 {
        self.loop_counts
            .borrow()
            .iter()
            .filter(|((rid, _), _)| *rid == method_rid)
            .map(|(_, count)| *count)
            .sum()
    }

    /// Whether a loop header has crossed the same hotness threshold used for
    /// lazy method compilation. This is the decision point an OSR backend can
    /// consume without reaching into the hook's counters.
    pub fn is_loop_hot(&self, method_rid: u32, target_pc: usize) -> bool {
        self.loop_count(method_rid, target_pc) >= u64::from(self.threshold.max(1))
    }

    /// Return the observed loop headers that are currently hot for a method.
    pub fn hot_loop_headers(&self, method_rid: u32) -> Vec<usize> {
        self.loop_counts
            .borrow()
            .iter()
            .filter_map(|(&(rid, target), &count)| {
                (rid == method_rid && count >= u64::from(self.threshold.max(1))).then_some(target)
            })
            .collect()
    }

    /// Drain unique loop-boundary requests that crossed the OSR threshold.
    /// The active frame continues in Tier 0 until frame transfer is available.
    pub fn take_osr_requests(&self) -> Vec<OsrRequest> {
        self.osr_requests.borrow_mut().drain(..).collect()
    }

    /// Drain the latest captured Tier 0 state for hot loop boundaries.
    pub fn take_osr_snapshots(&self) -> Vec<OsrSnapshot> {
        self.osr_snapshots.borrow_mut().drain(..).collect()
    }

    /// Check that a captured snapshot can be mapped back to the method's IL
    /// body before an OSR backend attempts frame replacement.
    pub fn is_valid_osr_snapshot(&self, snapshot: &OsrSnapshot) -> bool {
        if snapshot.method_rid == 0 || snapshot.target_pc > snapshot.pc {
            return false;
        }
        let Some(method) = self
            .module
            .tables
            .method_defs
            .get(snapshot.method_rid as usize - 1)
        else {
            return false;
        };
        let Ok(body) = zun_module::read_body(&self.module.code, method.body_offset) else {
            return false;
        };
        if snapshot.locals.len() != body.local_count as usize {
            return false;
        }
        let Ok(instrs) = zun_il::decode_all(body.code) else {
            return false;
        };
        let mut offset = 0usize;
        let mut boundaries = std::collections::HashSet::new();
        for instr in instrs {
            boundaries.insert(offset);
            offset += instr.encoded_len();
        }
        boundaries.insert(offset);
        boundaries.contains(&snapshot.target_pc) && boundaries.contains(&snapshot.pc)
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
        let packed: Vec<u64> = args
            .iter()
            .zip(kinds)
            .map(|(v, &k)| pack_value(v, k))
            .collect();
        let heap = zun_gc::current_heap_ptr()?;
        let result = unsafe { jit.call_packed_with_heap(method_rid, &packed, &mut *heap).ok()? };
        Some(unpack_value(result, jit.return_kind(method_rid)))
    }

    fn on_backedge(&self, method_rid: u32, target_pc: usize) {
        let count = {
            let mut counts = self.loop_counts.borrow_mut();
            let count = counts.entry((method_rid, target_pc)).or_insert(0);
            *count += 1;
            *count
        };

        // Compile eligible methods as soon as an interpreted loop proves that
        // they are hot. The current frame continues in Tier 0; a future OSR
        // bridge can consume the already-defined native body at this point.
        if count >= u64::from(self.threshold.max(1))
            && self.jit.borrow().is_compiled(method_rid)
            && !self.jit.borrow().is_defined(method_rid)
        {
            let _ = self
                .jit
                .borrow_mut()
                .ensure_compiled(self.module, method_rid);
        }

        if count >= u64::from(self.threshold.max(1))
            && self.jit.borrow().is_compiled(method_rid)
            && self.jit.borrow().is_defined(method_rid)
        {
            let mut requests = self.osr_requests.borrow_mut();
            if !requests
                .iter()
                .any(|request| request.method_rid == method_rid && request.target_pc == target_pc)
            {
                requests.push(OsrRequest { method_rid, target_pc });
            }
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
        self.on_backedge(method_rid, target_pc);
        if self.is_loop_hot(method_rid, target_pc)
            && self.jit.borrow().is_defined(method_rid)
        {
            let mut snapshots = self.osr_snapshots.borrow_mut();
            snapshots.retain(|snapshot| {
                snapshot.method_rid != method_rid || snapshot.target_pc != target_pc
            });
            snapshots.push(OsrSnapshot {
                method_rid,
                target_pc,
                pc,
                stack: stack.to_vec(),
                locals: locals.to_vec(),
            });
        }
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
