//! `zun:string` — machine-facing string primitives.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn length(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.first() {
        Some(NativeArg::Str(value)) => Ok(NativeValue::Int(value.encode_utf16().count() as i64)),
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
    let value_units: Vec<u16> = value.encode_utf16().collect();
    let needle_units: Vec<u16> = needle.encode_utf16().collect();
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
    let value_units: Vec<u16> = value.encode_utf16().collect();
    let needle_units: Vec<u16> = needle.encode_utf16().collect();
    let index = if needle_units.is_empty() {
        0
    } else {
        value_units
            .windows(needle_units.len())
            .position(|window| window == needle_units.as_slice())
            .map(|index| index as i64)
            .unwrap_or(-1)
    };
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
    let value_units: Vec<u16> = value.encode_utf16().collect();
    let needle_units: Vec<u16> = needle.encode_utf16().collect();
    let index = if needle_units.is_empty() {
        value_units.len() as i64
    } else {
        value_units
            .windows(needle_units.len())
            .rposition(|window| window == needle_units.as_slice())
            .map(|index| index as i64)
            .unwrap_or(-1)
    };
    Ok(NativeValue::Int(index))
}

fn replace(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match (args.first(), args.get(1), args.get(2)) {
        (
            Some(NativeArg::Str(value)),
            Some(NativeArg::Str(needle)),
            Some(NativeArg::Str(replacement)),
        ) => Ok(NativeValue::Str(value.replacen(needle, replacement, 1))),
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
    let units: Vec<u16> = value.encode_utf16().collect();
    let start = start.min(units.len());
    let end = end.min(units.len());
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    Ok(NativeValue::Str(String::from_utf16_lossy(
        &units[start..end],
    )))
}

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
        Some(NativeArg::Str(fill)) if !fill.is_empty() => fill,
        _ => {
            return Err(NativeError(
                "padStart: expected a non-empty fill string".to_string(),
            ))
        }
    };
    let length = value.encode_utf16().count();
    if target <= length {
        return Ok(NativeValue::Str(value.to_string()));
    }
    let padding_units = target - length;
    let mut padding = String::new();
    while padding.encode_utf16().count() < padding_units {
        padding.push_str(fill);
    }
    let padding = padding
        .encode_utf16()
        .take(padding_units)
        .collect::<Vec<_>>();
    let mut result = String::from_utf16_lossy(&padding);
    result.push_str(value);
    Ok(NativeValue::Str(result))
}

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
        Some(NativeArg::Str(fill)) if !fill.is_empty() => fill,
        _ => {
            return Err(NativeError(
                "padEnd: expected a non-empty fill string".to_string(),
            ))
        }
    };
    let length = value.encode_utf16().count();
    if target <= length {
        return Ok(NativeValue::Str(value.to_string()));
    }
    let padding_units = target - length;
    let mut padding = String::new();
    while padding.encode_utf16().count() < padding_units {
        padding.push_str(fill);
    }
    let padding = padding
        .encode_utf16()
        .take(padding_units)
        .collect::<Vec<_>>();
    let mut result = value.to_string();
    result.push_str(&String::from_utf16_lossy(&padding));
    Ok(NativeValue::Str(result))
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
        Some(NativeArg::Str(value)) => Ok(NativeValue::Str(value.trim_start().to_string())),
        _ => Err(NativeError("trimStart: expected a string".to_string())),
    }
}

fn trim_end(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.first() {
        Some(NativeArg::Str(value)) => Ok(NativeValue::Str(value.trim_end().to_string())),
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
    value
        .encode_utf16()
        .nth(index)
        .map(|unit| NativeValue::Int(unit as i64))
        .ok_or_else(|| NativeError(format!("charCodeAt: index {index} is out of bounds")))
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
        Some(NativeArg::Str(value)) => Ok(NativeValue::Str(value.trim().to_string())),
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
    Ok(NativeValue::Str(value.repeat(count)))
}

fn reverse(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("reverse: expected a string".to_string())),
    };
    let mut units: Vec<u16> = value.encode_utf16().collect();
    units.reverse();
    Ok(NativeValue::Str(String::from_utf16_lossy(&units)))
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
    let units: Vec<u16> = value.encode_utf16().collect();
    let len = units.len() as f64;
    let resolved = if relative < 0.0 { len + relative } else { relative };
    if resolved < 0.0 || resolved >= len {
        // Out of range yields `undefined`, per spec.
        return Ok(NativeValue::Void);
    }
    let index = resolved as usize;
    Ok(NativeValue::Str(String::from_utf16_lossy(&units[index..index + 1])))
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
    let units: Vec<u16> = value.encode_utf16().collect();
    let Some(&first) = units.get(index) else {
        return Err(NativeError(format!("codePointAt: index {index} is out of bounds")));
    };
    // A leading high surrogate followed by a low surrogate combines into an
    // astral code point; otherwise the bare code unit is the code point.
    if (0xD800..=0xDBFF).contains(&first) {
        if let Some(&second) = units.get(index + 1) {
            if (0xDC00..=0xDFFF).contains(&second) {
                let code = 0x10000
                    + (((first as u32 - 0xD800) << 10) | (second as u32 - 0xDC00));
                return Ok(NativeValue::Int(code as i64));
            }
        }
    }
    Ok(NativeValue::Int(first as i64))
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
    let units: Vec<u16> = value.encode_utf16().collect();
    let len = units.len() as f64;
    // Negative indices count from the end; both ends clamp into range. An
    // absent `end` is passed as a value past any string length, so it clamps
    // to `len` here.
    let normalize = |index: f64| -> usize {
        let resolved = if index < 0.0 { (len + index).max(0.0) } else { index.min(len) };
        resolved as usize
    };
    let begin = normalize(start);
    let finish = normalize(end);
    if begin >= finish {
        return Ok(NativeValue::Str(String::new()));
    }
    Ok(NativeValue::Str(String::from_utf16_lossy(&units[begin..finish])))
}

fn replace_all(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match (args.first(), args.get(1), args.get(2)) {
        (
            Some(NativeArg::Str(value)),
            Some(NativeArg::Str(search)),
            Some(NativeArg::Str(replacement)),
        ) => {
            // An empty search in JS inserts the replacement between every code
            // unit (and at both ends); Rust's `replace` matches that behavior.
            Ok(NativeValue::Str(value.replace(*search, replacement)))
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
        _ => return Err(NativeError("fromCharCode: expected a numeric code".to_string())),
    };
    // Each code is truncated to a UTF-16 code unit, per spec.
    Ok(NativeValue::Str(String::from_utf16_lossy(&[code])))
}

fn from_code_point1(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let code = match args.first() {
        Some(NativeArg::Int(code)) => *code,
        Some(NativeArg::Float(code)) if code.fract() == 0.0 => *code as i64,
        _ => return Err(NativeError("fromCodePoint: expected an integer code point".to_string())),
    };
    if !(0..=0x10FFFF).contains(&code) {
        return Err(NativeError(format!("fromCodePoint: invalid code point {code}")));
    }
    let code = code as u32;
    if code <= 0xFFFF {
        // Includes lone surrogates, which `from_utf16_lossy` renders faithfully.
        Ok(NativeValue::Str(String::from_utf16_lossy(&[code as u16])))
    } else {
        match char::from_u32(code) {
            Some(scalar) => Ok(NativeValue::Str(scalar.to_string())),
            None => Err(NativeError(format!("fromCodePoint: invalid code point {code}"))),
        }
    }
}

/// The machine-facing string primitives used by `zun:core`.
pub const MODULE: crate::Module = crate::Module {
    name: "zun:string",
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
    ],
};
