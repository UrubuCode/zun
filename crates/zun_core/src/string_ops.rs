//! Shared `String.prototype` algorithm core over UTF-16 code units.
//!
//! JS string methods have two surfaces in the runtime: the statically-typed
//! `zun:string` native floor (reached when the receiver is known to be
//! `string`) and the dynamic reimplementation used for `any`-typed receivers.
//! Both must behave bit-for-bit alike. To keep a single source of truth, the
//! post-coercion algorithm of each method lives here, over `&[u16]` code units
//! (or `&str`), with no heap, no `Intl`/ICU, and no argument coercion — the two
//! surfaces each marshal their receiver and arguments, then call in here.
//!
//! Code units are obtained from [`crate::wtf16::encode`]; the plane-15
//! lone-surrogate stand-in is untouched and lives entirely in `wtf16`.

use crate::wtf16;

/// ECMAScript ToIntegerOrInfinity on an already-numeric value (`-0` folds to `+0`).
pub fn integer_or_infinity(number: f64) -> f64 {
    if number.is_nan() {
        0.0
    } else if number.is_infinite() {
        number
    } else {
        number.trunc() + 0.0
    }
}

/// Clamp an integer-or-infinity into `[0, len]`.
pub fn clamp_index(number: f64, len: usize) -> usize {
    if number <= 0.0 {
        0
    } else if number >= len as f64 {
        len
    } else {
        number as usize
    }
}

/// Resolve a relative index (negative counts from the end) into `[0, len]`.
pub fn relative_index(number: f64, len: usize) -> usize {
    if number < 0.0 {
        let resolved = len as f64 + number;
        if resolved <= 0.0 {
            0
        } else {
            resolved as usize
        }
    } else {
        clamp_index(number, len)
    }
}

/// First index at or after `from` where `needle` occurs in `haystack`.
pub fn find_from(haystack: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    let last = haystack.len() - needle.len();
    (from..=last).find(|&start| haystack[start..start + needle.len()] == *needle)
}

/// Last index at or before `from` where `needle` occurs in `haystack`.
///
/// `from` is clamped so the match can still fit; an empty needle matches at
/// `from.min(len)`.
pub fn last_index_from(haystack: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    let start = from.min(haystack.len() - needle.len());
    (0..=start)
        .rev()
        .find(|&at| haystack[at..at + needle.len()] == *needle)
}

/// `substring`: both ends already clamped-or-not into range here, then ordered.
pub fn substring_units(units: &[u16], start: usize, end: usize) -> &[u16] {
    let start = start.min(units.len());
    let end = end.min(units.len());
    &units[start.min(end)..start.max(end)]
}

/// `slice`: `begin`/`end` are the already-resolved relative indices; an empty
/// range yields an empty slice.
pub fn slice_units(units: &[u16], begin: usize, end: usize) -> &[u16] {
    if begin >= end {
        &[]
    } else {
        &units[begin..end]
    }
}

/// `at`: resolve a relative (possibly negative) index to an in-range unit
/// position, or `None` when out of range.
pub fn at_index(len: usize, relative: f64) -> Option<usize> {
    let resolved = if relative < 0.0 {
        len as f64 + relative
    } else {
        relative
    };
    if resolved >= 0.0 && resolved < len as f64 {
        Some(resolved as usize)
    } else {
        None
    }
}

/// `codePointAt`: the code point at an in-range unit index (surrogate pairs
/// combine), or `None` when the index is out of range.
pub fn code_point_at(units: &[u16], index: usize) -> Option<u32> {
    let first = u32::from(*units.get(index)?);
    if (0xD800..0xDC00).contains(&first) {
        if let Some(&next) = units.get(index + 1) {
            let second = u32::from(next);
            if (0xDC00..0xE000).contains(&second) {
                return Some(0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00));
            }
        }
    }
    Some(first)
}

/// Build the padding/padded units for `padStart`/`padEnd`. The caller has
/// already decided padding is needed (`target > units.len()` and `fill`
/// non-empty); `fill` is cycled to fill the gap.
pub fn pad_units(units: &[u16], target: usize, fill: &[u16], at_start: bool) -> Vec<u16> {
    let missing = target - units.len();
    let padding: Vec<u16> = fill.iter().cycle().take(missing).copied().collect();
    let mut out = Vec::with_capacity(target);
    if at_start {
        out.extend_from_slice(&padding);
        out.extend_from_slice(units);
    } else {
        out.extend_from_slice(units);
        out.extend_from_slice(&padding);
    }
    out
}

/// Maximum string length (in code units) `repeat` will produce before a
/// RangeError; matches V8's `String::kMaxLength`.
pub const REPEAT_MAX: u128 = 536_870_888;

/// `repeat`: `recv` repeated `count` times, then WTF-16-normalized. `None`
/// signals the length overflow the caller turns into a RangeError; an empty
/// receiver never overflows.
pub fn repeat(recv: &str, count: usize) -> Option<String> {
    if recv.is_empty() {
        return Some(String::new());
    }
    if (recv.len() as u128) * (count as u128) > REPEAT_MAX {
        return None;
    }
    Some(wtf16::normalize(recv.repeat(count)))
}

/// Split `text` on a string `separator` into UTF-16-faithful pieces. An empty
/// separator yields one piece per code unit; the caller applies any limit.
pub fn split_on(text: &str, separator: &str) -> Vec<String> {
    let units: Vec<u16> = wtf16::encode(text).collect();
    let sep: Vec<u16> = wtf16::encode(separator).collect();
    if sep.is_empty() {
        return units.iter().map(|unit| wtf16::from_units(&[*unit])).collect();
    }
    let mut parts = Vec::new();
    let mut from = 0;
    while let Some(at) = find_from(&units, &sep, from) {
        parts.push(wtf16::from_units(&units[from..at]));
        from = at + sep.len();
    }
    parts.push(wtf16::from_units(&units[from..]));
    parts
}
