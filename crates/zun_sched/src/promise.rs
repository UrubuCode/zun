//! The promise object.
//!
//! Three concerns live in one cell: what state the promise is in, what is
//! waiting on it, and the one rule that a settlement happens at most once. Every
//! rule about a promise is enforced here, at the object, rather than re-checked
//! wherever one is settled — a promise that can be settled twice by one caller
//! and not another is a promise whose invariants are a convention.
//!
//! # The control/value split
//!
//! A [`Settlement`] carries **no value**. Whether a promise fulfilled or
//! rejected is control information the machine needs to know who to wake and
//! how; *what* it fulfilled with is a value, and the value lives in a side table
//! in the language layer (for now, the managed promise cell in `zun_gc`),
//! keyed by [`PromiseId`]. This machine never touches a `Value` or a heap
//! `Obj`, which is what keeps it a control machine and keeps the GC out of it.

use super::ContinuationId;

/// Identifies a promise: an index into a [`PromiseTable`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct PromiseId(pub(crate) u32);

impl PromiseId {
    /// The table index.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// How a promise ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Settlement {
    /// It produced a value.
    Fulfilled,
    /// It failed.
    ///
    /// A rejection resumes its waiter through the unwinding path rather than
    /// delivering a value, which is why this is one settlement with two
    /// outcomes and not two unrelated mechanisms.
    Rejected,
}

/// What a promise is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PromiseState {
    /// Not yet settled.
    Pending,
    /// Settled, one way or the other.
    Settled(Settlement),
}

/// Something waiting on a promise.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Waiter {
    /// A parked frame.
    Continuation(ContinuationId),
    /// Another promise, which settles the same way this one does.
    Promise(PromiseId),
}

/// What attaching a waiter did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Attachment {
    /// The promise is pending; the waiter will be woken when it settles.
    Parked,
    /// The promise had already settled; the waiter is runnable now.
    ReadyNow(Settlement),
}

/// Why an adoption was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdoptError {
    /// A promise cannot wait on itself, directly or through a chain.
    ///
    /// Nothing could ever settle it, so this is a program that hangs. Refusing
    /// is the only outcome that says so.
    Cycle,
}

/// What settling a promise produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SettleOutcome {
    /// Whether this settlement took effect.
    ///
    /// A second settlement is dropped rather than rejected: the first result
    /// wins, and a caller that settles twice is usually two paths racing rather
    /// than a mistake worth failing on.
    pub accepted: bool,
    /// Continuations that became runnable, in the order they attached.
    pub woken: Vec<ContinuationId>,
}

#[derive(Debug)]
struct PromiseCell {
    state: PromiseState,
    waiters: Vec<Waiter>,
}

/// Every promise one running program knows about.
#[derive(Debug, Default)]
pub struct PromiseTable {
    cells: Vec<PromiseCell>,
}

