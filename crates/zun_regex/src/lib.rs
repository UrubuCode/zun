//! `zun_regex`: "Engine B", a standalone backtracking ECMAScript regex engine.
//!
//! It provides what an RE2-style engine cannot: backreferences, lookahead and
//! lookbehind, with a mandatory step budget against catastrophic backtracking.
//!
//! # Indexing model
//!
//! Matching is by Unicode scalar value (`char`) over the input. **All
//! positions in the API are char indices**, not bytes and not UTF-16 code
//! units. The integration layer must convert if it needs another unit.
//!
//! # Pipeline
//!
//! pattern -> [`parser`] -> [`RegexNode`] -> [`compile`] (flat program) ->
//! [`matcher`] (explicit backtrack stack; lookbehind runs the body backwards).
//!
//! # Known deviations
//!
//! * Lone UTF-16 surrogate escapes (`\uD800`) become U+FFFD; surrogate pairs
//!   written as two `\u` escapes are combined into one char.
//! * Case folding for `i` uses single-char `to_uppercase` (JS `Canonicalize`
//!   approximation); `\w` / `\b` stay ASCII-only as in non-`u` JS.
//! * `\p{..}` and `\u{..}` (unicode-mode syntax) are not supported.

mod ast;
mod compile;
mod error;
mod flags;
mod matcher;
mod parser;
pub mod zil;

#[cfg(test)]
mod tests;

pub use ast::{AssertKind, CharClass, ClassItem, RegexNode};
pub use error::{RegexError, RegexParseError};
pub use flags::Flags;
pub use parser::{parse, ParsedPattern};

use std::rc::Rc;

use compile::Program;
use matcher::{Vm, NONE};

/// Default step budget per `find` call.
pub const DEFAULT_STEP_LIMIT: u64 = 10_000_000;

/// A compiled regex.
#[derive(Debug)]
pub struct Regex {
    program: Program,
    flags: Flags,
    names: Rc<Vec<(String, usize)>>,
    step_limit: u64,
    source: String,
}

/// A successful match. Index 0 is the whole match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    groups: Vec<Option<(usize, usize)>>,
    names: Rc<Vec<(String, usize)>>,
}

impl Match {
    /// Char index where the match starts.
    pub fn start(&self) -> usize {
        self.groups[0].unwrap().0
    }

    /// Char index one past the end of the match.
    pub fn end(&self) -> usize {
        self.groups[0].unwrap().1
    }

    /// Full-match span `(start, end)`.
    pub fn span(&self) -> (usize, usize) {
        self.groups[0].unwrap()
    }

    /// Span of group `i` (0 = whole match); `None` if unset or out of range.
    pub fn group(&self, i: usize) -> Option<(usize, usize)> {
        self.groups.get(i).copied().flatten()
    }

    /// Span of the named group.
    pub fn named(&self, name: &str) -> Option<(usize, usize)> {
        let (_, i) = self.names.iter().find(|(n, _)| n == name)?;
        self.group(*i)
    }

    /// Number of groups including group 0.
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Always false (group 0 always exists); provided for API symmetry.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// All group spans, index 0 first.
    pub fn groups(&self) -> &[Option<(usize, usize)>] {
        &self.groups
    }

    /// Group names as `(name, index)`.
    pub fn group_names(&self) -> &[(String, usize)] {
        &self.names
    }
}

impl Regex {
    /// Parses and compiles `pattern` with `flags`.
    pub fn compile(pattern: &str, flags: Flags) -> Result<Regex, RegexParseError> {
        let parsed = parser::parse(pattern)?;
        let program = compile::compile(&parsed.node, parsed.group_count, flags);
        Ok(Regex {
            program,
            flags,
            names: Rc::new(parsed.names),
            step_limit: DEFAULT_STEP_LIMIT,
            source: pattern.to_string(),
        })
    }

    /// Convenience: compile with a JS flag string such as `"gi"`.
    pub fn new(pattern: &str, flags: &str) -> Result<Regex, RegexParseError> {
        Regex::compile(pattern, Flags::parse(flags)?)
    }

