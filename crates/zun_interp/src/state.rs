use super::{CallHook, Output, Value};
use std::collections::HashMap;
use zun_module::Module;
use zun_gc::Obj;
use zun_sched::{Continuation, ContinuationId, PromiseId, PromiseTable, Scheduler, Settlement, TaskId};

pub(super) struct Interp<'a> {
    pub(super) module: &'a Module,
    pub(super) output: &'a mut dyn Output,
    pub(super) hook: Option<&'a dyn CallHook>,
    pub(super) loader: Option<&'a zun_loader::ModuleLoader>,
    /// Injected compiler for `new Function`; `None` throws (AOT / jitless).
    pub(super) eval_host: Option<&'a dyn super::EvalHost>,
    /// `new Function` results by source so repeats link (and leak a module copy) once.
    pub(super) eval_cache: HashMap<String, zun_core::FunctionHandle>,
    pub(super) allocs: u32,
    pub(super) gc_interval: u32,
    pub(super) minor_collections: u8,
    pub(super) types: zun_types::TypeSystem,
    pub(super) suspended: Option<ExecutionFrame>,
    pub(super) resume_frame: Option<ExecutionFrame>,
    /// Value produced by a generator yield at the current suspension point.
    pub(super) yielded: Option<Value>,
    /// Class computed-symbol-key methods: `(class TypeDef rid, symbol id)` →
    /// the method as a function value. Populated by `classSetSymbolFunction`
    /// in the module initializer and consulted by `instanceofClass` for a
    /// typed-class `[Symbol.hasInstance]`. The handles are environment-less
    /// (plain method rids), so they hold no managed references to root.
    pub(super) class_symbol_methods: HashMap<(u32, u64), zun_core::FunctionHandle>,
    /// Typed-class `valueOf`/`toString` instance methods by name, keyed
    /// `(class TypeDef rid, method name)`; populated by `classSetMethod`.
    pub(super) class_named_methods: HashMap<(u32, String), zun_core::FunctionHandle>,
    /// Literal field-initializer defaults `(class rid, field slot)`; populated
    /// by `classSetFieldDefault`, applied by runtime construction.
    pub(super) class_field_defaults: HashMap<(u32, u32), Value>,
    /// Pending class-decorator `addInitializer` callbacks, queued by
    /// `decoratorAddInitializer` and drained by `decoratorRunInitializers`.
    /// Rooted and relocated with the other decorator side tables (see
    /// `decorators.rs`), so a collection between the decorator call and the
    /// drain cannot free a queued closure environment.
    pub(super) decorator_initializers: Vec<zun_core::FunctionHandle>,
    /// Instance-bound `addInitializer` callbacks `(class rid, fn)`, run by
    /// `classInstanceInit` on every new instance of the class or a subclass.
    pub(super) instance_initializers: Vec<(u32, zun_core::FunctionHandle)>,
    /// Field decorator initializer-transforms `(class rid, field slot, fn)`.
    pub(super) field_transforms: Vec<(u32, u32, zun_core::FunctionHandle)>,
    /// Most recent executing frame/program counter, used for source-aware
    /// diagnostics at the public runner boundary.
    pub(super) active_frame: Option<(u32, u32)>,
    /// Live chain of interpreter activations, innermost last. Each entry is the
    /// callee's method rid and its current program counter; the top entry's pc
    /// is advanced as the method executes. This is the frame walk behind
    /// `Error`'s `.stack` capture (PLAN §3.1): a frame carries no managed
    /// references (just two `u32`s), so it needs no GC rooting or relocation.
    pub(super) call_stack: Vec<CallFrame>,
    /// The machine-level async substrate for `await`, `then`/`catch`/`finally`
    /// and the `Promise.*` combinators: the scheduler, its promise table, and
    /// the side tables that own parked interpreter frames and scheduled
    /// reactions keyed by [`ContinuationId`]. See [`AsyncState`]. Settlement now
    /// flows through the scheduler directly and reactions run as microtasks on
    /// the turn-drain (slice 3); the old eager-at-settle `then`/combinator path
    /// is retired.
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
    /// Scheduled `then`/`catch`/`finally`/combinator reactions, keyed by the id
    /// the scheduler holds. A reaction waits on a *source* promise and, when
    /// that source settles, runs its callback (or passes the settlement
    /// through) as a microtask and settles a *new* target promise. This table
    /// is the root owner for the source/target promises and the callbacks a
    /// pending reaction holds.
    pub(super) reactions: HashMap<ContinuationId, Reaction>,
    /// Shared aggregate state for `Promise.all/allSettled/race/any`: one entry
    /// per in-flight combinator, referenced by index from every per-input
    /// [`ReactionKind::Combinator`]. Kept here (single ownership) rather than
    /// inside the reactions so its rooted result array is rewritten exactly once
    /// on a moving collection.
    pub(super) aggregates: Vec<Aggregate>,
    pub(super) next_cont: u32,
    /// The macrotask timer source feeding the scheduler's task tier (ASYNC
    /// slice 4). Pending timers live here, keyed by due time; a due timer is
    /// scheduled as a [`TaskId`] on the scheduler and its callback fired as a
    /// macrotask. This table is the GC-root owner for the callbacks (and any
    /// forwarded arguments) of pending timers, rooted and rewritten alongside
    /// the reaction/parked-frame tables.
    pub(super) timers: TimerHeap,
    /// Monotonic allocator for timer [`TaskId`]s, which double as the numeric
    /// handles `setTimeout`/`setInterval`/`setImmediate` return.
    pub(super) next_task: u32,
}

