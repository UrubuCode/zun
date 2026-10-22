//! The abstract syntax tree.
//!
//! A deliberately small AST for the phase-1 TypeScript subset: top-level
//! functions, `let`, `if`/`else`, `while`, `return`, calls, and arithmetic.
//! Every node carries its [`Span`] so later passes can diagnose precisely.
//! Names are kept as spans into the source (resolved to symbols by `zun_sema`).

use zun_common::Span;

/// A parsed source file.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    /// Top-level function declarations, in source order.
    pub functions: Vec<Function>,
    /// Top-level class declarations, in source order.
    pub classes: Vec<Class>,
    /// `import { ... } from "..."` declarations, in source order.
    pub imports: Vec<Import>,
}

/// `import { a, b } from "module";`
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    /// The imported names.
    pub names: Vec<Name>,
    /// The module string (decoded, e.g. `"zun:fs"`).
    pub module: String,
    /// Span of the whole declaration.
    pub span: Span,
}

/// `class Name { field: type; method(...) {...} ... }`. Fields are initialized
/// positionally by `new`; methods receive an implicit `this`. Inheritance and
/// virtual dispatch arrive in a later step.
#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    /// The class name.
    pub name: Name,
    /// Base class name, if the class `extends` another.
    pub base: Option<Name>,
    /// Declared fields in source order; `new` initializes them positionally.
    pub fields: Vec<ClassField>,
    /// Declared methods in source order.
    pub methods: Vec<Function>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// A class field declaration: `name: type;`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassField {
    /// The field name.
    pub name: Name,
    /// The field's declared type.
    pub ty: TypeAnnotation,
    /// Whether the field was declared with the `static` modifier. Static
    /// field storage is not lowered yet, so sema reports it explicitly.
    pub is_static: bool,
}

/// `function name(params): ret { body }`
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    /// The function name.
    pub name: Name,
    /// Parameters in declaration order.
    pub params: Vec<Param>,
    /// Declared return type, if annotated.
    pub return_type: Option<TypeAnnotation>,
    /// Whether this declaration is implemented by a runtime intrinsic.
    pub is_extern: bool,
    /// Whether this is a class-static member. The parser does not expose the
    /// modifier yet; the field keeps the representation ready for the full
    /// static-member lowering pass.
    pub is_static: bool,
    /// Native library for a top-level source declaration.
    pub external_module: Option<String>,
    /// Whether this declaration uses the source-level `async` modifier.
    pub is_async: bool,
    /// Whether this declaration uses generator syntax (`function*`).
    pub is_generator: bool,
    /// The body block.
    pub body: Block,
    /// Span of the whole declaration.
    pub span: Span,
}

/// An anonymous function expression or arrow function.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionExpr {
    /// Parameters in declaration order.
    pub params: Vec<Param>,
    /// Declared return type, if present.
    pub return_type: Option<TypeAnnotation>,
    /// Function body. Expression-bodied arrows are represented by a single
    /// return statement so later phases share one body representation.
    pub body: Block,
    /// Whether this expression used arrow syntax.
    pub is_arrow: bool,
    /// Whether this expression uses the source-level `async` modifier.
    pub is_async: bool,
    /// Whether this expression uses generator syntax.
    pub is_generator: bool,
    /// Span of the whole expression.
    pub span: Span,
}

/// One segment of a template literal.
#[derive(Debug, Clone, PartialEq)]
pub enum TemplatePart {
    /// Literal text between interpolation markers.
    Text(String),
    /// An interpolated expression.
    Expr(Box<Expr>),
}

/// A parameter: `name: type`.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    /// The parameter name.
    pub name: Name,
    /// Declared type, if annotated.
    pub ty: Option<TypeAnnotation>,
    /// Optional default expression evaluated when the argument is omitted.
    pub default_value: Option<Box<Expr>>,
    /// Whether this is a variadic `...rest` parameter.
    pub is_rest: bool,
}

/// A type annotation. Phase 1 only knows named types (`number`, `string`,
/// `boolean`, `void`, user names).
#[derive(Debug, Clone, PartialEq)]
pub struct TypeAnnotation {
    /// The type name.
    pub name: Name,
}

/// An identifier occurrence: its text lives in the source at `span`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Name {
    /// Where the identifier appears.
    pub span: Span,
}

/// `{ statements }`
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Statements in order.
    pub statements: Vec<Stmt>,
    /// Span including the braces.
    pub span: Span,
}

/// A typed catch clause attached to a try statement.
#[derive(Debug, Clone, PartialEq)]
pub struct CatchClause {
    /// The local receiving the thrown value.
    pub binding: Name,
    /// The error type matched by this clause.
    pub ty: TypeAnnotation,
    /// The handler body.
    pub body: Block,
}