    /// Sets the per-call step budget.
    pub fn with_step_limit(mut self, limit: u64) -> Regex {
        self.step_limit = limit;
        self
    }

    /// The per-call step budget.
    pub fn step_limit(&self) -> u64 {
        self.step_limit
    }

    /// The flags this regex was compiled with.
    pub fn flags(&self) -> Flags {
        self.flags
    }

    /// The source pattern.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Number of capture groups (excluding group 0).
    pub fn group_count(&self) -> usize {
        self.program.group_count
    }

    /// Named groups as `(name, index)`.
    pub fn group_names(&self) -> &[(String, usize)] {
        &self.names
    }

    /// Finds the first match starting at char index `>= start`.
    pub fn find(&self, input: &str, start: usize) -> Result<Option<Match>, RegexError> {
        let chars: Vec<char> = input.chars().collect();
        self.find_chars(&chars, start)
    }

    /// Like [`Regex::find`] over a pre-decoded char slice.
    pub fn find_chars(&self, input: &[char], start: usize) -> Result<Option<Match>, RegexError> {
        self.find_chars_impl(input, start, true)
    }

    /// Shared body of [`Regex::find_chars`]. When `use_prefilter` is false, every
    /// position is tried (the pre-prefilter behavior) — used only to prove the
    /// prefilter never changes results.
    fn find_chars_impl(
        &self,
        input: &[char],
        start: usize,
        use_prefilter: bool,
    ) -> Result<Option<Match>, RegexError> {
        let mut vm = Vm::new(&self.program, input, self.step_limit);
        // Allocated once per call and reused across every start position.
        let mut caps = vec![NONE; 2 * (self.program.group_count + 1)];
        let mut regs = vec![0usize; self.program.reg_count];
        let mut s = start;
        loop {
            let cand = if use_prefilter {
                self.program.prefilter.next(input, s)
            } else {
                (s <= input.len()).then_some(s)
            };
            let Some(cand) = cand else { return Ok(None) };
            if let Some(m) = self.try_at(&mut vm, cand, &mut caps, &mut regs)? {
                return Ok(Some(m));
            }
            s = cand + 1;
        }
    }

    /// Test-only naive scan (prefilter disabled) for equivalence checking.
    #[cfg(test)]
    pub(crate) fn find_chars_naive(
        &self,
        input: &[char],
        start: usize,
    ) -> Result<Option<Match>, RegexError> {
        self.find_chars_impl(input, start, false)
    }

    /// Anchored match attempt at exactly `pos` (sticky semantics).
    pub fn match_at_chars(&self, input: &[char], pos: usize) -> Result<Option<Match>, RegexError> {
        if pos > input.len() {
            return Ok(None);
        }
        let mut vm = Vm::new(&self.program, input, self.step_limit);
        let mut caps = vec![NONE; 2 * (self.program.group_count + 1)];
        let mut regs = vec![0usize; self.program.reg_count];
        self.try_at(&mut vm, pos, &mut caps, &mut regs)
    }

    /// Whether the pattern matches anywhere in `input`.
    pub fn is_match(&self, input: &str) -> Result<bool, RegexError> {
        Ok(self.find(input, 0)?.is_some())
    }

    fn try_at(
        &self,
        vm: &mut Vm<'_>,
        start: usize,
        caps: &mut [usize],
        regs: &mut [usize],
    ) -> Result<Option<Match>, RegexError> {
        // Reset reused buffers for this attempt.
        caps.iter_mut().for_each(|c| *c = NONE);
        regs.iter_mut().for_each(|r| *r = 0);
        vm.reset();
        match vm.run(0, start, caps, regs)? {
            None => Ok(None),
            Some(end) => {
                caps[0] = start;
                caps[1] = end;
                let groups = caps
                    .chunks(2)
                    .map(|c| (c[0] != NONE && c[1] != NONE).then_some((c[0], c[1])))
                    .collect();
                Ok(Some(Match { groups, names: self.names.clone() }))
            }
        }
    }
}
