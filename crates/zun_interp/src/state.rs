use super::{CallHook, Output, Value};
use zun_module::Module;
use zun_gc::Obj;

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
}

impl<'a> Interp<'a> {
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
        rewritten
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
    pub(super) stack: &'a [Value],
    pub(super) locals: &'a [Value],
    pub(super) parent: Option<&'a RootLink<'a>>,
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
