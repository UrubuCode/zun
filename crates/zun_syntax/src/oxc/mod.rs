//! Native OXC parsing boundary for standard JavaScript and TypeScript.
//!
//! The existing parser remains authoritative for Zun-specific syntax while the
//! HIR adapter is built incrementally. This boundary lets callers validate
//! standard JS/TS with OXC without coupling the rest of the compiler to OXC's
//! arena-owned AST types.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, AssignmentOperator, AssignmentTarget, BinaryOperator,
    BindingPattern, ChainElement, Decorator as OxcDecorator,
    ClassElement, Declaration, ExportDefaultDeclarationKind, Expression, ForStatementInit,
    ForStatementLeft, ImportDeclarationSpecifier, LogicalOperator, MethodDefinitionKind,
    ModuleExportName, ObjectPropertyKind, PropertyKey, PropertyKind, Statement, UnaryOperator,
};
use oxc_ast::ast::Function as OxcFunction;
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use zun_common::Span;

use crate::ast::{BinaryOp, Block, Expr, Function, FunctionExpr, Name, Program, Stmt};

mod binding;
mod class;
mod destructure;
mod expr;
mod externs;
mod function;
mod hoist;
mod loops;
mod module;
mod stmt;
mod types;
mod util;

#[cfg(test)]
mod tests;

use binding::*;
use class::*;
use destructure::*;
use expr::*;
use externs::*;
use function::*;
use hoist::*;
use loops::*;
use module::*;
use stmt::*;
use types::*;
use util::*;

/// Parse standard JavaScript or TypeScript with OXC.
///
/// The returned diagnostics are formatted at the boundary so callers do not
/// need to retain OXC's arena or AST lifetimes. An empty vector means parsing
/// succeeded; semantic analysis remains a separate compiler stage.
pub fn diagnostics(source: &str, typescript: bool) -> Vec<String> {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs().with_typescript(typescript);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    parsed
        .diagnostics
        .into_iter()
        .map(|diagnostic| format!("{diagnostic:?}"))
        .collect()
}

/// Return whether OXC accepts the source as standard JS/TS syntax.
pub fn parses(source: &str, typescript: bool) -> bool {
    diagnostics(source, typescript).is_empty()
}

/// Convert the first OXC-supported subset into the existing Zun AST.
///
/// Unsupported constructs return a diagnostic instead of being silently
/// approximated; additional statement and expression adapters can be added
/// without changing the public frontend boundary.
pub fn parse_subset(source: &str, typescript: bool) -> Result<Program, Vec<String>> {
    // Zun-only `extern` declarations are rewritten (length-preserving) into
    // bodiless TypeScript signatures; TypeScript parsing is forced for them.
    let preprocessed = preprocess_externs(source);
    let (source, typescript) = match preprocessed.as_deref() {
        Some(text) => (text, true),
        None => (source, typescript),
    };
    let allocator = Allocator::default();
    let source_type = SourceType::mjs().with_typescript(typescript);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(parsed
            .diagnostics
            .into_iter()
            .map(|diagnostic| format!("{diagnostic:?}"))
            .collect());
    }

    // Reset the hoisted-class collector for this parse; the statement converter
    // pushes block-local / nested class declarations here while converting the
    // module body, and they are drained into `classes` before the Program is
    // built.
    HOISTED_CLASSES.with(|cells| cells.borrow_mut().clear());

    let mut functions = Vec::new();
    let mut imports = Vec::new();
    let mut classes = Vec::new();
    let mut enums: Vec<crate::ast::EnumDecl> = Vec::new();
    let mut top_level = Vec::new();
    let mut hoisted_functions: Vec<Stmt> = Vec::new();
    record_top_level_names(&parsed.program.body);
    // `var` declared inside a nested block / loop / branch of the module body
    // is module-scoped: hoist a `let x = undefined` to the head of the module
    // and turn the nested declaration into an assignment.
    let module_hoisted_lets = hoist_module_vars(&parsed.program.body);
    let module_code_spans = collect_module_code_spans(&parsed.program.body);
    for statement in parsed.program.body {
        TOP_LEVEL_BLOCK.with(|cell| {
            cell.set(match &statement {
                Statement::BlockStatement(block) => block.span.start,
                _ => u32::MAX,
            })
        });
        // TS type-only declarations erase: a type alias or interface carries no
        // runtime value, so it is dropped (annotations that name it resolve to
        // `any` on this path, like the native parser's erasure).
        if matches!(
            statement,
            Statement::TSTypeAliasDeclaration(_) | Statement::TSInterfaceDeclaration(_)
        ) {
            continue;
        }
        if let Statement::ImportDeclaration(import_declaration) = statement {
            imports.push(convert_import_declaration(&import_declaration)?);
            continue;
        }
        if let Statement::ExportNamedDeclaration(export) = &statement {
            check_export_specifiers(source, export)?;
            continue;
        }
        // `enum E { .. }` / `const enum` / `export enum`: members fold to constants in
        // sema, exactly as for the hand parser.
        let enum_declaration = match &statement {
            Statement::TSEnumDeclaration(declaration) => Some(declaration),
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::TSEnumDeclaration(declaration) => Some(declaration),
                _ => None,
            },
            _ => None,
        };
        if let Some(declaration) = enum_declaration {
            enums.push(convert_enum_declaration(declaration).map_err(|error| vec![error])?);
            continue;
        }
        let class_declaration = match &statement {
            Statement::ClassDeclaration(class_declaration) => Some(class_declaration),
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::ClassDeclaration(class_declaration) => Some(class_declaration),
                _ => None,
            },
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::ClassDeclaration(class_declaration) => {
                    Some(class_declaration)
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(class_declaration) = class_declaration {
            classes.push(convert_class_declaration(source, class_declaration)?);
            top_level.extend(CLASS_TRAILING.with(|cells| std::mem::take(&mut *cells.borrow_mut())));
            continue;
        }
        let function = match &statement {
            Statement::FunctionDeclaration(function) => Some(function),
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::FunctionDeclaration(function) => Some(function),
                _ => None,
            },
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(function) => Some(function),
                _ => None,
            },
            _ => None,
        };
        if let Some(function) = function {
            if lower_reassigned_function(
                source,
                &statement,
                function,
                &module_code_spans,
                &mut hoisted_functions,
                &mut top_level,
            )? {
                continue;
            }
        }
        let Some(function) = function else {
            // Any other top-level node is an executable module statement (a JS/TS
            // module body): convert it and collect it to run as the program.
            top_level.extend(
                convert_statement_list(source, std::slice::from_ref(&statement))
                    .map_err(|error| vec![error])?,
            );
            continue;
        };
        convert_top_level_function(source, function, &mut functions)?;
    }
    // Drain the block-local / nested classes collected while converting the
    // module body (and function/method bodies) into the program's class list.
    HOISTED_CLASSES.with(|cells| classes.append(&mut cells.borrow_mut()));
    HOISTED_VARS.with(|stack| stack.borrow_mut().pop());
    hoisted_functions.splice(0..0, module_hoisted_lets);
    hoisted_functions.append(&mut top_level);
    let top_level = hoisted_functions;
    Ok(Program {
        functions,
        classes,
        imports,
        // The OXC adapter does not surface type aliases/interfaces yet; they
        // erase, so this only limits alias resolution on that secondary path.
        type_aliases: Vec::new(),
        enums,
        top_level,
    })
}

