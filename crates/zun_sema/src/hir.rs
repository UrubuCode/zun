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

/// A fixed-width integer kind: its bit width and signedness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntKind {
    /// Bit width: 8, 16, 32, or 64.
    pub bits: u8,
    /// Whether the integer is signed.
    pub signed: bool,
}

impl IntKind {
    /// `i8`..`i64`/`u8`..`u64` shorthands.
    pub const I8: IntKind = IntKind {
        bits: 8,
        signed: true,
    };
    /// See [`IntKind::I8`].
    pub const I16: IntKind = IntKind {
        bits: 16,
        signed: true,
    };
    /// See [`IntKind::I8`].
    pub const I32: IntKind = IntKind {
        bits: 32,
        signed: true,
    };
    /// See [`IntKind::I8`].
    pub const I64: IntKind = IntKind {
        bits: 64,
        signed: true,
    };
    /// See [`IntKind::I8`].
    pub const U8: IntKind = IntKind {
        bits: 8,
        signed: false,
    };
    /// See [`IntKind::I8`].
    pub const U16: IntKind = IntKind {
        bits: 16,
        signed: false,
    };
    /// See [`IntKind::I8`].
    pub const U32: IntKind = IntKind {
        bits: 32,
        signed: false,
    };
    /// See [`IntKind::I8`].
    pub const U64: IntKind = IntKind {
        bits: 64,
        signed: false,
    };
}

/// A floating-point kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatKind {
    /// 32-bit IEEE-754.
    F32,
    /// 64-bit IEEE-754 (`number`/`float`).
    F64,
}

/// A checked type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    /// A fixed-width integer (`int` = i32; plus i8..i64, u8..u64).
    Int(IntKind),
    /// A floating-point number (`number`/`float` = f64; plus f32).
    Float(FloatKind),
    /// `string`.
    String,
    /// `boolean`.
    Boolean,
    /// A managed contiguous array.
    Array,
    /// A dictionary-backed dynamic object.
    DynamicObject,
    /// A dynamically tagged value.
    Any,
    /// `void` — only valid as a return type.
    Void,
    /// An instance of a user-declared class.
    Instance(ClassIndex),
    /// A type that failed to check. Propagates silently so one error does not
    /// cascade into dozens.
    Error,
}

impl Ty {
    /// `number`/`float` — 64-bit float.
    pub const F64: Ty = Ty::Float(FloatKind::F64);
    /// 32-bit float.
    pub const F32: Ty = Ty::Float(FloatKind::F32);
    /// `int` — 32-bit signed integer.
    pub const INT: Ty = Ty::Int(IntKind::I32);

    /// Whether this type is already poisoned, which suppresses follow-on
    /// mismatch diagnostics.
    pub fn is_error(self) -> bool {
        self == Ty::Error
    }

    /// Whether this is any integer type.
    pub fn is_int(self) -> bool {
        matches!(self, Ty::Int(_))
    }

    /// Whether this is any float type.
    pub fn is_float(self) -> bool {
        matches!(self, Ty::Float(_))
    }
}

/// Index of a function within the checked program.
pub type FuncIndex = u32;
/// A local variable slot within one function (params occupy the first slots).
pub type LocalSlot = u16;

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
    /// Call a runtime intrinsic by its stable id (see `zun_intrinsics`).
    CallBuiltin {
        /// The intrinsic dispatch id.
        id: u32,
        /// Arguments in order.
        args: Vec<Expr>,
    },
    /// Allocate a dynamic object.
    DynamicNew,
    /// Read a named property from a dynamic object.
    DynamicGet {
        /// The object expression.
        object: Box<Expr>,
        /// The property name.
        name: String,
    },
    /// Write an Any value to a named dynamic property.
    DynamicSet {
        /// The object expression.
        object: Box<Expr>,
        /// The property name.
        name: String,
        /// The dynamic value.
        value: Box<Expr>,
    },
    /// Explicitly box a statically typed value as Any.
    BoxAny {
        /// The value to box.
        value: Box<Expr>,
    },
    /// Call a function exported by another source module.
    ExternalCall {
        /// Module identity used by the linker.
        module: String,
        /// Exported function name.
        name: String,
        /// Arguments in source order.
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
    /// A numeric conversion (`as`). The target type is the expression's `ty`.
    Convert {
        /// The value being converted.
        value: Box<Expr>,
    },
    /// A poisoned expression that already produced a diagnostic.
    Error,
}
