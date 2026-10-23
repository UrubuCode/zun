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
    fn invalid_patterns_return_a_typed_error() {
        let error = RegexValue::new("[").expect_err("invalid pattern");
        assert!(!error.to_string().is_empty());
    }
}
