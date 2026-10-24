//! The machine-level async substrate: promises, continuations, and the order
//! they run in.
//!
//! # Why this is here
//!
//! A promise is not a language feature. It is a machine-level object with a
//! state machine, a wait set, and a scheduling discipline that decides when its
//! waiters run relative to everything else. Every client that wants asynchrony
//! needs the same object, and the observable ordering it produces is a property
//! of the machine rather than of whoever wrote the syntax. The language layer
//! adds only the value a promise carries and the policy for settling it.
//!
//! A continuation is a parked frame plus the thing it is waiting for.
//! Coroutines, generators, asynchronous functions and promise waiters are four
//! names for suspend-and-resume over a queue, which is the reason to build them
//! once, here, as [`ContinuationId`]s the scheduler owns.
//!
//! # What this is (slice 1) and is not (yet)
//!
//! This module is the control machine, standalone and unit-tested. It models
//! *control* only — which frame runs when, which promise settles which way,
//! which waiters wake. It deliberately holds **no value and no heap object**:
//! the settled value lives in a side table in the language layer (the managed
//! promise cell in `zun_gc`/`zun_vm` for now), keyed by [`PromiseId`]. This
//! mirrors the rts split, where [`Settlement`] carries no value.
//!
//! It is not yet wired into the interpreter. Slices 2 and 3 migrate `await`,
//! `then` and the combinators onto it (see the module-level notes on
//! [`Continuation`] and [`Scheduler::park`] for the integration points).
//!
//! # Deliberate deviation from rts
//!
//! rts carries a `SchedulerId` on every promise and a `Delivery::Elsewhere`
//! hand-off, because it runs one scheduler per thread-local region and a promise
//! created in one region can be awaited from another. Zun's Tier 0 interpreter
//! is single-threaded, so slice 1 keeps a single owner-less scheduler: no
//! `SchedulerId`, no cross-region `Delivery`.
//! The promise/continuation/queue split is otherwise mirrored one-for-one, and
//! re-introducing per-region ownership later is additive (a field on the cell
//! plus a `Delivery` return), not a redesign.

mod promise;
mod queue;

pub use promise::{
    AdoptError, Attachment, PromiseId, PromiseState, PromiseTable, SettleOutcome, Settlement,
};
pub use queue::{Queues, Ran};

/// Identifies a parked frame waiting on something.
///
/// # Integration point (slice 2)
///
/// A `ContinuationId` will be the handle for a parked interpreter frame —
/// `ExecutionOutcome::Suspended(frame)` in `zun_interp`, resumed by
/// `resume_suspended`. Slice 2 allocates one of these per suspended `await`,
/// stores the frame in a side table keyed by it (the same control/value split
/// this machine uses for promise values), parks it on the awaited promise via
/// [`Scheduler::park`], and on wake looks the frame back up to resume it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct ContinuationId(pub u32);

/// Identifies work that is not a resumption — a timer that fired, a readiness
/// notification, an entry point being started.
///
/// # Integration point (slice 3)
///
/// Tasks are the macrotask tier below continuations. `then`/combinators and the
/// turn-drain in slice 3 enqueue continuations for settled-promise callbacks and
/// tasks for timer-like work, relying on the ordering contract in [`queue`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct TaskId(pub u32);

/// A parked frame and the promise whose settlement makes it runnable.
///
/// # Integration points
///
/// - slice 2 (`await`): built from a suspended interpreter frame. The frame
///   itself (its stack, locals and resume pc — `FrameState::Suspended` in
///   `zun_interp`) is the *value/control-state* side and stays out of this
///   machine; only `id` and `waiting_on` live here.
/// - GC (slice 2+): a parked frame holds GC roots (locals, operand stack). The
///   side table that owns the frame for a `ContinuationId` is where those roots
///   are reported from during a collection. This machine holds no `Obj`, so it
///   needs no stack-map integration; the root-reporting obligation lands on the
///   frame side table, noted here so slice 2 wires it there rather than here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Continuation {
    /// Which parked frame this is.
    pub id: ContinuationId,
    /// The promise whose settlement makes it runnable.
    pub waiting_on: PromiseId,
}

