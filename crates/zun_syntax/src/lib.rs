//! The Zun TypeScript/JavaScript front-end: the shared [`ast`] and the OXC
//! adapter ([`oxc`]), which parses source into an [`ast::Program`].

pub mod ast;
pub mod oxc;

pub use ast::{
    BinaryOp, Block, Class, ClassField, Expr, Function, FunctionExpr, Import, Name, Param, Program,
    Stmt, TemplatePart, TypeAnnotation, UnaryOp,
};
