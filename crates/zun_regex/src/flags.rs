//! Regex flags.

use crate::error::RegexParseError;

/// Regex flags. Only `ignore_case`, `multiline` and `dot_all` affect matching
/// in this engine; `global`, `sticky`, `unicode` and `has_indices` are parsed
/// and stored for the embedder (`lastIndex` handling is the caller's job;
/// `sticky` callers should use [`crate::Regex::match_at_chars`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    /// `i`
    pub ignore_case: bool,
    /// `m`
    pub multiline: bool,
    /// `s`
    pub dot_all: bool,
    /// `g` (stored only)
    pub global: bool,
    /// `y` (stored only)
    pub sticky: bool,
    /// `u` (stored only; matching is always by Unicode scalar value)
    pub unicode: bool,
    /// `d` (stored only)
    pub has_indices: bool,
}

impl Flags {
    /// Parses a JS flag string such as `"gi"`. Unknown or duplicate flags are errors.
    pub fn parse(s: &str) -> Result<Flags, RegexParseError> {
        let mut f = Flags::default();
        for (i, c) in s.chars().enumerate() {
            let slot = match c {
                'i' => &mut f.ignore_case,
                'm' => &mut f.multiline,
                's' => &mut f.dot_all,
                'g' => &mut f.global,
                'y' => &mut f.sticky,
                'u' => &mut f.unicode,
                'd' => &mut f.has_indices,
                _ => return Err(RegexParseError::new(format!("unknown flag '{c}'"), i)),
            };
            if *slot {
                return Err(RegexParseError::new(format!("duplicate flag '{c}'"), i));
            }
            *slot = true;
        }
        Ok(f)
    }
}
