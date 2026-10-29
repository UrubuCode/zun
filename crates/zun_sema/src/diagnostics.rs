//! Source slicing and diagnostic emission for semantic checking.

use super::*;

impl Checker<'_> {
    /// Source text of a name. Dotted `Intl.X` names (the only dotted names the
    /// parser produces) are mapped here onto the prelude classes `IntlX`
    /// (`builtin/intl.zun`). This is the entire `Intl` namespace shim: a
    /// general `Ns.Member` resolution system is deliberately out of scope.
    pub(crate) fn text(&self, name: &ast::Name) -> &str {
        let raw = &self.source[name.span.start as usize..name.span.end as usize];
        match raw {
            "Intl.Collator" => "IntlCollator",
            "Intl.NumberFormat" => "IntlNumberFormat",
            "Intl.DateTimeFormat" => "IntlDateTimeFormat",
            "Intl.PluralRules" => "IntlPluralRules",
            "Intl.ListFormat" => "IntlListFormat",
            "Intl.RelativeTimeFormat" => "IntlRelativeTimeFormat",
            "Intl.Segmenter" => "IntlSegmenter",
            "Intl.getCanonicalLocales" => "intlGetCanonicalLocales",
            _ => raw,
        }
    }

    pub(crate) fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(
            Diagnostic::error(message)
                .with_code("SEM0001")
                .with_span(span),
        );
    }
}
