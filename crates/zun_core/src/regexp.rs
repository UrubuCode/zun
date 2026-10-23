//! Runtime-owned regular-expression values.
//!
//! A [`RegexValue`] holds one of two engines, chosen automatically per pattern
//! when it is compiled (PLAN §3.3, stage 2c):
//!
//! * **Engine A** (`regex` crate, RE2-style): linear time, immune to
//!   catastrophic backtracking, but no backreferences or lookaround. Default.
//! * **Engine B** (`zun_regex`): backtracking, supports backreferences and
//!   lookahead/lookbehind, guarded by a step budget.
//!
//! # Selection
//!
//! The pattern is parsed with `zun_regex::parse` and the AST is inspected
//! (deterministic, no trial compile): any `BackRef` or `Look` node selects
//! Engine B. Otherwise Engine A is used. If Engine A then rejects a pattern
//! that Engine B can parse (a JS-valid pattern RE2 does not understand), Engine
//! B is used as a fallback. If neither accepts it, the compile error is
//! returned.
//!
//! # Observable units
//!
//! Engine B works in char indices. Every public method here converts so that
//! results are identical to Engine A: [`RegexValue::find`] returns UTF-8 byte
//! offsets and [`RegexValue::search`] a UTF-16 code-unit index.

use std::sync::Arc;

#[derive(Debug, Clone)]
enum Engine {
    /// Engine A: RE2-style linear-time matcher.
    Re2(regex::Regex),
    /// Engine B: backtracking matcher with backrefs and lookaround.
    Backtrack(Arc<zun_regex::Regex>),
}

/// Which engine a [`RegexValue`] selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    /// Linear-time RE2-style engine.
    Re2,
    /// Backtracking engine (backreferences / lookaround).
    Backtracking,
}

/// A compiled regular expression kept in the Rust intrinsic floor.
#[derive(Debug, Clone)]
pub struct RegexValue {
    source: String,
    flags: String,
    engine: Engine,
}

/// Error returned when a pattern cannot be compiled or a match fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegexError {
    message: String,
}

impl core::fmt::Display for RegexError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RegexError {}

impl From<zun_regex::RegexError> for RegexError {
    fn from(error: zun_regex::RegexError) -> Self {
        Self {
            message: format!("regular expression failed: {error} (possible catastrophic backtracking)"),
        }
    }
}

/// True when the AST needs features Engine A cannot provide.
fn needs_backtracking(node: &zun_regex::RegexNode) -> bool {
    use zun_regex::RegexNode as N;
    match node {
        N::BackRef(_) | N::Look { .. } => true,
        N::Seq(items) | N::Alt(items) => items.iter().any(needs_backtracking),
        N::Repeat { node, .. } | N::Group { node, .. } => needs_backtracking(node),
        N::Empty | N::Char(_) | N::Any | N::Class(_) | N::Assertion(_) => false,
    }
}

/// Expand `$&`, `$1`..`$9`, `$$` in a replacement. `groups[0]` is the whole
/// match; a `$n` beyond the group count stays literal.
fn expand_replacement(replacement: &str, groups: &[Option<&str>], out: &mut String) {
    let mut chars = replacement.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('$') => {
                chars.next();
                out.push('$');
            }
            Some('&') => {
                chars.next();
                out.push_str(groups[0].unwrap_or(""));
            }
            Some(d) if ('1'..='9').contains(&d) => {
                let index = d as usize - '0' as usize;
                chars.next();
                if index < groups.len() {
                    if let Some(group) = groups[index] {
                        out.push_str(group);
                    }
                } else {
                    out.push('$');
                    out.push(d);
                }
            }
            _ => out.push('$'),
        }
    }
}

fn chars_to_string(chars: &[char], start: usize, end: usize) -> String {
    chars[start..end].iter().collect()
}

fn byte_offset(chars: &[char], index: usize) -> usize {
    chars[..index].iter().map(|c| c.len_utf8()).sum()
}

impl RegexValue {
    /// Compile a pattern, selecting the engine automatically.
    pub fn new(source: impl Into<String>) -> Result<Self, RegexError> {
        Self::with_flags(source, "")
    }

