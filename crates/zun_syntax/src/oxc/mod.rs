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
    TOP_LEVEL_NAMES.with(|names| {
        let mut names = names.borrow_mut();
        names.clear();
        for statement in parsed.program.body.iter() {
            if let Statement::VariableDeclaration(decl) = statement {
                for d in decl.declarations.iter() {
                    if let BindingPattern::BindingIdentifier(id) = &d.id {
                        names.insert(id.name.to_string());
                    }
                }
            }
        }
    });
    // `var` declared inside a nested block / loop / branch of the module body
    // is module-scoped: hoist a `let x = undefined` to the head of the module
    // and turn the nested declaration into an assignment.
    let mut module_hoisted_lets: Vec<Stmt> = Vec::new();
    {
        let mut sites = Vec::new();
        for (index, statement) in parsed.program.body.iter().enumerate() {
            collect_vars_in_statement(statement, index, false, &mut sites);
        }
        let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for site in &sites {
            let top_declared = sites.iter().any(|other| other.name == site.name && (!other.nested || other.in_head))
                || TOP_LEVEL_NAMES.with(|names| names.borrow().contains(&site.name));
            if site.nested && !site.in_head && !top_declared && names.insert(site.name.clone()) {
                module_hoisted_lets.push(Stmt::Let {
                    name: Name { span: Span::new(site.id_span.start, site.id_span.end) },
                    ty: None,
                    init: Expr::Unary {
                        op: crate::ast::UnaryOp::Void,
                        operand: Box::new(Expr::Number {
                            value: 0.0,
                            span: Span::new(site.id_span.start, site.id_span.end),
                        }),
                        span: Span::new(site.id_span.start, site.id_span.end),
                    },
                    span: Span::new(site.id_span.start, site.id_span.end),
                });
            }
        }
        HOISTED_VARS.with(|stack| stack.borrow_mut().push(names));
    }
    let module_code_spans: Vec<(usize, usize)> = parsed
        .program
        .body
        .iter()
        .filter(|other| !matches!(other, Statement::FunctionDeclaration(_) | Statement::ClassDeclaration(_)))
        .map(|other| (other.span().start as usize, other.span().end as usize))
        .collect();
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
            let Some(module) = import_declaration.source.value.as_str() else {
                return Err(vec![
                    "OXC adapter cannot represent non-UTF-8 import sources".into(),
                ]);
            };
            let mut names = Vec::new();
            if let Some(specifiers) = import_declaration.specifiers.as_deref() {
                for specifier in specifiers {
                    match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                            let ModuleExportName::IdentifierName(imported) = &specifier.imported
                            else {
                                return Err(vec![
                                    "OXC adapter supports identifier named imports only".into(),
                                ]);
                            };
                            names.push(crate::ast::ImportName {
                                imported: Name {
                                    span: Span::new(imported.span.start, imported.span.end),
                                },
                                local: Name {
                                    span: Span::new(
                                        specifier.local.span.start,
                                        specifier.local.span.end,
                                    ),
                                },
                                is_default: false,
                                is_namespace: false,
                            });
                        }
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                            names.push(crate::ast::ImportName {
                                imported: Name {
                                    span: Span::new(
                                        specifier.local.span.start,
                                        specifier.local.span.end,
                                    ),
                                },
                                local: Name {
                                    span: Span::new(
                                        specifier.local.span.start,
                                        specifier.local.span.end,
                                    ),
                                },
                                is_default: true,
                                is_namespace: false,
                            });
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                            names.push(crate::ast::ImportName {
                                imported: Name {
                                    span: Span::new(specifier.span.start, specifier.span.start),
                                },
                                local: Name {
                                    span: Span::new(
                                        specifier.local.span.start,
                                        specifier.local.span.end,
                                    ),
                                },
                                is_default: false,
                                is_namespace: true,
                            });
                        }
                    }
                }
            }
            imports.push(crate::ast::Import {
                names,
                module: module.to_owned(),
                span: Span::new(import_declaration.span.start, import_declaration.span.end),
            });
            continue;
        }
        if let Statement::ExportNamedDeclaration(export) = &statement {
            for specifier in &export.specifiers {
                let local = specifier.local.span();
                let exported = specifier.exported.span();
                if source.get(local.start as usize..local.end as usize)
                    != source.get(exported.start as usize..exported.end as usize)
                {
                    return Err(vec![
                        "OXC adapter supports unchanged local export names only".into(),
                    ]);
                }
            }
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
        // A top-level function declaration that the program reassigns
        // (`f = function () {}`, the string-array obfuscator idiom) is a mutable
        // binding: lower it to a `let` hoisted to the head of the module body.
        if let Some(function) = function {
            if let Some(id) = function.id.as_ref() {
                let name = &source[id.span.start as usize..id.span.end as usize];
                if !function.generator
                    && !function.r#async
                    && (name_is_reassigned(&source[function.span.start as usize..function.span.end as usize], name)
                        // Reassigned by top-level module code (never shadowed
                        // there, unlike text inside other functions).
                        || module_code_spans.iter().any(|&(start, end)| {
                            source.get(start..end).is_some_and(|text| name_is_reassigned(text, name))
                        })) {
                    let converted = convert_statement_list(source, std::slice::from_ref(&statement))
                        .map_err(|error| vec![error])?;
                    // Hoist to the module head only when earlier text uses the
                    // name; otherwise keep the textual position so the body
                    // can capture the globals declared before it.
                    if source[..function.span.start as usize]
                        .lines()
                        .filter(|line| !line.trim_start().starts_with("//"))
                        .any(|line| contains_word(line, name))
                    {
                        hoisted_functions.extend(converted);
                    } else {
                        top_level.extend(converted);
                    }
                    continue;
                }
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
        let Some(id) = function.id.as_ref() else {
            return Err(vec!["OXC adapter requires named functions".into()]);
        };
        let Some(body) = function.body.as_ref() else {
            // `extern function f(..): T from "lib";` arrives as a bodiless
            // overload signature (see `preprocess_externs`).
            if let Some(module) = extern_mark(id.span.start) {
                let mut prologue = Vec::new();
                let mut params = Vec::new();
                for parameter in &function.params.items {
                    params.push(
                        convert_formal_param(source, parameter, &mut prologue)
                            .map_err(|error| vec![error])?,
                    );
                }
                functions.push(Function {
                    name: Name { span: Span::new(id.span.start, id.span.end) },
                    type_params: Vec::new(),
                    decorators: Vec::new(),
                    params,
                    return_type: convert_type_annotation(source, function.return_type.as_deref())
                        .map_err(|error| vec![error])?,
                    is_extern: true,
                    is_static: false,
                    accessor: None,
                    external_module: module,
                    is_async: false,
                    is_generator: false,
                    body: Block { statements: Vec::new(), span: Span::new(function.span.start, function.span.end) },
                    span: Span::new(function.span.start, function.span.end),
                });
                continue;
            }
            return Err(vec!["OXC adapter requires function bodies".into()]);
        };
        let span = |span: oxc_span::Span| Span::new(span.start, span.end);
        let function_span = span(function.span);
        let mut params = Vec::with_capacity(function.params.items.len());
        let mut prologue = Vec::new();
        for parameter in &function.params.items {
            params.push(
                convert_formal_param(source, parameter, &mut prologue)
                    .map_err(|error| vec![error])?,
            );
        }
        // The function body converts through the single shared statement
        // converter (switch, labelled statements, loops, etc.), after any
        // destructured-parameter prologue.
        let mut statements = prologue;
        statements.extend(
            convert_function_statements(source, &body.statements, &param_names(&function.params)).map_err(|error| vec![error])?,
        );
        if let Some(rest) = function.params.rest.as_ref() {
            let BindingPattern::BindingIdentifier(identifier) = &rest.rest.argument else {
                return Err(vec![
                    "OXC adapter supports identifier rest parameters only".into()
                ]);
            };
            params.push(crate::ast::Param {
                name: Name {
                    span: span(identifier.span),
                },
                ty: convert_type_annotation(source, rest.type_annotation.as_deref())
                    .map_err(|error| vec![error])?,
                default_value: None,
                is_rest: true,
            });
        }
        inject_arguments_object(source, function_span, &mut params, &mut statements);
        functions.push(Function {
            name: Name {
                span: span(id.span),
            },
            // OXC adapter erases generic type parameters (see above).
            type_params: Vec::new(),
            decorators: Vec::new(),
            params,
            return_type: convert_type_annotation(source, function.return_type.as_deref())
                .map_err(|error| vec![error])?,
            is_extern: false,
            is_static: false,
            accessor: None,
            external_module: None,
            is_async: function.r#async,
            is_generator: function.generator,
            body: Block {
                statements,
                span: span(body.span),
            },
            span: function_span,
        });
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

/// Convert a single OXC class declaration into the AST class (shared by the
/// top-level scan and the statement converter for block-local classes).
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
