//! `zun:number` — numeric conversion primitives.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

/// The largest integer that `f64` represents exactly (`2^53 - 1`).
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Interpret a native argument as a strict `f64`, performing **no** coercion.
/// `number` immediates (`Float`/`Int`) and `Any` values carrying a number read
/// back; everything else (strings, booleans, objects, `undefined`) yields
/// `None` so the strict `Number.*` predicates answer `false` without coercing.
fn strict_number(arg: Option<&NativeArg>) -> Option<f64> {
    match arg {
        Some(NativeArg::Float(value)) => Some(*value),
        Some(NativeArg::Int(value)) => Some(*value as f64),
        Some(NativeArg::Any(value)) => value
            .as_float()
            .or_else(|| value.as_int().map(|int| int as f64)),
        _ => None,
    }
}

/// Read a radix argument (`Int` or `Float`), defaulting to `fallback` when the
/// slot is absent.
fn radix_arg(arg: Option<&NativeArg>, fallback: i64) -> i64 {
    match arg {
        Some(NativeArg::Int(value)) => *value,
        Some(NativeArg::Float(value)) => *value as i64,
        _ => fallback,
    }
}

fn is_nan_strict(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Bool(
        strict_number(args.first()).is_some_and(|value| value.is_nan()),
    ))
}

fn is_finite_strict(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Bool(
        strict_number(args.first()).is_some_and(|value| value.is_finite()),
    ))
}

fn is_integer_strict(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Bool(
        strict_number(args.first())
            .is_some_and(|value| value.is_finite() && value.fract() == 0.0),
    ))
}

fn is_safe_integer_strict(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Bool(strict_number(args.first()).is_some_and(
        |value| {
            value.is_finite()
                && value.fract() == 0.0
                && (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value)
        },
    )))
}

/// `Number.parseFloat(s)` — parse the longest leading numeric prefix, JS-style:
/// surrounding junk is ignored and an unparseable string yields `NaN` (never an
/// error).
fn parse_float_prefix(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let text = match args.first() {
        Some(NativeArg::Str(value)) => value.trim_start(),
        Some(NativeArg::Any(value)) if value.as_string().is_some() => "",
        _ => return Ok(NativeValue::Float(f64::NAN)),
    };
    // The longest prefix that parses as an `f64` is the JS result; Rust's
    // `FromStr` rejects trailing junk, so extend the window greedily.
    let mut best = f64::NAN;
    for end in 1..=text.len() {
        if !text.is_char_boundary(end) {
            continue;
        }
        if let Ok(value) = text[..end].parse::<f64>() {
            best = value;
        }
    }
    Ok(NativeValue::Float(best))
}

/// `Number.parseInt(s, radix?)` — parse an integer in `radix` (0 means detect:
/// `0x`/`0X` → 16, else 10). Leading whitespace and sign are honoured; parsing
/// stops at the first non-digit; no valid digit yields `NaN`.
fn parse_int_radix(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let text = match args.first() {
        Some(NativeArg::Str(value)) => value.trim(),
        _ => return Ok(NativeValue::Float(f64::NAN)),
    };
    let mut radix = radix_arg(args.get(1), 0);
    let mut sign = 1.0;
    let mut rest = text;
    if let Some(stripped) = text.strip_prefix('+') {
        rest = stripped;
    } else if let Some(stripped) = text.strip_prefix('-') {
        sign = -1.0;
        rest = stripped;
    }
    if radix == 0 || radix == 16 {
        if let Some(stripped) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
            rest = stripped;
            radix = 16;
        }
    }
    if radix == 0 {
        radix = 10;
    }
    if !(2..=36).contains(&radix) {
        return Ok(NativeValue::Float(f64::NAN));
    }
    let mut value = 0.0;
    let mut saw_digit = false;
    for ch in rest.chars() {
        match ch.to_digit(radix as u32) {
            Some(digit) => {
                value = value * radix as f64 + digit as f64;
                saw_digit = true;
            }
            None => break,
        }
    }
    if saw_digit {
        Ok(NativeValue::Float(sign * value))
    } else {
        Ok(NativeValue::Float(f64::NAN))
    }
}