    /// Compile a pattern with a JavaScript-style flags string.
    ///
    /// Supported: `i` (case-insensitive), `m` (multiline), `s` (dotAll), and
    /// `g` (global), which is stored but does not change matching yet: every
    /// operation acts on the first match (`lastIndex` iteration is deferred).
    /// `y`, `u`, `d`, unknown and duplicated flags are rejected with an error
    /// rather than silently ignored.
    pub fn with_flags(
        source: impl Into<String>,
        flags: impl Into<String>,
    ) -> Result<Self, RegexError> {
        let source = source.into();
        let flags = flags.into();
        let mut builder = regex::RegexBuilder::new(&source);
        let mut seen = String::new();
        for flag in flags.chars() {
            if seen.contains(flag) {
                return Err(RegexError {
                    message: format!("duplicate regular expression flag `{flag}`"),
                });
            }
            seen.push(flag);
            match flag {
                'i' => {
                    builder.case_insensitive(true);
                }
                'm' => {
                    builder.multi_line(true);
                }
                's' => {
                    builder.dot_matches_new_line(true);
                }
                'g' => {}
                other => {
                    return Err(RegexError {
                        message: format!("unsupported regular expression flag `{other}`"),
                    });
                }
            }
        }
        let backtrack = |source: &str, flags: &str| -> Result<Engine, zun_regex::RegexParseError> {
            zun_regex::Regex::new(source, flags).map(|re| Engine::Backtrack(Arc::new(re)))
        };
        let parsed = zun_regex::parse(&source);
        let engine = match &parsed {
            Ok(ast) if needs_backtracking(&ast.node) => backtrack(&source, &flags)
                .map_err(|error| RegexError { message: error.to_string() })?,
            _ => match builder.build() {
                Ok(compiled) => Engine::Re2(compiled),
                Err(re2_error) => {
                    if parsed.is_ok() {
                        // JS-valid pattern that RE2 rejects: fall back to Engine B.
                        backtrack(&source, &flags).map_err(|_| RegexError {
                            message: re2_error.to_string(),
                        })?
                    } else {
                        return Err(RegexError { message: re2_error.to_string() });
                    }
                }
            },
        };
        Ok(Self { source, flags, engine })
    }

    pub fn flags(&self) -> &str {
        &self.flags
    }

    /// The engine selected for this pattern.
    pub fn engine_kind(&self) -> EngineKind {
        match self.engine {
            Engine::Re2(_) => EngineKind::Re2,
            Engine::Backtrack(_) => EngineKind::Backtracking,
        }
    }

    /// UTF-16 code-unit index of the first match (JS `String.prototype.search`).
    pub fn search(&self, input: &str) -> Result<Option<usize>, RegexError> {
        match &self.engine {
            Engine::Re2(re) => Ok(re
                .find(input)
                .map(|m| input[..m.start()].encode_utf16().count())),
            Engine::Backtrack(re) => {
                let chars: Vec<char> = input.chars().collect();
                Ok(re
                    .find_chars(&chars, 0)?
                    .map(|m| chars[..m.start()].iter().map(|c| c.len_utf16()).sum()))
            }
        }
    }

    /// JS-style `split`: pieces between matches, with capture groups spliced in
    /// (`None` for a non-participating group). Empty matches at the start, at
    /// the end, or directly after the previous match do not split.
    pub fn split(&self, input: &str) -> Result<Vec<Option<String>>, RegexError> {
        if input.is_empty() {
            return Ok(if self.is_match(input)? {
                Vec::new()
            } else {
                vec![Some(String::new())]
            });
        }
        let mut pieces = Vec::new();
        match &self.engine {
            Engine::Re2(re) => {
                let mut last = 0;
                for captures in re.captures_iter(input) {
                    let whole = captures.get(0).expect("group 0 always exists");
                    if whole.is_empty()
                        && (whole.start() == 0
                            || whole.start() >= input.len()
                            || whole.start() == last)
                    {
                        continue;
                    }
                    pieces.push(Some(input[last..whole.start()].to_owned()));
                    for group in captures.iter().skip(1) {
                        pieces.push(group.map(|m| m.as_str().to_owned()));
                    }
                    last = whole.end();
                }
                pieces.push(Some(input[last..].to_owned()));
            }
            Engine::Backtrack(re) => {
                let chars: Vec<char> = input.chars().collect();
                let mut last = 0;
                let mut pos = 0;
                while pos <= chars.len() {
                    let Some(m) = re.find_chars(&chars, pos)? else { break };
                    let (start, end) = m.span();
                    pos = if end == start { end + 1 } else { end };
                    if start == end && (start == 0 || start >= chars.len() || start == last) {
                        continue;
                    }
                    pieces.push(Some(chars_to_string(&chars, last, start)));
                    for index in 1..m.len() {
                        pieces.push(m.group(index).map(|(a, b)| chars_to_string(&chars, a, b)));
                    }
                    last = end;
                }
                pieces.push(Some(chars_to_string(&chars, last, chars.len())));
            }
        }
        Ok(pieces)
    }

