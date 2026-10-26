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
    /// A GC-owned contiguous byte buffer.
    Buffer,
    /// A runtime-owned insertion-ordered hash map.
    Map,
    /// A runtime-owned insertion-ordered hash set.
    Set,
    /// A GC ephemeron-backed weak map.
    WeakMap,
    /// A GC ephemeron-backed weak set (weakly-held object members).
    WeakSet,
    /// A GC-backed single weak reference.
    WeakRef,
    /// A Rust-engine-backed regular expression.
    RegExp,
    /// An arbitrary-precision integer backed by the managed heap.
    BigInt,
    /// A GC-backed finalization callback registry.
    FinalizationRegistry,
    /// A managed target/handler proxy pair.
    Proxy,
    /// A managed asynchronous result object.
    Promise,
    /// A managed byte-buffer view with a fixed element representation.
    BufferView(BufferViewKind),
    /// A dictionary-backed dynamic object.
    DynamicObject,
    /// A unique symbol primitive (a first-class `SymbolId`-backed value). Its
    /// runtime representation is an immediate `Any` (`AnyTag::Symbol`).
    Symbol,
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
    // One variant per row of the typed-element master table in `zun_core`,
    // named exactly as its `TypedElement` variant is: the generated wiring in
    // `typed_arrays.rs` pairs them by name, and `typed_array_kinds.rs` fails
    // if the two lists drift apart.
    /// Signed 8-bit elements (`Int8Array`).
    I8,
    /// Unsigned 8-bit elements (`Uint8Array`).
    U8,
    /// Unsigned 8-bit elements that clamp rather than wrap
    /// (`Uint8ClampedArray`).
    U8Clamped,
    /// Signed 16-bit elements (`Int16Array`).
    I16,
    /// Unsigned 16-bit elements (`Uint16Array`).
    U16,
    /// Signed 32-bit elements (`Int32Array`).
    I32,
    /// Unsigned 32-bit elements (`Uint32Array`).
    U32,
    /// 32-bit floating-point elements (`Float32Array`).
    F32,
    /// 64-bit floating-point elements (`Float64Array`).
    F64,
    /// Signed 64-bit elements (`BigInt64Array`).
    I64,
    /// Unsigned 64-bit elements (`BigUint64Array`).
    U64,
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
    /// Index of the synthetic module initializer, if one was generated. This
    /// single, eager, no-arg, void function (the Zun `<Module>`-initializer
    /// analogue, doc 02 area 10) is run once before the entry point. Its first
    /// client is the declarative realm seed (PLAN §3.1). It lives in `functions`
    /// like any other function but is never named or called from user IL.
    pub module_init: Option<FuncIndex>,
    /// Synthetic closure functions kept separate until closure lowering is enabled.
    pub closures: Vec<ClosureFunction>,
    /// Static regexes detected at check time (the `[GeneratedRegex]` analog): a
    /// regex literal or a `new RegExp(const, const)` used directly by `.test` /
    /// `.isMatch`. `zun_ilgen` compiles each to a synthesized ZIL matcher method
    /// (via `zun_regex::zil`) and lowers the matching [`ExprKind::RegexMatch`]
    /// to a call to it, falling back to the runtime engine when the pattern is
    /// outside the compilable subset (lookaround/backref/etc.). A
    /// [`ExprKind::RegexMatch`] `matcher` indexes this vector.
    pub regex_matchers: Vec<CompiledRegex>,
    /// Source spans (start, end) of runtime code-generation sites
    /// (new Function / Function(src) / eval). Non-empty means the module
    /// requires the codegen capability.
    pub codegen_sites: Vec<(u32, u32)>,
}

/// A static regex to be synthesized to a ZIL matcher at IL-generation time.
#[derive(Debug, Clone)]
pub struct CompiledRegex {
    /// The pattern source (no enclosing slashes).
    pub pattern: String,
    /// The flag characters (e.g. `"i"`, `"s"`, `""`).
    pub flags: String,
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

/// A decorator carried from the AST. Inert in Phase 1 (syntax only).
///
/// This is the HIR-level carry of PLAN §3.5's decorator syntax. Phase 2 (the
/// macro-expansion hook) reads decorators to drive compile-time transforms;
/// `callee` is the decorator's source text (`foo`, `ns.dec`) and `args` is
/// `Some` iff the `@foo(...)` call form was used, holding each argument's
/// source text. See `check_classes` for where this is populated, and the AST
/// (`ast::Class`/`ast::ClassField`/`ast::Function` `decorators`) for the
/// field- and method-level lists that Phase 2 consumes before final sema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decorator {
    /// The decorator callee as written in source (`foo`, `ns.dec`).
    pub callee: String,
    /// Argument source texts, present iff the `@foo(...)` call form was used.
    pub args: Option<Vec<String>>,
}

