//! Source slicing and diagnostic emission for semantic checking.

use super::*;

/// Names whose span starts at or above this value are synthetic temporaries
/// rather than source slices; their text is ` synth#<index>`.
pub(crate) const SYNTHETIC_NAME_BASE: u32 = ast::SYNTHETIC_NAME_BASE;

impl Checker<'_> {
    /// Declare a fresh synthetic local in the current scope and return the
    /// local slot plus an AST name that resolves to it.
    pub(crate) fn synthetic_local(
        &self,
        cx: &mut FunctionCx,
        ty: Ty,
    ) -> (hir::LocalSlot, ast::Name) {
        let index = cx.locals.len() as u32;
        let name = ast::Name {
            span: Span::new(SYNTHETIC_NAME_BASE + index, SYNTHETIC_NAME_BASE + index),
        };
        let slot = cx.declare(&format!(" synth#{index}"), ty);
        (slot, name)
    }

    /// The name as it should appear in a diagnostic. `text` returns the
    /// uniqueness key of a class declared under the same name in several
    /// scopes (`Sub#64`), which must never reach the user. Only a trailing
    /// `#<digits>` after a non-empty stem is stripped, so a private member
    /// name such as `#x` is left alone.
    pub(crate) fn display_name(key: &str) -> &str {
        match key.rsplit_once('#') {
            Some((stem, suffix))
                if !stem.is_empty()
                    && !suffix.is_empty()
                    && suffix.bytes().all(|b| b.is_ascii_digit()) =>
            {
                stem
            }
            _ => key,
        }
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
        if let Some(scoped) = self.scoped_class_names.get(raw) {
            // A name declared as several scoped classes: pick the innermost
            // scope enclosing this occurrence (a member access `x.Name` is not
            // a class reference).
            let member = name.span.start > 0
                && self.source.as_bytes()[name.span.start as usize - 1] == b'.';
            if !member {
                let found = scoped
                    .iter()
                    .filter(|(scope, _)| {
                        scope.start <= name.span.start && name.span.end <= scope.end
                    })
                    .min_by_key(|(scope, _)| scope.end - scope.start);
                if let Some((_, key)) = found {
                    return key;
                }
            }
        }
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

    /// Whether `name` was written as a string-literal key (`obj['key']`) rather
    /// than an identifier (`obj.key`). The syntax adapter folds both into a field
    /// access whose span covers the key text, so the quote just before it tells
    /// them apart. TypeScript lets bracket access reach a member the class does
    /// not declare; dot access stays a type error.
    pub(crate) fn name_is_bracket_literal(&self, name: &ast::Name) -> bool {
        if ast::synthetic_ident_text(*name).is_some() || name.span.start >= SYNTHETIC_NAME_BASE {
            return false;
        }
        let start = name.span.start as usize;
        start > 0
            && matches!(self.source.as_bytes().get(start - 1), Some(b'\'' | b'"'))
            && matches!(self.source.as_bytes().get(name.span.end as usize), Some(b'\'' | b'"'))
    }

    pub(crate) fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(
            Diagnostic::error(message)
                .with_code("SEM0001")
                .with_span(span),
        );
    }
}
