//! Source slicing and diagnostic emission for semantic checking.

use super::*;

impl Checker<'_> {
    pub(crate) fn text(&self, name: &ast::Name) -> &str {
        &self.source[name.span.start as usize..name.span.end as usize]
    }

    pub(crate) fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(
            Diagnostic::error(message)
                .with_code("SEM0001")
                .with_span(span),
        );
    }
}
