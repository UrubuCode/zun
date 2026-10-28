use super::gc_policy::GC_INTERVAL;
use super::{CallHook, Interp, Output, RunError, Value};
use super::state::{AsyncState, ExecutionFrame, ExecutionOutcome, Microstep};
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
            eval_host: None,
            eval_cache: std::collections::HashMap::new(),
            allocs: 0,
            gc_interval: GC_INTERVAL,
            minor_collections: 0,
            types: self.types.clone(),
            suspended: None,
            resume_frame: None,
            yielded: None,
            class_symbol_methods: std::collections::HashMap::new(),
            class_named_methods: std::collections::HashMap::new(),
            class_field_defaults: std::collections::HashMap::new(),
            decorator_initializers: Vec::new(),
            instance_initializers: Vec::new(),
            field_transforms: Vec::new(),
            active_frame: None,
            call_stack: Vec::new(),
            async_state: AsyncState::default(),
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
    /// Whether the entry method has been dispatched once. After that, a poll
    /// resumes a woken `await` continuation from the scheduler (or a yielded
    /// frame via `pending`) rather than starting the body again.
    started: bool,
    /// Transport for a yield-style suspension (frame left `Runnable`). `await`
    /// suspensions are parked on the interpreter's scheduler instead.
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
                eval_host: None,
                eval_cache: std::collections::HashMap::new(),
                allocs: 0,
                gc_interval: GC_INTERVAL,
                minor_collections: 0,
                types,
                suspended: None,
                resume_frame: None,
                yielded: None,
                class_symbol_methods: std::collections::HashMap::new(),
                class_named_methods: std::collections::HashMap::new(),
                class_field_defaults: std::collections::HashMap::new(),
                decorator_initializers: Vec::new(),
                instance_initializers: Vec::new(),
                field_transforms: Vec::new(),
                active_frame: None,
                call_stack: Vec::new(),
                async_state: AsyncState::default(),
            },
            method_rid,
            args: Some(args),
            started: false,
            pending: None,
        })
    }

    pub fn poll(&mut self) -> Result<ResumeResult, RunError> {
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut self.heap as *mut _) };
        let outcome = if let Some(frame) = self.pending.take() {
            // A yield-style suspension resumes directly.
            self.interp.resume_suspended(frame)
        } else if !self.started {
            self.started = true;
            self.interp.dispatch_outcome(
                self.method_rid,
                self.args.take().unwrap_or_default(),
                None,
            )
        } else {
            // Drive the scheduler: run queued reactions (as microtasks) and
            // resume woken `await` continuations. A promise the host settled on
            // the heap is picked up by the reconcile pass inside `next_microstep`.
            // If nothing is runnable yet, the session stays suspended.
            loop {
                match self.interp.next_microstep()? {
                    Microstep::RanReaction => continue,
                    Microstep::Frame(frame) => break self.interp.resume_suspended(frame),
                    Microstep::Idle => {
                        // Microtasks are idle: fire the earliest-due timer to
                        // make progress on the unified loop. Only when no timer
                        // remains either does this poll stay suspended.
                        if self.interp.fire_one_timer()? {
                            continue;
                        }
                        return Ok(ResumeResult::Suspended);
                    }
                }
            }
        };
        match outcome {
            ExecutionOutcome::Complete(value) => {
                // Flush reactions and timers queued by the completed body before
                // reporting completion, matching the turn-drain of a plain
                // `run()` (the full unified loop).
                self.interp.run_event_loop()?;
                self.interp.deliver_finalization_callbacks()?;
                Ok(ResumeResult::Complete(value))
            }
            ExecutionOutcome::Suspended(frame) => {
                if let Some(value) = self.interp.yielded.take() {
                    // A yield: keep the frame on the direct-resume transport.
                    self.pending = Some(frame);
                    Ok(ResumeResult::Yielded(value))
                } else {
                    // An `await` on a pending promise: park on the scheduler.
                    self.interp.park_suspended(frame)?;
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

    /// Collect unreachable objects and compact the owned async frame(s) between
    /// poll steps. The parked `await` frames (root owner) and any yield-style
    /// `pending` frame are rooted and rewritten together.
    pub fn collect_and_compact_pending_frame(
        &mut self,
    ) -> Option<zun_gc::CollectionCompactionResult> {
        let mut roots = Vec::new();
        self.interp.append_parked_roots(&mut roots);
        if let Some(frame) = self.pending.as_ref() {
            frame.append_managed_roots(&mut roots);
        }
        if roots.is_empty() {
            return None;
        }
        let types = self.interp.types.clone();
        let interp = &mut self.interp;
        let pending = &mut self.pending;
        let mut rewrite = |relocation: &zun_gc::RelocationMap| {
            interp.rewrite_parked_frames(relocation);
            if let Some(frame) = pending.as_mut() {
                frame.rewrite_managed_references(relocation);
            }
        };
        let result = unsafe {
            self.heap.collect_and_compact_with_relocation(
                &mut roots,
                &|type_rid| types.reference_slots(type_rid),
                &mut rewrite,
            )
        };
        Some(result)
    }

    pub fn awaited_promise(&self) -> Option<zun_gc::Obj> {
        self.interp.parked_awaited_promise().or_else(|| {
            self.pending.as_ref().and_then(|frame| match frame.state {
                super::state::FrameState::Suspended { promise, .. } => Some(promise),
                super::state::FrameState::Runnable => None,
            })
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
    eval_host: Option<&dyn super::EvalHost>,
) -> Result<Value, RunError> {
    let mut heap = zun_gc::Heap::new();
    let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
    let types = zun_types::TypeSystem::from_module(module).map_err(|_| RunError::BadToken)?;
    let mut interp = Interp {
        module,
        output,
        hook,
        loader,
        eval_host,
        eval_cache: std::collections::HashMap::new(),
        allocs: 0,
        gc_interval: GC_INTERVAL,
        minor_collections: 0,
        types,
        suspended: None,
        resume_frame: None,
        yielded: None,
        class_symbol_methods: std::collections::HashMap::new(),
        class_named_methods: std::collections::HashMap::new(),
        class_field_defaults: std::collections::HashMap::new(),
        decorator_initializers: Vec::new(),
        instance_initializers: Vec::new(),
        field_transforms: Vec::new(),
        active_frame: None,
        call_stack: Vec::new(),
        async_state: AsyncState::default(),
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
    // The entry point may contain a top-level `await` on a pending promise, or
    // schedule `then`/combinator reactions: run it, then drain the scheduler to
    // empty (the turn-drain) so reactions queued by reactions run too.
    let first = interp.call(method_rid, args, None);
    let outcome = match first {
        Err(RunError::Suspended) => interp
            .suspended
            .take()
            .map(ExecutionOutcome::Suspended)
            .unwrap_or_else(|| {
                ExecutionOutcome::Fault(RunError::Malformed(
                    "suspended execution did not provide a frame".into(),
                ))
            }),
        other => ExecutionOutcome::from_result(other),
    };
    let result = match interp.drive_to_completion(outcome) {
        Ok(value) => value,
        Err(error) => return Err(interp.annotate_error(error)),
    };
    if let Err(error) = interp.deliver_finalization_callbacks() {
        return Err(interp.annotate_error(error));
    }
    Ok(result)
}

/// Construct an instance of a user class defined in `module` on the **current**
/// heap, mirroring [`run_method_with`] but invoking the class-construction twin
/// of `NewObj` instead of a method body.
///
/// Unlike [`run_method_with`], this does **not** install a fresh heap: the caller
/// must have a current heap installed, because the constructed instance lives on
/// it and is read back by the caller (the dynamic-namespace snapshot path).
///
/// The defining module's initializer is run first (on this same interpreter and
/// heap), so the class's field defaults, field transforms, and instance
/// initializers registered by it are present. That lets a caller supply fewer
/// args than the field count — trailing fields take their recorded defaults — and
/// makes decorator-driven instance hooks fire, matching normal entry-point
/// construction.
pub(crate) fn construct_class_loaded(
    scope: &zun_loader::ModuleScope,
    loader: &zun_loader::ModuleLoader,
    type_rid: u32,
    args: Vec<Value>,
    output: &mut dyn Output,
) -> Result<Value, RunError> {
    let module = scope.module();
    let types = zun_types::TypeSystem::from_module(module).map_err(|_| RunError::BadToken)?;
    let mut interp = Interp {
        module,
        output,
        hook: None,
        loader: Some(loader),
        eval_host: None,
        eval_cache: std::collections::HashMap::new(),
        allocs: 0,
        gc_interval: GC_INTERVAL,
        minor_collections: 0,
        types,
        suspended: None,
        resume_frame: None,
        yielded: None,
        class_symbol_methods: std::collections::HashMap::new(),
        class_named_methods: std::collections::HashMap::new(),
        class_field_defaults: std::collections::HashMap::new(),
        decorator_initializers: Vec::new(),
        instance_initializers: Vec::new(),
        field_transforms: Vec::new(),
        active_frame: None,
        call_stack: Vec::new(),
        async_state: AsyncState::default(),
    };
    // Run the defining module's initializer first so field defaults, field
    // transforms, and instance initializers are registered on this interpreter
    // before the instance is built (see `run_method_with` for the entry-point
    // equivalent). Without it, trailing-absent args have no defaults to fall back
    // on and decorator hooks never fire. The initializer runs on the current heap
    // (where the instance will live); its own side effects, such as seeding the
    // realm global, are intended.
    if let Some(init) = module.module_initializer {
        if let Err(error) = interp.call(init.rid(), Vec::new(), None) {
            return Err(interp.annotate_error(error));
        }
    }
    interp.construct_class_from_descriptor(type_rid | zun_natives::USER_CLASS_FLAG, args)
}

/// Run a method of a loaded module on the **current** heap, mirroring
/// [`super::run_method_loaded`] but without installing a fresh heap and without
/// running the module initializer.
///
/// This is required by the dynamic-namespace instance-method path: the receiver
/// (`this`) is a typed instance built earlier by [`construct_class_loaded`] and
/// living on the caller's current heap, so the method must execute against that
/// same heap. Installing a new heap (as [`super::run_method_loaded`] does) would
/// leave the receiver pointer dangling into a discarded heap.
pub(crate) fn run_method_loaded_on_current_heap(
    scope: &zun_loader::ModuleScope,
    loader: &zun_loader::ModuleLoader,
    method_rid: u32,
    args: Vec<Value>,
    output: &mut dyn Output,
) -> Result<Value, RunError> {
    let module = scope.module();
    let types = zun_types::TypeSystem::from_module(module).map_err(|_| RunError::BadToken)?;
    let mut interp = Interp {
        module,
        output,
        hook: None,
        loader: Some(loader),
        eval_host: None,
        eval_cache: std::collections::HashMap::new(),
        allocs: 0,
        gc_interval: GC_INTERVAL,
        minor_collections: 0,
        types,
        suspended: None,
        resume_frame: None,
        yielded: None,
        class_symbol_methods: std::collections::HashMap::new(),
        class_named_methods: std::collections::HashMap::new(),
        class_field_defaults: std::collections::HashMap::new(),
        decorator_initializers: Vec::new(),
        instance_initializers: Vec::new(),
        field_transforms: Vec::new(),
        active_frame: None,
        call_stack: Vec::new(),
        async_state: AsyncState::default(),
    };
    let first = interp.call(method_rid, args, None);
    let outcome = match first {
        Err(RunError::Suspended) => interp
            .suspended
            .take()
            .map(ExecutionOutcome::Suspended)
            .unwrap_or_else(|| {
                ExecutionOutcome::Fault(RunError::Malformed(
                    "suspended execution did not provide a frame".into(),
                ))
            }),
        other => ExecutionOutcome::from_result(other),
    };
    let result = match interp.drive_to_completion(outcome) {
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
                // The held value is delivered as it was registered: an `Any`
                // carrier unboxed back into a runtime value (object, string,
                // number, …), falling back to the raw `Any` if it cannot unbox.
                let held = zun_gc::with_current_heap(|heap| {
                    Value::unbox_any(record.held, heap)
                })
                .unwrap_or(Value::Any(record.held));
                let environment = callback
                    .environment()
                    .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) });
                let mut args = Vec::with_capacity(2);
                if let Some(environment) = environment {
                    args.push(Value::Object(environment));
                }
                args.push(held);
                self.call(callback.method_rid(), args, None)?;
            }
        }
    }

    /// Drive one synchronous turn to completion: run the entry's outcome, and
    /// whenever it is suspended on a pending `await`, park it and pump the
    /// scheduler (running reactions, resuming woken frames) until the entry
    /// resumes to a terminal result. On completion, drain any remaining queued
    /// reactions (the turn-drain) before returning.
    pub(super) fn drive_to_completion(
        &mut self,
        mut outcome: ExecutionOutcome,
    ) -> Result<Value, RunError> {
        loop {
            match outcome {
                ExecutionOutcome::Complete(value) => {
                    self.run_event_loop()?;
                    return Ok(value);
                }
                ExecutionOutcome::Thrown(value) => return Err(RunError::Thrown(value)),
                ExecutionOutcome::Fault(error) => return Err(error),
                ExecutionOutcome::Suspended(frame) => {
                    // A top-level `await` on a pending promise: park it, then
                    // pump the unified loop (microtasks, then a timer macrotask
                    // when microtasks go idle) until something resumes it.
                    self.park_suspended(frame)?;
                    outcome = loop {
                        match self.next_microstep()? {
                            Microstep::RanReaction => continue,
                            Microstep::Frame(frame) => break self.resume_suspended(frame),
                            Microstep::Idle => {
                                // No microtask is runnable: fire the earliest-due
                                // timer to make progress (its callback may settle
                                // the promise the entry awaits). Only when no
                                // timer remains either is the turn a real deadlock.
                                if self.fire_one_timer()? {
                                    continue;
                                }
                                return Err(RunError::Malformed(
                                    "async turn went idle with the entry point still suspended"
                                        .into(),
                                ));
                            }
                        }
                    };
                }
            }
        }
    }

    /// The unified micro+macrotask event loop (PLAN §5.1): drain the microtask
    /// queue (Promise reactions / awaits) to empty, then fire the single
    /// earliest-due ready timer as one macrotask, then drain microtasks again,
    /// and so on until **no microtask and no pending timer remain**. A
    /// `setTimeout(cb, 0)` therefore fires only after every currently-queued
    /// microtask has run (macrotask < microtask drain). Timers advance a virtual
    /// clock by fast-forward rather than sleeping, so a timed program terminates
    /// deterministically (see [`Interp::fire_one_timer`]).
    pub(super) fn run_event_loop(&mut self) -> Result<(), RunError> {
        loop {
            self.drain_microtasks()?;
            if !self.fire_one_timer()? {
                // Turn end: every microtask has drained and no timer remains.
                // Any promise that rejected with no handler ever attached (by
                // now) is an unhandled rejection — report it. Checking here,
                // not at settle time, is what makes `const p =
                // Promise.reject(...); p.catch(f)` *not* unhandled (the
                // `.catch` attaches before the drain completes and notices it).
                self.report_unhandled_rejections();
                return Ok(());
            }
        }
    }

    /// Report each promise that rejected with no handler attached by turn end
    /// (see [`Self::run_event_loop`]). The scheduler's `unnoticed` set is the
    /// source of truth (a handler attaching clears a promise from it); this
    /// maps each surviving `PromiseId` back to its heap cell for the message
    /// and emits a diagnostic line through the output sink. Draining the set
    /// means each rejection is reported at most once.
    pub(super) fn report_unhandled_rejections(&mut self) {
        if !self.async_state.promises.has_unnoticed_rejections() {
            return;
        }
        let unnoticed = self.async_state.promises.take_unnoticed_rejections();
        for pid in unnoticed {
            // Reverse the promise→id bridge to find the heap cell.
            let addr = self
                .async_state
                .pid_by_obj
                .iter()
                .find_map(|(&addr, &bridged)| (bridged == pid).then_some(addr));
            let message = match addr {
                Some(addr) => {
                    let promise = unsafe { zun_gc::Obj::from_addr(addr as *mut u64) };
                    zun_gc::with_current_heap(|heap| heap.promise_error(promise))
                        .unwrap_or_else(|| "Promise rejected".into())
                }
                None => "Promise rejected".into(),
            };
            self.output
                .println(&format!("UnhandledPromiseRejection: {message}"));
        }
    }

    /// Drain the scheduler's runnable queue to empty: run reactions, and resume
    /// any woken detached `await` frames (fire-and-forget async tasks). Reactions
    /// queued by reactions run in the same drain (the rts `drain_microtasks`
    /// model). An unhandled throw from a detached task is swallowed here so one
    /// failed task does not abort the turn; promise-level unhandled-rejection
    /// reporting is a later addition (the scheduler has no `unnoticed` list yet).
    pub(super) fn drain_microtasks(&mut self) -> Result<(), RunError> {
        loop {
            match self.next_microstep()? {
                Microstep::RanReaction => continue,
                Microstep::Frame(frame) => match self.resume_suspended(frame) {
                    ExecutionOutcome::Complete(_) => continue,
                    ExecutionOutcome::Suspended(frame) => {
                        self.park_suspended(frame)?;
                        continue;
                    }
                    ExecutionOutcome::Thrown(_) => continue,
                    ExecutionOutcome::Fault(error) => return Err(error),
                },
                Microstep::Idle => return Ok(()),
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