/// `(value).toString(radix)` — base-`radix` (2..=36) rendering, radix 10 using
/// the default JS numeric formatting. Non-finite values render as JS spells
/// them (`NaN`/`Infinity`/`-Infinity`).
fn to_string_radix(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("toString: expected a number".to_string())),
    };
    let radix = radix_arg(args.get(1), 10);
    if !(2..=36).contains(&radix) {
        return Err(NativeError(format!(
            "toString: radix must be between 2 and 36, found {radix}"
        )));
    }
    if value.is_nan() {
        return Ok(NativeValue::Str("NaN".to_string()));
    }
    if value.is_infinite() {
        return Ok(NativeValue::Str(
            if value < 0.0 { "-Infinity" } else { "Infinity" }.to_string(),
        ));
    }
    if radix == 10 {
        return Ok(NativeValue::Str(crate::format_float(value)));
    }
    Ok(NativeValue::Str(format_radix(value, radix as u32)))
}

/// Render a finite `f64` in `radix` (2..=36): full integer part plus up to 20
/// fractional digits.
fn format_radix(mut value: f64, radix: u32) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let negative = value.is_sign_negative() && value != 0.0;
    value = value.abs();
    let mut integer = value.trunc();
    let mut fraction = value.fract();
    let base = radix as f64;

    let mut integer_digits = Vec::new();
    if integer == 0.0 {
        integer_digits.push(b'0');
    } else {
        while integer > 0.0 {
            let digit = (integer % base) as usize;
            integer_digits.push(DIGITS[digit]);
            integer = (integer / base).trunc();
        }
        integer_digits.reverse();
    }

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(std::str::from_utf8(&integer_digits).unwrap());

    if fraction > 0.0 {
        out.push('.');
        let mut emitted = 0;
        while fraction > 0.0 && emitted < 20 {
            fraction *= base;
            let digit = fraction.trunc() as usize;
            out.push(DIGITS[digit] as char);
            fraction -= fraction.trunc();
            emitted += 1;
        }
    }
    out
}

fn parse_float(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let text = match args.first() {
        Some(NativeArg::Str(value)) => value.trim(),
        _ => return Err(NativeError("parseFloat: expected a string".to_string())),
    };
    text.parse::<f64>()
        .map(NativeValue::Float)
        .map_err(|_| NativeError(format!("parseFloat: invalid number `{text}`")))
}

fn to_fixed(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("toFixed: expected a number".to_string())),
    };
    let digits = match args.get(1) {
        Some(NativeArg::Int(value)) if (0..=100).contains(value) => *value as usize,
        _ => {
            return Err(NativeError(
                "toFixed: expected digits from 0 through 100".to_string(),
            ))
        }
    };
    Ok(NativeValue::Str(format!("{value:.digits$}")))
}

fn to_string(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.first() {
        Some(NativeArg::Float(value)) => Ok(NativeValue::Str(value.to_string())),
        Some(NativeArg::Int(value)) => Ok(NativeValue::Str(value.to_string())),
        _ => Err(NativeError("toString: expected a number".to_string())),
    }
}

fn is_finite(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("isFinite: expected a number".to_string())),
    };
    Ok(NativeValue::Bool(value.is_finite()))
}

fn is_nan(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("isNaN: expected a number".to_string())),
    };
    Ok(NativeValue::Bool(value.is_nan()))
}

fn is_negative_zero(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("isNegativeZero: expected a number".to_string())),
    };
    Ok(NativeValue::Bool(value == 0.0 && value.is_sign_negative()))
}

fn is_safe_integer(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("isSafeInteger: expected a number".to_string())),
    };
    Ok(NativeValue::Bool(
        value.is_finite()
            && value.fract() == 0.0
            && (-9_007_199_254_740_991.0..=9_007_199_254_740_991.0).contains(&value),
    ))
}

