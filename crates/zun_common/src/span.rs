//! Source spans.
//!
//! A [`Span`] is a half-open byte range `[start, end)` into a single source
//! file. Spans are the currency of diagnostics and debug info (sequence points);
//! they stay byte-based so they are cheap to carry and map directly onto the
//! original text.

/// A half-open byte range into a source file.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// Inclusive start byte offset.
    pub start: u32,
    /// Exclusive end byte offset.
    pub end: u32,
}

impl Span {
    /// A span covering `[start, end)`.
    #[inline]
    pub const fn new(start: u32, end: u32) -> Self {
        debug_assert!(start <= end, "span start must not exceed end");
        Self { start, end }
    }

    /// An empty span at `offset`, used for synthesized nodes with no source text.
    #[inline]
    pub const fn empty_at(offset: u32) -> Self {
        Self { start: offset, end: offset }
    }

    /// The number of bytes the span covers.
    #[inline]
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    /// Whether the span covers no bytes.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// The smallest span covering both `self` and `other`.
    #[inline]
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

impl core::fmt::Debug for Span {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

#[cfg(test)]
mod tests {
    use super::Span;

    #[test]
    fn len_and_empty() {
        assert_eq!(Span::new(3, 7).len(), 4);
        assert!(Span::empty_at(5).is_empty());
        assert_eq!(Span::empty_at(5).len(), 0);
    }

    #[test]
    fn merge_covers_both() {
        let merged = Span::new(2, 5).to(Span::new(8, 10));
        assert_eq!(merged, Span::new(2, 10));
    }
}
