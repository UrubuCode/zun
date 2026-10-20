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
    let result = match (&a, &b) {
        (Value::Int(x, k), Value::Int(y, _)) => {
            let (_, signed) = k.int_parts().expect("int kind");
            if signed { op.apply(*x, *y) } else { op.apply(*x as u64, *y as u64) }
        }
        (Value::Float(x, _), Value::Float(y, _)) => op.apply(*x, *y),
        _ => return Err(RunError::Malformed("comparison of mismatched numeric kinds".into())),
    };
    stack.push(Value::Bool(result));
    Ok(())
}

pub(crate) fn cmp(stack: &mut Vec<Value>, want_equal: bool) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let equal = match (&a, &b) {
        (Value::Float(x, _), Value::Float(y, _)) => x == y,
        (Value::Int(x, _), Value::Int(y, _)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Object(x), Value::Object(y)) => x.addr() == y.addr(),
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
