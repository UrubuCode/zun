//! JIT runtime thunks: self-contained `extern "C"` symbols linked into
//! generated native code by address. They form an FFI contract with the code
//! generator (`lib.rs`) — never rename a symbol or change its signature,
//! attributes or visibility.
//!
//! Grouped by subject; `lib.rs` re-exports every public symbol at the crate
//! root with `pub use thunks::*;`, preserving the original paths.

mod buffers;
mod collections;
mod misc;
mod string;

pub use buffers::*;
pub use collections::*;
pub use misc::*;
pub use string::*;
