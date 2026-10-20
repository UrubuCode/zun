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
    /// A runtime-owned insertion-ordered hash map.
    Map,
    /// A runtime-owned insertion-ordered hash set.
    Set,
    /// A GC ephemeron-backed weak map.
    WeakMap,
    /// A managed byte-buffer view with a fixed element representation.
    BufferView(BufferViewKind),
    /// A dictionary-backed dynamic object.
    DynamicObject,
    /// A dynamically tagged value.
    Any,
    /// A non-capturing function reference identified by function index.
    Function(FuncIndex),
    /// `void` — only valid as a return type.
    Void,
    /// An instance of a user-declared class.
    Instance(ClassIndex),
    /// A type that failed to check. Propagates silently so one error does not
    /// cascade into dozens.
    Error,
}

/// The element representation of a managed buffer view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferViewKind {
    /// Mixed-width, possibly unaligned view.
    DataView,
    /// Unsigned 8-bit elements.
    U8,
    /// Unsigned 16-bit elements.
    U16,
    /// Unsigned 32-bit elements.
    U32,
    /// 64-bit floating-point elements.
    F64,
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

    /// Whether this is the generic unary callback type.
    pub fn is_dynamic_function(self) -> bool {
        self == Ty::Function(DYNAMIC_FUNCTION)
    }
}

/// Index of a function within the checked program.
pub type FuncIndex = u32;
/// Sentinel function index for the generic unary `Function` callback type.
pub const DYNAMIC_FUNCTION: FuncIndex = u32::MAX;
/// Sentinel function index for the generic unary boolean-predicate callback type.
pub const DYNAMIC_PREDICATE: FuncIndex = u32::MAX - 1;
/// Sentinel function index for the generic binary reducer callback type.
pub const DYNAMIC_REDUCER: FuncIndex = u32::MAX - 2;
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
    /// Synthetic closure functions kept separate until closure lowering is enabled.
    pub closures: Vec<ClosureFunction>,
}

/// How a closure obtains a captured binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// Copy the captured value into the closure environment.
    ByValue,
    /// Share a heap cell because the binding is mutated.
    ByCell,
}

/// Where a captured binding is read from when constructing a nested closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureSource {
    /// A local in the enclosing function.
    Local(LocalSlot),
    /// An entry in the enclosing closure's environment.
    Environment {
        environment_slot: u16,
        mode: CaptureMode,
    },
}

/// One binding stored in a closure environment, in declaration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    /// Source binding name used for diagnostics and environment access.
    pub name: String,
    /// Local slot in the defining function.
    pub outer_slot: LocalSlot,
    /// Slot in the synthetic function's environment layout.
    pub environment_slot: u16,
    /// Static type of the binding.
    pub ty: Ty,
    /// Binding or snapshot semantics.
    pub mode: CaptureMode,
    /// Source location for this binding in the enclosing scope.
    pub source: CaptureSource,
}

/// A capture together with the expression evaluated in the enclosing scope
/// when the closure environment is constructed.
#[derive(Debug)]
pub struct ClosureCapture {
    /// Environment layout entry.
    pub capture: Capture,
    /// Value or cell reference supplied by the enclosing function.
    pub value: Box<Expr>,
}

