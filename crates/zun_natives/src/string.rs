//! `zun:string` — machine-facing string primitives.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn length(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.first() {
        Some(NativeArg::Str(value)) => Ok(NativeValue::Int(zun_core::wtf16::len16(value) as i64)),
        _ => Err(NativeError("length: expected a string".to_string())),
    }
}

fn concat(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match (args.first(), args.get(1)) {
        (Some(NativeArg::Str(left)), Some(NativeArg::Str(right))) => {
            Ok(NativeValue::Str(format!("{left}{right}")))
        }
        _ => Err(NativeError("concat: expected two strings".to_string())),
    }
}

fn compare(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match (args.first(), args.get(1)) {
        (Some(NativeArg::Str(left)), Some(NativeArg::Str(right))) => {
            let result = if left < right {
                -1
            } else if left > right {
                1
            } else {
                0
            };
            Ok(NativeValue::Int(result))
        }
        _ => Err(NativeError("compare: expected two strings".to_string())),
    }
}

fn to_int(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("toInt: expected a string".to_string())),
    };
    value
        .trim()
        .parse::<i64>()
        .map(NativeValue::Int)
        .map_err(|_| NativeError("toInt: invalid integer string".to_string()))
}

fn to_float(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("toFloat: expected a string".to_string())),
    };
    value
        .trim()
        .parse::<f64>()
        .map(NativeValue::Float)
        .map_err(|_| NativeError("toFloat: invalid number string".to_string()))
}

fn to_boolean(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value.trim(),
        _ => return Err(NativeError("toBoolean: expected a string".to_string())),
    };
    match value {
        "true" => Ok(NativeValue::Bool(true)),
        "false" => Ok(NativeValue::Bool(false)),
        _ => Err(NativeError("toBoolean: expected true or false".to_string())),
    }
}

fn contains(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match (args.first(), args.get(1)) {
        (Some(NativeArg::Str(value)), Some(NativeArg::Str(needle))) => {
            Ok(NativeValue::Bool(value.contains(needle)))
        }
        _ => Err(NativeError("contains: expected two strings".to_string())),
    }
}

fn count(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("count: expected a string".to_string())),
    };
    let needle = match args.get(1) {
        Some(NativeArg::Str(needle)) => needle,
        _ => return Err(NativeError("count: expected a search string".to_string())),
    };
    let value_units: Vec<u16> = zun_core::wtf16::encode(value).collect();
    let needle_units: Vec<u16> = zun_core::wtf16::encode(needle).collect();
    if needle_units.is_empty() {
        return Ok(NativeValue::Int(0));
    }
    let mut total = 0i64;
    let mut start = 0usize;
    while start + needle_units.len() <= value_units.len() {
        if value_units[start..start + needle_units.len()] == needle_units {
            total += 1;
            start += needle_units.len();
        } else {
            start += 1;
        }
    }
    Ok(NativeValue::Int(total))
}

fn index_of(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("indexOf: expected a string".to_string())),
    };
    let needle = match args.get(1) {
        Some(NativeArg::Str(needle)) => needle,
        _ => return Err(NativeError("indexOf: expected a search string".to_string())),
    };
    let value_units: Vec<u16> = zun_core::wtf16::encode(value).collect();
    let needle_units: Vec<u16> = zun_core::wtf16::encode(needle).collect();
    let index = zun_core::string_ops::find_from(&value_units, &needle_units, 0)
        .map(|index| index as i64)
        .unwrap_or(-1);
    Ok(NativeValue::Int(index))
}

fn last_index_of(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("lastIndexOf: expected a string".to_string())),
    };
    let needle = match args.get(1) {
        Some(NativeArg::Str(needle)) => needle,
        _ => {
            return Err(NativeError(
                "lastIndexOf: expected a search string".to_string(),
            ))
        }
    };
    let value_units: Vec<u16> = zun_core::wtf16::encode(value).collect();
    let needle_units: Vec<u16> = zun_core::wtf16::encode(needle).collect();
    let index =
        zun_core::string_ops::last_index_from(&value_units, &needle_units, value_units.len())
            .map(|index| index as i64)
            .unwrap_or(-1);
    Ok(NativeValue::Int(index))
}

