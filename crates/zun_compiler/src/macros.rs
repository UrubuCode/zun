//! Compile-time macro expansion for decorators (PLAN §3.5, Phase 2).
//!
//! This is the Zun analogue of a .NET/Roslyn **source generator**: an
//! attribute-triggered, compile-time transform that runs AFTER parsing and
//! BEFORE semantic analysis, reading a value-model of a decorated declaration
//! and *emitting ordinary code* that the rest of the pipeline then sees like
//! any other source. Decorator syntax (Phase 1) is the trigger; a decorator
//! whose callee names a **registered built-in macro** drives a transform, and
//! any other decorator stays **inert** (Phase-1 behavior).
//!
//! ## Parser/Emitter separation (the repo discipline)
//!
//! A macro never touches raw AST. It receives a [`ClassModel`] — an owned
//! value-model of the target (class name, fields, existing method names) with
//! no AST or source references — and returns a [`MacroExpansion`], an
//! *emit model* describing the members to generate. The pass alone lowers that
//! emit model into AST nodes. Keeping the model free of AST/compilation
//! references is what lets a future version cache macro output by value
//! (incremental generation, deferred for v1).
//!
//! ## Contract of failure
//!
//! A macro that cannot run (bad arguments, unsupported target) returns a
//! [`Diagnostic`] instead of transforming. The pass records the diagnostic and
//! leaves the target **unchanged** — never a panic, never a silent miscompile.
//!
//! ## Deferred
//!
//! * **User-defined / in-language macros** — a macro being Zun code the
//!   compiler runs. v1 ships Rust built-ins only; the registry
//!   ([`class_macro`]) is the single extension point a future in-language
//!   loader would feed.
//! * **Method- and field-level macros** — the pass walks those decorator lists
//!   too, but no built-in is registered for them yet, so they stay inert.
//! * **Incremental caching** of the expansion pipeline (PLAN §3.5: v1 need not
//!   be incremental).

use zun_common::{Diagnostic, Span};
use zun_syntax::ast;

/// Owned value-model of a decorated class handed to a macro.
///
/// Deliberately free of AST and source references so macros stay pure
/// value-to-value functions (the Parser/Emitter discipline).
#[derive(Debug, Clone)]
pub struct ClassModel {
    /// The class name.
    pub name: String,
    /// Declared fields in source order.
    pub fields: Vec<FieldModel>,
    /// Names of methods the class already declares, in source order.
    pub method_names: Vec<String>,
}

/// One field in a [`ClassModel`].
#[derive(Debug, Clone)]
pub struct FieldModel {
    /// The field name.
    pub name: String,
    /// The field's declared type, as written in source.
    pub ty: String,
}

/// The emit model a macro returns: declarations to add to the target.
///
/// A value-model of *output*, lowered to AST by the pass. Extending the macro
/// system means growing this model, never handing macros raw AST.
#[derive(Debug, Clone, Default)]
pub struct MacroExpansion {
    /// Methods to append to the decorated class.
    pub added_methods: Vec<GeneratedMethod>,
}

/// A method a macro asks the pass to generate.
#[derive(Debug, Clone)]
pub struct GeneratedMethod {
    /// The method name.
    pub name: String,
    /// The declared return type, if any.
    pub return_type: Option<String>,
    /// The method body, as emit-model statements.
    pub body: Vec<GeneratedStmt>,
}

/// A statement in a [`GeneratedMethod`] body.
///
/// Intentionally tiny for v1 — just enough to express the proof macro. New
/// variants are the sound way to grow macro expressiveness.
#[derive(Debug, Clone)]
pub enum GeneratedStmt {
    /// `return "<literal>";`
    ReturnString(String),
}

/// A built-in class macro: a pure value-model → emit-model function.
pub type ClassMacro = fn(&ClassModel) -> Result<MacroExpansion, Diagnostic>;

/// The built-in class-macro registry, keyed by decorator name.
///
/// This single table is the only place macro knowledge lives, and the hook a
/// future in-language macro loader would extend.
pub fn class_macro(name: &str) -> Option<ClassMacro> {
    match name {
        "autoToString" => Some(auto_to_string),
        _ => None,
    }
}

/// `@autoToString` — generate a `toString()` returning the class name.
///
/// The proof macro: it demonstrably adds a method whose output is observable at
/// run time (`new C().toString() === "C"`). It fails cleanly when the class
/// already declares `toString`, exercising the diagnostic + fallback contract.
fn auto_to_string(model: &ClassModel) -> Result<MacroExpansion, Diagnostic> {
    if model.method_names.iter().any(|name| name == "toString") {
        return Err(Diagnostic::error(format!(
            "@autoToString: class `{}` already declares a `toString` method",
            model.name
        ))
        .with_code("MACRO0001"));
    }
    Ok(MacroExpansion {
        added_methods: vec![GeneratedMethod {
            name: "toString".to_owned(),
            return_type: Some("string".to_owned()),
            body: vec![GeneratedStmt::ReturnString(model.name.clone())],
        }],
    })
}

