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
}