fn replace(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match (args.first(), args.get(1), args.get(2)) {
        (
            Some(NativeArg::Str(value)),
            Some(NativeArg::Str(needle)),
            Some(NativeArg::Str(replacement)),
        ) => {
            let Some(start) = value.find(needle) else {
                return Ok(NativeValue::Str(value.to_string()));
            };
            let end = start + needle.len();
            let mut out = String::from(&value[..start]);
            zun_core::expand_substitution(
                replacement,
                &[Some(needle)],
                &[None],
                &value[..start],
                &value[end..],
                &mut out,
            );
            out.push_str(&value[end..]);
            Ok(NativeValue::Str(out))
        }
        _ => Err(NativeError(
            "replace: expected a string, search string, and replacement".to_string(),
        )),
    }
}

fn substring(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("substring: expected a string".to_string())),
    };
    let start = match args.get(1) {
        Some(NativeArg::Int(index)) if *index >= 0 => *index as usize,
        _ => {
            return Err(NativeError(
                "substring: expected a non-negative start".to_string(),
            ))
        }
    };
    let end = match args.get(2) {
        Some(NativeArg::Int(index)) if *index >= 0 => *index as usize,
        _ => {
            return Err(NativeError(
                "substring: expected a non-negative end".to_string(),
            ))
        }
    };
    let units: Vec<u16> = zun_core::wtf16::encode(value).collect();
    Ok(NativeValue::Str(zun_core::wtf16::from_units(
        zun_core::string_ops::substring_units(&units, start, end),
    )))
}

#[allow(clippy::redundant_guards)] // guard reads clearer than an empty-string pattern here
fn pad_start(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("padStart: expected a string".to_string())),
    };
    let target = match args.get(1) {
        Some(NativeArg::Int(target)) if *target >= 0 => *target as usize,
        _ => {
            return Err(NativeError(
                "padStart: expected a non-negative target length".to_string(),
            ))
        }
    };
    let fill = match args.get(2) {
        // An empty fill leaves the string unchanged.
        Some(NativeArg::Str(fill)) if fill.is_empty() => {
            return Ok(NativeValue::Str(value.to_string()))
        }
        Some(NativeArg::Str(fill)) => fill,
        _ => return Err(NativeError("padStart: expected a fill string".to_string())),
    };
    let units: Vec<u16> = zun_core::wtf16::encode(value).collect();
    if target <= units.len() {
        return Ok(NativeValue::Str(value.to_string()));
    }
    let fill: Vec<u16> = zun_core::wtf16::encode(fill).collect();
    Ok(NativeValue::Str(zun_core::wtf16::from_units(
        &zun_core::string_ops::pad_units(&units, target, &fill, true),
    )))
}

#[allow(clippy::redundant_guards)] // guard reads clearer than an empty-string pattern here
fn pad_end(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("padEnd: expected a string".to_string())),
    };
    let target = match args.get(1) {
        Some(NativeArg::Int(target)) if *target >= 0 => *target as usize,
        _ => {
            return Err(NativeError(
                "padEnd: expected a non-negative target length".to_string(),
            ))
        }
    };
    let fill = match args.get(2) {
        // An empty fill leaves the string unchanged.
        Some(NativeArg::Str(fill)) if fill.is_empty() => {
            return Ok(NativeValue::Str(value.to_string()))
        }
        Some(NativeArg::Str(fill)) => fill,
        _ => return Err(NativeError("padEnd: expected a fill string".to_string())),
    };
    let units: Vec<u16> = zun_core::wtf16::encode(value).collect();
    if target <= units.len() {
        return Ok(NativeValue::Str(value.to_string()));
    }
    let fill: Vec<u16> = zun_core::wtf16::encode(fill).collect();
    Ok(NativeValue::Str(zun_core::wtf16::from_units(
        &zun_core::string_ops::pad_units(&units, target, &fill, false),
    )))
}

fn to_lower_case(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.first() {
        Some(NativeArg::Str(value)) => Ok(NativeValue::Str(value.to_lowercase())),
        _ => Err(NativeError("toLowerCase: expected a string".to_string())),
    }
}

fn to_upper_case(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.first() {
        Some(NativeArg::Str(value)) => Ok(NativeValue::Str(value.to_uppercase())),
        _ => Err(NativeError("toUpperCase: expected a string".to_string())),
    }
}

fn trim_start(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.first() {
        Some(NativeArg::Str(value)) => Ok(NativeValue::Str(
            zun_core::wtf16::js_trim_start(value).to_string(),
        )),
        _ => Err(NativeError("trimStart: expected a string".to_string())),
    }
}

