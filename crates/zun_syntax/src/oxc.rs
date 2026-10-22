//! Native OXC parsing boundary for standard JavaScript and TypeScript.
//!
//! The existing parser remains authoritative for Zun-specific syntax while the
//! HIR adapter is built incrementally. This boundary lets callers validate
//! standard JS/TS with OXC without coupling the rest of the compiler to OXC's
//! arena-owned AST types.

use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

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

#[cfg(test)]
mod tests {
    use super::{diagnostics, parses};

    #[test]
    fn parses_standard_typescript_without_using_zun_ast() {
        assert!(parses("const answer: number = 42;", true));
    }

    #[test]
    fn reports_standard_typescript_syntax_errors() {
        let errors = diagnostics("const answer: = 42;", true);
        assert!(!errors.is_empty());
    }
}
