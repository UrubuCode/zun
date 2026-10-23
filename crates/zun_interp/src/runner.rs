use super::gc_policy::GC_INTERVAL;
use super::{CallHook, Interp, Output, RunError, Value};
use super::state::{ExecutionFrame, ExecutionOutcome};
use zun_il::{decode_one, Opcode};
use zun_module::{read_body, Module};

/// Result of one poll of a persistent Tier 0 execution session.
#[derive(Debug)]
pub enum ResumeResult {
    Complete(Value),
    Yielded(Value),
    Suspended,
}

/// Result of advancing a synchronous generator.
#[derive(Debug)]
pub enum GeneratorResult {
    /// The generator produced one value and can be resumed.
    Yielded(Value),
    /// The generator returned and cannot produce more values.
    Complete(Value),
}

/// Iterator-shaped result returned by a generator `next()` operation.
///
/// Keeping the terminal return value alongside `done` is important for the
/// JavaScript iterator contract: the value yielded before completion and the
/// value returned by the generator are distinct observations.
#[derive(Clone, Debug)]
pub struct GeneratorIteratorResult {
    pub value: Value,
    pub done: bool,
}

/// Owned Tier 0 generator state. Host callbacks are supplied only for the
/// duration of `resume`, so no borrowed execution context is retained.
#[derive(Debug)]
pub struct OwnedGeneratorSession<'a> {
    module: &'a Module,
    heap: zun_gc::Heap,
    types: zun_types::TypeSystem,
    method_rid: u32,
    args: Option<Vec<Value>>,
    pending: Option<ExecutionFrame>,
}

impl<'a> OwnedGeneratorSession<'a> {
    pub fn new(module: &'a Module, method_rid: u32, args: Vec<Value>) -> Result<Self, RunError> {
        let types = zun_types::TypeSystem::from_module(module).map_err(|_| RunError::BadToken)?;
        Ok(Self { module, heap: zun_gc::Heap::new(), types, method_rid, args: Some(args), pending: None })
    }

    pub fn resume(
        &mut self,
        output: &mut dyn Output,
        hook: Option<&dyn CallHook>,
        loader: Option<&zun_loader::ModuleLoader>,
    ) -> Result<ResumeResult, RunError> {
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut self.heap as *mut _) };
        let mut interp = Interp {
            module: self.module,
            output,
            hook,
            loader,
            allocs: 0,
            gc_interval: GC_INTERVAL,
            minor_collections: 0,
            types: self.types.clone(),
            suspended: None,
            resume_frame: None,
            yielded: None,
            static_fields: std::collections::HashMap::new(),
            active_frame: None,
        };
        let outcome = if let Some(frame) = self.pending.take() {
            interp.resume_suspended(frame)
        } else {
            interp.dispatch_outcome(self.method_rid, self.args.take().unwrap_or_default(), None)
        };
        match outcome {
            ExecutionOutcome::Complete(value) => {
                interp.deliver_finalization_callbacks()?;
                Ok(ResumeResult::Complete(value))
            }
            ExecutionOutcome::Suspended(frame) => {
                self.pending = Some(frame);
                if let Some(value) = interp.yielded.take() {
                    Ok(ResumeResult::Yielded(value))
                } else {
                    Ok(ResumeResult::Suspended)
                }
            }
            ExecutionOutcome::Thrown(value) => Err(RunError::Thrown(value)),
            ExecutionOutcome::Fault(error) => Err(interp.annotate_error(error)),
        }
    }

    pub fn heap_mut(&mut self) -> &mut zun_gc::Heap { &mut self.heap }

    /// Compact the owned generator heap between resume steps.
    ///
    /// This is an explicit maintenance operation. It is safe to use only
    /// while the session is not executing a resume step.
    pub fn compact_pending_frames(&mut self) -> Option<zun_gc::CompactionResult> {
        let mut frame = self.pending.take()?;
        let mut roots = Vec::new();
        frame.append_managed_roots(&mut roots);
        let types = self.types.clone();
        let mut rewrite = |relocation: &zun_gc::RelocationMap| {
            frame.rewrite_managed_references(relocation);
        };
        let result = unsafe {
            self.heap.compact_with_relocation(
                &mut roots,
                &|type_rid| types.reference_slots(type_rid),
                &mut rewrite,
            )
        };
        self.pending = Some(frame);
        Some(result)
    }

    /// Collect unreachable objects and compact the owned generator heap
    /// between iterator steps.
    pub fn collect_and_compact_pending_frames(
        &mut self,
    ) -> Option<zun_gc::CollectionCompactionResult> {
        let mut frame = self.pending.take()?;
        let mut roots = Vec::new();
        frame.append_managed_roots(&mut roots);
        let types = self.types.clone();
        let mut rewrite = |relocation: &zun_gc::RelocationMap| {
            frame.rewrite_managed_references(relocation);
        };
        let result = unsafe {
            self.heap.collect_and_compact_with_relocation(
                &mut roots,
                &|type_rid| types.reference_slots(type_rid),
                &mut rewrite,
            )
        };
        self.pending = Some(frame);
        Some(result)
    }
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

