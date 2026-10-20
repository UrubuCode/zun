//! Compiler driver for the source-to-ZIL pipeline.
//!
//! The compiler owns the front-end boundary: source text enters here and a
//! self-contained [`zun_module::Module`] leaves here. Runtime crates consume
//! only that module format and never depend on the compiler or its AST.

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
pub fn compile_scope(name: impl Into<String>, source: &str) -> Result<zun_loader::ModuleScope, Vec<Diagnostic>> {
    let module = compile(source)?;
    zun_loader::ModuleScope::from_bytes(name, &module.write())
        .map_err(|error| vec![Diagnostic::error(error.to_string())])
}

/// Compile and register a set of named source modules.
pub fn compile_modules<'a, I>(sources: I) -> Result<zun_loader::ModuleLoader, Vec<Diagnostic>>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut loader = zun_loader::ModuleLoader::new();
    for (name, source) in sources {
        let scope = compile_scope(name, source)?;
        loader.insert(scope).map_err(|error| vec![Diagnostic::error(error.to_string())])?;
    }
    Ok(loader)
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
}
