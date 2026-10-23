//! Runtime-owned regular-expression values.

/// A compiled regular expression kept in the Rust intrinsic floor.
#[derive(Debug, Clone)]
pub struct RegexValue {
    source: String,
    flags: String,
    compiled: regex::Regex,
}

/// Error returned when a pattern cannot be compiled.
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

impl RegexValue {
    /// Compile a pattern using the runtime regex engine.
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
        let compiled = builder.build().map_err(|error| RegexError {
            message: error.to_string(),
        })?;
        Ok(Self { source, flags, compiled })
    }

    pub fn flags(&self) -> &str {
        &self.flags
    }

    /// UTF-16 code-unit index of the first match (JS `String.prototype.search`).
    pub fn search(&self, input: &str) -> Option<usize> {
        self.compiled
            .find(input)
            .map(|m| input[..m.start()].encode_utf16().count())
    }

    /// JS-style `split`: pieces between matches, with capture groups spliced in
    /// (`None` for a non-participating group). Empty matches at the start, at
    /// the end, or directly after the previous match do not split.
    pub fn split(&self, input: &str) -> Vec<Option<String>> {
        if input.is_empty() {
            return if self.compiled.is_match(input) {
                Vec::new()
            } else {
                vec![Some(String::new())]
            };
        }
        let mut pieces = Vec::new();
        let mut last = 0;
        for captures in self.compiled.captures_iter(input) {
            let whole = captures.get(0).expect("group 0 always exists");
            if whole.is_empty() && (whole.start() == 0 || whole.start() >= input.len() || whole.start() == last) {
                continue;
            }
            pieces.push(Some(input[last..whole.start()].to_owned()));
            for group in captures.iter().skip(1) {
                pieces.push(group.map(|m| m.as_str().to_owned()));
            }
            last = whole.end();
        }
        pieces.push(Some(input[last..].to_owned()));
        pieces
    }

    /// Replace the FIRST match (global iteration is deferred). In
    /// `replacement`, `$&` is the whole match, `$1`..`$9` are capture groups
    /// (empty if non-participating; a literal `$n` if the group does not
    /// exist) and `$$` is a literal `$`.
    pub fn replace_first(&self, input: &str, replacement: &str) -> String {
        let Some(captures) = self.compiled.captures(input) else {
            return input.to_owned();
        };
        let whole = captures.get(0).expect("group 0 always exists");
        let mut out = String::from(&input[..whole.start()]);
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
                    out.push_str(whole.as_str());
                }
                Some(d) if ('1'..='9').contains(&d) => {
                    let index = d as usize - '0' as usize;
                    chars.next();
                    if index < captures.len() {
                        if let Some(group) = captures.get(index) {
                            out.push_str(group.as_str());
                        }
                    } else {
                        out.push('$');
                        out.push(d);
                    }
                }
                _ => out.push('$'),
            }
        }
        out.push_str(&input[whole.end()..]);
        out
    }

    /// Run the first-match search and return the full match followed by each
    /// capture group (`None` for a group that did not participate).
    pub fn captures(&self, input: &str) -> Option<Vec<Option<String>>> {
        let captures = self.compiled.captures(input)?;
        Some(
            captures
                .iter()
                .map(|group| group.map(|m| m.as_str().to_owned()))
                .collect(),
        )
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn is_match(&self, input: &str) -> bool {
        self.compiled.is_match(input)
    }

    pub fn find(&self, input: &str) -> Option<(usize, usize)> {
        self.compiled.find(input).map(|match_| (match_.start(), match_.end()))
    }
}

#[cfg(test)]
mod tests {
    use super::RegexValue;

    #[test]
    fn compiles_and_matches_in_the_runtime_floor() {
        let value = RegexValue::new(r"zun-[0-9]+").expect("valid pattern");
        assert!(value.is_match("zun-42"));
        assert!(!value.is_match("zun-runtime"));
        assert_eq!(value.find("prefix zun-42 suffix"), Some((7, 13)));
        assert_eq!(value.source(), r"zun-[0-9]+");
    }

    #[test]
    fn flags_change_matching_and_bad_flags_are_rejected() {
        assert!(!RegexValue::new("abc").unwrap().is_match("ABC"));
        assert!(RegexValue::with_flags("abc", "i").unwrap().is_match("ABC"));
        assert!(!RegexValue::new("^b$").unwrap().is_match("a\nb\nc"));
        assert!(RegexValue::with_flags("^b$", "m").unwrap().is_match("a\nb\nc"));
        assert!(!RegexValue::new("a.b").unwrap().is_match("a\nb"));
        assert!(RegexValue::with_flags("a.b", "s").unwrap().is_match("a\nb"));
        assert!(RegexValue::with_flags("a", "g").is_ok());
        assert!(RegexValue::with_flags("a", "y").is_err());
        assert!(RegexValue::with_flags("a", "ii").is_err());
    }

    #[test]
    fn captures_report_groups_and_non_participating_groups() {
        let value = RegexValue::new(r"(a)(x)?(b)").unwrap();
        assert_eq!(
            value.captures("zab"),
            Some(vec![Some("ab".into()), Some("a".into()), None, Some("b".into())])
        );
        assert_eq!(value.captures("zzz"), None);
    }

    #[test]
    fn search_split_and_replace_follow_js_shapes() {
        let digits = RegexValue::new(r"[0-9]+").unwrap();
        assert_eq!(digits.search("ab12"), Some(2));
        assert_eq!(digits.search("abc"), None);
        assert_eq!(
            digits.split("a1b22c"),
            vec![Some("a".into()), Some("b".into()), Some("c".into())]
        );
        let with_group = RegexValue::new(r"(,)").unwrap();
        assert_eq!(
            with_group.split("a,b"),
            vec![Some("a".into()), Some(",".into()), Some("b".into())]
        );
        let empty = RegexValue::new("").unwrap();
        assert_eq!(empty.split("abc").len(), 3);
        let pair = RegexValue::new(r"(\w)-(\w)").unwrap();
        assert_eq!(pair.replace_first("x a-b c-d", "$2+$1 [$&] $$ $3"), "x b+a [a-b] $ $3 c-d");
        assert_eq!(pair.replace_first("nothing", "z"), "nothing");
    }

    #[test]
    fn invalid_patterns_return_a_typed_error() {
        let error = RegexValue::new("[").expect_err("invalid pattern");
        assert!(!error.to_string().is_empty());
    }
}