/// Tier 0 generator boundary with an iterator-shaped next operation.
pub struct GeneratorSession<'a> {
    owned: OwnedGeneratorSession<'a>,
    done: bool,
    current: Option<Value>,
    completion: Option<Value>,
}

impl core::fmt::Debug for GeneratorSession<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("GeneratorSession")
            .field("method_rid", &self.owned.method_rid)
            .field("suspended", &self.owned.pending.is_some())
            .field("done", &self.done)
            .finish()
    }
}

impl<'a> GeneratorSession<'a> {
    /// Create a generator session for a checked generator method.
    pub fn new(
        module: &'a Module,
        method_rid: u32,
        args: Vec<Value>,
        _output: &mut dyn Output,
    ) -> Result<Self, RunError> {
        Self::new_with_context(module, method_rid, args, _output, None, None)
    }

    /// Create a generator while preserving the host/JIT context used by its
    /// body.  A generator is resumed by a different call to `next`, but its
    /// native hook and module loader are part of the state-machine boundary,
    /// not per-poll state.
    pub fn new_with_context(
        module: &'a Module,
        method_rid: u32,
        args: Vec<Value>,
        _output: &mut dyn Output,
        _hook: Option<&dyn CallHook>,
        _loader: Option<&zun_loader::ModuleLoader>,
    ) -> Result<Self, RunError> {
        let method = module
            .tables
            .method_defs
            .get(method_rid as usize - 1)
            .ok_or(RunError::BadToken)?;
        let body = read_body(&module.code, method.body_offset)
            .map_err(|error| RunError::Malformed(error.to_string()))?;
        let mut offset = 0;
        let mut is_generator = false;
        while offset < body.code.len() {
            let (instruction, size) = decode_one(&body.code[offset..])
                .map_err(|error| RunError::Malformed(error.to_string()))?;
            is_generator |= instruction.opcode == Opcode::Yield;
            offset += size;
        }
        if !is_generator {
            return Err(RunError::Malformed(
                "generator session requires a method containing yield".into(),
            ));
        }
        Ok(Self {
            owned: OwnedGeneratorSession::new(module, method_rid, args)?,
            done: false,
            current: None,
            completion: None,
        })
    }

    /// Advance the generator once.
    pub fn next(&mut self) -> Result<GeneratorResult, RunError> {
        if self.done {
            return Ok(GeneratorResult::Complete(
                self.completion
                    .clone()
                    .expect("completed generator stores its result"),
            ));
        }
        let mut output = super::CaptureOutput::default();
        match self.owned.resume(&mut output, None, None)? {
            ResumeResult::Yielded(value) => {
                self.current = Some(value.clone());
                Ok(GeneratorResult::Yielded(value))
            }
            ResumeResult::Complete(value) => {
                self.done = true;
                self.current = None;
                self.completion = Some(value.clone());
                Ok(GeneratorResult::Complete(value))
            }
            ResumeResult::Suspended => Err(RunError::Malformed(
                "generator suspended without yielding a value".into(),
            )),
        }
    }