/// A statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `let name: ty = init;`
    Let {
        /// The declared name.
        name: Name,
        /// Optional type annotation.
        ty: Option<TypeAnnotation>,
        /// The initializer (mandatory in phase 1).
        init: Expr,
        /// Span of the whole statement.
        span: Span,
    },
    /// `let [a, b] = array;` fixed-length array destructuring.
    ArrayDestructure {
        /// Declared binding names in index order.
        names: Vec<Name>,
        /// The source array expression.
        init: Expr,
        /// Span of the whole statement.
        span: Span,
    },
    /// `let {a, b} = object;` dynamic-property destructuring.
    ObjectDestructure {
        /// Declared binding names; each name is also the property key.
        names: Vec<Name>,
        /// The source object expression.
        init: Expr,
        /// Span of the whole statement.
        span: Span,
    },
    /// `expr;`
    Expr(Expr),
    /// `if (cond) { then } else { otherwise }`
    If {
        /// The condition.
        cond: Expr,
        /// Taken when the condition is truthy.
        then_block: Block,
        /// Taken otherwise, if present.
        else_block: Option<Block>,
        /// Span of the whole statement.
        span: Span,
    },
    /// `while (cond) { body }`
    While {
        /// The loop condition.
        cond: Expr,
        /// The loop body.
        body: Block,
        /// Span of the whole statement.
        span: Span,
    },
    /// `do { body } while (cond);`.
    DoWhile {
        /// The loop body.
        body: Block,
        /// The condition checked after each body execution.
        cond: Expr,
        /// Span of the whole loop.
        span: Span,
    },
    /// `for (init; cond; update) { body }`.
    For {
        /// Optional loop initializer.
        init: Option<Box<Stmt>>,
        /// Optional condition; omitted means always true.
        cond: Option<Expr>,
        /// Optional update expression.
        update: Option<Expr>,
        /// Loop body.
        body: Block,
        /// Span of the whole loop.
        span: Span,
    },
    /// `for (let name of iterable) { body }`.
    ForOf {
        /// The loop binding.
        name: Name,
        /// The iterable expression.
        iterable: Expr,
        /// The loop body.
        body: Block,
        /// Span of the whole statement.
        span: Span,
    },
    /// `label: statement`.
    Labeled {
        /// The label name.
        label: Name,
        /// The labeled statement.
        statement: Box<Stmt>,
        /// Span of the whole statement.
        span: Span,
    },
    /// `break label;`.
    Break {
        /// Optional target label.
        label: Option<Name>,
        /// Source location of the statement.
        span: Span,
    },
    /// `continue label;`.
    Continue {
        /// Optional target label.
        label: Option<Name>,
        /// Source location of the statement.
        span: Span,
    },
    /// `return expr;` / `return;`
    Return {
        /// The returned value, if any.
        value: Option<Expr>,
        /// Span of the whole statement.
        span: Span,
    },
    /// `throw expression;`
    Throw {
        /// The value being thrown.
        value: Expr,
        /// Span of the whole statement.
        span: Span,
    },
    /// `try { ... } catch (...) { ... } finally { ... }`.
    Try {
        /// Protected body.
        body: Block,
        /// Optional typed catch clauses.
        catches: Vec<CatchClause>,
        /// Optional cleanup body.
        finally: Option<Block>,
        /// Span of the whole statement.
        span: Span,
    },
}

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `&&`
    And,
    /// `||`
    Or,
    /// `??`
    Nullish,
}

/// A unary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// `-`
    Neg,
    /// `!`
    Not,
}

