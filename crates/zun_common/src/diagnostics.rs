//! Diagnostics.
//!
//! A uniform error/warning type shared by every pass. Each stage pushes
//! [`Diagnostic`]s into a sink rather than printing directly, so a driver can
//! collect, sort, and render them however it likes (terminal, LSP, JSON).

use crate::span::Span;

/// How serious a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Compilation cannot proceed.
    Error,
    /// Suspicious but not fatal.
    Warning,
    /// Informational note, usually attached to an error.
    Note,
}

/// A single diagnostic message anchored at a source span.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Severity of the message.
    pub severity: Severity,
    /// Stable machine-readable code (e.g. `"ZIL0001"`); empty if none.
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Primary source location, if the diagnostic has one.
    pub span: Option<Span>,
}

impl Diagnostic {
    /// Start a new error.
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Severity::Error, message)
    }

    /// Start a new warning.
    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, message)
    }

    fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            severity,
            code: String::new(),
            message: message.into(),
            span: None,
        }
    }

    /// Attach a stable code.
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = code.into();
        self
    }

    /// Attach a primary span.
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

/// A collector of diagnostics. Tracks whether any error was recorded.
#[derive(Debug, Default)]
pub struct DiagnosticSink {
    items: Vec<Diagnostic>,
    error_count: usize,
}

impl DiagnosticSink {
    /// Create an empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a diagnostic.
    pub fn push(&mut self, diag: Diagnostic) {
        if diag.severity == Severity::Error {
            self.error_count += 1;
        }
        self.items.push(diag);
    }

    /// Whether any error-severity diagnostic was recorded.
    pub fn has_errors(&self) -> bool {
        self.error_count > 0
    }

    /// All recorded diagnostics in insertion order.
    pub fn items(&self) -> &[Diagnostic] {
        &self.items
    }
}

#[cfg(test)]
mod tests {
    use super::{Diagnostic, DiagnosticSink};
    use crate::span::Span;

    #[test]
    fn builder_sets_fields() {
        let d = Diagnostic::error("bad opcode")
            .with_code("ZIL0001")
            .with_span(Span::new(0, 4));
        assert_eq!(d.code, "ZIL0001");
        assert_eq!(d.span, Some(Span::new(0, 4)));
    }

    #[test]
    fn sink_tracks_errors() {
        let mut sink = DiagnosticSink::new();
        sink.push(Diagnostic::warning("hmm"));
        assert!(!sink.has_errors());
        sink.push(Diagnostic::error("boom"));
        assert!(sink.has_errors());
        assert_eq!(sink.items().len(), 2);
    }
}
