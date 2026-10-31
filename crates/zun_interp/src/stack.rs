//! Stack helper errors and operations for Tier 0 execution.

use super::RunError;
use super::value::Value;
use super::value;

#[derive(Clone, Copy)]
pub(crate) enum Ordering {
    Lt,
    Le,
    Gt,
    Ge,
}

impl Ordering {
    pub(crate) fn apply<T: PartialOrd>(self, a: T, b: T) -> bool {
        match self {
            Ordering::Lt => a < b,
            Ordering::Le => a <= b,
            Ordering::Gt => a > b,
            Ordering::Ge => a >= b,
        }
    }
}

pub(crate) fn compare(stack: &mut Vec<Value>, op: Ordering) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    // A boxed `Any` operand (e.g. an untyped callback parameter) is unboxed
    // and compared by its dynamic value.
    let unbox = |v: Value| -> Value {
        match v {
            Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)).unwrap_or(v),
            other => other,
        }
    };
    let (a, b) = (unbox(a), unbox(b));
    let result = match (&a, &b) {
        (Value::Int(x, k), Value::Int(y, _)) => {
            let (_, signed) = k.int_parts().expect("int kind");
            if signed { op.apply(*x, *y) } else { op.apply(*x as u64, *y as u64) }
        }
        (Value::Float(x, _), Value::Float(y, _)) => op.apply(*x, *y),
        (Value::Int(x, _), Value::Float(y, _)) => op.apply(*x as f64, *y),
        (Value::Float(x, _), Value::Int(y, _)) => op.apply(*x, *y as f64),
        (Value::Str(x), Value::Str(y)) => op.apply(x.as_ref() as &str, y.as_ref() as &str),
        _ => return Err(RunError::Malformed(format!("comparison of mismatched numeric kinds: {a:?} vs {b:?}"))),
    };
    stack.push(Value::Bool(result));
    Ok(())
}

/// Project a boxed `Any` operand onto the primitive it carries, so a dynamic
/// value read from an object compares against an unboxed literal (`o.v === true`).
/// Opaque `Any` payloads (class descriptors, symbols) stay boxed.
fn unbox_for_compare(value: Value) -> Value {
    match value {
        Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
            .unwrap_or(Value::Any(any)),
        other => other,
    }
}

pub(crate) fn cmp(stack: &mut Vec<Value>, want_equal: bool) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    // A dynamic (`any`) operand compares by its underlying value.
    let unbox = |value: Value| match value {
        Value::Any(any) => {
            zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)).unwrap_or(Value::Any(any))
        }
        other => other,
    };
    let (a, b) = if matches!(a, Value::Any(_)) || matches!(b, Value::Any(_)) {
        (unbox(a), unbox(b))
    } else {
        (a, b)
    };
    let equal = match (&a, &b) {
        (Value::Float(x, _), Value::Float(y, _)) => x == y,
        (Value::Int(x, _), Value::Float(y, _)) | (Value::Float(y, _), Value::Int(x, _)) => {
            (*x as f64) == *y
        }
        (Value::Int(x, _), Value::Int(y, _)) => x == y,
        (Value::Void, Value::Void) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Object(x), Value::Object(y)) => x.addr() == y.addr(),
        // Function values compare by identity (same method + captured
        // environment), so `f == f` is true. Two distinct closures / bound
        // functions differ, matching JS reference equality.
        (Value::Function(x), Value::Function(y)) => x == y,
        // A Map/Set/etc. receiver may be carried as a boxed object `Any`;
        // reference identity compares the heap addresses.
        (Value::Any(x), Value::Any(y)) if x.as_object().is_some() => x.as_object() == y.as_object(),
        (Value::Any(x), Value::Object(y)) | (Value::Object(y), Value::Any(x)) => {
            x.as_object() == Some(y.addr() as usize)
        }
        (Value::Null, Value::Null) => true,
        (Value::Any(x), Value::Null) | (Value::Null, Value::Any(x)) => {
            x.tag() == zun_core::AnyTag::Null
        }
        _ => false,
    };
    stack.push(Value::Bool(equal == want_equal));
    Ok(())
}

pub(crate) fn underflow() -> RunError {
    RunError::Malformed("stack underflow".to_string())
}

pub(crate) fn pop(stack: &mut Vec<Value>) -> Result<Value, RunError> {
    stack.pop().ok_or_else(underflow)
}

pub(crate) fn pop_n(stack: &mut Vec<Value>, n: usize) -> Result<Vec<Value>, RunError> {
    if stack.len() < n {
        return Err(underflow());
    }
    Ok(stack.split_off(stack.len() - n))
}

