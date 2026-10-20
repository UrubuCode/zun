//! Minimal Promise and microtask primitives for the VM scheduler.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

const PENDING: u8 = 0;
const RESOLVED: u8 = 1;
const REJECTED: u8 = 2;

/// Observable settlement state of a [`Promise`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromiseState {
    Pending,
    Resolved,
    Rejected,
}

/// Result of advancing a stackless asynchronous state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll<T> {
    /// The machine suspended and must be resumed later.
    Pending,
    /// The machine completed with a value.
    Ready(T),
    /// The machine completed with an exception-like failure.
    Failed(String),
}

struct Inner<T> {
    value: Option<T>,
    error: Option<String>,
    continuation: Option<Box<dyn FnOnce(T) + Send + 'static>>,
    rejection: Option<Box<dyn FnOnce(String) + Send + 'static>>,
}

/// A single-assignment future with one success and one failure continuation,
/// matching the runtime representation planned for Promise state machines.
pub struct Promise<T> {
    state: Arc<AtomicU8>,
    inner: Arc<Mutex<Inner<T>>>,
}

/// A stackless state machine whose suspension point is represented by `state`.
///
/// Compiler-generated async bodies can store locals in the closure and update
/// the state number before returning [`Poll::Pending`]. Completion is published
/// once through the associated Promise.
pub struct AsyncStateMachine<T, F> {
    state: usize,
    poller: F,
    completion: Promise<T>,
    completed: bool,
    result: Option<T>,
    failure: Option<String>,
}

impl<T, F> core::fmt::Debug for AsyncStateMachine<T, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AsyncStateMachine")
            .field("state", &self.state)
            .field("completed", &self.completed)
            .finish_non_exhaustive()
    }
}

impl<T: Clone + Send + 'static, F: FnMut(&mut usize) -> Poll<T>> AsyncStateMachine<T, F> {
    /// Construct a state machine starting at state zero.
    pub fn new(poller: F) -> Self {
        Self {
            state: 0,
            poller,
            completion: Promise::new(),
            completed: false,
            result: None,
            failure: None,
        }
    }

    /// Advance the machine once.
    pub fn poll(&mut self) -> Poll<T> {
        if self.completed {
            return self.failure.as_ref().map_or_else(
                || {
                    Poll::Ready(
                        self.result
                            .as_ref()
                            .expect("completed machine has a result")
                            .clone(),
                    )
                },
                |error| Poll::Failed(error.clone()),
            );
        }
        match (self.poller)(&mut self.state) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(value) => {
                self.completed = true;
                self.completion.resolve(value.clone());
                self.result = Some(value.clone());
                Poll::Ready(value)
            }
            Poll::Failed(error) => {
                self.completed = true;
                self.completion.reject(error.clone());
                self.failure = Some(error.clone());
                Poll::Failed(error)
            }
        }
    }

    /// The current compiler-visible suspension state.
    pub fn state(&self) -> usize {
        self.state
    }

    /// Whether the state machine has reached its terminal state.
    ///
    /// The compiler-generated resume path can use this to avoid scheduling a
    /// second continuation after the completion value has been published.
    pub fn is_completed(&self) -> bool {
        self.completed
    }

    /// Observe completion through the machine's Promise.
    pub fn promise(&self) -> &Promise<T> {
        &self.completion
    }
}

impl<T> core::fmt::Debug for Promise<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Promise")
            .field(
                "resolved",
                &(self.state.load(Ordering::Acquire) == RESOLVED),
            )
            .field(
                "rejected",
                &(self.state.load(Ordering::Acquire) == REJECTED),
            )
            .finish_non_exhaustive()
    }
}