/// Run macro expansion over a parsed program.
///
/// Returns the (possibly extended) source text and the transformed program.
/// Generated identifiers are interned into a suffix appended to the original
/// source so every emitted [`Span`] resolves in the returned source string,
/// exactly as a hand-written token would — the rest of the pipeline cannot
/// tell generated code from authored code. Macro diagnostics are appended to
/// `diagnostics`; a failing or unregistered macro leaves its target unchanged.
pub fn expand(
    source: &str,
    mut program: ast::Program,
    diagnostics: &mut Vec<Diagnostic>,
) -> (String, ast::Program) {
    let mut emitter = Emitter::new(source.len() as u32);

    for class in &mut program.classes {
        // Snapshot the model before mutating, so a macro never observes members
        // added by an earlier macro on the same target in this pass.
        let model = ClassModel {
            name: source_text(source, class.name.span).to_owned(),
            fields: class
                .fields
                .iter()
                .map(|field| FieldModel {
                    name: source_text(source, field.name.span).to_owned(),
                    ty: source_text(source, field.ty.name.span).to_owned(),
                })
                .collect(),
            method_names: class
                .methods
                .iter()
                .map(|method| source_text(source, method.name.span).to_owned())
                .collect(),
        };

        for decorator in &class.decorators {
            let Some(name) = decorator_name(source, decorator) else {
                continue;
            };
            let Some(macro_fn) = class_macro(name) else {
                // Unregistered decorator: stays inert (Phase-1 behavior).
                continue;
            };
            match macro_fn(&model) {
                Ok(expansion) => {
                    for generated in expansion.added_methods {
                        class.methods.push(emitter.lower_method(generated));
                    }
                }
                Err(diagnostic) => diagnostics.push(diagnostic),
            }
        }
    }

    let mut extended = String::with_capacity(source.len() + emitter.buffer.len());
    extended.push_str(source);
    extended.push_str(&emitter.buffer);
    (extended, program)
}

/// Resolve a decorator's callee to a plain name, if it is a simple identifier.
///
/// `@foo` and `@foo(...)` resolve to `foo`; member-access callees (`@ns.dec`)
/// have no built-in macro and are left for a future resolver.
fn decorator_name<'a>(source: &'a str, decorator: &ast::Decorator) -> Option<&'a str> {
    match &decorator.callee {
        ast::Expr::Ident(name) => Some(source_text(source, name.span)),
        _ => None,
    }
}

fn source_text(source: &str, span: Span) -> &str {
    &source[span.start as usize..span.end as usize]
}

/// Interns generated identifiers into a source suffix and lowers emit-model
/// nodes into AST, so emitted spans resolve in the extended source.
struct Emitter {
    base: u32,
    buffer: String,
}

impl Emitter {
    fn new(base: u32) -> Self {
        Self {
            base,
            buffer: String::new(),
        }
    }

    /// Append `text` to the suffix and return a span pointing at it in the
    /// extended source. A leading space keeps tokens from fusing.
    fn intern(&mut self, text: &str) -> Span {
        self.buffer.push(' ');
        let start = self.base + self.buffer.len() as u32;
        self.buffer.push_str(text);
        let end = self.base + self.buffer.len() as u32;
        Span::new(start, end)
    }

    fn name(&mut self, text: &str) -> ast::Name {
        ast::Name {
            span: self.intern(text),
        }
    }

    fn lower_method(&mut self, method: GeneratedMethod) -> ast::Function {
        let name = self.name(&method.name);
        let return_type = method
            .return_type
            .map(|ty| ast::TypeAnnotation { name: self.name(&ty), is_array: false });
        let statements = method
            .body
            .into_iter()
            .map(|stmt| self.lower_stmt(stmt))
            .collect();
        // A synthetic but valid span for nodes the checker only uses for
        // diagnostics; it falls inside the interned suffix.
        let span = name.span;
        ast::Function {
            name,
            decorators: Vec::new(),
            params: Vec::new(),
            return_type,
            is_extern: false,
            is_static: false,
            accessor: None,
            external_module: None,
            is_async: false,
            is_generator: false,
            body: ast::Block { statements, span },
            span,
        }
    }

    fn lower_stmt(&mut self, stmt: GeneratedStmt) -> ast::Stmt {
        match stmt {
            GeneratedStmt::ReturnString(value) => {
                let span = self.intern(&value);
                ast::Stmt::Return {
                    value: Some(ast::Expr::String { value, span }),
                    span,
                }
            }
        }
    }
}