impl PromiseTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a pending promise.
    pub fn create(&mut self) -> PromiseId {
        let id = PromiseId(self.cells.len() as u32);
        self.cells.push(PromiseCell {
            state: PromiseState::Pending,
            waiters: Vec::new(),
        });
        id
    }

    /// What a promise is doing.
    pub fn state(&self, id: PromiseId) -> PromiseState {
        self.cell(id).state
    }

    /// Attaches a parked frame to a promise.
    pub fn attach(&mut self, id: PromiseId, waiter: ContinuationId) -> Attachment {
        match self.cell(id).state {
            PromiseState::Settled(settlement) => Attachment::ReadyNow(settlement),
            PromiseState::Pending => {
                self.cell_mut(id).waiters.push(Waiter::Continuation(waiter));
                Attachment::Parked
            }
        }
    }

    /// Makes one promise settle however another does.
    ///
    /// Adoption is not a separate mechanism: the adopting promise becomes an
    /// ordinary waiter on the other one. That is what makes a chain of them
    /// terminate without special handling — each link wakes the next exactly
    /// once, through the same path a parked frame takes.
    pub fn adopt(&mut self, id: PromiseId, other: PromiseId) -> Result<Attachment, AdoptError> {
        if self.would_cycle(id, other) {
            return Err(AdoptError::Cycle);
        }

        match self.cell(other).state {
            PromiseState::Settled(settlement) => {
                self.settle(id, settlement);
                Ok(Attachment::ReadyNow(settlement))
            }
            PromiseState::Pending => {
                self.cell_mut(other).waiters.push(Waiter::Promise(id));
                Ok(Attachment::Parked)
            }
        }
    }

    /// Settles a promise, waking everything that waited on it.
    ///
    /// Adopting promises are settled transitively here rather than left to a
    /// later pass, so that by the time this returns, every promise that could
    /// have settled has, and every continuation that became runnable is reported
    /// once, in attach order.
    pub fn settle(&mut self, id: PromiseId, settlement: Settlement) -> SettleOutcome {
        if self.cell(id).state != PromiseState::Pending {
            return SettleOutcome {
                accepted: false,
                woken: Vec::new(),
            };
        }

        let mut woken = Vec::new();
        // A FIFO so adopting promises settle in the order they attached, which
        // keeps woken continuations in a stable, attach-ordered sequence.
        let mut pending = std::collections::VecDeque::new();
        pending.push_back((id, settlement));

        while let Some((current, settlement)) = pending.pop_front() {
            if self.cell(current).state != PromiseState::Pending {
                continue;
            }
            self.cell_mut(current).state = PromiseState::Settled(settlement);

            for waiter in std::mem::take(&mut self.cell_mut(current).waiters) {
                match waiter {
                    Waiter::Continuation(continuation) => woken.push(continuation),
                    Waiter::Promise(adopter) => pending.push_back((adopter, settlement)),
                }
            }
        }

        SettleOutcome {
            accepted: true,
            woken,
        }
    }

    /// How many promises exist.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether none exist.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Whether `id` already reaches `other` through an adoption chain, which
    /// adopting `other` would close into a loop nothing outside could break.
    fn would_cycle(&self, id: PromiseId, other: PromiseId) -> bool {
        if id == other {
            return true;
        }
        let mut seen = vec![id];
        let mut frontier = vec![id];
        while let Some(current) = frontier.pop() {
            for waiter in &self.cell(current).waiters {
                if let Waiter::Promise(next) = waiter {
                    if *next == other {
                        return true;
                    }
                    if !seen.contains(next) {
                        seen.push(*next);
                        frontier.push(*next);
                    }
                }
            }
        }
        false
    }

    fn cell(&self, id: PromiseId) -> &PromiseCell {
        self.cells
            .get(id.index())
            .expect("promise belongs to this table")
    }

    fn cell_mut(&mut self, id: PromiseId) -> &mut PromiseCell {
        self.cells
            .get_mut(id.index())
            .expect("promise belongs to this table")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_promise_is_pending() {
        let mut table = PromiseTable::new();
        let promise = table.create();
        assert_eq!(table.state(promise), PromiseState::Pending);
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn attach_parks_while_pending_and_wakes_in_attach_order() {
        let mut table = PromiseTable::new();
        let promise = table.create();
        assert_eq!(table.attach(promise, ContinuationId(0)), Attachment::Parked);
        assert_eq!(table.attach(promise, ContinuationId(1)), Attachment::Parked);
        let outcome = table.settle(promise, Settlement::Fulfilled);
        assert!(outcome.accepted);
        assert_eq!(outcome.woken, vec![ContinuationId(0), ContinuationId(1)]);
    }

    #[test]
    fn attach_after_settle_is_ready_now_and_does_not_queue() {
        let mut table = PromiseTable::new();
        let promise = table.create();
        table.settle(promise, Settlement::Rejected);
        assert_eq!(
            table.attach(promise, ContinuationId(0)),
            Attachment::ReadyNow(Settlement::Rejected)
        );
    }

    #[test]
    fn double_settle_keeps_the_first_result() {
        let mut table = PromiseTable::new();
        let promise = table.create();
        assert!(table.settle(promise, Settlement::Fulfilled).accepted);
        assert!(!table.settle(promise, Settlement::Rejected).accepted);
        assert_eq!(table.state(promise), PromiseState::Settled(Settlement::Fulfilled));
    }

    #[test]
    fn a_long_adoption_chain_settles_end_to_end() {
        let mut table = PromiseTable::new();
        let a = table.create();
        let b = table.create();
        let c = table.create();
        table.adopt(a, b).unwrap();
        table.adopt(b, c).unwrap();
        table.attach(a, ContinuationId(0));
        let outcome = table.settle(c, Settlement::Fulfilled);
        assert!(outcome.accepted);
        assert_eq!(table.state(a), PromiseState::Settled(Settlement::Fulfilled));
        assert_eq!(table.state(b), PromiseState::Settled(Settlement::Fulfilled));
        assert_eq!(outcome.woken, vec![ContinuationId(0)]);
    }
}