fn trim_end(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.first() {
        Some(NativeArg::Str(value)) => Ok(NativeValue::Str(
            zun_core::wtf16::js_trim_end(value).to_string(),
        )),
        _ => Err(NativeError("trimEnd: expected a string".to_string())),
    }
}

fn char_code_at(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("charCodeAt: expected a string".to_string())),
    };
    let index = match args.get(1) {
        Some(NativeArg::Int(index)) if *index >= 0 => *index as usize,
        _ => {
            return Err(NativeError(
                "charCodeAt: expected a non-negative index".to_string(),
            ))
        }
    };
    zun_core::wtf16::encode(value)
        .nth(index)
        .map(|unit| NativeValue::Int(unit as i64))
        .ok_or_else(|| NativeError(format!("charCodeAt: index {index} is out of bounds")))
}

/// `String.prototype.charCodeAt` with a JS `number` index: ToIntegerOrInfinity
/// is applied, and an out-of-range index (negative, past the end, ±Infinity)
/// yields `NaN` — never an error. Always returns a `number`, so the typed floor
/// needs no boxing and no out-of-bounds error (unlike the integer-index native).
fn char_code_at_num(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("charCodeAt: expected a string".to_string())),
    };
    let index = numeric_arg(args.get(1))?;
    match unit_at(value, index) {
        Some(unit) => Ok(NativeValue::Float(f64::from(unit))),
        None => Ok(NativeValue::Float(f64::NAN)),
    }
}

/// `String.prototype.charAt` with a JS `number` index: ToIntegerOrInfinity is
/// applied, and an out-of-range index yields the empty string `""` (never an
/// error, never `undefined`). Always returns a `string`.
fn char_at_num(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("charAt: expected a string".to_string())),
    };
    let index = numeric_arg(args.get(1))?;
    match unit_at(value, index) {
        Some(unit) => Ok(NativeValue::Str(zun_core::wtf16::from_units(&[unit]))),
        None => Ok(NativeValue::Str(String::new())),
    }
}

/// `str[i]` — bracket indexing. Unlike `charAt`/`at`, this uses the ECMAScript
/// canonical array-index rule: the key resolves to a character only when `i` is
/// a non-negative integer strictly inside the string; a fractional, negative, or
/// out-of-range index is simply a missing property and yields `undefined`.
fn char_index(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("string index: expected a string".to_string())),
    };
    let index = numeric_arg(args.get(1))?;
    // Canonical index: a non-negative integer in [0, len). Fractional or
    // negative indices are non-canonical keys -> `undefined`. No `fract()`
    // rounding (unlike `charAt`): `s[1.5]` is a missing property.
    if index.is_finite() && index >= 0.0 && index.fract() == 0.0 {
        match zun_core::wtf16::encode(value).nth(index as usize) {
            Some(unit) => Ok(NativeValue::Str(zun_core::wtf16::from_units(&[unit]))),
            None => Ok(NativeValue::Void),
        }
    } else {
        Ok(NativeValue::Void)
    }
}

/// Read a numeric native argument (an `int` or a `float`) as an `f64`.
fn numeric_arg(arg: Option<&NativeArg>) -> Result<f64, NativeError> {
    match arg {
        Some(NativeArg::Float(index)) => Ok(*index),
        Some(NativeArg::Int(index)) => Ok(*index as f64),
        _ => Err(NativeError("expected a numeric index".to_string())),
    }
}

/// The UTF-16 code unit of `value` at a JS `number` index, under
/// `charCodeAt`/`charAt` semantics: ToIntegerOrInfinity is applied (`NaN` folds
/// to `0`, `±Infinity` stays out of range), and the unit is read lazily with
/// `nth` — no per-call allocation of the whole string. `None` when the index is
/// negative, non-finite, or past the end (the caller maps that to `NaN` / `""`).
fn unit_at(value: &str, index: f64) -> Option<u16> {
    let i = zun_core::string_ops::integer_or_infinity(index);
    if i.is_finite() && i >= 0.0 {
        zun_core::wtf16::encode(value).nth(i as usize)
    } else {
        None
    }
}

fn starts_with(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match (args.first(), args.get(1)) {
        (Some(NativeArg::Str(value)), Some(NativeArg::Str(prefix))) => {
            Ok(NativeValue::Bool(value.starts_with(prefix)))
        }
        _ => Err(NativeError("startsWith: expected two strings".to_string())),
    }
}

