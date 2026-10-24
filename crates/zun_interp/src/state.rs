use super::{CallHook, Output, Value};
use std::collections::HashMap;
use zun_module::Module;
use zun_gc::Obj;
use zun_sched::{Continuation, ContinuationId, PromiseId, PromiseTable, Scheduler, Settlement};

pub(super) struct Interp<'a> {
    pub(super) module: &'a Module,
    pub(super) output: &'a mut dyn Output,
    pub(super) hook: Option<&'a dyn CallHook>,
    pub(super) loader: Option<&'a zun_loader::ModuleLoader>,
    pub(super) allocs: u32,
    pub(super) gc_interval: u32,
    pub(super) minor_collections: u8,
    pub(super) types: zun_types::TypeSystem,
    pub(super) suspended: Option<ExecutionFrame>,
    pub(super) resume_frame: Option<ExecutionFrame>,
    /// Value produced by a generator yield at the current suspension point.
    pub(super) yielded: Option<Value>,
    pub(super) static_fields: HashMap<(u32, u32), Value>,
    /// Most recent executing frame/program counter, used for source-aware
    /// diagnostics at the public runner boundary.
    pub(super) active_frame: Option<(u32, u32)>,
    /// Live chain of interpreter activations, innermost last. Each entry is the
    /// callee's method rid and its current program counter; the top entry's pc
    /// is advanced as the method executes. This is the frame walk behind
    /// `Error`'s `.stack` capture (PLAN §3.1): a frame carries no managed
    /// references (just two `u32`s), so it needs no GC rooting or relocation.
    pub(super) call_stack: Vec<CallFrame>,
    /// The machine-level async substrate for `await`: the scheduler, its promise
    /// table, and the side table that owns parked interpreter frames keyed by
    /// [`ContinuationId`]. See [`AsyncState`]. `then`/combinators still run on
    /// the old eager-at-settle path (slice 3 migrates them).
    pub(super) async_state: AsyncState,
}

/// The `await` substrate owned by one interpreter session.
///
/// A managed promise (a heap `Obj`) is bridged to a [`PromiseId`] on the
/// scheduler via [`AsyncState::pid_by_obj`]; its settled *value* stays in the
/// language-layer promise cell in `zun_gc` (read back by re-executing `await`),
/// so this substrate holds control/ordering state only. Parked frames live in
/// [`AsyncState::parked`], which is their GC-root owner.
#[derive(Debug, Default)]
pub(super) struct AsyncState {
    pub(super) scheduler: Scheduler,
    pub(super) promises: PromiseTable,
    /// Heap promise address → its scheduler `PromiseId`. Keys are rewritten on a
    /// moving collection so a promise relocated while a frame is parked keeps
    /// its identity.
    pub(super) pid_by_obj: HashMap<usize, PromiseId>,
    /// Parked frames (the control/value state + GC roots), keyed by the id the
    /// scheduler holds. This table is the root owner for a parked `await`.
    pub(super) parked: HashMap<ContinuationId, ParkedFrame>,
    pub(super) next_cont: u32,
}

/// A parked interpreter frame and the scheduler promise whose settlement wakes
/// it.
#[derive(Debug)]
pub(super) struct ParkedFrame {
    pub(super) frame: ExecutionFrame,
    pub(super) pid: PromiseId,
}

impl AsyncState {
    /// The scheduler `PromiseId` bridged to a managed promise `Obj`, creating it
    /// on first use.
    fn pid_for(&mut self, promise: Obj) -> PromiseId {
        let addr = promise.addr() as usize;
        match self.pid_by_obj.get(&addr) {
            Some(pid) => *pid,
            None => {
                let pid = self.promises.create();
                self.pid_by_obj.insert(addr, pid);
                pid
            }
        }
    }
}

/// One live interpreter activation, used for runtime stack capture.
#[derive(Debug, Clone, Copy)]
pub(super) struct CallFrame {
    pub(super) method_rid: u32,
    pub(super) pc: u32,
}

impl<'a> Interp<'a> {
    pub(super) fn annotate_error(&self, error: super::RunError) -> super::RunError {
        match self.active_frame {
            Some((method, pc)) => error.annotate(self.module, method, pc),
            None => error,
        }
    }

    /// Rewrite owned suspended frames after a moving collection.
    pub(super) fn rewrite_suspended_frames(
        &mut self,
        relocation: &zun_gc::RelocationMap,
    ) -> usize {
        let mut rewritten = 0;
        if let Some(frame) = self.suspended.as_mut() {
            rewritten += frame.rewrite_managed_references(relocation);
        }
        if let Some(frame) = self.resume_frame.as_mut() {
            rewritten += frame.rewrite_managed_references(relocation);
        }
        rewritten += self.rewrite_parked_frames(relocation);
        rewritten
    }