/// A min-heap (by due time) of pending timers — the timer source of the
/// unified event loop. Kept as a `Vec` with linear min-selection rather than a
/// `BinaryHeap` so its managed callbacks can be iterated and rewritten in place
/// on a moving collection; timer populations are small on Tier 0.
#[derive(Debug, Default)]
pub(super) struct TimerHeap {
    pub(super) entries: Vec<TimerEntry>,
    /// Virtual current time (ms). Seeded lazily from the real `nowMillis` clock
    /// when the first timer is armed, then advanced by fast-forward: when only
    /// not-yet-due timers remain, the loop jumps the clock to the earliest due
    /// time instead of sleeping, so `run()` terminates deterministically
    /// without real wall-clock blocking (PLAN §5.1).
    pub(super) clock: i64,
    pub(super) seeded: bool,
    /// Insertion order, the tie-breaker among equal due times so timers armed
    /// earlier fire first (FIFO within a due instant).
    pub(super) next_seq: u64,
}

/// One pending timer: the scheduler task it feeds, when it is due, the callback
/// to run, and — for an interval — the period it re-arms with.
#[derive(Debug)]
pub(super) struct TimerEntry {
    pub(super) task: TaskId,
    pub(super) due: i64,
    pub(super) seq: u64,
    /// `Some(period_ms)` for an interval (re-arms itself), `None` for a one-shot.
    pub(super) period: Option<i64>,
    /// The callback closure (`Value::Function`).
    pub(super) callback: Value,
    /// Extra arguments forwarded to the callback (currently always empty; the
    /// `...args` tail is reserved).
    pub(super) args: Vec<Value>,
}

/// Which `Promise.*` combinator an [`Aggregate`] is driving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CombinatorKind {
    All,
    AllSettled,
    Race,
    Any,
}

/// What a scheduled reaction does when its source promise settles.
#[derive(Debug)]
pub(super) enum ReactionKind {
    /// `then(onFulfilled?, onRejected?)` / `catch`. A missing callback forwards
    /// the matching settlement to the target (pass-through), which is also how
    /// a pending adopted promise is wired to its adopter.
    Then {
        on_fulfilled: Option<Value>,
        on_rejected: Option<Value>,
    },
    /// `finally(onFinally)`: runs the cleanup on settlement and passes the
    /// source settlement through unless the cleanup throws.
    Finally { on_finally: Option<Value> },
    /// One input of a combinator; updates `aggregates[aggregate]` at `index`.
    Combinator { aggregate: usize, index: usize },
    /// Thenable-adoption job (ECMAScript NewPromiseResolveThenableJob): calls
    /// `thenable.then(resolve, reject)` with fresh settlers for the reaction's
    /// target. `then` is the callable (`Value::Function`), `thenable` its
    /// receiver object (kept alive here).
    ThenableJob { thenable: Value, then: Value },
    /// Resumption of an `async` coroutine awaiting the source promise; the
    /// reaction target is the async function's own result promise.
    AsyncStep { generator: Value },
    /// `queueMicrotask(cb)`: runs `cb` with no arguments; the result is ignored.
    Microtask { callback: Value },
}

