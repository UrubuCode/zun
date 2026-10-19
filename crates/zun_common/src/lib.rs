//! Shared primitives for the Zun platform.
//!
//! This crate sits at the bottom of the dependency graph: both the compile-time
//! and runtime crates may depend on it, and it depends on nothing of ours. It
//! holds small, stable building blocks — typed indices, string interning, source
//! spans, and diagnostics — that would otherwise be duplicated across layers.

pub mod diagnostics;
pub mod id;
pub mod intern;
pub mod span;

pub use diagnostics::{Diagnostic, Severity};
pub use intern::{Interner, Symbol};
pub use span::Span;