fn ends_with(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match (args.first(), args.get(1)) {
        (Some(NativeArg::Str(value)), Some(NativeArg::Str(suffix))) => {
            Ok(NativeValue::Bool(value.ends_with(suffix)))
        }
        _ => Err(NativeError("endsWith: expected two strings".to_string())),
    }
}

fn trim(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.first() {
        Some(NativeArg::Str(value)) => Ok(NativeValue::Str(
            zun_core::wtf16::js_trim(value).to_string(),
        )),
        _ => Err(NativeError("trim: expected a string".to_string())),
    }
}

fn repeat(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("repeat: expected a string".to_string())),
    };
    let count = match args.get(1) {
        Some(NativeArg::Int(count)) if *count >= 0 => *count as usize,
        _ => {
            return Err(NativeError(
                "repeat: expected a non-negative count".to_string(),
            ))
        }
    };
    zun_core::string_ops::repeat(value, count)
        .map(NativeValue::Str)
        .ok_or_else(|| NativeError("RangeError: Invalid string length".to_string()))
}

/// `key in new String(s)`: a code-unit index below the length, or `length`.
fn string_has_property(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match (args.first(), args.get(1)) {
        (Some(NativeArg::Str(value)), Some(NativeArg::Str(key))) => {
            if *key == "length" {
                return Ok(NativeValue::Bool(true));
            }
            let canonical = key
                .parse::<u32>()
                .ok()
                .filter(|index| index.to_string() == **key);
            Ok(NativeValue::Bool(canonical.is_some_and(|index| {
                (index as usize) < zun_core::wtf16::len16(value)
            })))
        }
        _ => Err(NativeError(
            "stringHasProperty: expected strings".to_string(),
        )),
    }
}

fn is_well_formed(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.first() {
        Some(NativeArg::Str(value)) => Ok(NativeValue::Bool(
            !value
                .chars()
                .any(|c| zun_core::wtf16::lone_unit(c).is_some()),
        )),
        _ => Err(NativeError("isWellFormed: expected a string".to_string())),
    }
}

fn to_well_formed(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.first() {
        Some(NativeArg::Str(value)) => Ok(NativeValue::Str(
            value
                .chars()
                .map(|c| {
                    if zun_core::wtf16::lone_unit(c).is_some() {
                        '�'
                    } else {
                        c
                    }
                })
                .collect(),
        )),
        _ => Err(NativeError("toWellFormed: expected a string".to_string())),
    }
}

fn reverse(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("reverse: expected a string".to_string())),
    };
    let mut units: Vec<u16> = zun_core::wtf16::encode(value).collect();
    units.reverse();
    Ok(NativeValue::Str(zun_core::wtf16::from_units(&units)))
}

fn at(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("at: expected a string".to_string())),
    };
    // The index arrives as a JS `number` (f64) so a negative value — counted
    // from the end — survives the boundary. Indices are UTF-16 code units.
    let relative = match args.get(1) {
        Some(NativeArg::Float(index)) => *index,
        Some(NativeArg::Int(index)) => *index as f64,
        _ => return Err(NativeError("at: expected a numeric index".to_string())),
    };
    let units: Vec<u16> = zun_core::wtf16::encode(value).collect();
    match zun_core::string_ops::at_index(units.len(), relative) {
        Some(index) => Ok(NativeValue::Str(zun_core::wtf16::from_units(
            &units[index..index + 1],
        ))),
        // Out of range yields `undefined`, per spec.
        None => Ok(NativeValue::Void),
    }
}

fn code_point_at(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("codePointAt: expected a string".to_string())),
    };
    let index = match args.get(1) {
        Some(NativeArg::Int(index)) if *index >= 0 => *index as usize,
        _ => {
            return Err(NativeError(
                "codePointAt: expected a non-negative index".to_string(),
            ))
        }
    };
    let units: Vec<u16> = zun_core::wtf16::encode(value).collect();
    // A leading high surrogate followed by a low surrogate combines into an
    // astral code point; otherwise the bare code unit is the code point. An
    // out-of-range index is an error on this typed floor (the dynamic surface
    // returns `undefined` instead).
    match zun_core::string_ops::code_point_at(&units, index) {
        Some(code) => Ok(NativeValue::Int(i64::from(code))),
        None => Err(NativeError(format!(
            "codePointAt: index {index} is out of bounds"
        ))),
    }
}