impl<T: Send + 'static> Promise<T> {
    /// Create a pending promise.
    pub fn new() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(PENDING)),
            inner: Arc::new(Mutex::new(Inner {
                value: None,
                error: None,
                continuation: None,
                rejection: None,
            })),
        }
    }

    /// Resolve the promise. Returns `false` when it was already resolved.
    pub fn resolve(&self, value: T) -> bool {
        if self
            .state
            .compare_exchange(PENDING, RESOLVED, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        let continuation = {
            let mut inner = self.inner.lock().expect("promise mutex poisoned");
            if inner.continuation.is_some() {
                inner.continuation.take()
            } else {
                inner.value = Some(value);
                return true;
            }
        };
        continuation.expect("continuation disappeared")(value);
        true
    }

    /// Register the single continuation. A late registration runs immediately.
    pub fn then(&self, continuation: impl FnOnce(T) + Send + 'static) {
        let mut continuation: Option<Box<dyn FnOnce(T) + Send + 'static>> =
            Some(Box::new(continuation));
        let value = {
            let mut inner = self.inner.lock().expect("promise mutex poisoned");
            if self.state.load(Ordering::Acquire) == PENDING {
                assert!(
                    inner.continuation.is_none(),
                    "promise already has a continuation"
                );
                inner.continuation = continuation.take();
                None
            } else if self.state.load(Ordering::Acquire) == RESOLVED {
                inner.value.take()
            } else {
                None
            }
        };
        if let Some(value) = value {
            continuation.take().expect("late continuation missing")(value);
        }
    }

    /// Reject the promise. Returns `false` when it was already settled.
    pub fn reject(&self, error: impl Into<String>) -> bool {
        let error = error.into();
        if self
            .state
            .compare_exchange(PENDING, REJECTED, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        let rejection = {
            let mut inner = self.inner.lock().expect("promise mutex poisoned");
            inner.error = Some(error.clone());
            if inner.rejection.is_some() {
                inner.rejection.take()
            } else {
                return true;
            }
        };
        rejection.expect("rejection disappeared")(error);
        true
    }

    /// Register a rejection continuation. A late registration runs immediately.
    pub fn catch(&self, rejection: impl FnOnce(String) + Send + 'static) {
        let mut rejection: Option<Box<dyn FnOnce(String) + Send + 'static>> =
            Some(Box::new(rejection));
        let error = {
            let mut inner = self.inner.lock().expect("promise mutex poisoned");
            match self.state.load(Ordering::Acquire) {
                PENDING => {
                    assert!(
                        inner.rejection.is_none(),
                        "promise already has a rejection handler"
                    );
                    inner.rejection = rejection.take();
                    None
                }
                REJECTED => inner.error.clone(),
                _ => None,
            }
        };
        if let Some(error) = error {
            rejection.take().expect("late rejection missing")(error);
        }
    }

    /// Transform the resolved value into a new promise.
    ///
    /// The continuation remains single-assignment: the returned promise is
    /// settled when this promise resolves or rejects, whether the callback is
    /// registered before or after that settlement.
    pub fn map<U: Send + 'static>(
        &self,
        transform: impl FnOnce(T) -> U + Send + 'static,
    ) -> Promise<U> {
        let mapped = Promise::new();
        let target = mapped.clone_handle();
        self.then(move |value| {
            target.resolve(transform(value));
        });
        let target = mapped.clone_handle();
        self.catch(move |error| {
            target.reject(error);
        });
        mapped
    }

    /// Chain an asynchronous transform and flatten its returned promise.
    pub fn then_async<U: Send + 'static>(
        &self,
        transform: impl FnOnce(T) -> Promise<U> + Send + 'static,
    ) -> Promise<U> {
        let chained = Promise::new();
        let target = chained.clone_handle();
        self.then(move |value| {
            let next = transform(value);
            let success = target.clone();
            next.then(move |value| success.resolve(value));
            let failure = target.clone();
            next.catch(move |error| failure.reject(error));
        });
        let target = chained.clone_handle();
        self.catch(move |error| target.reject(error));
        chained
    }

    /// Run a cleanup callback after settlement while preserving the original
    /// value or rejection in the returned promise.
    pub fn finally(&self, cleanup: impl FnOnce() + Send + 'static) -> Promise<T> {
        let completed = Promise::new();
        let cleanup = Arc::new(Mutex::new(Some(cleanup)));
        let success = completed.clone_handle();
        let success_cleanup = Arc::clone(&cleanup);
        self.then(move |value| {
            if let Some(cleanup) = success_cleanup
                .lock()
                .expect("promise mutex poisoned")
                .take()
            {
                cleanup();
            }
            success.resolve(value);
        });
        let failure = completed.clone_handle();
        let failure_cleanup = Arc::clone(&cleanup);
        self.catch(move |error| {
            if let Some(cleanup) = failure_cleanup
                .lock()
                .expect("promise mutex poisoned")
                .take()
            {
                cleanup();
            }
            failure.reject(error);
        });
        completed
    }

    /// Register a continuation that is always delivered through a microtask
    /// queue, including when this promise has already resolved.
    pub fn then_scheduled(
        &self,
        queue: Arc<Mutex<MicrotaskQueue>>,
        continuation: impl FnOnce(T) + Send + 'static,
    ) {
        self.then(move |value| {
            let mut queue = queue.lock().expect("microtask mutex poisoned");
            queue.schedule(move || continuation(value));
        });
    }

    /// Register a rejection continuation delivered through the microtask queue.
    pub fn catch_scheduled(
        &self,
        queue: Arc<Mutex<MicrotaskQueue>>,
        rejection: impl FnOnce(String) + Send + 'static,
    ) {
        self.catch(move |error| {
            let mut queue = queue.lock().expect("microtask mutex poisoned");
            queue.schedule(move || rejection(error));
        });
    }

    /// Whether the promise has already been resolved.
    pub fn is_resolved(&self) -> bool {
        self.state.load(Ordering::Acquire) == RESOLVED
    }

    /// Return the current single-assignment settlement state.
    pub fn state(&self) -> PromiseState {
        match self.state.load(Ordering::Acquire) {
            RESOLVED => PromiseState::Resolved,
            REJECTED => PromiseState::Rejected,
            _ => PromiseState::Pending,
        }
    }

    /// Whether the promise has been rejected.
    pub fn is_rejected(&self) -> bool {
        self.state.load(Ordering::Acquire) == REJECTED
    }

    /// Return the stored rejection message, if any.
    pub fn error(&self) -> Option<String> {
        if !self.is_rejected() {
            return None;
        }
        self.inner
            .lock()
            .expect("promise mutex poisoned")
            .error
            .clone()
    }
}