    /// Rewrite every parked `await` frame and the promise→id bridge after a
    /// moving collection, so a frame that was parked across the collection and
    /// the identity of the promise it waits on both survive relocation.
    pub(super) fn rewrite_parked_frames(
        &mut self,
        relocation: &zun_gc::RelocationMap,
    ) -> usize {
        let mut rewritten = 0;
        for parked in self.async_state.parked.values_mut() {
            rewritten += parked.frame.rewrite_managed_references(relocation);
        }
        // Keys in the promise→id bridge are heap addresses, which move with the
        // objects they name.
        if !self.async_state.pid_by_obj.is_empty() {
            let old: Vec<(usize, PromiseId)> = self.async_state.pid_by_obj.drain().collect();
            for (addr, pid) in old {
                let obj = unsafe { Obj::from_addr(addr as *mut u64) };
                let relocated = relocation.relocate(obj).addr() as usize;
                self.async_state.pid_by_obj.insert(relocated, pid);
            }
        }
        rewritten
    }

    /// Append the GC roots held by every parked `await` frame. These frames are
    /// owned by [`AsyncState::parked`] and are rooted from here while parked,
    /// exactly as the actively suspended frame is.
    pub(super) fn append_parked_roots(&self, roots: &mut Vec<Obj>) {
        for parked in self.async_state.parked.values() {
            parked.frame.append_managed_roots(roots);
        }
    }

    /// Park an `await`-suspended frame on the scheduler and store it in the
    /// frame side table, returning its continuation id. The scheduler holds only
    /// the id; this table owns the frame (and its roots).
    pub(super) fn park_suspended(
        &mut self,
        frame: ExecutionFrame,
    ) -> Result<ContinuationId, super::RunError> {
        let promise = match frame.state {
            FrameState::Suspended { promise, .. } => promise,
            FrameState::Runnable => {
                return Err(super::RunError::Malformed(
                    "parked a frame that is not awaiting a promise".into(),
                ))
            }
        };
        let pid = self.async_state.pid_for(promise);
        let cid = ContinuationId(self.async_state.next_cont);
        self.async_state.next_cont += 1;
        let state = &mut self.async_state;
        state
            .scheduler
            .park(&mut state.promises, Continuation::new(cid, pid));
        state.parked.insert(cid, ParkedFrame { frame, pid });
        Ok(cid)
    }

    /// Drive the scheduler from the current heap settlement of each parked
    /// promise: a promise the host (or an earlier turn) settled on the heap now
    /// settles its scheduler `PromiseId`, which wakes the parked continuation.
    ///
    /// This is the transitional bridge — the heap promise cell remains the
    /// settlement source of truth (the old `then` path reads it too), and the
    /// scheduler learns of a settlement here. Slice 3 routes settlement through
    /// the scheduler directly.
    pub(super) fn reconcile_settlements(&mut self) {
        let mut to_settle: Vec<(PromiseId, Settlement)> = Vec::new();
        for parked in self.async_state.parked.values() {
            if let FrameState::Suspended { promise, .. } = parked.frame.state {
                let settlement = zun_gc::with_current_heap(|heap| heap.promise_state(promise));
                match settlement {
                    Some(zun_core::ManagedPromiseState::Resolved) => {
                        to_settle.push((parked.pid, Settlement::Fulfilled))
                    }
                    Some(zun_core::ManagedPromiseState::Rejected) => {
                        to_settle.push((parked.pid, Settlement::Rejected))
                    }
                    _ => {}
                }
            }
        }
        let state = &mut self.async_state;
        for (pid, settlement) in to_settle {
            state.scheduler.settle(&mut state.promises, pid, settlement);
        }
    }

    /// Take the next frame the scheduler reports runnable, removing it from the
    /// parked side table. Returns `None` when nothing is runnable yet.
    pub(super) fn next_runnable_frame(&mut self) -> Option<ExecutionFrame> {
        while let Some(ran) = self.async_state.scheduler.next() {
            if let zun_sched::Ran::Continuation(cid) = ran {
                if let Some(parked) = self.async_state.parked.remove(&cid) {
                    return Some(parked.frame);
                }
            }
        }
        None
    }

    /// The promise the single parked `await` frame is waiting on, if any.
    pub(super) fn parked_awaited_promise(&self) -> Option<Obj> {
        self.async_state
            .parked
            .values()
            .find_map(|parked| match parked.frame.state {
                FrameState::Suspended { promise, .. } => Some(promise),
                FrameState::Runnable => None,
            })
    }