impl Continuation {
    /// A parked frame waiting on a promise.
    pub fn new(id: ContinuationId, waiting_on: PromiseId) -> Self {
        Self { id, waiting_on }
    }
}

/// The scheduler: it owns the runnable queue and settles promises on behalf of
/// the running program.
///
/// Promises live in a shared [`PromiseTable`] passed in at each call, rather
/// than inside the scheduler, so that the table (the wait sets) and the queue
/// (what runs next) stay separable — the same split rts keeps for the sake of
/// multiple schedulers sharing one table later.
#[derive(Debug, Default)]
pub struct Scheduler {
    queues: Queues,
}

impl Scheduler {
    /// An idle scheduler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Its queues, for enqueuing work and asking what runs next.
    pub fn queues(&mut self) -> &mut Queues {
        &mut self.queues
    }

    /// Settles a promise and makes its woken continuations runnable, in the
    /// order they attached.
    ///
    /// Adopting promises are settled transitively inside
    /// [`PromiseTable::settle`]; this only has to enqueue the continuations that
    /// fell out of that cascade.
    pub fn settle(
        &mut self,
        promises: &mut PromiseTable,
        promise: PromiseId,
        settlement: Settlement,
    ) -> SettleOutcome {
        let outcome = promises.settle(promise, settlement);
        self.queues.wake_all(outcome.woken.iter().copied());
        outcome
    }

    /// Parks a frame on a promise, or reports that it can run immediately.
    ///
    /// A promise that has already settled does not run its waiter inline: the
    /// continuation is enqueued rather than run here, so that the ordering is the
    /// same whether the promise settled before or after the wait. A waiter that
    /// ran inline when it happened to be late would make a program's behaviour
    /// depend on timing that is not otherwise observable.
    pub fn park(&mut self, promises: &mut PromiseTable, continuation: Continuation) -> Attachment {
        let attachment = promises.attach(continuation.waiting_on, continuation.id);
        if let Attachment::ReadyNow(_) = attachment {
            self.queues.wake(continuation.id);
        }
        attachment
    }

    /// Takes the next thing to run, or `None` when nothing is runnable.
    pub fn next(&mut self) -> Option<Ran> {
        self.queues.next()
    }