impl<T: Send + 'static> Promise<T> {
    fn clone_handle(&self) -> PromiseHandle<T> {
        PromiseHandle {
            state: Arc::clone(&self.state),
            inner: Arc::clone(&self.inner),
        }
    }
}

/// A non-owning resolver used internally while composing promises.
struct PromiseHandle<T> {
    state: Arc<AtomicU8>,
    inner: Arc<Mutex<Inner<T>>>,
}

impl<T> Clone for PromiseHandle<T> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> PromiseHandle<T> {
    fn resolve(&self, value: T)
    where
        T: Send + 'static,
    {
        if self
            .state
            .compare_exchange(PENDING, RESOLVED, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let continuation = {
            let mut inner = self.inner.lock().expect("promise mutex poisoned");
            if inner.continuation.is_some() {
                inner.continuation.take()
            } else {
                inner.value = Some(value);
                return;
            }
        };
        continuation.expect("continuation disappeared")(value);
    }

    fn reject(&self, error: String)
    where
        T: Send + 'static,
    {
        if self
            .state
            .compare_exchange(PENDING, REJECTED, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let rejection = {
            let mut inner = self.inner.lock().expect("promise mutex poisoned");
            inner.error = Some(error.clone());
            inner.rejection.take()
        };
        if let Some(rejection) = rejection {
            rejection(error);
        }
    }
}

impl<T: Send + 'static> Default for Promise<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// FIFO queue for Promise continuations and other VM microtasks.
#[derive(Default)]
pub struct MicrotaskQueue {
    tasks: VecDeque<Box<dyn FnOnce() + Send + 'static>>,
}

impl core::fmt::Debug for MicrotaskQueue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MicrotaskQueue")
            .field("len", &self.tasks.len())
            .finish()
    }
}

impl MicrotaskQueue {
    /// Enqueue a task for later execution.
    pub fn schedule(&mut self, task: impl FnOnce() + Send + 'static) {
        self.tasks.push_back(Box::new(task));
    }

    /// Execute one task, returning whether one was available.
    pub fn run_one(&mut self) -> bool {
        let Some(task) = self.tasks.pop_front() else {
            return false;
        };
        task();
        true
    }

    /// Drain all currently queued tasks.
    pub fn run_all(&mut self) {
        while self.run_one() {}
    }

    /// Number of queued tasks.
    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Whether no tasks are queued.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

/// Single-threaded VM scheduler backed by the microtask queue.
#[derive(Debug, Default)]
pub struct Scheduler {
    microtasks: MicrotaskQueue,
}

impl Scheduler {
    /// Create an idle scheduler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue a microtask.
    pub fn schedule(&mut self, task: impl FnOnce() + Send + 'static) {
        self.microtasks.schedule(task);
    }

