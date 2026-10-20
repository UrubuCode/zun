//! Compiler driver for the source-to-ZIL pipeline.
//!
//! The compiler owns the front-end boundary: source text enters here and a
//! self-contained [`zun_module::Module`] leaves here. Runtime crates consume
//! only that module format and never depend on the compiler or its AST.

use std::collections::{HashMap, HashSet};
use zun_common::{Diagnostic, Severity};
use zun_module::Module;

/// Compile source text with the standard prelude into a `.zm` module.
pub fn compile(source: &str) -> Result<Module, Vec<Diagnostic>> {
    let source_with_prelude = zun_prelude::with_prelude(source);
    let source = source_with_prelude.as_str();
    let mut diagnostics = Vec::new();
    let ast = zun_syntax::parse(source, &mut diagnostics);
    if diagnostics.iter().any(is_error) {
        return Err(diagnostics);
    }
    let hir = zun_sema::check(source, &ast, &mut diagnostics);
    if diagnostics.iter().any(is_error) {
        return Err(diagnostics);
    }
    Ok(zun_ilgen::lower(&hir))
}

/// Compile one named source into a loadable module scope.
pub fn compile_scope(
    name: impl Into<String>,
    source: &str,
) -> Result<zun_loader::ModuleScope, Vec<Diagnostic>> {
    let module = compile(source)?;
    zun_loader::ModuleScope::from_bytes(name, &module.write())
        .map_err(|error| vec![Diagnostic::error(error.to_string())])
}

/// Compile and register a set of named source modules.
pub fn compile_modules<'a, I>(sources: I) -> Result<zun_loader::ModuleLoader, Vec<Diagnostic>>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let sources: Vec<_> = sources.into_iter().collect();
    validate_imports(sources.iter().copied())?;
    let mut loader = zun_loader::ModuleLoader::new();
    for (name, source) in sources {
        let scope = compile_scope(name, source)?;
        loader
            .insert(scope)
            .map_err(|error| vec![Diagnostic::error(error.to_string())])?;
    }
    Ok(loader)
}

/// Validate imports against the complete source set before module emission.
///
/// This is the compiler-side half of module linking: native `zun:*` exports
/// are checked through the intrinsic registry, while user-module exports are
/// collected from all source files. The later HIR/linker pass will attach the
/// resolved member-reference tokens to imported calls.
pub fn validate_imports<'a, I>(sources: I) -> Result<(), Vec<Diagnostic>>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut modules: HashMap<&str, HashSet<String>> = HashMap::new();
    let mut parsed = Vec::new();
    let mut diagnostics = Vec::new();
    for (module, source) in sources {
        if modules.contains_key(module) {
            diagnostics.push(Diagnostic::error(format!(
                "duplicate source module `{module}`"
            )));
            continue;
        }
        let mut parse_diagnostics = Vec::new();
        let program = zun_syntax::parse(source, &mut parse_diagnostics);
        diagnostics.extend(parse_diagnostics);
        let exports = program
            .functions
            .iter()
            .map(|function| {
                source[function.name.span.start as usize..function.name.span.end as usize]
                    .to_owned()
            })
            .chain(program.classes.iter().map(|class| {
                source[class.name.span.start as usize..class.name.span.end as usize].to_owned()
            }))
            .collect();
        modules.insert(module, exports);
        parsed.push((module, source, program));
    }
    if diagnostics.iter().any(is_error) {
        return Err(diagnostics);
    }

    for (module, source, program) in parsed {
        for import in program.imports {
            for name in import.names {
                let imported = &source[name.span.start as usize..name.span.end as usize];
                let found = if import.module.starts_with("zun:") {
                    zun_natives::resolve(&import.module, imported).is_some()
                } else {
                    modules
                        .get(import.module.as_str())
                        .is_some_and(|exports| exports.contains(imported))
                };
                if !found {
                    diagnostics.push(
                        Diagnostic::error(format!(
                            "module `{}` has no export named `{imported}`",
                            import.module
                        ))
                        .with_span(import.span),
                    );
                }
            }
        }
        let _ = module;
    }
    if diagnostics.iter().any(is_error) {
        Err(diagnostics)
    } else {
        Ok(())
    }
}

fn is_error(diagnostic: &Diagnostic) -> bool {
    diagnostic.severity == Severity::Error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_a_minimal_entry_point() {
        let module = compile("function main(): int { return 7; }").expect("compile");
        assert!(module.entry_point.is_some());
        assert!(!module.code.is_empty());
    }

    #[test]
    fn returns_frontend_diagnostics() {
        let diagnostics = compile("function main( { }").expect_err("must reject malformed source");
        assert!(diagnostics.iter().any(|d| d.severity == Severity::Error));
    }

    #[test]
    fn compiles_named_modules_into_one_registry() {
        let loader = compile_modules([
            ("core", "function core(): int { return 1; }"),
            ("app", "function main(): int { return 0; }"),
        ])
        .expect("compile modules");
        assert_eq!(loader.len(), 2);
        assert!(loader.get("core").is_some());
        assert!(loader.get("app").is_some());
    }

    #[test]
    fn validates_user_module_exports_before_linking() {
        validate_imports([
            ("core", "function answer(): int { return 42; }"),
            (
                "app",
                "import { answer } from \"core\";\nfunction main(): int { return 0; }",
            ),
        ])
        .expect("import should resolve");
        let error = validate_imports([
            ("core", "function answer(): int { return 42; }"),
            (
                "app",
                "import { missing } from \"core\";\nfunction main(): int { return 0; }",
            ),
        ])
        .expect_err("missing export should diagnose");
        assert!(error.iter().any(|d| d.message.contains("missing")));
    }

    #[test]
    fn standard_library_source_is_compilable_by_zun() {
        let source =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../std/math.zun"))
                .expect("std/math.zun");
        compile(&source).expect("standard library module should compile");
    }
}
