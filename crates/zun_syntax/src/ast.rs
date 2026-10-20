//! The abstract syntax tree.
//!
//! A deliberately small AST for the phase-1 TypeScript subset: top-level
//! functions, `let`, `if`/`else`, `while`, `return`, calls, and arithmetic.
//! Every node carries its [`Span`] so later passes can diagnose precisely.
//! Names are kept as spans into the source (resolved to symbols by `zun_sema`).

use zun_common::Span;

/// A parsed source file.
#[derive(Debug, PartialEq)]
pub struct Program {
    /// Top-level function declarations, in source order.
    pub functions: Vec<Function>,
    /// Top-level class declarations, in source order.
    pub classes: Vec<Class>,
    /// `import { ... } from "..."` declarations, in source order.
    pub imports: Vec<Import>,
}

/// `import { a, b } from "module";`
#[derive(Debug, PartialEq)]
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
#[derive(Debug, PartialEq)]
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
#[derive(Debug, PartialEq)]
pub struct ClassField {
    /// The field name.
    pub name: Name,
    /// The field's declared type.
    pub ty: TypeAnnotation,
}

/// `function name(params): ret { body }`
#[derive(Debug, PartialEq)]
pub struct Function {
    /// The function name.
    pub name: Name,
    /// Parameters in declaration order.
    pub params: Vec<Param>,
    /// Declared return type, if annotated.
    pub return_type: Option<TypeAnnotation>,
    /// Whether this declaration is implemented by a runtime intrinsic.
    pub is_extern: bool,
    /// The body block.
    pub body: Block,
    /// Span of the whole declaration.
    pub span: Span,
}

/// A parameter: `name: type`.
#[derive(Debug, PartialEq)]
pub struct Param {
    /// The parameter name.
    pub name: Name,
    /// Declared type, if annotated.
    pub ty: Option<TypeAnnotation>,
}

/// A type annotation. Phase 1 only knows named types (`number`, `string`,
/// `boolean`, `void`, user names).
#[derive(Debug, PartialEq)]
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
#[derive(Debug, PartialEq)]
pub struct Block {
    /// Statements in order.
    pub statements: Vec<Stmt>,
    /// Span including the braces.
    pub span: Span,
}

/// A typed catch clause attached to a try statement.
#[derive(Debug, PartialEq)]
pub struct CatchClause {
    /// The local receiving the thrown value.
    pub binding: Name,
    /// The error type matched by this clause.
    pub ty: TypeAnnotation,
    /// The handler body.
    pub body: Block,
}

/// A statement.
#[derive(Debug, PartialEq)]
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
    /// `break;`
    Break {
        /// Source location of the statement.
        span: Span,
    },
    /// `continue;`
    Continue {
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
#[derive(Debug, PartialEq)]
pub enum Expr {
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
    /// `name = value`
    Assign {
        /// The assigned variable.
        target: Name,
        /// The new value.
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
            | Expr::Bool { span, .. }
            | Expr::Null { span }
            | Expr::Assign { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Call { span, .. }
            | Expr::MethodCall { span, .. }
            | Expr::Cast { span, .. }
            | Expr::New { span, .. }
            | Expr::Field { span, .. }
            | Expr::FieldAssign { span, .. }
            | Expr::Index { span, .. } => *span,
            Expr::IndexAssign { span, .. } => *span,
            Expr::Ident(name) => name.span,
        }
    }
}