    /// Advance the generator using the source-level iterator result shape.
    pub fn next_result(&mut self) -> Result<GeneratorIteratorResult, RunError> {
        match self.next()? {
            GeneratorResult::Yielded(value) => Ok(GeneratorIteratorResult {
                value,
                done: false,
            }),
            GeneratorResult::Complete(value) => Ok(GeneratorIteratorResult { value, done: true }),
        }
    }

    /// Advance through the owned state-machine path. Host context is borrowed
    /// only for this resume and is never retained by the generator state.
    pub fn next_with_context(
        &mut self,
        output: &mut dyn Output,
        hook: Option<&dyn CallHook>,
        loader: Option<&zun_loader::ModuleLoader>,
    ) -> Result<GeneratorResult, RunError> {
        if self.done {
            return Ok(GeneratorResult::Complete(
                self.completion.clone().expect("completed generator stores its result"),
            ));
        }
        match self.owned.resume(output, hook, loader)? {
            ResumeResult::Yielded(value) => {
                self.current = Some(value.clone());
                Ok(GeneratorResult::Yielded(value))
            }
            ResumeResult::Complete(value) => {
                self.done = true;
                self.current = None;
                self.completion = Some(value.clone());
                Ok(GeneratorResult::Complete(value))
            }
            ResumeResult::Suspended => Err(RunError::Malformed(
                "generator suspended without yielding a value".into(),
            )),
        }
    }

    pub fn next_result_with_context(
        &mut self,
        output: &mut dyn Output,
        hook: Option<&dyn CallHook>,
        loader: Option<&zun_loader::ModuleLoader>,
    ) -> Result<GeneratorIteratorResult, RunError> {
        match self.next_with_context(output, hook, loader)? {
            GeneratorResult::Yielded(value) => Ok(GeneratorIteratorResult { value, done: false }),
            GeneratorResult::Complete(value) => Ok(GeneratorIteratorResult { value, done: true }),
        }
    }

    /// Access the generator heap for host integration.
    pub fn heap_mut(&mut self) -> &mut zun_gc::Heap {
        self.owned.heap_mut()
    }

    /// Compact the generator's owned heap between iterator steps.
    pub fn compact_pending_frames(&mut self) -> Option<zun_gc::CompactionResult> {
        self.owned.compact_pending_frames()
    }

    /// Collect unreachable objects and compact between iterator steps.
    pub fn collect_and_compact_pending_frames(
        &mut self,
    ) -> Option<zun_gc::CollectionCompactionResult> {
        self.owned.collect_and_compact_pending_frames()
    }

    /// Return the value produced by the most recent successful `next` call.
    /// The value is cleared once the generator completes, matching the
    /// iterator contract described by the runtime design.
    pub fn current(&self) -> Option<&Value> {
        self.current.as_ref()
    }

    /// Report whether the generator has reached its terminal result.
    pub fn is_done(&self) -> bool {
        self.done
    }
}

impl Iterator for GeneratorSession<'_> {
    type Item = Value;

    fn next(&mut self) -> Option<Self::Item> {
        match GeneratorSession::next(self).ok()? {
            GeneratorResult::Yielded(value) => Some(value),
            GeneratorResult::Complete(_) => None,
        }
    }
}