    /// Runs until nothing is runnable, reporting what ran in order.
    ///
    /// Only useful for reasoning about ordering — a real event loop blocks
    /// waiting for external readiness instead of stopping. Keeping it means the
    /// ordering contract is executable rather than merely described.
    pub fn run_until_idle(&mut self) -> Vec<Ran> {
        let mut ran = Vec::new();
        while let Some(next) = self.queues.next() {
            ran.push(next);
        }
        ran
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_before_settle_parks_then_wakes_on_settle() {
        let mut promises = PromiseTable::new();
        let mut scheduler = Scheduler::new();
        let promise = promises.create();

        let attachment = scheduler.park(&mut promises, Continuation::new(ContinuationId(0), promise));
        assert_eq!(attachment, Attachment::Parked);
        assert!(scheduler.next().is_none(), "a parked waiter is not runnable yet");

        let outcome = scheduler.settle(&mut promises, promise, Settlement::Fulfilled);
        assert!(outcome.accepted);
        assert_eq!(scheduler.next(), Some(Ran::Continuation(ContinuationId(0))));
        assert_eq!(scheduler.next(), None);
    }

    #[test]
    fn attach_after_settle_is_ready_now() {
        let mut promises = PromiseTable::new();
        let mut scheduler = Scheduler::new();
        let promise = promises.create();
        scheduler.settle(&mut promises, promise, Settlement::Fulfilled);

        let attachment = scheduler.park(&mut promises, Continuation::new(ContinuationId(7), promise));
        assert_eq!(attachment, Attachment::ReadyNow(Settlement::Fulfilled));
        assert_eq!(
            scheduler.next(),
            Some(Ran::Continuation(ContinuationId(7))),
            "a late waiter is enqueued, not run inline"
        );
    }

    #[test]
    fn double_settle_is_a_no_op() {
        let mut promises = PromiseTable::new();
        let mut scheduler = Scheduler::new();
        let promise = promises.create();

        let first = scheduler.settle(&mut promises, promise, Settlement::Fulfilled);
        assert!(first.accepted);
        let second = scheduler.settle(&mut promises, promise, Settlement::Rejected);
        assert!(!second.accepted, "the first settlement wins");
        assert!(second.woken.is_empty());
        assert_eq!(promises.state(promise), PromiseState::Settled(Settlement::Fulfilled));
    }

    #[test]
    fn adoption_chains_the_settlement() {
        let mut promises = PromiseTable::new();
        let mut scheduler = Scheduler::new();
        let outer = promises.create();
        let inner = promises.create();

        // A waiter parked on the outer promise.
        scheduler.park(&mut promises, Continuation::new(ContinuationId(1), outer));
        // outer adopts inner: it settles however inner does.
        let attachment = promises.adopt(outer, inner).expect("adoption is acyclic");
        assert_eq!(attachment, Attachment::Parked);

        // Settling inner cascades to outer, waking outer's waiter.
        let outcome = scheduler.settle(&mut promises, inner, Settlement::Fulfilled);
        assert!(outcome.accepted);
        assert_eq!(
            promises.state(outer),
            PromiseState::Settled(Settlement::Fulfilled),
            "the outer promise adopted the inner settlement"
        );
        assert_eq!(scheduler.next(), Some(Ran::Continuation(ContinuationId(1))));
    }

    #[test]
    fn adoption_of_an_already_settled_promise_is_ready_now() {
        let mut promises = PromiseTable::new();
        let outer = promises.create();
        let inner = promises.create();
        promises.settle(inner, Settlement::Rejected);

        let attachment = promises.adopt(outer, inner).expect("adoption is acyclic");
        assert_eq!(attachment, Attachment::ReadyNow(Settlement::Rejected));
        assert_eq!(promises.state(outer), PromiseState::Settled(Settlement::Rejected));
    }

    #[test]
    fn adoption_cycle_is_refused() {
        let mut promises = PromiseTable::new();
        let a = promises.create();
        let b = promises.create();
        let c = promises.create();

        assert_eq!(promises.adopt(a, a), Err(AdoptError::Cycle), "self-adoption");
        promises.adopt(a, b).expect("a waits on b");
        promises.adopt(b, c).expect("b waits on c");
        // c adopting a would close the loop a -> b -> c -> a: nothing could settle it.
        assert_eq!(promises.adopt(c, a), Err(AdoptError::Cycle));
    }

    #[test]
    fn queue_drains_runnable_continuations_in_order() {
        let mut promises = PromiseTable::new();
        let mut scheduler = Scheduler::new();
        let promise = promises.create();
        for id in [2u32, 0, 1] {
            scheduler.park(&mut promises, Continuation::new(ContinuationId(id), promise));
        }
        scheduler.settle(&mut promises, promise, Settlement::Fulfilled);

        let ran = scheduler.run_until_idle();
        assert_eq!(
            ran,
            vec![
                Ran::Continuation(ContinuationId(2)),
                Ran::Continuation(ContinuationId(0)),
                Ran::Continuation(ContinuationId(1)),
            ],
            "waiters wake in the order they attached"
        );
    }

    #[test]
    fn a_rejection_marks_the_rejected_settlement_for_the_unwinding_path() {
        let mut promises = PromiseTable::new();
        let mut scheduler = Scheduler::new();
        let promise = promises.create();
        scheduler.park(&mut promises, Continuation::new(ContinuationId(0), promise));

        let outcome = scheduler.settle(&mut promises, promise, Settlement::Rejected);
        assert!(outcome.accepted);
        assert_eq!(
            promises.state(promise),
            PromiseState::Settled(Settlement::Rejected),
            "the waiter resumes through the unwinding path, not value delivery"
        );
        assert_eq!(outcome.woken, vec![ContinuationId(0)]);
    }
}