/// A scheduled reaction: the control ids the scheduler holds, the heap promises
/// it reads from and settles, and the callback policy.
#[derive(Debug)]
pub(super) struct Reaction {
    /// The source promise whose settlement wakes this reaction
    /// (`Value::Object`).
    pub(super) source: Value,
    /// The new promise this reaction settles (`Value::Object`).
    pub(super) target: Value,
    pub(super) kind: ReactionKind,
}

/// Shared state for one combinator: the result holder it fills and the counter
/// that decides when the aggregate target settles.
#[derive(Debug)]
pub(super) struct Aggregate {
    pub(super) kind: CombinatorKind,
    /// The aggregate's result promise (`Value::Object`).
    pub(super) target: Value,
    /// The managed array collecting per-input results/descriptors
    /// (`Value::Object`), or `Value::Void` for `race`/`any` (no array result).
    pub(super) holder: Value,
    /// Inputs still outstanding before the aggregate can conclude.
    pub(super) remaining: usize,
    /// Set once the target has settled, so later inputs are ignored.
    pub(super) settled: bool,
}

/// A parked interpreter frame and the scheduler promise whose settlement wakes
/// it.
#[derive(Debug)]
pub(super) struct ParkedFrame {
    pub(super) frame: ExecutionFrame,
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
        self.rewrite_decorator_tables(relocation);
        self.async_state.rewrite_after_move(relocation)
    }
}

impl AsyncState {
    /// Rewrite every parked `await` frame, every pending reaction and
    /// aggregate, and the promise→id bridge after a moving collection, so the
    /// control state carried across a collection survives relocation.
    pub(super) fn rewrite_after_move(&mut self, relocation: &zun_gc::RelocationMap) -> usize {
        let mut rewritten = 0;
        for parked in self.parked.values_mut() {
            rewritten += parked.frame.rewrite_managed_references(relocation);
        }
        for reaction in self.reactions.values_mut() {
            rewritten += usize::from(reaction.source.rewrite_managed_reference(relocation));
            rewritten += usize::from(reaction.target.rewrite_managed_reference(relocation));
            match &mut reaction.kind {
                ReactionKind::Then {
                    on_fulfilled,
                    on_rejected,
                } => {
                    if let Some(value) = on_fulfilled {
                        rewritten += usize::from(value.rewrite_managed_reference(relocation));
                    }
                    if let Some(value) = on_rejected {
                        rewritten += usize::from(value.rewrite_managed_reference(relocation));
                    }
                }
                ReactionKind::Finally { on_finally } => {
                    if let Some(value) = on_finally {
                        rewritten += usize::from(value.rewrite_managed_reference(relocation));
                    }
                }
                ReactionKind::Combinator { .. } => {}
                ReactionKind::AsyncStep { generator } => {
                    rewritten += usize::from(generator.rewrite_managed_reference(relocation));
                }
                ReactionKind::Microtask { callback } => {
                    rewritten += usize::from(callback.rewrite_managed_reference(relocation));
                }
                ReactionKind::ThenableJob { thenable, then } => {
                    rewritten += usize::from(thenable.rewrite_managed_reference(relocation));
                    rewritten += usize::from(then.rewrite_managed_reference(relocation));
                }
            }
        }
        for aggregate in self.aggregates.iter_mut() {
            rewritten += usize::from(aggregate.target.rewrite_managed_reference(relocation));
            rewritten += usize::from(aggregate.holder.rewrite_managed_reference(relocation));
        }
        for timer in self.timers.entries.iter_mut() {
            rewritten += usize::from(timer.callback.rewrite_managed_reference(relocation));
            rewritten += Value::rewrite_managed_references(&mut timer.args, relocation);
        }
        // Keys in the promise→id bridge are heap addresses, which move with the
        // objects they name.
        if !self.pid_by_obj.is_empty() {
            let old: Vec<(usize, PromiseId)> = self.pid_by_obj.drain().collect();
            for (addr, pid) in old {
                let obj = unsafe { Obj::from_addr(addr as *mut u64) };
                let relocated = relocation.relocate(obj).addr() as usize;
                self.pid_by_obj.insert(relocated, pid);
            }
        }
        rewritten
    }
}