fn slice(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("slice: expected a string".to_string())),
    };
    let read = |arg: Option<&NativeArg>| -> Option<f64> {
        match arg {
            Some(NativeArg::Float(index)) => Some(*index),
            Some(NativeArg::Int(index)) => Some(*index as f64),
            _ => None,
        }
    };
    let Some(start) = read(args.get(1)) else {
        return Err(NativeError("slice: expected a numeric start".to_string()));
    };
    let Some(end) = read(args.get(2)) else {
        return Err(NativeError("slice: expected a numeric end".to_string()));
    };
    let units: Vec<u16> = zun_core::wtf16::encode(value).collect();
    // Negative indices count from the end; both ends clamp into range. An
    // absent `end` is passed as a value past any string length, so it clamps
    // to `len` here.
    let begin = zun_core::string_ops::relative_index(start, units.len());
    let finish = zun_core::string_ops::relative_index(end, units.len());
    Ok(NativeValue::Str(zun_core::wtf16::from_units(
        zun_core::string_ops::slice_units(&units, begin, finish),
    )))
}

fn replace_all(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match (args.first(), args.get(1), args.get(2)) {
        (
            Some(NativeArg::Str(value)),
            Some(NativeArg::Str(search)),
            Some(NativeArg::Str(replacement)),
        ) => {
            // An empty search inserts the replacement between every UTF-16 code
            // unit (and at both ends), tearing astral pairs.
            if search.is_empty() {
                let units: Vec<u16> = zun_core::wtf16::encode(value).collect();
                let needs_context = replacement.contains("$`") || replacement.contains("$'");
                let mut out_units: Vec<u16> = Vec::with_capacity(units.len() * 2 + 1);
                for at in 0..=units.len() {
                    let (before, after) = if needs_context {
                        (
                            zun_core::wtf16::from_units(&units[..at]),
                            zun_core::wtf16::from_units(&units[at..]),
                        )
                    } else {
                        (String::new(), String::new())
                    };
                    let mut piece = String::new();
                    zun_core::expand_substitution(
                        replacement,
                        &[Some(search)],
                        &[None],
                        &before,
                        &after,
                        &mut piece,
                    );
                    out_units.extend(zun_core::wtf16::encode(&piece));
                    if let Some(&unit) = units.get(at) {
                        out_units.push(unit);
                    }
                }
                return Ok(NativeValue::Str(zun_core::wtf16::from_units(&out_units)));
            }
            let mut out = String::new();
            let mut last = 0;
            let mut from = 0;
            let mut boundaries: Vec<usize> = Vec::new();
            if search.is_empty() {
                boundaries = value.char_indices().map(|(index, _)| index).collect();
                boundaries.push(value.len());
            }
            let mut next_empty = 0;
            loop {
                let start = if search.is_empty() {
                    let Some(&at) = boundaries.get(next_empty) else {
                        break;
                    };
                    next_empty += 1;
                    at
                } else {
                    let Some(at) = value[from..].find(*search) else {
                        break;
                    };
                    from + at
                };
                let end = start + search.len();
                out.push_str(&value[last..start]);
                zun_core::expand_substitution(
                    replacement,
                    &[Some(search)],
                    &[None],
                    &value[..start],
                    &value[end..],
                    &mut out,
                );
                last = end;
                from = end;
            }
            out.push_str(&value[last..]);
            Ok(NativeValue::Str(out))
        }
        _ => Err(NativeError(
            "replaceAll: expected a string, search string, and replacement".to_string(),
        )),
    }
}

fn from_char_code1(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let code = match args.first() {
        Some(NativeArg::Int(code)) => *code as u16,
        Some(NativeArg::Float(code)) => *code as u16,
        _ => {
            return Err(NativeError(
                "fromCharCode: expected a numeric code".to_string(),
            ))
        }
    };
    // Each code is truncated to a UTF-16 code unit, per spec.
    Ok(NativeValue::Str(zun_core::wtf16::from_units(&[code])))
}

fn from_code_point1(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let code = match args.first() {
        Some(NativeArg::Int(code)) => *code,
        Some(NativeArg::Float(code)) if code.fract() == 0.0 => *code as i64,
        _ => return Err(NativeError("RangeError: Invalid code point".to_string())),
    };
    if !(0..=0x10FFFF).contains(&code) {
        return Err(NativeError(format!(
            "RangeError: Invalid code point {code}"
        )));
    }
    let code = code as u32;
    if code <= 0xFFFF {
        // Includes lone surrogates, which `from_utf16_lossy` renders faithfully.
        Ok(NativeValue::Str(zun_core::wtf16::from_units(
            &[code as u16],
        )))
    } else {
        match char::from_u32(code) {
            Some(scalar) => Ok(NativeValue::Str(scalar.to_string())),
            None => Err(NativeError(format!(
                "RangeError: Invalid code point {code}"
            ))),
        }
    }
}

