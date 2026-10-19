//! The Zun TypeScript/JavaScript front-end: lexer, AST, and parser.
//!
//! Phase-1 scope: top-level functions, `let`, `if`/`else`, `while`, `return`,
//! calls, and arithmetic over numbers, strings, and booleans. The parser never
//! aborts — it reports [`Diagnostic`]s and recovers, so a driver always gets a
//! best-effort [`Program`] plus the full error list.
//!
//! [`Diagnostic`]: zun_common::Diagnostic

pub mod ast;
pub mod lexer;
pub mod parser;

pub use ast::{BinaryOp, Block, Expr, Function, Name, Param, Program, Stmt, UnaryOp};
pub use parser::parse;
