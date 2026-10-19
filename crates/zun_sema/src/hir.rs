//! The typed high-level IR.
//!
//! Sema lowers the AST into this form: names become local slots or function
//! indices, every expression carries its checked [`Ty`], and lexical scoping is
//! already resolved (shadowing got distinct slots). `zun_ilgen` consumes HIR
//! without ever looking back at the AST or source text.

use zun_common::Span;
pub use zun_syntax::ast::{BinaryOp, UnaryOp};

/// Index of a class within the checked program.
pub type ClassIndex = u32;

/// A checked type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    /// `number` — IEEE-754 f64, as in JS.
    Number,
    /// `string`.
    String,
    /// `boolean`.
    Boolean,
    /// `void` — only valid as a return type.
    Void,
    /// An instance of a user-declared class.
    Instance(ClassIndex),
    /// A type that failed to check. Propagates silently so one error does not
    /// cascade into dozens.
    Error,
}

impl Ty {
    /// Whether this type (or the other) is already poisoned, in which case
    /// further mismatch diagnostics are suppressed.
    pub fn is_error(self) -> bool {
        self == Ty::Error
    }
}

/// Index of a function within the checked program.
pub type FuncIndex = u32;
/// A local variable slot within one function (params occupy the first slots).
pub type LocalSlot = u16;

/// A built-in operation provided by the runtime rather than user code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    /// `print(value)` — write a value and a newline to stdout.
    Print,
}

/// A checked program.
#[derive(Debug)]
pub struct Program {
    /// All functions; a [`FuncIndex`] indexes this vector.
    pub functions: Vec<Function>,
    /// All classes; a [`ClassIndex`] indexes this vector.
    pub classes: Vec<Class>,
    /// Index of `main`, if the program declares one.
    pub main: Option<FuncIndex>,
}

/// A checked class: its base, full field layout, and methods.
#[derive(Debug)]
pub struct Class {
    /// The class name.
    pub name: String,
    /// Base class index, if this class `extends` another.
    pub base: Option<ClassIndex>,
    /// The full field layout, inherited fields first then own, in slot order.
    pub fields: Vec<ClassField>,
    /// Global function indices of this class's own methods, in declaration
    /// order (their MethodDef RIDs are contiguous and in this order).
    pub methods: Vec<FuncIndex>,
}

/// A checked class field.
#[derive(Debug)]
pub struct ClassField {
    /// The field name.
    pub name: String,
    /// The field's type.
    pub ty: Ty,
}

/// A checked function.
#[derive(Debug)]
pub struct Function {
    /// The function name (owned; the HIR outlives the source text).
    pub name: String,
    /// Number of parameters; they occupy local slots `0..param_count`.
    pub param_count: u16,
    /// Types of all local slots, parameters first.
    pub locals: Vec<Ty>,
    /// Declared return type.
    pub return_ty: Ty,
    /// The body statements.
    pub body: Vec<Stmt>,
}

/// A checked statement.
#[derive(Debug)]
pub enum Stmt {
    /// Store `init` into a local slot (covers `let`).
    Let {
        /// Destination slot.
        slot: LocalSlot,
        /// The initializer.
        init: Expr,
    },
    /// Evaluate for effect; the result (if any) is discarded.
    Expr(Expr),
    /// `if (cond) { then } else { otherwise }`.
    If {
        /// Boolean condition.
        cond: Expr,
        /// Taken when true.
        then_body: Vec<Stmt>,
        /// Taken when false (possibly empty).
        else_body: Vec<Stmt>,
    },
    /// `while (cond) { body }`.
    While {
        /// Boolean condition.
        cond: Expr,
        /// The loop body.
        body: Vec<Stmt>,
    },
    /// Return, with a value unless the function returns `void`.
    Return(Option<Expr>),
}

/// A checked expression: a kind plus its type and origin.
#[derive(Debug)]
pub struct Expr {
    /// What the expression is.
    pub kind: ExprKind,
    /// Its checked type.
    pub ty: Ty,
    /// Where it came from, for later diagnostics (e.g. division by zero
    /// warnings, debug info).
    pub span: Span,
}

/// The kinds of checked expressions.
#[derive(Debug)]
pub enum ExprKind {
    /// A number literal.
    Number(f64),
    /// A string literal.
    String(String),
    /// A boolean literal.
    Boolean(bool),
    /// Read a local slot.
    Local(LocalSlot),
    /// Write a local slot; the expression's value is the stored value.
    Assign {
        /// Destination slot.
        slot: LocalSlot,
        /// The stored value.
        value: Box<Expr>,
    },
    /// A binary operation. Operand types already checked.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
    /// A unary operation.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// The operand.
        operand: Box<Expr>,
    },
    /// Call a user function.
    Call {
        /// Index into [`Program::functions`].
        func: FuncIndex,
        /// Arguments in order.
        args: Vec<Expr>,
    },
    /// Call a runtime builtin.
    CallBuiltin {
        /// Which builtin.
        builtin: Builtin,
        /// Arguments in order.
        args: Vec<Expr>,
    },
    /// A virtual method call, dispatched on the receiver's runtime class.
    MethodCall {
        /// The statically resolved target (the method visible in the receiver's
        /// static class); the runtime may dispatch to an override of it.
        func: FuncIndex,
        /// Arguments, with the receiver (`this`) first.
        args: Vec<Expr>,
    },
    /// Allocate a class instance, initializing fields positionally.
    New {
        /// The class being instantiated.
        class: ClassIndex,
        /// One initializer per field, in field-slot order.
        args: Vec<Expr>,
    },
    /// Read `object`'s field at `field_slot`.
    FieldGet {
        /// The object expression.
        object: Box<Expr>,
        /// The class whose layout defines the slot.
        class: ClassIndex,
        /// Zero-based field slot.
        field_slot: u16,
    },
    /// Write `object`'s field at `field_slot`; the stored value is the result.
    FieldSet {
        /// The object expression.
        object: Box<Expr>,
        /// The class whose layout defines the slot.
        class: ClassIndex,
        /// Zero-based field slot.
        field_slot: u16,
        /// The stored value.
        value: Box<Expr>,
    },
    /// A poisoned expression that already produced a diagnostic.
    Error,
}