impl<'a> Interp<'a> {
    /// Append the GC roots held by every parked `await` frame, every pending
    /// reaction (its source/target promises and callbacks), and every in-flight
    /// combinator aggregate (its target and result holder). All of these are
    /// owned by [`AsyncState`] and are rooted from here while they sit in their
    /// side tables, exactly as the actively suspended frame is.
    pub(super) fn append_parked_roots(&self, roots: &mut Vec<Obj>) {
        self.append_decorator_roots(roots);
        for parked in self.async_state.parked.values() {
            parked.frame.append_managed_roots(roots);
        }
        for reaction in self.async_state.reactions.values() {
            let mut values = vec![reaction.source.clone(), reaction.target.clone()];
            match &reaction.kind {
                ReactionKind::Then {
                    on_fulfilled,
                    on_rejected,
                } => {
                    values.extend(on_fulfilled.clone());
                    values.extend(on_rejected.clone());
                }
                ReactionKind::Finally { on_finally } => values.extend(on_finally.clone()),
                ReactionKind::Combinator { .. } => {}
                ReactionKind::AsyncStep { generator } => {
                    values.push(generator.clone());
                }
                ReactionKind::Microtask { callback } => {
                    values.push(callback.clone());
                }
                ReactionKind::ThenableJob { thenable, then } => {
                    values.push(thenable.clone());
                    values.push(then.clone());
                }
            }
            super::roots::collect_frame_roots(&values, &[], roots);
        }
        for aggregate in &self.async_state.aggregates {
            super::roots::collect_frame_roots(
                &[aggregate.target.clone(), aggregate.holder.clone()],
                &[],
                roots,
            );
        }
        for timer in &self.async_state.timers.entries {
            let mut values = vec![timer.callback.clone()];
            values.extend(timer.args.iter().cloned());
            super::roots::collect_frame_roots(&values, &[], roots);
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
        let pid = self.bridge_pid(promise);
        let cid = ContinuationId(self.async_state.next_cont);
        self.async_state.next_cont += 1;
        let state = &mut self.async_state;
        state
            .scheduler
            .park(&mut state.promises, Continuation::new(cid, pid));
        state.parked.insert(cid, ParkedFrame { frame });
        Ok(cid)
    }

    /// The scheduler [`PromiseId`] bridged to a managed promise `Obj`, created
    /// on first use. Crucially, a newly created bridge is *reconciled* against
    /// the heap immediately: if the heap promise has already settled, the
    /// scheduler promise is settled to match now, so a reaction or frame that
    /// attaches afterwards sees `ReadyNow` (enqueued) rather than parking on a
    /// scheduler promise that can never settle. This is the attach-after-settle
    /// half of keeping the heap cell and the scheduler in step; the settle
    /// paths ([`Interp::settle_promise_resolve`]/`settle_promise_reject`) are
    /// the settle-after-attach half.
    pub(super) fn bridge_pid(&mut self, promise: Obj) -> PromiseId {
        let addr = promise.addr() as usize;
        if let Some(pid) = self.async_state.pid_by_obj.get(&addr) {
            return *pid;
        }
        let pid = self.async_state.promises.create();
        self.async_state.pid_by_obj.insert(addr, pid);
        let settlement = zun_gc::with_current_heap(|heap| heap.promise_state(promise));
        let settlement = match settlement {
            Some(zun_core::ManagedPromiseState::Resolved) => Some(Settlement::Fulfilled),
            Some(zun_core::ManagedPromiseState::Rejected) => Some(Settlement::Rejected),
            _ => None,
        };
        if let Some(settlement) = settlement {
            let state = &mut self.async_state;
            state.scheduler.settle(&mut state.promises, pid, settlement);
        }
        pid
    }

    /// Settle the scheduler promise bridged to `promise` (if any) to `settlement`,
    /// waking every reaction and parked frame attached to it. Called whenever
    /// the heap promise cell is resolved/rejected so control flows through the
    /// scheduler directly (the settle-after-attach half of heap↔scheduler sync).
    /// Mark a promise as noticed (handled), so a rejection it carries is not
    /// reported as unhandled at turn end. Bridges the promise first so the
    /// scheduler knows its identity even on `await`'s synchronous fast path,
    /// which settles an already-rejected promise without attaching a waiter.
    pub(super) fn notice_promise(&mut self, promise: Obj) {
        let pid = self.bridge_pid(promise);
        self.async_state.promises.notice(pid);
    }

    pub(super) fn wake_scheduler(&mut self, promise: Obj, settlement: Settlement) {
        let addr = promise.addr() as usize;
        if let Some(pid) = self.async_state.pid_by_obj.get(&addr).copied() {
            let state = &mut self.async_state;
            state.scheduler.settle(&mut state.promises, pid, settlement);
        }
    }

    /// Safety-net reconciliation: settle the scheduler promise of any bridged
    /// heap promise that has settled on the heap without the scheduler being
    /// told (for example a promise the *host* resolved directly through
    /// `heap_mut()` between polls, which never runs the in-program settle path).
    ///
    /// Settlement normally flows through the scheduler directly
    /// ([`Interp::settle_promise_resolve`]/`settle_promise_reject` and
    /// [`Interp::bridge_pid`]); this pass is kept only to cover host-driven
    /// settlement, and double-settle is a no-op, so running it is always safe.
    pub(super) fn reconcile_settlements(&mut self) {
        let mut to_settle: Vec<(PromiseId, Settlement)> = Vec::new();
        for (&addr, &pid) in &self.async_state.pid_by_obj {
            if self.async_state.promises.state(pid) != zun_sched::PromiseState::Pending {
                continue;
            }
            let promise = unsafe { Obj::from_addr(addr as *mut u64) };
            match zun_gc::with_current_heap(|heap| heap.promise_state(promise)) {
                Some(zun_core::ManagedPromiseState::Resolved) => {
                    to_settle.push((pid, Settlement::Fulfilled))
                }
                Some(zun_core::ManagedPromiseState::Rejected) => {
                    to_settle.push((pid, Settlement::Rejected))
                }
                _ => {}
            }
        }
        let state = &mut self.async_state;
        for (pid, settlement) in to_settle {
            state.scheduler.settle(&mut state.promises, pid, settlement);
        }
    }

    /// Advance the scheduler by one microtask: pull the next runnable
    /// continuation and either run its reaction (synchronously, settling the
    /// reaction's target) or hand back the woken `await` frame for the caller to
    /// resume. Reactions are drained in place; only frames cross back out,
    /// because resuming a frame re-enters the interpreter and may suspend again.
    pub(super) fn next_microstep(&mut self) -> Result<Microstep, super::RunError> {
        loop {
            // Pick up any host-driven heap settlement before asking what runs.
            self.reconcile_settlements();
            match self.async_state.scheduler.next() {
                Some(zun_sched::Ran::Continuation(cid)) => {
                    if self.async_state.reactions.contains_key(&cid) {
                        self.run_reaction(cid)?;
                        return Ok(Microstep::RanReaction);
                    }
                    if let Some(parked) = self.async_state.parked.remove(&cid) {
                        return Ok(Microstep::Frame(parked.frame));
                    }
                    // A woken id with no owner (cancelled or already taken):
                    // skip it and keep draining.
                    continue;
                }
                // Tasks (timers/macrotasks) are slice 4; ignore them here.
                Some(zun_sched::Ran::Task(_)) => continue,
                None => return Ok(Microstep::Idle),
            }
        }
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

/// Result of advancing the scheduler by one microtask via
/// [`Interp::next_microstep`].
pub(super) enum Microstep {
    /// A reaction ran to completion in place; keep draining.
    RanReaction,
    /// An `await` frame woke and must be resumed by the caller.
    Frame(ExecutionFrame),
    /// Nothing is runnable.
    Idle,
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

    #[test]
    fn moving_collection_rewrites_a_pending_timers_callback() {
        // A timer callback (and any forwarded arg) is a GC root held by the
        // timer heap; a moving collection while it is pending must relocate it.
        let old = unsafe { Obj::from_addr(0x5000usize as *mut u64) };
        let new = unsafe { Obj::from_addr(0x6000usize as *mut u64) };
        let mut relocation = zun_gc::RelocationMap::new();
        relocation.insert(old, new);

        let mut state = AsyncState::default();
        state.timers.entries.push(TimerEntry {
            task: TaskId(0),
            due: 10,
            seq: 0,
            period: None,
            callback: Value::Object(old),
            args: vec![Value::Object(old)],
        });

        let rewritten = state.rewrite_after_move(&relocation);
        assert_eq!(rewritten, 2, "both the callback and its arg are rewritten");
        assert!(matches!(state.timers.entries[0].callback, Value::Object(object) if object == new));
        assert!(matches!(state.timers.entries[0].args[0], Value::Object(object) if object == new));
    }
}
