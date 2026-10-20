//! Runtime-owned regular-expression values.

/// A compiled regular expression kept in the Rust intrinsic floor.
#[derive(Debug, Clone)]
pub struct RegexValue {
    source: String,
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
        let source = source.into();
        let compiled = regex::Regex::new(&source).map_err(|error| RegexError {
            message: error.to_string(),
        })?;
        Ok(Self { source, compiled })
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
    fn invalid_patterns_return_a_typed_error() {
        let error = RegexValue::new("[").expect_err("invalid pattern");
        assert!(!error.to_string().is_empty());
    }
}
