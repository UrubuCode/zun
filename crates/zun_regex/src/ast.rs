//! The regex AST.

/// Zero-width position assertions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssertKind {
    /// `^`
    Start,
    /// `$`
    End,
    /// `\b`
    WordBoundary,
    /// `\B`
    NotWordBoundary,
}

/// One element of a character class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassItem {
    /// Inclusive range (a single char is `Range(c, c)`).
    Range(char, char),
    /// `\d`
    Digit,
    /// `\D`
    NotDigit,
    /// `\w`
    Word,
    /// `\W`
    NotWord,
    /// `\s`
    Space,
    /// `\S`
    NotSpace,
}

/// A character class `[...]` / `[^...]`, or a lone class escape like `\d`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharClass {
    /// Members; matched as a union.
    pub items: Vec<ClassItem>,
    /// `[^...]`
    pub negated: bool,
}

/// `\w` membership (ASCII word characters).
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// JS WhiteSpace + LineTerminator.
pub(crate) fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}'
            | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}'
            | '\u{3000}' | '\u{feff}'
    )
}

/// JS LineTerminator.
pub(crate) fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// Case canonicalization used for `i` matching (JS non-unicode `Canonicalize`:
/// single-char uppercase mapping, never mapping non-ASCII to ASCII).
pub(crate) fn canonicalize(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_uppercase();
    }
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(u), None) if !u.is_ascii() => u,
        _ => c,
    }
}

impl ClassItem {
    fn matches_exact(self, c: char) -> bool {
        match self {
            ClassItem::Range(lo, hi) => lo <= c && c <= hi,
            ClassItem::Digit => c.is_ascii_digit(),
            ClassItem::NotDigit => !c.is_ascii_digit(),
            ClassItem::Word => is_word_char(c),
            ClassItem::NotWord => !is_word_char(c),
            ClassItem::Space => is_space(c),
            ClassItem::NotSpace => !is_space(c),
        }
    }
}

impl CharClass {
    /// Tests membership of `c`. With `icase`, case variants of `c` also count
    /// (before negation, as in the spec).
    pub fn matches(&self, c: char, icase: bool) -> bool {
        let mut hit = self.items.iter().any(|i| i.matches_exact(c));
        if !hit && icase {
            let up = canonicalize(c);
            let mut lo = c.to_lowercase();
            let lower = match (lo.next(), lo.next()) {
                (Some(l), None) => l,
                _ => c,
            };
            hit = self
                .items
                .iter()
                .any(|i| (up != c && i.matches_exact(up)) || (lower != c && i.matches_exact(lower)));
        }
        hit != self.negated
    }
}

/// The parsed regex tree. Non-capturing groups are folded into their contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegexNode {
    /// Matches the empty string.
    Empty,
    /// A literal character.
    Char(char),
    /// `.`
    Any,
    /// A character class or class escape.
    Class(CharClass),
    /// Concatenation.
    Seq(Vec<RegexNode>),
    /// Alternation (ordered).
    Alt(Vec<RegexNode>),
    /// Quantified node. `max == None` means unbounded.
    Repeat {
        /// Quantified body.
        node: Box<RegexNode>,
        /// Minimum iterations.
        min: u32,
        /// Maximum iterations.
        max: Option<u32>,
        /// Greedy (true) or lazy (false).
        greedy: bool,
    },
    /// Capturing group; `index` is 1-based.
    Group {
        /// 1-based capture index.
        index: usize,
        /// Name for `(?<name>...)`.
        name: Option<String>,
        /// Group body.
        node: Box<RegexNode>,
    },
    /// `^ $ \b \B`
    Assertion(AssertKind),
    /// Backreference to a capture index (named references are resolved at parse time).
    BackRef(usize),
    /// Lookahead / lookbehind.
    Look {
        /// `true` for lookbehind.
        behind: bool,
        /// `true` for the negative form.
        negate: bool,
        /// Assertion body.
        node: Box<RegexNode>,
    },
}

impl RegexNode {
    /// Smallest and largest capture index inside this node, if any.
    pub(crate) fn group_range(&self) -> Option<(usize, usize)> {
        fn merge(a: Option<(usize, usize)>, b: Option<(usize, usize)>) -> Option<(usize, usize)> {
            match (a, b) {
                (Some((a0, a1)), Some((b0, b1))) => Some((a0.min(b0), a1.max(b1))),
                (x, None) | (None, x) => x,
            }
        }
        match self {
            RegexNode::Seq(v) | RegexNode::Alt(v) => {
                v.iter().fold(None, |acc, n| merge(acc, n.group_range()))
            }
            RegexNode::Repeat { node, .. } | RegexNode::Look { node, .. } => node.group_range(),
            RegexNode::Group { index, node, .. } => merge(Some((*index, *index)), node.group_range()),
            _ => None,
        }
    }
}