    /// Compact the heap while suspended interpreter frames are owned and
    /// stopped, rewriting their managed values during the forwarding pass.
    ///
    /// # Safety
    /// The caller must ensure no managed execution is active while this
    /// operation runs. The current interpreter is single-threaded, so this is
    /// an explicit opt-in boundary rather than an automatic GC policy.
    pub(super) unsafe fn compact_suspended_frames(
        &mut self,
    ) -> Option<zun_gc::CompactionResult> {
        let mut roots = Vec::new();
        if let Some(frame) = self.suspended.as_ref() {
            frame.append_managed_roots(&mut roots);
        }
        if let Some(frame) = self.resume_frame.as_ref() {
            frame.append_managed_roots(&mut roots);
        }
        if roots.is_empty() {
            return None;
        }

        let types = self.types.clone();
        let mut rewrite = |relocation: &zun_gc::RelocationMap| {
            self.rewrite_suspended_frames(relocation);
        };
        Some(zun_gc::with_current_heap_mut(|heap| unsafe {
            heap.compact_with_relocation(
                &mut roots,
                &|type_rid| types.reference_slots(type_rid),
                &mut rewrite,
            )
        }))
    }
}

/// A link in the chain of suspended caller frames, used to find precise GC
/// roots.
pub(super) struct RootLink<'a> {
    pub(super) stack: *mut [Value],
    pub(super) locals: *mut [Value],
    pub(super) parent: Option<&'a RootLink<'a>>,
}

impl<'a> RootLink<'a> {
    /// Read the frame roots while the linked activation remains live.
    pub(super) unsafe fn append_roots(&self, roots: &mut Vec<Obj>) {
        unsafe {
            super::roots::collect_frame_roots(&*self.stack, &*self.locals, roots);
        }
        if let Some(parent) = self.parent {
            unsafe { parent.append_roots(roots) };
        }
    }

    /// Rewrite every linked activation after a moving collection.
    pub(super) unsafe fn rewrite_managed_references(
        &self,
        relocation: &zun_gc::RelocationMap,
    ) -> usize {
        let mut rewritten = 0;
        unsafe {
            rewritten += Value::rewrite_managed_references(&mut *self.stack, relocation);
            rewritten += Value::rewrite_managed_references(&mut *self.locals, relocation);
        }
        if let Some(parent) = self.parent {
            rewritten += unsafe { parent.rewrite_managed_references(relocation) };
        }
        rewritten
    }
}

/// Explicit interpreter activation record. Keeping the resumable pieces
/// together is the first step toward async suspension; the synchronous loop
/// still owns and advances one frame at a time for now.
#[derive(Debug)]
pub(super) struct ExecutionFrame {
    pub(super) method_rid: u32,
    pub(super) pc: usize,
    pub(super) stack: Vec<Value>,
    pub(super) locals: Vec<Value>,
    pub(super) state: FrameState,
    pub(super) parent: Option<Box<ExecutionFrame>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameState {
    Runnable,
    Suspended { promise: Obj, resume_pc: usize },
}

/// Internal outcome used by the resumable interpreter boundary.
pub(super) enum ExecutionOutcome {
    Complete(Value),
    Suspended(ExecutionFrame),
    Thrown(Value),
    Fault(super::RunError),
}

impl ExecutionOutcome {
    pub(super) fn from_result(result: Result<Value, super::RunError>) -> Self {
        match result {
            Ok(value) => Self::Complete(value),
            Err(super::RunError::Thrown(value)) => Self::Thrown(value),
            Err(error) => Self::Fault(error),
        }
    }

    pub(super) fn into_result(self) -> Result<Value, super::RunError> {
        match self {
            Self::Complete(value) => Ok(value),
            Self::Thrown(value) => Err(super::RunError::Thrown(value)),
            Self::Fault(error) => Err(error),
            Self::Suspended(_) => Err(super::RunError::Malformed(
                "suspended execution cannot cross the synchronous boundary".into(),
            )),
        }
    }
}

impl ExecutionFrame {
    pub(super) fn new(method_rid: u32, locals: Vec<Value>) -> Self {
        Self {
            method_rid,
            pc: 0,
            stack: Vec::new(),
            locals,
            state: FrameState::Runnable,
            parent: None,
        }
    }

    pub(super) fn snapshot(method_rid: u32, pc: usize, stack: &[Value], locals: &[Value]) -> Self {
        Self {
            method_rid,
            pc,
            stack: stack.to_vec(),
            locals: locals.to_vec(),
            state: FrameState::Runnable,
            parent: None,
        }
    }

