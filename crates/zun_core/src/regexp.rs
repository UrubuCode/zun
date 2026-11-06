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
    /// The `lastIndex` own data property (a JS number; clamped on read).
    last_index: f64,
    /// Whether `lastIndex` has moved into the object's dynamic property table
    /// (so it can carry attributes and any value); `last_index` is then stale.
    materialized: bool,
    /// Lazily built backtracking twin of an RE2-engine regex, used when a
    /// non-unicode pattern meets input with astral characters (whose halves
    /// the RE2 engine cannot address individually).
    fallback: std::rc::Rc<std::cell::OnceCell<Option<Arc<zun_regex::Regex>>>>,
}

/// The chars of `input` with every astral character replaced by its two
/// lone-surrogate stand-ins (the UTF-16 code-unit view of a non-unicode pattern).
fn split_astral(input: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(input.len());
    for c in input.chars() {
        if c.len_utf8() == 4 && crate::wtf16::lone_unit(c).is_none() {
            for unit in crate::wtf16::encode(c.encode_utf8(&mut [0u8; 4])) {
                out.extend(crate::wtf16::from_units(&[unit]).chars());
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A located match (see [`RegexValue::match_full`]).
#[derive(Debug, Clone)]
pub struct MatchFull {
    /// UTF-8 byte offset of the match start.
    pub start_byte: usize,
    /// UTF-8 byte offset of the match end.
    pub end_byte: usize,
    /// UTF-16 index of the match start.
    pub start16: usize,
    /// UTF-16 index of the match end.
    pub end16: usize,
    /// The whole match followed by the capture groups.
    pub groups: Vec<Option<String>>,
    /// UTF-16 `[start, end)` of the whole match and each capture group.
    pub group_spans16: Vec<Option<(usize, usize)>>,
}

/// UTF-8 byte offset of UTF-16 index `index` (clamped to the end; an index
/// inside a surrogate pair maps to the end of that character).
pub fn byte_of_utf16(input: &str, index: usize) -> usize {
    let mut units = 0usize;
    for (byte, c) in input.char_indices() {
        if units >= index {
            return byte;
        }
        units += if crate::wtf16::lone_unit(c).is_some() { 1 } else { c.len_utf16() };
    }
    input.len()
}

/// Error returned when a pattern cannot be compiled or a match fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegexError {
    message: String,
}

impl RegexError {
    /// An error carrying `message` (a leading `TypeError:` names the thrown class).
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
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
///
/// `.` without `s` must exclude CR / U+2028 / U+2029 (RE2 only excludes LF)
/// and multiline anchors must treat all four line terminators alike.
fn needs_backtracking(node: &zun_regex::RegexNode, dot_all: bool, multiline: bool, unicode: bool) -> bool {
    use zun_regex::RegexNode as N;
    match node {
        N::BackRef(_) | N::Look { .. } => true,
        N::Any => !dot_all || !unicode,
        // ``/`B` are Unicode-aware in RE2 but ASCII-only in JS.
        N::Assertion(kind) => {
            multiline
                || matches!(kind, zun_regex::AssertKind::WordBoundary | zun_regex::AssertKind::NotWordBoundary)
        }
        // `d`/`w`/`s` are Unicode-aware in RE2 but ASCII / JS-space in JS.
        N::Class(class) => {
            (class.negated && !unicode)
                || class.items.iter().any(|item| match item {
                    // A range reaching the surrogate block must see lone-surrogate units.
                    zun_regex::ClassItem::Range(_, hi) => !unicode && (*hi as u32) >= 0xD800,
                    _ => true,
                })
        }
        N::Seq(items) | N::Alt(items) => items.iter().any(|n| needs_backtracking(n, dot_all, multiline, unicode)),
        // Captures inside a repeated body are reset on each iteration, which
        // RE2 does not do.
        N::Repeat { node, .. } => node.group_range().is_some() || needs_backtracking(node, dot_all, multiline, unicode),
        N::Group { node, .. } => needs_backtracking(node, dot_all, multiline, unicode),
        N::Empty | N::Char(_) => false,
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

/// ECMAScript GetSubstitution: `$$`, `$&`, `$\``, `$'`, `$n`, `$nn` and
/// `$<name>`. `before`/`after` are the subject text around the match.
pub fn expand_substitution(
    replacement: &str,
    groups: &[Option<&str>],
    names: &[Option<String>],
    before: &str,
    after: &str,
    out: &mut String,
) {
    let chars: Vec<char> = replacement.chars().collect();
    let has_named = names.iter().any(|n| n.is_some());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '$' || i + 1 >= chars.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let next = chars[i + 1];
        match next {
            '$' => {
                out.push('$');
                i += 2;
            }
            '&' => {
                out.push_str(groups[0].unwrap_or(""));
                i += 2;
            }
            '`' => {
                out.push_str(before);
                i += 2;
            }
            '\'' => {
                out.push_str(after);
                i += 2;
            }
            d if d.is_ascii_digit() => {
                let one = d as usize - '0' as usize;
                let two = chars
                    .get(i + 2)
                    .filter(|c| c.is_ascii_digit())
                    .map(|c| one * 10 + (*c as usize - '0' as usize));
                let m = groups.len() - 1;
                if let Some(n) = two.filter(|n| *n >= 1 && *n <= m) {
                    out.push_str(groups[n].unwrap_or(""));
                    i += 3;
                } else if one >= 1 && one <= m {
                    out.push_str(groups[one].unwrap_or(""));
                    i += 2;
                } else {
                    out.push('$');
                    i += 1;
                }
            }
            '<' if has_named => {
                if let Some(close) = chars[i + 2..].iter().position(|c| *c == '>') {
                    let name: String = chars[i + 2..i + 2 + close].iter().collect();
                    if let Some(index) = names.iter().position(|n| n.as_deref() == Some(name.as_str())) {
                        out.push_str(groups.get(index).copied().flatten().unwrap_or(""));
                    }
                    i += close + 3;
                } else {
                    out.push('$');
                    i += 1;
                }
            }
            _ => {
                out.push('$');
                i += 1;
            }
        }
    }
}

fn chars_to_string(chars: &[char], start: usize, end: usize) -> String {
    // Re-pair adjacent lone-surrogate halves split out of an astral character.
    crate::wtf16::normalize(chars[start..end].iter().collect())
}

/// Byte offset in the ORIGINAL input of char index `index`. An astral
/// character fed unit-wise (hi stand-in, lo stand-in) counts once, so an index
/// between its halves lands on the end of the character.
fn byte_offset(chars: &[char], index: usize) -> usize {
    let mut total = 0;
    let mut prev_hi = false;
    for c in &chars[..index] {
        let unit = crate::wtf16::lone_unit(*c);
        let is_lo = matches!(unit, Some(0xDC00..=0xDFFF));
        if !(is_lo && prev_hi) {
            total += c.len_utf8();
        }
        prev_hi = matches!(unit, Some(0xD800..=0xDBFF));
    }
    total
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
        if flags.contains('u') || flags.contains('v') {
            // Unicode mode rejects identity escapes of ordinary letters (`q`).
            let bytes = source.as_bytes();
            let group_count = zun_regex::parse(&source).map(|p| p.group_count).unwrap_or(0);
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    let next = bytes[i + 1];
                    // No legacy octal / identity fallbacks in unicode mode.
                    if next.is_ascii_digit() {
                        let mut j = i + 1;
                        let mut value = 0usize;
                        while j < bytes.len() && bytes[j].is_ascii_digit() {
                            value = value.saturating_mul(10).saturating_add((bytes[j] - b'0') as usize);
                            j += 1;
                        }
                        let bad = if next == b'0' { j > i + 2 } else { value > group_count };
                        if bad {
                            return Err(RegexError { message: "invalid escape in unicode mode".into() });
                        }
                        i = j;
                        continue;
                    }
                    if next == b'p' || next == b'P' {
                        let bad = || RegexError { message: "invalid property name in unicode mode".into() };
                        if bytes.get(i + 2) != Some(&b'{') {
                            return Err(bad());
                        }
                        let Some(close) = source[i + 3..].find('}') else { return Err(bad()) };
                        let body = &source[i + 3..i + 3 + close];
                        let well_formed = match body.split_once('=') {
                            Some((key, value)) => {
                                ["General_Category", "gc", "Script", "sc", "Script_Extensions", "scx"].contains(&key)
                                    && value.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                            }
                            None => body.chars().next().is_some_and(|c| c.is_ascii_uppercase()),
                        } && !body.contains(' ');
                        if !well_formed || regex::Regex::new(&format!(r"\p{{{body}}}")).is_err() {
                            return Err(bad());
                        }
                        i += 4 + close;
                        continue;
                    }
                    if next == b'k' && bytes.get(i + 2) != Some(&b'<') {
                        return Err(RegexError { message: "invalid named reference".into() });
                    }
                    if next.is_ascii_alphabetic() && !b"dDwWsSbBfnrtvcxukpPgG".contains(&next) {
                        return Err(RegexError { message: format!("invalid escape `\\{}`", next as char) });
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
        }
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
                // Unicode mode: the engines already match by code point.
                'u' | 'v' => {}
                // Sticky and hasIndices only change the stateful match drivers.
                'y' | 'd' => {}
                other => {
                    return Err(RegexError {
                        message: format!("unsupported regular expression flag `{other}`"),
                    });
                }
            }
        }
        let backtrack = |source: &str, flags: &str| -> Result<Engine, zun_regex::RegexParseError> {
            // The backtracking engine knows `u` but not `v` (a superset of it).
            let engine_flags = if flags.contains('v') { flags.replace('v', "u") } else { flags.to_owned() };
            // Without `u` the pattern is a sequence of UTF-16 code units.
            let unit_source: String;
            let source = if engine_flags.contains('u') || source.is_ascii() {
                source
            } else {
                unit_source = split_astral(source).into_iter().collect();
                &unit_source
            };
            zun_regex::Regex::new(source, &engine_flags).map(|re| Engine::Backtrack(Arc::new(re)))
        };
        let parsed = zun_regex::parse(&source);
        let engine = match &parsed {
            // Non-unicode `i` canonicalizes through toUpperCase (U+212A stays
            // distinct from "k"), which only the backtracking engine models.
            Ok(_) if flags.contains('i') && !flags.contains('u') && !flags.contains('v') => backtrack(&source, &flags)
                .map_err(|error| RegexError { message: error.to_string() })?,
            Ok(ast) if needs_backtracking(&ast.node, flags.contains('s'), flags.contains('m'), flags.contains('u') || flags.contains('v')) => backtrack(&source, &flags)
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
        Ok(Self { source, flags, engine, last_index: 0.0, materialized: false, fallback: Default::default() })
    }

    /// The chars the backtracking engine runs over. Without the `u` flag a
    /// pattern sees UTF-16 code units, so an astral character becomes its two
    /// lone-surrogate stand-ins.
    fn chars_of(&self, input: &str) -> Vec<char> {
        if self.flags.contains('u') || self.flags.contains('v') || input.is_ascii() {
            return input.chars().collect();
        }
        split_astral(input)
    }

    pub fn flags(&self) -> &str {
        &self.flags
    }

    /// The raw `lastIndex` slot.
    pub fn last_index(&self) -> f64 {
        self.last_index
    }

    /// Overwrite the `lastIndex` slot.
    pub fn set_last_index(&mut self, value: f64) {
        self.last_index = value;
    }

    /// Whether `lastIndex` lives in the object's property table.
    pub fn is_last_index_materialized(&self) -> bool {
        self.materialized
    }

    /// Mark `lastIndex` as living in the object's property table.
    pub fn mark_last_index_materialized(&mut self) {
        self.materialized = true;
    }

    /// The engine selected for this pattern.
    pub fn engine_kind(&self) -> EngineKind {
        match self.engine {
            Engine::Re2(_) => EngineKind::Re2,
            Engine::Backtrack(_) => EngineKind::Backtracking,
        }
    }

    fn u_mode(&self) -> bool {
        self.flags.contains('u') || self.flags.contains('v')
    }

    /// First match at or after the UTF-16 index `start16` (exactly at it when
    /// `sticky`). `Ok(None)` when there is no match or `start16` lies beyond
    /// the input. Positions are reported both as UTF-8 byte offsets (a position
    /// inside a surrogate pair collapses to the end of the character) and as
    /// UTF-16 indices.
    pub fn match_full(
        &self,
        input: &str,
        start16: usize,
        sticky: bool,
    ) -> Result<Option<MatchFull>, RegexError> {
        let twin: Option<Arc<zun_regex::Regex>> = match &self.engine {
            Engine::Re2(_) if !self.u_mode() && input.chars().any(|c| c.len_utf8() == 4) => self
                .fallback
                .get_or_init(|| {
                    let unit_source: String = split_astral(&self.source).into_iter().collect();
                    zun_regex::Regex::new(&unit_source, &self.flags).ok().map(Arc::new)
                })
                .clone(),
            _ => None,
        };
        let engine = match (&self.engine, &twin) {
            (Engine::Re2(_), Some(re)) => Engine::Backtrack(re.clone()),
            (other, _) => other.clone(),
        };
        match &engine {
            Engine::Re2(re) => {
                let mut units = 0usize;
                let mut start_byte = None;
                for (byte, c) in input.char_indices() {
                    if units >= start16 {
                        start_byte = Some(byte);
                        break;
                    }
                    units += if crate::wtf16::lone_unit(c).is_some() { 1 } else { c.len_utf16() };
                }
                let start_byte = match start_byte {
                    Some(byte) => byte,
                    None if units >= start16 => input.len(),
                    None => return Ok(None),
                };
                let Some(captures) = re.captures_at(input, start_byte) else { return Ok(None) };
                let whole = captures.get(0).expect("group 0 always exists");
                if sticky && whole.start() != start_byte {
                    return Ok(None);
                }
                Ok(Some(MatchFull {
                    start_byte: whole.start(),
                    end_byte: whole.end(),
                    start16: crate::wtf16::len16(&input[..whole.start()]),
                    end16: crate::wtf16::len16(&input[..whole.end()]),
                    groups: captures.iter().map(|group| group.map(|m| m.as_str().to_owned())).collect(),
                    group_spans16: captures
                        .iter()
                        .map(|group| {
                            group.map(|m| {
                                (crate::wtf16::len16(&input[..m.start()]), crate::wtf16::len16(&input[..m.end()]))
                            })
                        })
                        .collect(),
                }))
            }
            Engine::Backtrack(re) => {
                let chars = self.chars_of(input);
                let unit_len = |c: char| if crate::wtf16::lone_unit(c).is_some() { 1 } else { c.len_utf16() };
                let u_mode = self.u_mode();
                let pos = if u_mode {
                    let mut units = 0usize;
                    let mut index = 0usize;
                    while index < chars.len() && units < start16 {
                        units += unit_len(chars[index]);
                        index += 1;
                    }
                    if units < start16 {
                        return Ok(None);
                    }
                    index
                } else {
                    start16
                };
                if pos > chars.len() {
                    return Ok(None);
                }
                let found = if sticky { re.match_at_chars(&chars, pos)? } else { re.find_chars(&chars, pos)? };
                let units_before = |index: usize| -> usize {
                    if u_mode {
                        chars[..index].iter().map(|c| unit_len(*c)).sum()
                    } else {
                        index
                    }
                };
                Ok(found.map(|m| {
                    let (start, end) = m.span();
                    MatchFull {
                        start_byte: byte_offset(&chars, start),
                        end_byte: byte_offset(&chars, end),
                        start16: units_before(start),
                        end16: units_before(end),
                        groups: (0..m.len()).map(|i| m.group(i).map(|(a, b)| chars_to_string(&chars, a, b))).collect(),
                        group_spans16: (0..m.len())
                            .map(|i| m.group(i).map(|(a, b)| (units_before(a), units_before(b))))
                            .collect(),
                    }
                }))
            }
        }
    }

    /// [`Self::match_full`] with byte offsets only.
    #[allow(clippy::type_complexity)]
    pub fn match_from(
        &self,
        input: &str,
        start16: usize,
        sticky: bool,
    ) -> Result<Option<(usize, usize, Vec<Option<String>>)>, RegexError> {
        Ok(self.match_full(input, start16, sticky)?.map(|m| (m.start_byte, m.end_byte, m.groups)))
    }

    /// The matches of a global (or single) scan with the ECMAScript advance
    /// rules: after an empty match the position moves on by one code unit (one
    /// code point in unicode mode); a match may begin where the last one ended.
    pub fn match_all_full(&self, input: &str, global: bool) -> Result<Vec<MatchFull>, RegexError> {
        let total = crate::wtf16::len16(input);
        let sticky = self.flags.contains('y');
        let mut out = Vec::new();
        let mut pos = 0usize;
        while pos <= total {
            let Some(m) = self.match_full(input, pos, sticky)? else { break };
            let (start16, end16) = (m.start16, m.end16);
            out.push(m);
            if !global {
                break;
            }
            pos = if end16 == start16 {
                let step = if self.u_mode() && end16 < total {
                    let at = byte_of_utf16(input, end16);
                    match input[at..].chars().next() {
                        Some(c) if crate::wtf16::lone_unit(c).is_none() && c.len_utf16() == 2 => 2,
                        _ => 1,
                    }
                } else {
                    1
                };
                end16 + step
            } else {
                end16
            };
        }
        Ok(out)
    }

    /// UTF-16 code-unit index of the first match (JS `String.prototype.search`).
    pub fn search(&self, input: &str) -> Result<Option<usize>, RegexError> {
        // A sticky regexp matches only at index 0.
        if self.flags.contains('y') {
            return Ok(self.match_full(input, 0, true)?.map(|found| found.start16));
        }
        match &self.engine {
            Engine::Re2(re) => Ok(re
                .find(input)
                .map(|m| crate::wtf16::len16(&input[..m.start()]))),
            Engine::Backtrack(re) => {
                let chars = self.chars_of(input);
                Ok(re
                    .find_chars(&chars, 0)?
                    .map(|m| {
                        chars[..m.start()]
                            .iter()
                            .map(|c| if crate::wtf16::lone_unit(*c).is_some() { 1 } else { c.len_utf16() })
                            .sum()
                    }))
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
        // Without `u`, an empty pattern splits by UTF-16 code unit (an astral
        // character yields two lone-surrogate pieces).
        if !self.flags.contains('u') && matches!(self.source.as_str(), "" | "(?:)") {
            return Ok(crate::wtf16::encode(input)
                .map(|unit| Some(crate::wtf16::from_units(&[unit])))
                .collect());
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
                let chars = self.chars_of(input);
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
                let chars = self.chars_of(input);
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
                let chars = self.chars_of(input);
                Ok(re.find_chars(&chars, 0)?.map(|m| {
                    (0..m.len())
                        .map(|i| m.group(i).map(|(a, b)| chars_to_string(&chars, a, b)))
                        .collect()
                }))
            }
        }
    }

    /// All non-overlapping matches, each as a group vector (like `captures`).
    /// Used by `String.prototype.matchAll`; an empty match advances one unit so
    /// iteration terminates.
    pub fn captures_all(&self, input: &str) -> Result<Vec<Vec<Option<String>>>, RegexError> {
        let mut matches = Vec::new();
        match &self.engine {
            Engine::Re2(re) => {
                for captures in re.captures_iter(input) {
                    matches.push(
                        captures
                            .iter()
                            .map(|group| group.map(|m| m.as_str().to_owned()))
                            .collect(),
                    );
                }
            }
            Engine::Backtrack(re) => {
                let chars = self.chars_of(input);
                let mut pos = 0;
                while pos <= chars.len() {
                    let Some(m) = re.find_chars(&chars, pos)? else { break };
                    let (start, end) = m.span();
                    matches.push(
                        (0..m.len())
                            .map(|i| m.group(i).map(|(a, b)| chars_to_string(&chars, a, b)))
                            .collect(),
                    );
                    pos = if end == start { end + 1 } else { end };
                }
            }
        }
        Ok(matches)
    }

    /// Matches with UTF-8 byte offsets: every non-overlapping match when
    /// `global`, otherwise only the first. Each entry is `(start, end, groups)`
    /// where `groups[0]` is the whole match (`None` for a group that did not
    /// participate).
    #[allow(clippy::type_complexity)]
    pub fn match_spans(
        &self,
        input: &str,
        global: bool,
    ) -> Result<Vec<(usize, usize, Vec<Option<String>>)>, RegexError> {
        Ok(self
            .match_all_full(input, global)?
            .into_iter()
            .map(|m| (m.start_byte, m.end_byte, m.groups))
            .collect())
    }

    /// Like [`Self::match_spans`] with UTF-16 offsets (exact inside a surrogate pair).
    #[allow(clippy::type_complexity)]
    pub fn match_spans16(
        &self,
        input: &str,
        global: bool,
    ) -> Result<Vec<(usize, usize, Vec<Option<String>>)>, RegexError> {
        Ok(self
            .match_all_full(input, global)?
            .into_iter()
            .map(|m| (m.start16, m.end16, m.groups))
            .collect())
    }

    /// Replace the first match, or every match when the regex carries the `g`
    /// flag, expanding `$&`, `$1`..`$9` and `$$` in `replacement`.
    pub fn replace_template(&self, input: &str, replacement: &str) -> Result<String, RegexError> {
        let global = self.flags.contains('g');
        let matches = self
            .match_spans16(input, global)?
            .into_iter()
            .map(|(start, end, groups)| (start, end, groups))
            .collect::<Vec<_>>();
        Ok(self.expand_matches(input, &matches, replacement))
    }

    /// Substitute `replacement` (with `$` tokens expanded) for each located
    /// match `(start16, end16, groups)` in `input`.
    #[allow(clippy::type_complexity)]
    pub fn expand_matches(
        &self,
        input: &str,
        matches: &[(usize, usize, Vec<Option<String>>)],
        replacement: &str,
    ) -> String {
        let mut out = String::new();
        let mut last = 0;
        let names = self.group_names();
        let units: Vec<u16> = crate::wtf16::encode(input).collect();
        let needs_context = replacement.contains("$`") || replacement.contains("$'");
        let mut pieces: Vec<u16> = Vec::new();
        for (start, end, groups) in matches.iter().map(|(a, b, c)| (*a, *b, c)) {
            pieces.extend_from_slice(&units[last..start]);
            let refs: Vec<Option<&str>> = groups.iter().map(|g| g.as_deref()).collect();
            let (before, after) = if needs_context {
                (crate::wtf16::from_units(&units[..start]), crate::wtf16::from_units(&units[end..]))
            } else {
                (String::new(), String::new())
            };
            let mut piece = String::new();
            expand_substitution(replacement, &refs, &names, &before, &after, &mut piece);
            pieces.extend(crate::wtf16::encode(&piece));
            last = end;
        }
        pieces.extend_from_slice(&units[last..]);
        out.push_str(&crate::wtf16::from_units(&pieces));
        out
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Capture-group names by group number (index 0 is the whole match and is
    /// always `None`; an unnamed group is `None`). Scanned from the source so it
    /// is independent of the selected engine.
    pub fn group_names(&self) -> Vec<Option<String>> {
        let chars: Vec<char> = self.source.chars().collect();
        let mut names: Vec<Option<String>> = vec![None];
        let mut index = 0;
        let mut in_class = false;
        while index < chars.len() {
            match chars[index] {
                '\\' => index += 1,
                '[' if !in_class => in_class = true,
                ']' if in_class => in_class = false,
                '(' if !in_class => {
                    if chars.get(index + 1) == Some(&'?') {
                        if chars.get(index + 2) == Some(&'<')
                            && !matches!(chars.get(index + 3), Some('=') | Some('!'))
                        {
                            let mut name = String::new();
                            let mut at = index + 3;
                            while at < chars.len() && chars[at] != '>' {
                                name.push(chars[at]);
                                at += 1;
                            }
                            names.push(Some(name));
                        }
                    } else {
                        names.push(None);
                    }
                }
                _ => {}
            }
            index += 1;
        }
        names
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
                let chars = self.chars_of(input);
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