/// The machine-facing string primitives used by `zun:core`.
/// `String.prototype.normalize(form?)` — Unicode Normalization Forms via the
/// baked ICU4X normalizer. The lowering supplies `"NFC"` when the form is
/// omitted; an unrecognized form is a RangeError-equivalent (spec).
/// Normalize `value` to `form` (`NFC`/`NFD`/`NFKC`/`NFKD`); `None` for an unknown form.
pub fn normalize_form(value: &str, form: &str) -> Option<String> {
    use icu::normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
    Some(match form {
        "NFC" => ComposingNormalizerBorrowed::new_nfc()
            .normalize(value)
            .into_owned(),
        "NFD" => DecomposingNormalizerBorrowed::new_nfd()
            .normalize(value)
            .into_owned(),
        "NFKC" => ComposingNormalizerBorrowed::new_nfkc()
            .normalize(value)
            .into_owned(),
        "NFKD" => DecomposingNormalizerBorrowed::new_nfkd()
            .normalize(value)
            .into_owned(),
        _ => return None,
    })
}

fn normalize(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    use icu::normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
    let (Some(NativeArg::Str(value)), Some(NativeArg::Str(form))) = (args.first(), args.get(1))
    else {
        return Err(NativeError(
            "normalize: expected (string, form)".to_string(),
        ));
    };
    let normalized = match *form {
        "NFC" => ComposingNormalizerBorrowed::new_nfc().normalize(value),
        "NFD" => DecomposingNormalizerBorrowed::new_nfd().normalize(value),
        "NFKC" => ComposingNormalizerBorrowed::new_nfkc().normalize(value),
        "NFKD" => DecomposingNormalizerBorrowed::new_nfkd().normalize(value),
        other => {
            return Err(NativeError(format!(
                "RangeError: The normalization form should be one of NFC, NFD, NFKC, NFKD (found `{other}`)"
            )))
        }
    };
    Ok(NativeValue::Str(normalized.into_owned()))
}

/// `String.prototype.localeCompare(that, locale?)` — locale-aware comparison via
/// the baked ICU4X collator. The trailing locale is a BCP-47 string; `""`
/// selects the root collation. Returns the ECMAScript -1 / 0 / 1.
fn locale_compare(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    use core::cmp::Ordering;
    use icu::collator::{options::CollatorOptions, Collator, CollatorPreferences};
    let (Some(NativeArg::Str(value)), Some(NativeArg::Str(that)), Some(NativeArg::Str(locale))) =
        (args.first(), args.get(1), args.get(2))
    else {
        return Err(NativeError(
            "localeCompare: expected (string, string, string)".to_string(),
        ));
    };
    let locale = intl_locale(locale);
    let collator = Collator::try_new(
        CollatorPreferences::from(&locale),
        CollatorOptions::default(),
    )
    .map_err(|e| NativeError(format!("localeCompare: collator init failed: {e:?}")))?;
    let ordering = match collator.compare(value, that) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    };
    Ok(NativeValue::Int(ordering))
}

/// `String.prototype.toLocaleLowerCase(locale?)` — locale-aware lowercasing via
/// the baked ICU4X case mapper; `""` is the root (`und`) locale.
fn to_locale_lower_case(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    use icu::casemap::CaseMapper;
    match (args.first(), args.get(1)) {
        (Some(NativeArg::Str(value)), Some(NativeArg::Str(locale))) => Ok(NativeValue::Str(
            CaseMapper::new()
                .lowercase_to_string(value, &intl_locale(locale).id)
                .into_owned(),
        )),
        _ => Err(NativeError(
            "toLocaleLowerCase: expected (string, string)".to_string(),
        )),
    }
}

/// `String.prototype.toLocaleUpperCase(locale?)` — locale-aware uppercasing via
/// the baked ICU4X case mapper; `""` is the root (`und`) locale.
fn to_locale_upper_case(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    use icu::casemap::CaseMapper;
    match (args.first(), args.get(1)) {
        (Some(NativeArg::Str(value)), Some(NativeArg::Str(locale))) => Ok(NativeValue::Str(
            CaseMapper::new()
                .uppercase_to_string(value, &intl_locale(locale).id)
                .into_owned(),
        )),
        _ => Err(NativeError(
            "toLocaleUpperCase: expected (string, string)".to_string(),
        )),
    }
}