impl core::fmt::Debug for ResumableSession<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ResumableSession")
            .field("method_rid", &self.method_rid)
            .field("has_arguments", &self.args.is_some())
            .field("suspended", &self.pending.is_some())
            .finish()
    }
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
                minor_collections: 0,
                types,
                suspended: None,
                resume_frame: None,
                yielded: None,
                static_fields: std::collections::HashMap::new(),
                active_frame: None,
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
            ExecutionOutcome::Complete(value) => {
                self.interp.deliver_finalization_callbacks()?;
                Ok(ResumeResult::Complete(value))
            }
            ExecutionOutcome::Suspended(frame) => {
                self.pending = Some(frame);
                if let Some(value) = self.interp.yielded.take() {
                    Ok(ResumeResult::Yielded(value))
                } else {
                    Ok(ResumeResult::Suspended)
                }
            }
            ExecutionOutcome::Thrown(value) => Err(RunError::Thrown(value)),
            ExecutionOutcome::Fault(error) => Err(error),
        }
    }

    pub fn heap_mut(&mut self) -> &mut zun_gc::Heap {
        &mut self.heap
    }

    /// Collect unreachable objects and compact the owned async frame between
    /// poll steps.
    pub fn collect_and_compact_pending_frame(
        &mut self,
    ) -> Option<zun_gc::CollectionCompactionResult> {
        let mut frame = self.pending.take()?;
        let mut roots = Vec::new();
        frame.append_managed_roots(&mut roots);
        let types = self.interp.types.clone();
        let mut rewrite = |relocation: &zun_gc::RelocationMap| {
            frame.rewrite_managed_references(relocation);
        };
        let result = unsafe {
            self.heap.collect_and_compact_with_relocation(
                &mut roots,
                &|type_rid| types.reference_slots(type_rid),
                &mut rewrite,
            )
        };
        self.pending = Some(frame);
        Some(result)
    }

    pub fn awaited_promise(&self) -> Option<zun_gc::Obj> {
        self.pending.as_ref().and_then(|frame| match frame.state {
            super::state::FrameState::Suspended { promise, .. } => Some(promise),
            super::state::FrameState::Runnable => None,
        })
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
    let mut interp = Interp {
        module,
        output,
        hook,
        loader,
        allocs: 0,
        gc_interval: GC_INTERVAL,
        minor_collections: 0,
        types,
        suspended: None,
        resume_frame: None,
        yielded: None,
        static_fields: std::collections::HashMap::new(),
        active_frame: None,
    };
    // Run the module initializer once before the entry point (PLAN §3.1, doc 02
    // area 10): it seeds the realm global on this same heap so `globalThis.Map`
    // holds its class descriptor before `main` runs. It runs only when invoking
    // the entry point itself, not for arbitrary host method calls, and an error
    // it raises is surfaced like any other startup error.
    if let (Some(init), Some(entry)) = (module.module_initializer, module.entry_point) {
        if entry.rid() == method_rid && init.rid() != method_rid {
            if let Err(error) = interp.call(init.rid(), Vec::new(), None) {
                return Err(interp.annotate_error(error));
            }
        }
    }
    let result = match interp.call(method_rid, args, None) {
        Ok(value) => value,
        Err(error) => return Err(interp.annotate_error(error)),
    };
    if let Err(error) = interp.deliver_finalization_callbacks() {
        return Err(interp.annotate_error(error));
    }
    Ok(result)
}

impl Interp<'_> {
    /// Deliver callbacks queued by completed GC cycles on this interpreter's
    /// heap. Callback invocation reuses the same interpreter and heap, so
    /// closure environments and managed arguments remain valid.
    fn deliver_finalization_callbacks(&mut self) -> Result<(), RunError> {
        loop {
            let records =
                zun_gc::with_current_heap_mut(|heap| heap.finalization_drain_records());
            if records.is_empty() {
                return Ok(());
            }
            for record in records {
                let Some(callback) = record.callback else {
                    continue;
                };
                let token = Value::Int(record.token as i64, zun_il::NumKind::U64);
                let environment = callback
                    .environment()
                    .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) });
                let mut args = Vec::with_capacity(2);
                if let Some(environment) = environment {
                    args.push(Value::Object(environment));
                }
                args.push(token);
                self.call(callback.method_rid(), args, None)?;
            }
        }
    }

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
            // A JIT allocation may collect while the interpreter caller is
            // parked in this dispatch. Publish the caller chain as native
            // roots for that interval; the active Cranelift stack map covers
            // the JIT frame itself.
            let mut native_roots = Vec::new();
            super::roots::collect_frame_roots(&args, &[], &mut native_roots);
            if let Some(frame) = parent {
                unsafe { frame.append_roots(&mut native_roots) };
            }
            let _native_roots = unsafe { zun_gc::NativeRootScope::enter(&native_roots) };
            if let Some(result) = hook.try_call(method_rid, &args) {
                return Ok(result);
            }
        }
        self.call(method_rid, args, parent)
    }
}