/// Rewrite a Rust exponential rendering (`"1.2345e3"`, `"-2e-7"`) into the JS
/// shape, which always spells the exponent sign (`"1.2345e+3"`, `"-2e-7"`).
fn js_exponent(text: String) -> String {
    let Some(pos) = text.find('e') else {
        return text;
    };
    let (mantissa, exponent) = text.split_at(pos);
    let exponent = &exponent[1..];
    match exponent.strip_prefix('-') {
        Some(rest) => format!("{mantissa}e-{rest}"),
        None => format!("{mantissa}e+{exponent}"),
    }
}

/// Parse the decimal exponent Rust assigns to `value` when rendered with
/// `fraction` fractional digits (e.g. `1.2e2` → `2`). Rust never emits a `+`,
/// so a plain `parse` reads the sign.
fn decimal_exponent(value: f64, fraction: usize) -> i32 {
    let text = format!("{value:.fraction$e}");
    let pos = text.find('e').expect("exponential rendering has an `e`");
    text[pos + 1..]
        .parse::<i32>()
        .expect("exponent is an integer")
}

/// `(value).toExponential(fractionDigits?)` — JS exponential notation. With no
/// argument the shortest round-tripping mantissa is used; otherwise exactly
/// `fractionDigits` (0..=100) fraction digits. The sema intercept passes `-1`
/// for the absent argument (outside the valid range, so unambiguous).
fn to_exponential(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("toExponential: expected a number".to_string())),
    };
    if value.is_nan() {
        return Ok(NativeValue::Str("NaN".to_string()));
    }
    if value.is_infinite() {
        return Ok(NativeValue::Str(
            if value < 0.0 { "-Infinity" } else { "Infinity" }.to_string(),
        ));
    }
    let fraction = match args.get(1) {
        Some(NativeArg::Int(-1)) | None => None,
        Some(NativeArg::Int(digits)) if (0..=100).contains(digits) => Some(*digits as usize),
        _ => {
            return Err(NativeError(
                "toExponential: fractionDigits must be from 0 through 100".to_string(),
            ))
        }
    };
    let rendered = match fraction {
        Some(fraction) => format!("{value:.fraction$e}"),
        None => format!("{value:e}"),
    };
    Ok(NativeValue::Str(js_exponent(rendered)))
}

/// `(value).toPrecision(precision?)` — significant-digit formatting. With no
/// argument it behaves like the default `toString` (sema passes `0`, which is
/// outside the valid 1..=100 range). Otherwise it renders `precision`
/// significant digits, switching to exponential notation when the decimal
/// exponent is `< -6` or `>= precision` (per ECMAScript Number::toPrecision).
fn to_precision(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("toPrecision: expected a number".to_string())),
    };
    let precision = match args.get(1) {
        // `0` is the absent-argument sentinel: behave like `toString`.
        Some(NativeArg::Int(0)) | None => return Ok(NativeValue::Str(crate::format_float(value))),
        Some(NativeArg::Int(precision)) if (1..=100).contains(precision) => *precision,
        _ => {
            return Err(NativeError(
                "toPrecision: precision must be from 1 through 100".to_string(),
            ))
        }
    };
    if value.is_nan() {
        return Ok(NativeValue::Str("NaN".to_string()));
    }
    if value.is_infinite() {
        return Ok(NativeValue::Str(
            if value < 0.0 { "-Infinity" } else { "Infinity" }.to_string(),
        ));
    }
    let precision = precision as i32;
    // `precision - 1` fraction digits give exactly `precision` significant
    // digits in the exponential rendering; read the exponent back from it.
    let exponent = decimal_exponent(value, (precision - 1) as usize);
    if exponent < -6 || exponent >= precision {
        let rendered = format!("{value:.*e}", (precision - 1) as usize);
        return Ok(NativeValue::Str(js_exponent(rendered)));
    }
    // Fixed notation: the digits after the point fill the remaining
    // significance below the units place (never negative here).
    let fraction = (precision - 1 - exponent).max(0) as usize;
    Ok(NativeValue::Str(format!("{value:.fraction$}")))
}