pub(crate) fn bin_float(
    stack: &mut Vec<Value>,
    f: impl Fn(f64, f64) -> f64,
) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let kind = a.float_kind();
    let result = value::round_float(f(a.as_number(), b.as_number()), kind);
    stack.push(Value::Float(result, kind));
    Ok(())
}

pub(crate) fn un_float(stack: &mut Vec<Value>, f: impl Fn(f64) -> f64) -> Result<(), RunError> {
    let a = pop(stack)?;
    let kind = a.float_kind();
    let result = value::round_float(f(a.as_number()), kind);
    stack.push(Value::Float(result, kind));
    Ok(())
}

/// `Math.min` / IEEE-754 2019 `minimum`, which is what `min.f64` means and what
/// Cranelift's `fmin` emits.
///
/// Deliberately not `f64::min`: that is IEEE-754 2008 `minNum`, which returns
/// the *non*-NaN operand instead of propagating NaN, and which the Rust
/// documentation says may return either operand when they compare equal — so it
/// picks between `+0.0` and `-0.0` non-deterministically. Both differ from JS,
/// and both would make Tier 0 disagree with Tier 1.
pub(crate) fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == 0.0 && b == 0.0 {
        // The only pair that compares equal while being distinguishable:
        // -0.0 is the smaller zero.
        return if a.is_sign_negative() { a } else { b };
    }
    if a < b {
        a
    } else {
        b
    }
}

/// `Math.round`: halves go toward +Infinity, and a zero result keeps the sign
/// of the input.
///
/// Deliberately not `floor(x + 0.5)`, the usual shortcut. For
/// `0.49999999999999994` the addition rounds up to exactly `1.0`, so the
/// shortcut answers 1 where JS answers 0. Taking the fractional part instead
/// keeps the comparison exact.
pub(crate) fn js_round(x: f64) -> f64 {
    let truncated = x.floor();
    // NaN and the infinities fall out of this untouched: the subtraction
    // gives NaN, the comparison is false, and `truncated` is already right.
    let rounded = if x - truncated >= 0.5 {
        truncated + 1.0
    } else {
        truncated
    };
    if rounded == 0.0 {
        // -0.4 rounds to -0, not +0; +0 and -0 pass through unchanged.
        0.0f64.copysign(x)
    } else {
        rounded
    }
}

/// `Math.sign`: NaN and both zeros pass through unchanged, everything else
/// becomes ±1.
pub(crate) fn js_sign(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 {
        x
    } else {
        1.0f64.copysign(x)
    }
}

/// `Math.max` / IEEE-754 2019 `maximum`. See [`js_min`] for why `f64::max` is
/// not used.
pub(crate) fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_positive() { a } else { b };
    }
    if a > b {
        a
    } else {
        b
    }
}

/// `min`/`max` over integers, comparing by the operand kind's signedness.
///
/// The i64 container holds values already wrapped to their kind, so a signed
/// kind compares correctly as i64 and an unsigned one as u64. Comparing a
/// `u64` above 2^63 as i64 would otherwise invert the result.
pub(crate) fn bin_int_minmax(stack: &mut Vec<Value>, is_min: bool) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let kind = a.int_kind();
    let (_, signed) = kind.int_parts().expect("int kind");
    let (x, y) = (a.as_int(), b.as_int());
    let take_left = if signed {
        if is_min {
            x < y
        } else {
            x > y
        }
    } else {
        let (ux, uy) = (x as u64, y as u64);
        if is_min {
            ux < uy
        } else {
            ux > uy
        }
    };
    let result = if take_left { x } else { y };
    stack.push(Value::Int(value::wrap_int(result, kind), kind));
    Ok(())
}

/// Bit-counting over the operand kind's width.
///
/// `f` receives the operand masked to its width plus that width, because the
/// answers are width-relative: `clz` of `1u8` is 7, and `ctz` of `0u8` is 8.
/// Reading the i64 container directly would answer for 64 bits every time.
pub(crate) fn un_int_bits(
    stack: &mut Vec<Value>,
    f: impl Fn(u64, u32) -> u64,
) -> Result<(), RunError> {
    let a = pop(stack)?;
    let kind = a.int_kind();
    let (bits, _) = kind.int_parts().expect("int kind");
    let bits = bits as u32;
    let mask = if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    let masked = (a.as_int() as u64) & mask;
    let result = f(masked, bits) as i64;
    stack.push(Value::Int(value::wrap_int(result, kind), kind));
    Ok(())
}

/// The kind of integer shift to perform.
#[derive(Clone, Copy)]
pub(crate) enum ShiftOp {
    /// Left shift.
    Left,
    /// Right shift: arithmetic for a signed operand kind, logical for unsigned.
    Right,
    /// Unsigned/logical right shift regardless of kind (JS `>>>`).
    UnsignedRight,
}

