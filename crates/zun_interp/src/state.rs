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
    pub(super) types: zun_types::TypeSystem,
    pub(super) suspended: Option<ExecutionFrame>,
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
pub(super) struct ExecutionFrame {
    pub(super) method_rid: u32,
    pub(super) pc: usize,
    pub(super) stack: Vec<Value>,
    pub(super) locals: Vec<Value>,
    pub(super) state: FrameState,
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
}
