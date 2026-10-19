//! Zun metadata: the data model that describes types, members, and references.
//!
//! Metadata is the half of the `.zm` format that ZIL code points into. It is
//! deliberately split into:
//!
//! - **tokens** ([`token`]): compact, layout-independent handles ZIL uses to
//!   name rows, so code never embeds offsets.
//! - **heaps** ([`heap`]): content-addressed pools of strings and blobs that
//!   rows reference by offset.
//!
//! Definition and reference tables build on these and land in a later step.

pub mod heap;
pub mod token;

pub use heap::{BlobHeap, BlobIndex, StringHeap, StringIndex};
pub use token::{TableKind, Token};