    /// Run at most `budget` tasks, returning how many were executed.
    pub fn run(&mut self, budget: usize) -> usize {
        let mut ran = 0;
        while ran < budget && self.microtasks.run_one() {
            ran += 1;
        }
        ran
    }

    /// Drain all pending microtasks.
    pub fn run_until_idle(&mut self) {
        self.microtasks.run_all();
    }

    /// Number of pending microtasks.
    pub fn pending(&self) -> usize {
        self.microtasks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    #[test]
    fn continuation_runs_when_promise_resolves() {
        let promise = Promise::new();
        let seen = Arc::new(Mutex::new(None));
        let target = Arc::clone(&seen);
        promise.then(move |value| *target.lock().unwrap() = Some(value));
        assert!(promise.resolve(42));
        assert_eq!(*seen.lock().unwrap(), Some(42));
        assert!(!promise.resolve(7));
    }

    #[test]
    fn late_continuation_runs_immediately() {
        let promise = Promise::new();
        assert!(promise.resolve(9));
        let seen = Arc::new(Mutex::new(None));
        let target = Arc::clone(&seen);
        promise.then(move |value| *target.lock().unwrap() = Some(value));
        assert_eq!(*seen.lock().unwrap(), Some(9));
    }

    #[test]
    fn map_composes_pending_and_already_resolved_promises() {
        let promise = Promise::new();
        let mapped = promise.map(|value| value + 1);
        let seen = Arc::new(Mutex::new(None));
        let target = Arc::clone(&seen);
        mapped.then(move |value| *target.lock().unwrap() = Some(value));
        assert!(promise.resolve(41));
        assert_eq!(*seen.lock().unwrap(), Some(42));

        let resolved = Promise::new();
        assert!(resolved.resolve(9));
        let mapped = resolved.map(|value| value * 2);
        let seen = Arc::new(Mutex::new(None));
        let target = Arc::clone(&seen);
        mapped.then(move |value| *target.lock().unwrap() = Some(value));
        assert_eq!(*seen.lock().unwrap(), Some(18));
    }

    #[test]
    fn scheduled_continuation_waits_for_the_microtask_turn() {
        let queue = Arc::new(Mutex::new(MicrotaskQueue::default()));
        let promise = Promise::new();
        let seen = Arc::new(Mutex::new(None));
        let target = Arc::clone(&seen);
        promise.then_scheduled(Arc::clone(&queue), move |value| {
            *target.lock().unwrap() = Some(value)
        });
        assert!(promise.resolve(5));
        assert_eq!(*seen.lock().unwrap(), None);
        assert_eq!(queue.lock().unwrap().run_one(), true);
        assert_eq!(*seen.lock().unwrap(), Some(5));
    }

    #[test]
    fn scheduled_rejection_waits_for_the_microtask_turn() {
        let queue = Arc::new(Mutex::new(MicrotaskQueue::default()));
        let promise = Promise::<u32>::new();
        let seen = Arc::new(Mutex::new(None));
        let target = Arc::clone(&seen);
        promise.catch_scheduled(Arc::clone(&queue), move |error| {
            *target.lock().unwrap() = Some(error)
        });
        assert!(promise.reject("boom"));
        assert_eq!(*seen.lock().unwrap(), None);
        assert!(queue.lock().unwrap().run_one());
        assert_eq!(seen.lock().unwrap().as_deref(), Some("boom"));
    }

    #[test]
    fn state_machine_preserves_state_across_suspension() {
        let mut machine = AsyncStateMachine::new(|state| match *state {
            0 => {
                *state = 1;
                Poll::Pending
            }
            1 => Poll::Ready(42),
            _ => unreachable!(),
        });
        assert_eq!(machine.poll(), Poll::Pending);
        assert_eq!(machine.state(), 1);
        assert_eq!(machine.poll(), Poll::Ready(42));
        assert_eq!(machine.poll(), Poll::Ready(42));
        assert!(machine.promise().is_resolved());
    }

    #[test]
    fn state_machine_reports_synchronous_completion() {
        let mut machine = AsyncStateMachine::new(|_| Poll::Ready(7));
        assert!(!machine.is_completed());
        assert_eq!(machine.poll(), Poll::Ready(7));
        assert!(machine.is_completed());
    }

    #[test]
    fn promise_rejection_is_single_assignment_and_observable() {
        let promise = Promise::<u32>::new();
        let seen = Arc::new(Mutex::new(None));
        let target = Arc::clone(&seen);
        promise.catch(move |error| *target.lock().unwrap() = Some(error));
        assert!(promise.reject("boom"));
        assert!(!promise.reject("later"));
        assert!(promise.is_rejected());
        assert_eq!(promise.error().as_deref(), Some("boom"));
        assert_eq!(seen.lock().unwrap().as_deref(), Some("boom"));
    }

    #[test]
    fn promise_state_reports_pending_resolved_and_rejected() {
        let promise = Promise::<u32>::new();
        assert_eq!(promise.state(), PromiseState::Pending);
        assert!(promise.resolve(1));
        assert_eq!(promise.state(), PromiseState::Resolved);
        let failed = Promise::<u32>::new();
        assert!(failed.reject("boom"));
        assert_eq!(failed.state(), PromiseState::Rejected);
    }

    #[test]
    fn map_propagates_rejection_without_running_transform() {
        let promise = Promise::<u32>::new();
        let mapped = promise.map(|_| panic!("transform must not run"));
        assert!(promise.reject("boom"));
        assert!(mapped.is_rejected());
        assert_eq!(mapped.error().as_deref(), Some("boom"));
    }

    #[test]
    fn then_async_flattens_inner_promise_and_propagates_failures() {
        let source = Promise::<u32>::new();
        let chained = source.then_async(|_| {
            let promise = Promise::new();
            assert!(promise.resolve(42));
            promise
        });
        assert!(source.resolve(7));
        assert!(chained.is_resolved());
        assert_eq!(chained.error(), None);

        let failed = Promise::<u32>::new();
        let chained: Promise<u32> = failed.then_async(|_| panic!("transform must not run"));
        assert!(failed.reject("boom"));
        assert_eq!(chained.error().as_deref(), Some("boom"));
    }

    #[test]
    fn finally_runs_on_success_and_preserves_value() {
        let promise = Promise::new();
        let cleaned = Arc::new(AtomicBool::new(false));
        let marker = Arc::clone(&cleaned);
        let completed = promise.finally(move || marker.store(true, Ordering::SeqCst));
        assert!(promise.resolve(42));
        assert_eq!(completed.state(), PromiseState::Resolved);
        assert!(cleaned.load(Ordering::SeqCst));
    }

    #[test]
    fn finally_runs_on_failure_and_preserves_error() {
        let promise = Promise::<u32>::new();
        let cleaned = Arc::new(AtomicBool::new(false));
        let marker = Arc::clone(&cleaned);
        let completed = promise.finally(move || marker.store(true, Ordering::SeqCst));
        assert!(promise.reject("boom"));
        assert_eq!(completed.state(), PromiseState::Rejected);
        assert_eq!(completed.error().as_deref(), Some("boom"));
        assert!(cleaned.load(Ordering::SeqCst));
    }

    #[test]
    fn state_machine_failure_rejects_completion_promise() {
        let mut machine = AsyncStateMachine::new(|_| Poll::<u32>::Failed("boom".into()));
        assert_eq!(machine.poll(), Poll::Failed("boom".into()));
        assert!(machine.is_completed());
        assert!(machine.promise().is_rejected());
        assert_eq!(machine.promise().error().as_deref(), Some("boom"));
        assert_eq!(machine.poll(), Poll::Failed("boom".into()));
    }

    #[test]
    fn microtasks_are_fifo_and_can_schedule_more_work() {
        let mut queue = MicrotaskQueue::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let first = Arc::clone(&seen);
        queue.schedule(move || first.lock().unwrap().push(1));
        let second = Arc::clone(&seen);
        queue.schedule(move || second.lock().unwrap().push(2));
        queue.run_all();
        assert_eq!(*seen.lock().unwrap(), vec![1, 2]);
        assert!(queue.is_empty());
    }

    #[test]
    fn scheduler_respects_budget_and_drains_later() {
        let mut scheduler = Scheduler::new();
        scheduler.schedule(|| {});
        scheduler.schedule(|| {});
        assert_eq!(scheduler.run(1), 1);
        assert_eq!(scheduler.pending(), 1);
        scheduler.run_until_idle();
        assert_eq!(scheduler.pending(), 0);
    }
}
