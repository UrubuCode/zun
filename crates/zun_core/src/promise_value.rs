use crate::Any;

/// Settlement state stored by a managed Promise object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedPromiseState {
    Pending,
    Resolved,
    Rejected,
}

/// Heap-side payload for a Promise state machine.
///
/// The result is an explicit dynamic value so the heap can trace an object
/// result without coupling the core contract to the interpreter's `Value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedPromise {
    state: ManagedPromiseState,
    result: Option<Any>,
    error: Option<String>,
    /// A rejection reason carried as a first-class value (an object such as an
    /// `AggregateError`, or any boxed value) when the rejection was raised with
    /// one. `error` keeps the display string either way (the message, or the
    /// stringified reason) so the string surface — `await`'s thrown `Value`,
    /// `allSettled` descriptors — still works. When `reason` is set, the value
    /// side takes precedence (delivered to `catch`/`await`); when it is `None`,
    /// the rejection was a plain string message.
    reason: Option<Any>,
}

impl ManagedPromise {
    pub fn new() -> Self {
        Self {
            state: ManagedPromiseState::Pending,
            result: None,
            error: None,
            reason: None,
        }
    }

    pub fn state(&self) -> ManagedPromiseState {
        self.state
    }

    pub fn is_ready(&self) -> bool {
        self.state != ManagedPromiseState::Pending
    }

    pub fn result(&self) -> Option<Any> {
        self.result
    }

    pub fn rewrite_object_references(&mut self, map: impl Fn(usize) -> usize + Copy) -> usize {
        let mut rewritten = 0;
        if let Some(value) = self.result {
            let mapped = value.map_object_address(map);
            if mapped != value {
                self.result = Some(mapped);
                rewritten += 1;
            }
        }
        if let Some(value) = self.reason {
            let mapped = value.map_object_address(map);
            if mapped != value {
                self.reason = Some(mapped);
                rewritten += 1;
            }
        }
        rewritten
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The first-class rejection reason, if the rejection carried one (as
    /// opposed to a plain string message).
    pub fn reason(&self) -> Option<Any> {
        self.reason
    }

    pub fn resolve(&mut self, value: Any) -> bool {
        if self.state != ManagedPromiseState::Pending {
            return false;
        }
        self.state = ManagedPromiseState::Resolved;
        self.result = Some(value);
        true
    }

    pub fn reject(&mut self, error: impl Into<String>) -> bool {
        if self.state != ManagedPromiseState::Pending {
            return false;
        }
        self.state = ManagedPromiseState::Rejected;
        self.error = Some(error.into());
        true
    }

    /// Reject with a first-class reason value plus its display message. The
    /// value is delivered to `catch`/`await`; the message keeps the string
    /// surface working.
    pub fn reject_value(&mut self, reason: Any, message: impl Into<String>) -> bool {
        if self.state != ManagedPromiseState::Pending {
            return false;
        }
        self.state = ManagedPromiseState::Rejected;
        self.error = Some(message.into());
        self.reason = Some(reason);
        true
    }
}

impl Default for ManagedPromise {
    fn default() -> Self {
        Self::new()
    }
}
