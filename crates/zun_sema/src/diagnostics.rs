//! Source slicing and diagnostic emission for semantic checking.

use super::*;

/// Names whose span starts at or above this value are synthetic temporaries
/// rather than source slices; their text is ` synth#<index>`.
pub(crate) const SYNTHETIC_NAME_BASE: u32 = ast::SYNTHETIC_NAME_BASE;

impl Checker<'_> {
    /// Declare a fresh synthetic local in the current scope and return the
    /// local slot plus an AST name that resolves to it.
    pub(crate) fn synthetic_local(&self, cx: &mut FunctionCx, ty: Ty) -> (hir::LocalSlot, ast::Name) {
        let index = cx.locals.len() as u32;
        let name = ast::Name {
            span: Span::new(SYNTHETIC_NAME_BASE + index, SYNTHETIC_NAME_BASE + index),
        };
        let slot = cx.declare(&format!(" synth#{index}"), ty);
        (slot, name)
    }

    /// Source text of a name. Dotted `Intl.X` names (the only dotted names the
    /// parser produces) are mapped here onto the prelude classes `IntlX`
    /// (`builtin/intl.zun`). This is the entire `Intl` namespace shim: a
    /// general `Ns.Member` resolution system is deliberately out of scope.
    pub(crate) fn text(&self, name: &ast::Name) -> &str {
        if let Some(text) = ast::synthetic_ident_text(*name) {
            let leaked: &'static str = Box::leak(text.into_boxed_str());
            return leaked;
        }
        if name.span.start >= SYNTHETIC_NAME_BASE {
            // A compiler-synthesized temporary (see `synthetic_name`).
            let leaked: &'static str = Box::leak(
                format!(" synth#{}", name.span.start - SYNTHETIC_NAME_BASE).into_boxed_str(),
            );
            return leaked;
        }
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
            "String.prototype" => "__thisString",
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