    /// Replace the FIRST match (global iteration is deferred). In
    /// `replacement`, `$&` is the whole match, `$1`..`$9` are capture groups
    /// (empty if non-participating; a literal `$n` if the group does not
    /// exist) and `$$` is a literal `$`.
    pub fn replace_first(&self, input: &str, replacement: &str) -> Result<String, RegexError> {
        match &self.engine {
            Engine::Re2(re) => {
                let Some(captures) = re.captures(input) else {
                    return Ok(input.to_owned());
                };
                let whole = captures.get(0).expect("group 0 always exists");
                let mut out = String::from(&input[..whole.start()]);
                let groups: Vec<Option<&str>> =
                    captures.iter().map(|g| g.map(|m| m.as_str())).collect();
                expand_replacement(replacement, &groups, &mut out);
                out.push_str(&input[whole.end()..]);
                Ok(out)
            }
            Engine::Backtrack(re) => {
                let chars: Vec<char> = input.chars().collect();
                let Some(m) = re.find_chars(&chars, 0)? else {
                    return Ok(input.to_owned());
                };
                let texts: Vec<Option<String>> = (0..m.len())
                    .map(|i| m.group(i).map(|(a, b)| chars_to_string(&chars, a, b)))
                    .collect();
                let groups: Vec<Option<&str>> = texts.iter().map(|t| t.as_deref()).collect();
                let mut out = chars_to_string(&chars, 0, m.start());
                expand_replacement(replacement, &groups, &mut out);
                out.push_str(&chars_to_string(&chars, m.end(), chars.len()));
                Ok(out)
            }
        }
    }

