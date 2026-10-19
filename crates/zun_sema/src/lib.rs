//! Name resolution and type checking for the Zun front-end.
//!
//! Input: the AST from `zun_syntax`. Output: the typed [`hir`], where every
//! name is a resolved local slot or function index and every expression carries
//! its checked type. Sound-by-default: phase 1 rejects what it cannot type
//! (there is no `any` yet), and type errors poison expressions with
//! [`hir::Ty::Error`] rather than cascading.

pub mod check;
pub mod hir;

pub use check::check;
pub use hir::{Program, Ty};
