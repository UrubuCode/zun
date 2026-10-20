//! Minimal Promise and microtask primitives for the VM scheduler.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

const PENDING: u8 = 0;
const RESOLVED: u8 = 1;

struct Inner<T> {
    value: Option<T>,
    continuation: Option<Box<dyn FnOnce(T) + Send + 'static>>,
}

/// A single-assignment future with one continuation, matching the runtime
/// representation planned for Promise state machines.
pub struct Promise<T> {
    state: AtomicU8,
    inner: Mutex<Inner<T>>,
}

impl<T> core::fmt::Debug for Promise<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Promise")
            .field("resolved", &(self.state.load(Ordering::Acquire) == RESOLVED))
            .finish_non_exhaustive()
    }
}

impl<T: Send + 'static> Promise<T> {
    /// Create a pending promise.
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(PENDING),
            inner: Mutex::new(Inner {
                value: None,
                continuation: None,
            }),
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
                assert!(inner.continuation.is_none(), "promise already has a continuation");
                inner.continuation = continuation.take();
                None
            } else {
                inner.value.take()
            }
        };
        if let Some(value) = value {
            continuation
                .take()
                .expect("late continuation missing")(value);
        }
    }

    /// Whether the promise has already been resolved.
    pub fn is_resolved(&self) -> bool {
        self.state.load(Ordering::Acquire) == RESOLVED
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
