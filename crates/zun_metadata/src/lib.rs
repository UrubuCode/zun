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
//! - **tables** ([`tables`]): typed definition and reference rows that address
//!   heaps by index and each other by token.

pub mod heap;
pub mod tables;
pub mod token;

pub use heap::{BlobHeap, BlobIndex, StringHeap, StringIndex};
pub use tables::{
    FieldDef, MemberRef, MethodDef, ModuleRef, Tables, TypeDef, TypeRef,
};
pub use token::{TableKind, Token};
