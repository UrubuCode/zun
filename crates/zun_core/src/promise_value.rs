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
}

impl ManagedPromise {
    pub fn new() -> Self {
        Self {
            state: ManagedPromiseState::Pending,
            result: None,
            error: None,
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

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
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
}

impl Default for ManagedPromise {
    fn default() -> Self {
        Self::new()
    }
}