/// A checked class: its base, full field layout, and methods.
#[derive(Debug)]
pub struct Class {
    /// The class name.
    pub name: String,
    /// Base class index, if this class `extends` another.
    pub base: Option<ClassIndex>,
    /// Decorators applied to the class, carried from the AST. Inert in Phase 1.
    pub decorators: Vec<Decorator>,
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
    /// Whether the field is stored on the class rather than an instance.
    pub is_static: bool,
    /// Whether this field originated from `#` private syntax.
    pub is_private: bool,
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
    /// Whether this function is a class-static member.
    pub is_static: bool,
    /// Whether this function is a generator state-machine candidate.
    pub is_generator: bool,
    /// Structural state-machine plan for an async function. The lowering pass
    /// consumes this after Promise suspension points have an IL representation.
    pub async_plan: Option<AsyncStateMachinePlan>,
    /// Structural state-machine plan for a generator function.
    pub generator_plan: Option<GeneratorStateMachinePlan>,
    /// The body statements.
    pub body: Vec<Stmt>,
}

/// Compiler-owned metadata for a stackless async state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncStateMachinePlan {
    /// State zero is the entry state; later states resume after an await.
    pub state_count: u16,
    /// Number of source await expressions represented by the plan.
    pub await_count: u16,
    /// Locals whose lifetime crosses a suspension point.
    pub hoisted_locals: Vec<LocalSlot>,
}

/// Compiler-owned metadata for a stackless generator state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratorStateMachinePlan {
    /// State zero is entry; each yield adds a resumable state.
    pub state_count: u16,
    /// Number of source yield expressions.
    pub yield_count: u16,
    /// Locals conservatively preserved by the suspended iterator object.
    pub hoisted_locals: Vec<LocalSlot>,
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
    /// A flat sequence of statements lowered from one source construct
    /// (e.g. a destructuring declaration desugared into a temp plus per-name
    /// binding stores). Emitted inline; introduces no new scope.
    Group(Vec<Stmt>),
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

/// One populated member of a [`ExprKind::DynamicObjectLiteral`].
#[derive(Debug)]
pub enum ObjectEntry {
    /// A data property: `key: value` (or shorthand). The value is boxed to
    /// `Any`.
    Data {
        /// How the property key is produced.
        key: ObjectKey,
        /// The property value, already boxed to `Any`.
        value: Expr,
    },
    /// Install the member via a native builtin that takes the object as its
    /// first argument (method shorthand, computed-symbol method, accessor, or
    /// object spread). At lowering time the object handle is duplicated, `args`
    /// are pushed after it, the builtin is invoked, and its result discarded.
    ///
    /// Used for:
    /// - method shorthand → `dynamicSetFunction(obj, name, fn)`;
    /// - computed-symbol method → `dynamicSetSymbolFunction(obj, sym, fn)`;
    /// - getter/setter → `objectDefineProperty(obj, name, descriptor)`;
    /// - spread → `objectAssign(obj, source)`.
    Install {
        /// Native dispatch id (resolved in sema).
        builtin: u32,
        /// Arguments pushed after the (duplicated) object handle.
        args: Vec<Expr>,
    },
}

/// One part of an [`ExprKind::ArraySpreadLiteral`].
#[derive(Debug)]
pub enum ArrayPart {
    /// A present element pushed at the next index.
    Element(Expr),
    /// An elision/hole: push `undefined` at the next index.
    Hole,
    /// Splice an iterable source's elements in via `arraySpreadAppend`.
    Spread {
        /// `arraySpreadAppend` native dispatch id.
        builtin: u32,
        /// The iterable source (boxed to `Any`).
        source: Expr,
    },
}