    pub(super) fn suspend(&mut self, promise: Obj, resume_pc: usize) {
        self.state = FrameState::Suspended { promise, resume_pc };
    }

    pub(super) fn resume(&mut self) -> Option<Obj> {
        let FrameState::Suspended { promise, resume_pc } = self.state else {
            return None;
        };
        self.pc = resume_pc;
        self.state = FrameState::Runnable;
        Some(promise)
    }

    /// Rewrite all managed values in this owned frame and its caller chain.
    pub(super) fn rewrite_managed_references(
        &mut self,
        relocation: &zun_gc::RelocationMap,
    ) -> usize {
        let mut rewritten = Value::rewrite_managed_references(&mut self.stack, relocation);
        rewritten += Value::rewrite_managed_references(&mut self.locals, relocation);
        if let FrameState::Suspended { promise, .. } = &mut self.state {
            let relocated = relocation.relocate(*promise);
            if relocated != *promise {
                *promise = relocated;
                rewritten += 1;
            }
        }
        if let Some(parent) = self.parent.as_mut() {
            rewritten += parent.rewrite_managed_references(relocation);
        }
        rewritten
    }

    pub(super) fn append_managed_roots(&self, roots: &mut Vec<Obj>) {
        super::roots::collect_frame_roots(&self.stack, &self.locals, roots);
        if let FrameState::Suspended { promise, .. } = self.state {
            roots.push(promise);
        }
        if let Some(parent) = self.parent.as_ref() {
            parent.append_managed_roots(roots);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RunError;

    #[test]
    fn frame_preserves_resume_pc_and_promise_identity() {
        let promise = unsafe { Obj::from_addr(0x1000usize as *mut u64) };
        let mut frame = ExecutionFrame::new(7, vec![Value::Void]);
        frame.suspend(promise, 42);
        assert_eq!(frame.state, FrameState::Suspended { promise, resume_pc: 42 });
        assert_eq!(frame.resume(), Some(promise));
        assert_eq!(frame.pc, 42);
        assert_eq!(frame.state, FrameState::Runnable);
    }

    #[test]
    fn execution_outcome_can_own_a_suspended_frame() {
        let frame = ExecutionFrame::new(3, vec![Value::Void]);
        let outcome = ExecutionOutcome::Suspended(frame);
        assert!(matches!(outcome, ExecutionOutcome::Suspended(_)));
    }

    #[test]
    fn execution_outcome_adapts_existing_sync_results() {
        let outcome = ExecutionOutcome::from_result(Ok(Value::Bool(true)));
        assert!(matches!(outcome, ExecutionOutcome::Complete(Value::Bool(true))));
        let result = ExecutionOutcome::from_result(Err(RunError::BadToken)).into_result();
        assert!(matches!(result, Err(RunError::BadToken)));
    }

    #[test]
    fn moving_collection_rewrites_owned_frame_chains() {
        let old = unsafe { Obj::from_addr(0x1000usize as *mut u64) };
        let new = unsafe { Obj::from_addr(0x2000usize as *mut u64) };
        let mut relocation = zun_gc::RelocationMap::new();
        relocation.insert(old, new);

        let mut frame = ExecutionFrame::new(1, vec![Value::Object(old)]);
        frame.stack.push(Value::Any(zun_core::Any::object(0x1000)));
        frame.parent = Some(Box::new(ExecutionFrame::new(2, vec![Value::Object(old)])));

        assert_eq!(frame.rewrite_managed_references(&relocation), 3);
        assert!(matches!(frame.locals[0], Value::Object(object) if object == new));
        assert!(matches!(frame.stack[0], Value::Any(any) if any.as_object() == Some(0x2000)));
        assert!(matches!(frame.parent.as_ref().unwrap().locals[0], Value::Object(object) if object == new));
    }

    #[test]
    fn moving_collection_rewrites_suspended_promise() {
        let old = unsafe { Obj::from_addr(0x3000usize as *mut u64) };
        let new = unsafe { Obj::from_addr(0x4000usize as *mut u64) };
        let mut relocation = zun_gc::RelocationMap::new();
        relocation.insert(old, new);
        let mut frame = ExecutionFrame::new(1, vec![]);
        frame.suspend(old, 9);

        let mut roots = Vec::new();
        frame.append_managed_roots(&mut roots);
        assert_eq!(roots, vec![old]);
        assert_eq!(frame.rewrite_managed_references(&relocation), 1);
        assert!(matches!(frame.state, FrameState::Suspended { promise, .. } if promise == new));
    }
}
