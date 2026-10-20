use super::gc_policy::GC_INTERVAL;
use super::{CallHook, Interp, Output, RunError, Value};
use super::state::{ExecutionFrame, ExecutionOutcome};
use zun_module::Module;

/// Result of one poll of a persistent Tier 0 execution session.
#[derive(Debug)]
pub enum ResumeResult {
    Complete(Value),
    Suspended,
}

/// Persistent interpreter boundary for async executions.
///
/// The heap and suspended frame outlive an individual poll, matching the
/// stackless continuation model described by the async runtime design.
pub struct ResumableSession<'a> {
    heap: zun_gc::Heap,
    interp: Interp<'a>,
    method_rid: u32,
    args: Option<Vec<Value>>,
    pending: Option<ExecutionFrame>,
}

impl<'a> ResumableSession<'a> {
    pub fn new(
        module: &'a Module,
        method_rid: u32,
        args: Vec<Value>,
        output: &'a mut dyn Output,
        hook: Option<&'a dyn CallHook>,
        loader: Option<&'a zun_loader::ModuleLoader>,
    ) -> Result<Self, RunError> {
        let types = zun_types::TypeSystem::from_module(module).map_err(|_| RunError::BadToken)?;
        Ok(Self {
            heap: zun_gc::Heap::new(),
            interp: Interp {
                module,
                output,
                hook,
                loader,
                allocs: 0,
                gc_interval: GC_INTERVAL,
                types,
                suspended: None,
                resume_frame: None,
            },
            method_rid,
            args: Some(args),
            pending: None,
        })
    }

    pub fn poll(&mut self) -> Result<ResumeResult, RunError> {
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut self.heap as *mut _) };
        let outcome = if let Some(frame) = self.pending.take() {
            self.interp.resume_suspended(frame)
        } else {
            self.interp.dispatch_outcome(
                self.method_rid,
                self.args.take().unwrap_or_default(),
                None,
            )
        };
        match outcome {
            ExecutionOutcome::Complete(value) => Ok(ResumeResult::Complete(value)),
            ExecutionOutcome::Suspended(frame) => {
                self.pending = Some(frame);
                Ok(ResumeResult::Suspended)
            }
            ExecutionOutcome::Thrown(value) => Err(RunError::Thrown(value)),
            ExecutionOutcome::Fault(error) => Err(error),
        }
    }

    pub fn heap_mut(&mut self) -> &mut zun_gc::Heap {
        &mut self.heap
    }
}

pub(super) fn run_method_with(
    module: &Module,
    method_rid: u32,
    args: Vec<Value>,
    output: &mut dyn Output,
    hook: Option<&dyn CallHook>,
    loader: Option<&zun_loader::ModuleLoader>,
) -> Result<Value, RunError> {
    let mut heap = zun_gc::Heap::new();
    let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
    let types = zun_types::TypeSystem::from_module(module).map_err(|_| RunError::BadToken)?;
    Interp {
        module,
        output,
        hook,
        loader,
        allocs: 0,
        gc_interval: GC_INTERVAL,
        types,
        suspended: None,
        resume_frame: None,
    }
    .call(method_rid, args, None)
}

impl Interp<'_> {
    pub(super) fn propagate_suspended(
        &mut self,
        mut frame: ExecutionFrame,
        caller: ExecutionFrame,
    ) -> RunError {
        frame.parent = Some(Box::new(caller));
        self.suspended = Some(frame);
        RunError::Suspended
    }

    /// Resume a previously suspended frame from its saved program counter.
    /// The scheduler owns the decision about when the awaited Promise settled.
    pub(super) fn resume_suspended(&mut self, mut frame: ExecutionFrame) -> ExecutionOutcome {
        loop {
            let caller = frame.parent.take();
            let method_rid = frame.method_rid;
            self.resume_frame = Some(frame);
            match self.dispatch_outcome(method_rid, Vec::new(), None) {
                ExecutionOutcome::Complete(value) => {
                    let Some(mut caller) = caller else {
                        return ExecutionOutcome::Complete(value);
                    };
                    caller.stack.push(value);
                    frame = *caller;
                }
                ExecutionOutcome::Suspended(mut suspended) => {
                    suspended.parent = caller;
                    return ExecutionOutcome::Suspended(suspended);
                }
                other => return other,
            }
        }
    }

    pub(super) fn dispatch_outcome(
        &mut self,
        method_rid: u32,
        args: Vec<Value>,
        parent: Option<&super::RootLink>,
    ) -> ExecutionOutcome {
        match self.dispatch(method_rid, args, parent) {
            Err(RunError::Suspended) => self
                .suspended
                .take()
                .map(ExecutionOutcome::Suspended)
                .unwrap_or_else(|| {
                    ExecutionOutcome::Fault(RunError::Malformed(
                        "suspended execution did not provide a frame".into(),
                    ))
                }),
            result => ExecutionOutcome::from_result(result),
        }
    }

    /// Route a call to the JIT hook when it handles the method, otherwise
    /// continue with the Tier 0 interpreter.
    pub(super) fn dispatch(
        &mut self,
        method_rid: u32,
        args: Vec<Value>,
        parent: Option<&super::RootLink>,
    ) -> Result<Value, RunError> {
        if let Some(hook) = self.hook {
            if let Some(result) = hook.try_call(method_rid, &args) {
                return Ok(result);
            }
        }
        self.call(method_rid, args, parent)
    }
}