thread_local! {
    /// Statements a converted expression needs executed before the statement
    /// that contains it (the `yield*` forwarding loop). Drained by
    /// `convert_statement_list` right after each statement is converted.
    static PENDING_PRELUDE: std::cell::RefCell<Vec<Stmt>> = const { std::cell::RefCell::new(Vec::new()) };
    static YIELD_STAR_COUNTER: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

thread_local! {
    /// Names bound by top-level variable declarations of the module.
    static TOP_LEVEL_NAMES: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

thread_local! {
    /// Start offset of the bare block currently being converted as a direct
    /// child of the module body. Such a block stays flattened into the module
    /// scope (its bindings are module globals); every other bare block that
    /// declares `let`/`const` keeps its own scope.
    static TOP_LEVEL_BLOCK: std::cell::Cell<u32> = const { std::cell::Cell::new(u32::MAX) };
}

thread_local! {
    /// Name span of the binding an anonymous class expression initializes
    /// (`const C = class {}`), consumed by the class-expression converter.
    static CLASS_NAME_HINT: std::cell::RefCell<Option<Span>> =
        const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// Block-local / nested class declarations collected while converting a
    /// module body. Drained into the program's class list in `parse_subset`.
    /// Thread-local so the recursive statement converter can reach it without
    /// threading a collector through every conversion function.
    static HOISTED_CLASSES: std::cell::RefCell<Vec<crate::ast::Class>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Statements that install class members with run-time computed keys
    /// (`Object.defineProperty(C.prototype, key, ...)`), queued while a class
    /// is converted and emitted right after the class declaration.
    static CLASS_TRAILING: std::cell::RefCell<Vec<Stmt>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    /// Lexical stack of the classes being converted: each entry carries the
    /// class name span and the `#private` names it declares (name, is_static),
    /// so `#x in obj` can resolve to the declaring class.
    static PRIVATE_SCOPES: std::cell::RefCell<Vec<(Span, Vec<(String, bool)>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    /// `var` names hoisted to the function currently being converted
    /// (innermost last). A `var x = e` of a hoisted name converts to the
    /// assignment `x = e`; the declaration itself is emitted once at the top
    /// of the function body.
    static HOISTED_VARS: std::cell::RefCell<Vec<std::collections::HashSet<String>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    /// Names of the named function / class expressions being converted: inside
    /// their bodies the name is an immutable binding.
    static IMMUTABLE_SELF_NAMES: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    static LAST_PARAMS_SIMPLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

thread_local! {
    /// Parameter names that a hoisted function declaration of the body shadows.
    /// Source text of the destructuring parameters seen so far in the parameter
    /// list being converted.
    static PATTERN_PARAM_TEXT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static PARAM_SHADOWS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    static EMIT_SHADOW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