    /// Run the first-match search and return the full match followed by each
    /// capture group (`None` for a group that did not participate).
    pub fn captures(&self, input: &str) -> Result<Option<Vec<Option<String>>>, RegexError> {
        match &self.engine {
            Engine::Re2(re) => Ok(re.captures(input).map(|captures| {
                captures
                    .iter()
                    .map(|group| group.map(|m| m.as_str().to_owned()))
                    .collect()
            })),
            Engine::Backtrack(re) => {
                let chars: Vec<char> = input.chars().collect();
                Ok(re.find_chars(&chars, 0)?.map(|m| {
                    (0..m.len())
                        .map(|i| m.group(i).map(|(a, b)| chars_to_string(&chars, a, b)))
                        .collect()
                }))
            }
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn is_match(&self, input: &str) -> Result<bool, RegexError> {
        match &self.engine {
            Engine::Re2(re) => Ok(re.is_match(input)),
            Engine::Backtrack(re) => Ok(re.is_match(input)?),
        }
    }

    /// First match as UTF-8 byte offsets `(start, end)`.
    pub fn find(&self, input: &str) -> Result<Option<(usize, usize)>, RegexError> {
        match &self.engine {
            Engine::Re2(re) => Ok(re.find(input).map(|m| (m.start(), m.end()))),
            Engine::Backtrack(re) => {
                let chars: Vec<char> = input.chars().collect();
                Ok(re.find_chars(&chars, 0)?.map(|m| {
                    (byte_offset(&chars, m.start()), byte_offset(&chars, m.end()))
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EngineKind, RegexValue};

    #[test]
    fn compiles_and_matches_in_the_runtime_floor() {
        let value = RegexValue::new(r"zun-[0-9]+").expect("valid pattern");
        assert!(value.is_match("zun-42").unwrap());
        assert!(!value.is_match("zun-runtime").unwrap());
        assert_eq!(value.find("prefix zun-42 suffix").unwrap(), Some((7, 13)));
        assert_eq!(value.source(), r"zun-[0-9]+");
        assert_eq!(value.engine_kind(), EngineKind::Re2);
    }

    #[test]
    fn flags_change_matching_and_bad_flags_are_rejected() {
        assert!(!RegexValue::new("abc").unwrap().is_match("ABC").unwrap());
        assert!(RegexValue::with_flags("abc", "i").unwrap().is_match("ABC").unwrap());
        assert!(!RegexValue::new("^b$").unwrap().is_match("a\nb\nc").unwrap());
        assert!(RegexValue::with_flags("^b$", "m").unwrap().is_match("a\nb\nc").unwrap());
        assert!(!RegexValue::new("a.b").unwrap().is_match("a\nb").unwrap());
        assert!(RegexValue::with_flags("a.b", "s").unwrap().is_match("a\nb").unwrap());
        assert!(RegexValue::with_flags("a", "g").is_ok());
        assert!(RegexValue::with_flags("a", "y").is_err());
        assert!(RegexValue::with_flags("a", "ii").is_err());
    }

    #[test]
    fn captures_report_groups_and_non_participating_groups() {
        let value = RegexValue::new(r"(a)(x)?(b)").unwrap();
        assert_eq!(
            value.captures("zab").unwrap(),
            Some(vec![Some("ab".into()), Some("a".into()), None, Some("b".into())])
        );
        assert_eq!(value.captures("zzz").unwrap(), None);
    }

    #[test]
    fn search_split_and_replace_follow_js_shapes() {
        let digits = RegexValue::new(r"[0-9]+").unwrap();
        assert_eq!(digits.search("ab12").unwrap(), Some(2));
        assert_eq!(digits.search("abc").unwrap(), None);
        assert_eq!(
            digits.split("a1b22c").unwrap(),
            vec![Some("a".into()), Some("b".into()), Some("c".into())]
        );
        let with_group = RegexValue::new(r"(,)").unwrap();
        assert_eq!(
            with_group.split("a,b").unwrap(),
            vec![Some("a".into()), Some(",".into()), Some("b".into())]
        );
        let empty = RegexValue::new("").unwrap();
        assert_eq!(empty.split("abc").unwrap().len(), 3);
        let pair = RegexValue::new(r"(\w)-(\w)").unwrap();
        assert_eq!(
            pair.replace_first("x a-b c-d", "$2+$1 [$&] $$ $3").unwrap(),
            "x b+a [a-b] $ $3 c-d"
        );
        assert_eq!(pair.replace_first("nothing", "z").unwrap(), "nothing");
    }

    #[test]
    fn invalid_patterns_return_a_typed_error() {
        let error = RegexValue::new("[").expect_err("invalid pattern");
        assert!(!error.to_string().is_empty());
        assert!(RegexValue::new(r"(?<=a").is_err());
        assert!(RegexValue::new(r"(a)\1(").is_err());
    }

    #[test]
    fn backrefs_and_lookaround_select_engine_b() {
        let backref = RegexValue::new(r"(\w)\1").unwrap();
        assert_eq!(backref.engine_kind(), EngineKind::Backtracking);
        assert!(backref.is_match("hello").unwrap());
        assert!(!backref.is_match("helo").unwrap());
        let ahead = RegexValue::new(r"foo(?=bar)").unwrap();
        assert_eq!(ahead.engine_kind(), EngineKind::Backtracking);
        assert_eq!(ahead.find("xfoobar").unwrap(), Some((1, 4)));
        assert!(!ahead.is_match("foobaz").unwrap());
        let behind = RegexValue::new(r"(?<=\$)[0-9]+").unwrap();
        assert_eq!(behind.engine_kind(), EngineKind::Backtracking);
        assert_eq!(
            behind.captures("cost $42").unwrap(),
            Some(vec![Some("42".into())])
        );
        // Nested inside a group / quantifier is still detected.
        assert_eq!(
            RegexValue::new(r"(?:a(?!b))+").unwrap().engine_kind(),
            EngineKind::Backtracking
        );
    }

    #[test]
    fn engine_b_units_match_engine_a_units() {
        // Multi-byte and astral chars before the match: byte offsets are
        // UTF-8, search is UTF-16, for both engines.
        let text = "é😀x12";
        let a = RegexValue::new(r"[0-9]+").unwrap();
        let b = RegexValue::new(r"(?<=x)[0-9]+").unwrap();
        assert_eq!(a.engine_kind(), EngineKind::Re2);
        assert_eq!(b.engine_kind(), EngineKind::Backtracking);
        assert_eq!(a.find(text).unwrap(), b.find(text).unwrap());
        assert_eq!(a.find(text).unwrap(), Some((7, 9)));
        assert_eq!(a.search(text).unwrap(), b.search(text).unwrap());
        assert_eq!(a.search(text).unwrap(), Some(4));
        assert_eq!(a.captures(text).unwrap(), b.captures(text).unwrap());
    }

    #[test]
    fn engine_b_split_and_replace_match_engine_a() {
        let b = RegexValue::new(r"(?<=a)-(?=b)").unwrap();
        assert_eq!(
            b.split("a-b-c a-b").unwrap(),
            vec![Some("a".into()), Some("b-c a".into()), Some("b".into())]
        );
        let b = RegexValue::new(r"(\w)\1").unwrap();
        assert_eq!(b.replace_first("xaay", "[$1$&$2]").unwrap(), "x[aaa$2]y");
    }

    #[test]
    fn catastrophic_pattern_errors_instead_of_hanging() {
        let re = RegexValue::new(r"(a+)+\1b").unwrap();
        assert_eq!(re.engine_kind(), EngineKind::Backtracking);
        let input = format!("{}!", "a".repeat(60));
        let error = re.is_match(&input).expect_err("step limit");
        assert!(error.to_string().contains("step limit"));
        assert!(re.find(&input).is_err());
    }
}