/// The key of a data [`ObjectEntry`].
#[derive(Debug)]
pub enum ObjectKey {
    /// A statically known string key stored via `StoreDynamic`.
    Named(String),
    /// A runtime-evaluated key stored via `StoreDynamicValue`; a `Symbol` value
    /// routes to the symbol-keyed slot, a string to the ordinary property.
    Computed(Expr),
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
    /// Suspend the async state machine until the operand's Promise settles.
    /// This remains gated until async state-machine lowering is enabled.
    Await { operand: Box<Expr> },
    /// Dynamic `import(specifier)`: a Promise of the module namespace object.
    /// Async and dynamic, so it is lowered to the Tier-0-only `import.dynamic`.
    ImportDynamic { specifier: Box<Expr> },
    /// Yield a value from a generator state machine.
    ///
    /// The frontend preserves this node even while generator execution
    /// remains gated; lowering emits the existing verified yield opcode so
    /// the resumable runtime can consume it in a later phase.
    Yield { value: Box<Expr> },
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
    /// Create a managed generator iterator with its initial arguments.
    GeneratorCall {
        func: FuncIndex,
        args: Vec<Expr>,
        native_id: u32,
    },
    /// `Function.prototype.bind`: allocate a bound function value that fixes the
    /// `[thisArg, ...partialArgs]` prefix of `callee`. Lowered by building a
    /// managed array of the prepended arguments and invoking `native_id`
    /// (`functionBind`), which returns a first-class callable bound function.
    FunctionBind {
        /// The target function value being bound.
        callee: Box<Expr>,
        /// The prepended arguments `[thisArg, ...partialArgs]`.
        args: Vec<Expr>,
        /// Resolved `functionBind` native builtin id.
        native_id: u32,
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
    /// Null-safe indirect call through a function value.
    OptionalIndirectCall {
        callee: Box<Expr>,
        args: Vec<Expr>,
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
    /// Load the per-realm global singleton.
    RealmGlobal,
    /// Read a named property from a dynamic object.
    DynamicGet {
        /// The object expression.
        object: Box<Expr>,
        /// The property name.
        name: String,
    },
    /// Read a dynamic property whose name is evaluated at runtime.
    DynamicGetValue { object: Box<Expr>, key: Box<Expr> },
    /// Read a dynamic property, returning null when the receiver is null.
    OptionalDynamicGet { object: Box<Expr>, name: String },
    /// Read a dynamic property by key, returning null for a nullish receiver.
    OptionalDynamicGetValue { object: Box<Expr>, key: Box<Expr> },
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
    /// Write an Any value using a runtime-evaluated property name.
    DynamicSetValue { object: Box<Expr>, key: Box<Expr>, value: Box<Expr> },
    /// Allocate a `DynamicObject` and populate it from an object literal.
    ///
    /// Entries are stored in source order; the object is the result value.
    DynamicObjectLiteral { entries: Vec<ObjectEntry> },
    /// Test a static regex (the `[GeneratedRegex]` analog). `matcher` indexes
    /// [`Program::regex_matchers`]; `args[0]` is the subject string. Lowered by
    /// `zun_ilgen` to a call to the synthesized ZIL matcher (or, when the
    /// pattern is outside the compilable subset, to the runtime regex engine).
    /// Only the boolean `test` / `isMatch` shape is represented here; `exec` and
    /// the string `match`/`search`/etc. paths stay on the runtime engine.
    RegexMatch {
        /// Index into [`Program::regex_matchers`].
        matcher: u32,
        /// The subject (`args[0]`) string expression.
        args: Vec<Expr>,
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
    /// Invoke a class-static method without an implicit receiver.
    ///
    /// This node is reserved for the static-member lowering pass; the current
    /// source grammar does not construct it yet.
    StaticCall {
        /// Statically resolved method index.
        func: FuncIndex,
        /// Arguments in declaration order, excluding `this`.
        args: Vec<Expr>,
    },
    /// Read a statically declared class field.
    StaticFieldGet { class: ClassIndex, field_slot: u16 },
    /// Write a statically declared class field; the value is the result.
    StaticFieldSet { class: ClassIndex, field_slot: u16, value: Box<Expr> },
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
    /// Allocate a managed array of length `elements.len()` from an array
    /// literal and store each element at its positional index.
    ///
    /// Elements keep their natural static type; the `StoreArray` boundary boxes
    /// each value to `Any`, matching the dynamic array element model.
    ArrayLiteral {
        /// Elements in source order; element `i` is stored at index `i`. A
        /// `None` is an elision/hole: the index counts toward `length` and
        /// reads as `undefined` (the true hole-vs-`undefined` distinction,
        /// observable only through `in`/`hasOwnProperty`, is not represented).
        elements: Vec<Option<Expr>>,
    },
    /// Build a managed array that contains a spread (`[a, ...xs, b]`), growing
    /// it element-by-element so iterable sources can be spliced in at their
    /// source position.
    ArraySpreadLiteral {
        /// Parts in source order.
        parts: Vec<ArrayPart>,
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