/// `Number.prototype.toLocaleString()` — root-locale digit grouping via baked ICU4X.
fn number_to_locale_string(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Float(v)) => *v,
        Some(NativeArg::Int(v)) => *v as f64,
        _ => return Err(NativeError("toLocaleString: expected a number".to_string())),
    };
    if value.is_nan() {
        return Ok(NativeValue::Str("NaN".to_string()));
    }
    if value.is_infinite() {
        let text = if value < 0.0 { "-∞" } else { "∞" };
        return Ok(NativeValue::Str(text.to_string()));
    }
    // JS caps the default fraction digits at 3.
    let text = format!("{value:.3}");
    let mut decimal: icu::decimal::input::Decimal = text
        .parse()
        .map_err(|_| NativeError("toLocaleString: unrepresentable number".to_string()))?;
    decimal.trim_end();
    let formatter = icu::decimal::DecimalFormatter::try_new(
        icu::decimal::DecimalFormatterPreferences::default(),
        Default::default(),
    )
    .map_err(|_| NativeError("toLocaleString: no decimal data".to_string()))?;
    Ok(NativeValue::Str(formatter.format(&decimal).to_string()))
}

/// Runtime-backed numeric conversion functions.
pub const MODULE: crate::Module = crate::Module {
    name: "number",
    functions: &[
        NativeFn {
            name: "parseFloat",
            params: &[Str],
            ret: F64,
            imp: parse_float,
        },
        NativeFn {
            name: "toFixed",
            params: &[F64, I32],
            ret: Str,
            imp: to_fixed,
        },
        NativeFn {
            name: "toString",
            params: &[F64],
            ret: Str,
            imp: to_string,
        },
        NativeFn {
            name: "isFinite",
            params: &[F64],
            ret: Bool,
            imp: is_finite,
        },
        NativeFn {
            name: "isNaN",
            params: &[F64],
            ret: Bool,
            imp: is_nan,
        },
        NativeFn {
            name: "isNegativeZero",
            params: &[F64],
            ret: Bool,
            imp: is_negative_zero,
        },
        NativeFn {
            name: "isSafeInteger",
            params: &[F64],
            ret: Bool,
            imp: is_safe_integer,
        },
        // Strict `Number.*` statics: the argument is inspected without coercion,
        // so a non-number answers `false` (distinct from the global `isNaN`/
        // `isFinite`, which coerce). The parameter is `Opaque` so any value type
        // reaches the native intact.
        NativeFn {
            name: "isNaNStrict",
            params: &[Opaque],
            ret: Bool,
            imp: is_nan_strict,
        },
        NativeFn {
            name: "isFiniteStrict",
            params: &[Opaque],
            ret: Bool,
            imp: is_finite_strict,
        },
        NativeFn {
            name: "isIntegerStrict",
            params: &[Opaque],
            ret: Bool,
            imp: is_integer_strict,
        },
        NativeFn {
            name: "isSafeIntegerStrict",
            params: &[Opaque],
            ret: Bool,
            imp: is_safe_integer_strict,
        },
        NativeFn {
            name: "parseFloatPrefix",
            params: &[Str],
            ret: F64,
            imp: parse_float_prefix,
        },
        NativeFn {
            name: "parseIntRadix",
            params: &[Str, I32],
            ret: F64,
            imp: parse_int_radix,
        },
        NativeFn {
            name: "toStringRadix",
            params: &[F64, I32],
            ret: Str,
            imp: to_string_radix,
        },
        // Instance formatting (NUCLEUS #2). Appended last so existing dispatch
        // ids stay stable. Both take the optional digit/precision count as a
        // second `I32`; the sema intercept passes an out-of-range sentinel
        // (`-1` for `toExponential`, `0` for `toPrecision`) for the absent arg.
        NativeFn {
            name: "toExponential",
            params: &[F64, I32],
            ret: Str,
            imp: to_exponential,
        },
        NativeFn {
            name: "toPrecision",
            params: &[F64, I32],
            ret: Str,
            imp: to_precision,
        },
        NativeFn {
            name: "numberToLocaleString",
            params: &[F64],
            ret: Str,
            imp: number_to_locale_string,
        },
    ],
};