/// Parse a BCP-47 locale string for `Intl`; empty or malformed input falls
/// back to the root (`und`) locale.
pub(crate) fn intl_locale(text: &str) -> icu::locale::Locale {
    icu::locale::Locale::try_from_str(text).unwrap_or(icu::locale::Locale::UNKNOWN)
}

/// `new Intl.Collator(locale).compare(a, b)` — runtime-locale ICU4X collation.
fn intl_collator_compare(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    use core::cmp::Ordering;
    use icu::collator::{options::CollatorOptions, Collator, CollatorPreferences};
    let (Some(NativeArg::Str(locale)), Some(NativeArg::Str(a)), Some(NativeArg::Str(b))) =
        (args.first(), args.get(1), args.get(2))
    else {
        return Err(NativeError(
            "Intl.Collator.compare: expected (string, string, string)".to_string(),
        ));
    };
    let locale = intl_locale(locale);
    let collator = Collator::try_new(
        CollatorPreferences::from(&locale),
        CollatorOptions::default(),
    )
    .map_err(|e| NativeError(format!("Intl.Collator: collator init failed: {e:?}")))?;
    Ok(NativeValue::Int(match collator.compare(a, b) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }))
}

pub const MODULE: crate::Module = crate::Module {
    name: "string",
    functions: &[
        NativeFn {
            name: "length",
            params: &[Str],
            ret: U64,
            imp: length,
        },
        NativeFn {
            name: "concat",
            params: &[Str, Str],
            ret: Str,
            imp: concat,
        },
        NativeFn {
            name: "compare",
            params: &[Str, Str],
            ret: I32,
            imp: compare,
        },
        NativeFn {
            name: "toInt",
            params: &[Str],
            ret: I32,
            imp: to_int,
        },
        NativeFn {
            name: "toFloat",
            params: &[Str],
            ret: F64,
            imp: to_float,
        },
        NativeFn {
            name: "toBoolean",
            params: &[Str],
            ret: Bool,
            imp: to_boolean,
        },
        NativeFn {
            name: "contains",
            params: &[Str, Str],
            ret: Bool,
            imp: contains,
        },
        NativeFn {
            name: "count",
            params: &[Str, Str],
            ret: I32,
            imp: count,
        },
        NativeFn {
            name: "includes",
            params: &[Str, Str],
            ret: Bool,
            imp: contains,
        },
        NativeFn {
            name: "indexOf",
            params: &[Str, Str],
            ret: I32,
            imp: index_of,
        },
        NativeFn {
            name: "lastIndexOf",
            params: &[Str, Str],
            ret: I32,
            imp: last_index_of,
        },
        NativeFn {
            name: "replace",
            params: &[Str, Str, Str],
            ret: Str,
            imp: replace,
        },
        NativeFn {
            name: "substring",
            params: &[Str, U64, U64],
            ret: Str,
            imp: substring,
        },
        NativeFn {
            name: "padStart",
            params: &[Str, U64, Str],
            ret: Str,
            imp: pad_start,
        },
        NativeFn {
            name: "padEnd",
            params: &[Str, U64, Str],
            ret: Str,
            imp: pad_end,
        },
        NativeFn {
            name: "toLowerCase",
            params: &[Str],
            ret: Str,
            imp: to_lower_case,
        },
        NativeFn {
            name: "encodeURIComponent",
            params: &[Str],
            ret: Str,
            imp: encode_uri_component,
        },
        NativeFn {
            name: "decodeURIComponent",
            params: &[Str],
            ret: Str,
            imp: decode_uri_component,
        },
        NativeFn {
            name: "toUpperCase",
            params: &[Str],
            ret: Str,
            imp: to_upper_case,
        },
        NativeFn {
            name: "trimStart",
            params: &[Str],
            ret: Str,
            imp: trim_start,
        },
        NativeFn {
            name: "trimEnd",
            params: &[Str],
            ret: Str,
            imp: trim_end,
        },
        NativeFn {
            name: "charCodeAt",
            params: &[Str, U64],
            ret: U32,
            imp: char_code_at,
        },
        // JS-`number`-index char access (ToIntegerOrInfinity, out-of-range ->
        // NaN / "" / undefined). These keep a typed result so a statically typed
        // string receiver leaves the dynamic `stringMethod` path.
        NativeFn {
            name: "charCodeAtNum",
            params: &[Str, F64],
            ret: F64,
            imp: char_code_at_num,
        },
        NativeFn {
            name: "charAtNum",
            params: &[Str, F64],
            ret: Str,
            imp: char_at_num,
        },
        NativeFn {
            name: "charIndex",
            params: &[Str, F64],
            ret: Str,
            imp: char_index,
        },
        NativeFn {
            name: "startsWith",
            params: &[Str, Str],
            ret: Bool,
            imp: starts_with,
        },
        NativeFn {
            name: "endsWith",
            params: &[Str, Str],
            ret: Bool,
            imp: ends_with,
        },
        NativeFn {
            name: "trim",
            params: &[Str],
            ret: Str,
            imp: trim,
        },
        NativeFn {
            name: "repeat",
            params: &[Str, U64],
            ret: Str,
            imp: repeat,
        },
        NativeFn {
            name: "stringHasProperty",
            params: &[Str, Str],
            ret: Bool,
            imp: string_has_property,
        },
        NativeFn {
            name: "isWellFormed",
            params: &[Str],
            ret: Bool,
            imp: is_well_formed,
        },
        NativeFn {
            name: "toWellFormed",
            params: &[Str],
            ret: Str,
            imp: to_well_formed,
        },
        NativeFn {
            name: "reverse",
            params: &[Str],
            ret: Str,
            imp: reverse,
        },
        // Indices are UTF-16 code units, matching `charCodeAt`/`substring`.
        NativeFn {
            name: "at",
            params: &[Str, F64],
            ret: Str,
            imp: at,
        },
        NativeFn {
            name: "codePointAt",
            params: &[Str, U64],
            ret: U32,
            imp: code_point_at,
        },
        NativeFn {
            name: "slice",
            params: &[Str, F64, F64],
            ret: Str,
            imp: slice,
        },
        NativeFn {
            name: "replaceAll",
            params: &[Str, Str, Str],
            ret: Str,
            imp: replace_all,
        },
        // Single-code helpers folded by the compiler for the variadic
        // `String.fromCharCode(...)` / `String.fromCodePoint(...)` statics.
        NativeFn {
            name: "fromCharCode1",
            params: &[U32],
            ret: Str,
            imp: from_char_code1,
        },
        NativeFn {
            name: "fromCodePoint1",
            params: &[U32],
            ret: Str,
            imp: from_code_point1,
        },
        // ECMA-402 / Unicode surface (NUCLEUS: String #1). Appended LAST so every
        // earlier dispatch id stays stable. Pure value→value natives (like
        // `toUpperCase`), backed by the baked ICU4X data.
        NativeFn {
            name: "stringNormalize",
            params: &[Str, Str],
            ret: Str,
            imp: normalize,
        },
        NativeFn {
            name: "localeCompare",
            params: &[Str, Str, Str],
            ret: I32,
            imp: locale_compare,
        },
        NativeFn {
            name: "toLocaleLowerCase",
            params: &[Str, Str],
            ret: Str,
            imp: to_locale_lower_case,
        },
        NativeFn {
            name: "toLocaleUpperCase",
            params: &[Str, Str],
            ret: Str,
            imp: to_locale_upper_case,
        },
        NativeFn {
            name: "intlCollatorCompare",
            params: &[Str, Str, Str],
            ret: I32,
            imp: intl_collator_compare,
        },
    ],
};

fn encode_uri_component(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let Some(NativeArg::Str(value)) = args.first() else {
        return Err(NativeError(
            "encodeURIComponent: expected a string".to_string(),
        ));
    };
    let mut out = String::new();
    for byte in value.as_bytes() {
        let b = *byte;
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    Ok(NativeValue::Str(out))
}

fn decode_uri_component(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let Some(NativeArg::Str(value)) = args.first() else {
        return Err(NativeError(
            "decodeURIComponent: expected a string".to_string(),
        ));
    };
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok());
            match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(b) => {
                    out.push(b);
                    i += 3;
                }
                None => return Err(NativeError("URIError: URI malformed".to_string())),
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    match String::from_utf8(out) {
        Ok(text) => Ok(NativeValue::Str(text)),
        Err(_) => Err(NativeError("URIError: URI malformed".to_string())),
    }
}