/// An expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// An anonymous `function` or arrow function expression.
    Function(FunctionExpr),
    /// A numeric literal.
    Number {
        /// The parsed value (JS numbers are f64).
        value: f64,
        /// Source location.
        span: Span,
    },
    /// A string literal, with escapes already processed.
    String {
        /// The decoded contents.
        value: String,
        /// Source location including quotes.
        span: Span,
    },
    /// A backtick template literal.
    Template {
        /// Literal and interpolated segments in source order.
        parts: Vec<TemplatePart>,
        /// Source location including backticks.
        span: Span,
    },
    /// `true` / `false`.
    Bool {
        /// The literal value.
        value: bool,
        /// Source location.
        span: Span,
    },
    /// `null`.
    Null {
        /// Source location.
        span: Span,
    },
    /// A variable reference.
    Ident(Name),
    /// `Class.method(args)` static member call.
    StaticMethodCall {
        /// The class name used as the static namespace.
        class: Name,
        /// The static method name.
        method: Name,
        /// Arguments in source order.
        args: Vec<Expr>,
        /// Source location.
        span: Span,
    },
    /// `name = value`
    Assign {
        /// The assigned variable.
        target: Name,
        /// The new value.
        value: Box<Expr>,
        /// Span of the whole expression.
        span: Span,
    },
    /// `await expression` (validated and lowered by the async compiler path).
    Await {
        /// The awaited expression.
        operand: Box<Expr>,
        /// Span of the whole expression.
        span: Span,
    },
    /// `yield expression` (generator suspension point).
    Yield {
        /// The value produced by the generator.
        value: Box<Expr>,
        /// Span of the whole expression.
        span: Span,
    },
    /// `lhs op rhs`
    Binary {
        /// The operator.
        op: BinaryOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
        /// Span of the whole expression.
        span: Span,
    },
    /// `condition ? when_true : when_false`.
    Conditional {
        /// The boolean condition.
        condition: Box<Expr>,
        /// Value selected when the condition is true.
        when_true: Box<Expr>,
        /// Value selected when the condition is false.
        when_false: Box<Expr>,
        /// Span of the whole expression.
        span: Span,
    },
    /// `op operand`
    Unary {
        /// The operator.
        op: UnaryOp,
        /// The operand.
        operand: Box<Expr>,
        /// Span of the whole expression.
        span: Span,
    },
    /// `callee(args)` — phase 1 callees are plain names.
    Call {
        /// The called function's name.
        callee: Name,
        /// Arguments in order.
        args: Vec<Expr>,
        /// Span of the whole expression.
        span: Span,
    },
    /// `callee(args)` where the callee is an arbitrary expression.
    ComputedCall {
        /// Function-valued expression being invoked.
        callee: Box<Expr>,
        /// Arguments in order.
        args: Vec<Expr>,
        /// Span of the whole call.
        span: Span,
    },
    /// `receiver.method(args)` — an instance method call.
    MethodCall {
        /// The object the method is called on.
        receiver: Box<Expr>,
        /// The method name.
        method: Name,
        /// Arguments in order (not counting the implicit `this`).
        args: Vec<Expr>,
        /// Span of the whole expression.
        span: Span,
    },
    /// `...expr` in an argument list.
    Spread { expr: Box<Expr>, span: Span },
    /// `object?.field` — a null-safe dynamic property read.
    OptionalField {
        object: Box<Expr>,
        field: Name,
        span: Span,
    },
    /// `object?.method(args)` — a null-safe managed method call.
    OptionalMethodCall {
        object: Box<Expr>,
        method: Name,
        args: Vec<Expr>,
        span: Span,
    },
    /// `expr as Type` — a numeric conversion.
    Cast {
        /// The value being converted.
        expr: Box<Expr>,
        /// The target type.
        ty: TypeAnnotation,
        /// Span of the whole expression.
        span: Span,
    },
    /// `new Class(args)` — positional field initialization.
    New {
        /// The class being instantiated.
        class: Name,
        /// Initializer arguments, one per field in declaration order.
        args: Vec<Expr>,
        /// Span of the whole expression.
        span: Span,
    },
    /// `object.field` — read a field.
    Field {
        /// The object expression.
        object: Box<Expr>,
        /// The field name.
        field: Name,
        /// Span of the whole expression.
        span: Span,
    },
    /// `object.field = value` — write a field; the value is the result.
    FieldAssign {
        /// The object expression.
        object: Box<Expr>,
        /// The field name.
        field: Name,
        /// The stored value.
        value: Box<Expr>,
        /// Span of the whole expression.
        span: Span,
    },
    /// `object[index]` — read an array element.
    Index {
        /// The array expression.
        object: Box<Expr>,
        /// The numeric index expression.
        index: Box<Expr>,
        /// Span of the whole expression.
        span: Span,
    },
    /// `object[index] = value` — write an array element.
    IndexAssign {
        /// The array expression.
        object: Box<Expr>,
        /// The numeric index expression.
        index: Box<Expr>,
        /// The assigned value.
        value: Box<Expr>,
        /// Span of the whole expression.
        span: Span,
    },
}

impl Expr {
    /// The source span of this expression.
    pub fn span(&self) -> Span {
        match self {
            Expr::Number { span, .. }
            | Expr::String { span, .. }
            | Expr::Template { span, .. }
            | Expr::Bool { span, .. }
            | Expr::Null { span }
            | Expr::Assign { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Conditional { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Call { span, .. }
            | Expr::ComputedCall { span, .. }
            | Expr::MethodCall { span, .. }
            | Expr::StaticMethodCall { span, .. }
            | Expr::OptionalField { span, .. }
            | Expr::OptionalMethodCall { span, .. }
            | Expr::Spread { span, .. }
            | Expr::Cast { span, .. }
            | Expr::New { span, .. }
            | Expr::Field { span, .. }
            | Expr::FieldAssign { span, .. }
            | Expr::Index { span, .. } => *span,
            Expr::IndexAssign { span, .. } => *span,
            Expr::Await { span, .. } => *span,
            Expr::Yield { span, .. } => *span,
            Expr::Function(function) => function.span,
            Expr::Ident(name) => name.span,
        }
    }
}