/// A closure body represented as a synthetic function with an explicit hidden
/// environment parameter. It is not produced until closure sema is enabled.
#[derive(Debug)]
pub struct ClosureFunction {
    /// Stable diagnostic/debug name.
    pub name: String,
    /// Number of user-visible parameters; the environment is lowered first.
    pub param_count: u16,
    /// User-visible rest parameter index, when present.
    pub rest_param: Option<u16>,
    /// Default expressions aligned with the user-visible parameters.
    pub defaults: Vec<Option<Expr>>,
    /// Types of parameters and locals in the synthetic body.
    pub locals: Vec<Ty>,
    /// Checked return type.
    pub return_ty: Ty,
    /// Captured environment layout.
    pub captures: Vec<Capture>,
    /// Checked body.
    pub body: Vec<Stmt>,
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
    /// Parameter index that receives variadic arguments, when present.
    pub rest_param: Option<u16>,
    /// Default expressions aligned with the declared parameters.
    pub defaults: Vec<Option<Expr>>,
    /// Types of all local slots, parameters first.
    pub locals: Vec<Ty>,
    /// Declared return type.
    pub return_ty: Ty,
    /// Whether this function is intended to lower to a stackless async state
    /// machine. The lowering pass remains gated until await suspension is
    /// represented in IL.
    pub is_async: bool,
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
    /// Initialize consecutive locals from an Array by index.
    ArrayDestructure {
        /// Destination local slots in index order.
        slots: Vec<LocalSlot>,
        /// Source array expression.
        init: Expr,
    },
    /// Initialize locals from dynamic properties by matching their names.
    ObjectDestructure {
        /// Destination local slots and property names in order.
        bindings: Vec<(LocalSlot, String)>,
        /// Source dynamic object expression.
        init: Expr,
    },
    /// Initialize a local whose storage is a shared closure cell.
    BoxedLet {
        /// Destination local slot containing the cell object.
        slot: LocalSlot,
        /// Initial cell value.
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
    /// `do { body } while (cond)` loop.
    DoWhile {
        /// The loop body.
        body: Vec<Stmt>,
        /// The condition checked after the body.
        cond: Expr,
    },
    /// `for (init; cond; update) { body }` loop.
    For {
        /// Optional initializer statement.
        init: Option<Box<Stmt>>,
        /// Loop condition.
        cond: Expr,
        /// Optional update expression.
        update: Option<Expr>,
        /// Loop body.
        body: Vec<Stmt>,
    },
    /// Iterate through the runtime iterator protocol.
    ForOf {
        iterator: Expr,
        iterator_slot: LocalSlot,
        done: Expr,
        next: Expr,
        binding: LocalSlot,
        body: Vec<Stmt>,
    },
    /// Execute a labeled statement.
    Labeled { label: String, body: Box<Stmt> },
    /// Exit the innermost enclosing loop or the named loop.
    Break(Option<String>),
    /// Continue the innermost enclosing loop or the named loop.
    Continue(Option<String>),
    /// Return, with a value unless the function returns `void`.
    Return(Option<Expr>),
    /// Transfer control to an enclosing exception handler.
    Throw(Expr),
    /// Protected body with typed handlers and optional cleanup.
    Try {
        body: Vec<Stmt>,
        catches: Vec<CatchClause>,
        finally_body: Option<Vec<Stmt>>,
    },
}

/// A checked exception handler.
#[derive(Debug)]
pub struct CatchClause {
    pub slot: LocalSlot,
    pub ty: Ty,
    pub body: Vec<Stmt>,
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
    /// The explicit JavaScript `null` value.
    Null,
    /// Read a local slot.
    Local(LocalSlot),
    /// Read the value from a shared cell stored in a local slot.
    CellLoad { slot: LocalSlot },
    /// Produce the shared cell object stored in a local slot.
    CellRef { slot: LocalSlot },
    /// Read the cell object behind a capture in the hidden environment.
    CaptureRef { environment_slot: u16 },
    /// Store a value through a shared cell.
    CellStore { slot: LocalSlot, value: Box<Expr> },
    /// Load a top-level function value.
    FunctionRef { func: FuncIndex },
    /// Construct a closure value from a synthetic function and environment.
    Closure {
        /// Index into [`Program::closures`].
        function: u32,
        /// Bindings copied/shared into the environment.
        captures: Vec<ClosureCapture>,
    },
    /// Read one entry from the hidden closure environment parameter.
    CaptureLoad {
        /// Environment layout slot.
        environment_slot: u16,
        /// Static type after unboxing.
        ty: Ty,
        /// Whether the entry is a shared mutable cell.
        mode: CaptureMode,
    },
    /// Write one entry in the hidden closure environment parameter.
    CaptureStore {
        /// Environment layout slot.
        environment_slot: u16,
        /// Static type of the assigned value.
        ty: Ty,
        /// Whether the entry is a shared mutable cell.
        mode: CaptureMode,
        /// Assigned value.
        value: Box<Expr>,
    },
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
    /// Select one of two values without evaluating the other branch.
    Conditional {
        condition: Box<Expr>,
        when_true: Box<Expr>,
        when_false: Box<Expr>,
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
    /// Call a function with an explicit argument count for its trailing rest parameter.
    CallRest {
        /// Index into [`Program::functions`].
        func: FuncIndex,
        /// Arguments in source order; the callee packs the trailing values.
        args: Vec<Expr>,
    },
    /// Call a zero-fixed-parameter rest function with an existing Array.
    CallRestArray {
        /// Index into [`Program::functions`].
        func: FuncIndex,
        /// Array expression containing the arguments.
        args: Box<Expr>,
    },
    /// Call a rest function with fixed arguments followed by an Array spread.
    CallRestArrayFixed {
        /// Index into [`Program::functions`].
        func: FuncIndex,
        /// Fixed arguments before the spread.
        fixed_args: Vec<Expr>,
        /// Array expression containing the rest arguments.
        args: Box<Expr>,
    },
    /// Invoke a function-valued expression indirectly.
    IndirectCall {
        /// Function value expression.
        callee: Box<Expr>,
        /// Arguments in declaration order.
        args: Vec<Expr>,
        /// Function index providing the static signature token.
        func: FuncIndex,
    },
    /// Invoke a function value with a trailing rest parameter.
    IndirectCallRest {
        /// Function-valued expression.
        callee: Box<Expr>,
        /// Arguments in source order.
        args: Vec<Expr>,
        /// Function index providing the static signature token.
        func: FuncIndex,
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
    /// Read a dynamic property, returning null when the receiver is null.
    OptionalDynamicGet { object: Box<Expr>, name: String },
    /// Null-safe call to a runtime-managed method. The receiver is args[0].
    OptionalCallBuiltin { id: u32, args: Vec<Expr> },
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
    /// Virtually invoke a method whose trailing parameter is variadic.
    MethodCallRest {
        /// Statically resolved method index.
        func: FuncIndex,
        /// Arguments including the receiver.
        args: Vec<Expr>,
    },
    /// Virtually invoke a zero-fixed-parameter rest method with an Array.
    MethodCallRestArray {
        /// Statically resolved method index.
        func: FuncIndex,
        /// Receiver expression.
        receiver: Box<Expr>,
        /// Array expression containing rest arguments.
        args: Box<Expr>,
    },
    /// Virtually invoke a rest method with fixed arguments and an Array.
    MethodCallRestArrayFixed {
        /// Statically resolved method index.
        func: FuncIndex,
        /// Receiver expression.
        receiver: Box<Expr>,
        /// Fixed arguments before the spread.
        fixed_args: Vec<Expr>,
        /// Array expression containing rest arguments.
        args: Box<Expr>,
    },
    /// Allocate a class instance, initializing fields positionally.
    New {
        /// The class being instantiated.
        class: ClassIndex,
        /// One initializer per field, in field-slot order.
        args: Vec<Expr>,
    },
    /// Allocate a managed array with a numeric length.
    ArrayNew {
        /// Number of elements to allocate.
        length: Box<Expr>,
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
    /// Read an element from a managed array.
    ArrayGet {
        /// The array expression.
        object: Box<Expr>,
        /// The numeric index expression.
        index: Box<Expr>,
    },
    /// Write an element in a managed array.
    ArraySet {
        /// The array expression.
        object: Box<Expr>,
        /// The numeric index expression.
        index: Box<Expr>,
        /// The assigned value.
        value: Box<Expr>,
    },
    /// Read the length of a managed array.
    ArrayLength {
        /// The array expression.
        object: Box<Expr>,
    },
    /// Remove and return the last element of a managed array.
    ArrayPop {
        /// The array expression.
        object: Box<Expr>,
    },
    /// Append a value and return the new array length.
    ArrayPush {
        /// The array expression.
        object: Box<Expr>,
        /// The value expression.
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
