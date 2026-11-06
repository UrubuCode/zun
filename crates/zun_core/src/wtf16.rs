//! UTF-16 code-unit view of dynamic strings that may hold lone surrogates.
//!
//! Dynamic strings are stored as Rust `String`s, which cannot hold lone
//! surrogates (U+D800..=U+DFFF). A lone surrogate code unit `u` is therefore
//! stored as the plane-15 private-use scalar `U+F0000 + (u - 0xD800)`. The
//! helpers here translate between that storage form and real UTF-16 units so
//! `length`, indexing, `charCodeAt`, and friends see one unit for a lone
//! surrogate, exactly as ECMAScript does.

use std::borrow::Cow;

const LONE_BASE: u32 = 0xF0000;
const LONE_END: u32 = 0xF07FF;

/// The surrogate code unit a stored scalar stands for, if it is a lone-surrogate stand-in.
pub fn lone_unit(c: char) -> Option<u16> {
    let v = c as u32;
    if (LONE_BASE..=LONE_END).contains(&v) {
        Some((v - LONE_BASE + 0xD800) as u16)
    } else {
        None
    }
}

/// UTF-16 code units of `text`, with lone-surrogate stand-ins expanded to one unit.
pub fn encode(text: &str) -> impl Iterator<Item = u16> + '_ {
    text.chars().flat_map(|c| {
        let mut buf = [0u16; 2];
        let (first, second) = match lone_unit(c) {
            Some(unit) => (unit, None),
            None => {
                let n = c.encode_utf16(&mut buf).len();
                (buf[0], if n == 2 { Some(buf[1]) } else { None })
            }
        };
        std::iter::once(first).chain(second)
    })
}

/// Number of UTF-16 code units in `text`.
pub fn len16(text: &str) -> usize {
    text.chars().map(|c| if lone_unit(c).is_some() { 1 } else { c.len_utf16() }).sum()
}

/// Build the storage form from UTF-16 units; unpaired surrogates become stand-ins.
pub fn from_units(units: &[u16]) -> String {
    let mut out = String::with_capacity(units.len());
    let mut i = 0;
    while i < units.len() {
        let u = units[i];
        if (0xD800..0xDC00).contains(&u) && i + 1 < units.len() && (0xDC00..0xE000).contains(&units[i + 1]) {
            let cp = 0x10000 + (((u as u32) - 0xD800) << 10) + ((units[i + 1] as u32) - 0xDC00);
            out.push(char::from_u32(cp).unwrap());
            i += 2;
        } else if (0xD800..0xE000).contains(&u) {
            out.push(char::from_u32(LONE_BASE + (u as u32 - 0xD800)).unwrap());
            i += 1;
        } else {
            out.push(char::from_u32(u as u32).unwrap());
            i += 1;
        }
    }
    out
}

/// Text safe to hand to a real UTF-8 sink: stand-ins become U+FFFD, as Node's stdout does.
pub fn to_utf8_lossy(text: &str) -> Cow<'_, str> {
    if text.chars().any(|c| lone_unit(c).is_some()) {
        Cow::Owned(text.chars().map(|c| if lone_unit(c).is_some() { '\u{FFFD}' } else { c }).collect())
    } else {
        Cow::Borrowed(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lone_surrogates_round_trip() {
        let s = from_units(&[0x61, 0xD800, 0x62, 0xDE00, 0xD83D, 0xDE00]);
        assert_eq!(len16(&s), 6);
        assert_eq!(encode(&s).collect::<Vec<_>>(), vec![0x61, 0xD800, 0x62, 0xDE00, 0xD83D, 0xDE00]);
        assert_eq!(to_utf8_lossy(&s), "a\u{FFFD}b\u{FFFD}\u{1F600}");
    }
}

/// Concatenate two strings, merging a trailing lone high surrogate on the left
/// with a leading lone low surrogate on the right into one astral scalar.
pub fn concat(left: &str, right: &str) -> String {
    let high = left.chars().next_back().and_then(lone_unit).filter(|u| (0xD800..0xDC00).contains(u));
    let low = right.chars().next().and_then(lone_unit).filter(|u| (0xDC00..0xE000).contains(u));
    if let (Some(h), Some(l)) = (high, low) {
        let head = &left[..left.len() - 4];
        let tail = &right[4..];
        let mut out = String::with_capacity(left.len() + right.len());
        out.push_str(head);
        out.push_str(&from_units(&[h, l]));
        out.push_str(tail);
        out
    } else {
        let mut out = String::with_capacity(left.len() + right.len());
        out.push_str(left);
        out.push_str(right);
        out
    }
}

/// ECMAScript WhiteSpace or LineTerminator (note: U+0085 is not, U+FEFF is).
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | '\u{20}' | '\u{a0}' | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// `String.prototype.trim` with the ECMAScript whitespace set.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_space)
}

/// `String.prototype.trimStart`.
pub fn js_trim_start(text: &str) -> &str {
    text.trim_start_matches(is_js_space)
}

/// `String.prototype.trimEnd`.
pub fn js_trim_end(text: &str) -> &str {
    text.trim_end_matches(is_js_space)
}

/// Re-pair adjacent lone-surrogate stand-ins (a high followed by a low) that
/// string building (join, repeat) may have placed side by side.
pub fn normalize(text: String) -> String {
    if !text.chars().any(|c| lone_unit(c).is_some()) {
        return text;
    }
    let units: Vec<u16> = encode(&text).collect();
    from_units(&units)
}