/// Integer shift, width- and signedness-generic.
///
/// The shift amount is masked to the operand kind's bit width (`& (bits - 1)`),
/// which matches both JS's 32-bit shift masking and Cranelift's shift
/// semantics, so Tier 0 and Tier 1 agree.
pub(crate) fn bin_int_shift(stack: &mut Vec<Value>, op: ShiftOp) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let kind = a.int_kind();
    let (bits, signed) = kind.int_parts().expect("int kind");
    let bits = bits as u32;
    let amount = (b.as_int() as u64 & (bits as u64 - 1)) as u32;
    let mask = if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 };
    let value = a.as_int();
    let result = match op {
        ShiftOp::Left => ((value as u64) << amount) as i64,
        ShiftOp::Right if signed => value >> amount,
        // Logical: read the operand as unsigned of its width first, so a signed
        // negative value's high bits do not leak in from the i64 container.
        ShiftOp::Right | ShiftOp::UnsignedRight => (((value as u64) & mask) >> amount) as i64,
    };
    stack.push(Value::Int(value::wrap_int(result, kind), kind));
    Ok(())
}

/// Integer exponentiation `a ** b`, wrapping to the operand kind.
///
/// Exponentiation by squaring under wrapping multiplication; a negative
/// exponent produces 1 (the empty product). The algorithm is irrelevant to the
/// result — wrapping multiply is a ring homomorphism — so Tier 1 may use any
/// equivalent sequence.
pub(crate) fn bin_int_pow(stack: &mut Vec<Value>) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let kind = a.int_kind();
    let (_, signed) = kind.int_parts().expect("int kind");
    let exp_raw = b.as_int();
    // The exponent is read by the operand kind's signedness: a signed negative
    // exponent is the empty product (1); an unsigned exponent uses its full
    // value. Squaring is logarithmic in the exponent, so a huge unsigned
    // exponent still terminates. This matches the Tier 1 lowering.
    let mut exp: u64 = if signed {
        if exp_raw < 0 { 0 } else { exp_raw as u64 }
    } else {
        exp_raw as u64
    };
    let mut result = 1i64;
    let mut base = a.as_int();
    while exp > 0 {
        if exp & 1 == 1 {
            result = result.wrapping_mul(base);
        }
        exp >>= 1;
        if exp > 0 {
            base = base.wrapping_mul(base);
        }
    }
    stack.push(Value::Int(value::wrap_int(result, kind), kind));
    Ok(())
}

pub(crate) fn bin_int(
    stack: &mut Vec<Value>,
    f: impl Fn(i64, i64) -> i64,
) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let kind = a.int_kind();
    stack.push(Value::Int(
        value::wrap_int(f(a.as_int(), b.as_int()), kind),
        kind,
    ));
    Ok(())
}

/// Integer divide or remainder, choosing signed versus unsigned by kind.
pub(crate) fn bin_int_signed_aware(
    stack: &mut Vec<Value>,
    is_div: bool,
) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let kind = a.int_kind();
    let (_, signed) = kind.int_parts().expect("int kind");
    let (x, y) = (a.as_int(), b.as_int());
    if y == 0 {
        return Err(RunError::Malformed("integer division by zero".into()));
    }
    let result = if signed {
        if is_div {
            x.wrapping_div(y)
        } else {
            x.wrapping_rem(y)
        }
    } else {
        let (ux, uy) = (x as u64, y as u64);
        let unsigned_result = if is_div { ux / uy } else { ux % uy };
        unsigned_result as i64
    };
    stack.push(Value::Int(value::wrap_int(result, kind), kind));
    Ok(())
}

/// Convert a numeric value according to the interpreter's numeric lattice.
pub(crate) fn convert(v: &Value, target: zun_il::NumKind) -> Result<Value, RunError> {
    use zun_il::NumKind;
    let as_f64 = match v {
        Value::Int(n, _) => *n as f64,
        Value::Float(n, _) => *n,
        other => return Err(RunError::Malformed(format!("cannot convert {other:?}"))),
    };
    Ok(if target.is_int() {
        let truncated = match v {
            Value::Int(n, _) => *n,
            Value::Float(n, _) => *n as i64,
            _ => unreachable!(),
        };
        Value::Int(value::wrap_int(truncated, target), target)
    } else {
        match target {
            NumKind::F32 => Value::Float(as_f64 as f32 as f64, NumKind::F32),
            _ => Value::Float(as_f64, NumKind::F64),
        }
    })
}
