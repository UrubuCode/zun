//! `zun:math` — deterministic numeric intrinsics.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn float_arg(args: &[NativeArg], name: &str) -> Result<f64, NativeError> {
    match args.first() {
        Some(NativeArg::Float(value)) => Ok(*value),
        Some(NativeArg::Int(value)) => Ok(*value as f64),
        _ => Err(NativeError(format!("{name}: expected a number"))),
    }
}

fn abs(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Float(float_arg(args, "abs")?.abs()))
}

fn sqrt(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Float(float_arg(args, "sqrt")?.sqrt()))
}

fn cbrt(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Float(float_arg(args, "cbrt")?.cbrt()))
}

fn sign(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = float_arg(args, "sign")?;
    let result = if value.is_nan() || value == 0.0 {
        value
    } else if value.is_sign_positive() {
        1.0
    } else {
        -1.0
    };
    Ok(NativeValue::Float(result))
}

fn min(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let left = float_arg(args, "min")?;
    let right = match args.get(1) {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("min: expected two numbers".to_string())),
    };
    Ok(NativeValue::Float(left.min(right)))
}

fn max(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let left = float_arg(args, "max")?;
    let right = match args.get(1) {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("max: expected two numbers".to_string())),
    };
    Ok(NativeValue::Float(left.max(right)))
}

fn receiver_value(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.get(1) {
        Some(NativeArg::Float(value)) => Ok(NativeValue::Float(*value)),
        Some(NativeArg::Int(value)) => Ok(NativeValue::Float(*value as f64)),
        _ => Err(NativeError(
            "receiverValue: expected a numeric value".to_string(),
        )),
    }
}

fn unary_float(
    args: &[NativeArg],
    name: &str,
    operation: fn(f64) -> f64,
) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError(format!("{name}: expected a number"))),
    };
    Ok(NativeValue::Float(operation(value)))
}

fn floor(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "floor", f64::floor)
}

fn ceil(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "ceil", f64::ceil)
}

fn round(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "round", f64::round)
}

fn trunc(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "trunc", f64::trunc)
}

fn pow(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let base = float_arg(args, "pow")?;
    let exponent = match args.get(1) {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("pow: expected two numbers".to_string())),
    };
    Ok(NativeValue::Float(base.powf(exponent)))
}

fn sin(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "sin", f64::sin)
}

fn cos(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "cos", f64::cos)
}

fn exp(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "exp", f64::exp)
}

fn log(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "log", f64::ln)
}

fn log10(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "log10", f64::log10)
}

fn log2(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "log2", f64::log2)
}

fn expm1(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "expm1", f64::exp_m1)
}

fn log1p(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "log1p", f64::ln_1p)
}

fn sinh(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "sinh", f64::sinh)
}

fn cosh(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "cosh", f64::cosh)
}

fn tanh(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "tanh", f64::tanh)
}

fn asinh(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "asinh", f64::asinh)
}

fn acosh(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "acosh", f64::acosh)
}

fn atanh(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "atanh", f64::atanh)
}

fn tan(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "tan", f64::tan)
}

fn atan2(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let y = float_arg(args, "atan2")?;
    let x = match args.get(1) {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("atan2: expected two numbers".to_string())),
    };
    Ok(NativeValue::Float(y.atan2(x)))
}

fn hypot(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let left = float_arg(args, "hypot")?;
    let right = match args.get(1) {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("hypot: expected two numbers".to_string())),
    };
    Ok(NativeValue::Float(left.hypot(right)))
}

/// The runtime-backed numeric portion of the standard library.
pub const MODULE: crate::Module = crate::Module {
    name: "math",
    functions: &[
        NativeFn {
            name: "abs",
            params: &[F64],
            ret: F64,
            imp: abs,
        },
        NativeFn {
            name: "sqrt",
            params: &[F64],
            ret: F64,
            imp: sqrt,
        },
        NativeFn {
            name: "min",
            params: &[F64, F64],
            ret: F64,
            imp: min,
        },
        NativeFn {
            name: "max",
            params: &[F64, F64],
            ret: F64,
            imp: max,
        },
        NativeFn {
            name: "sqrtNumber",
            params: &[F64],
            ret: F64,
            imp: sqrt,
        },
        NativeFn {
            name: "cbrtNumber",
            params: &[F64],
            ret: F64,
            imp: cbrt,
        },
        NativeFn {
            name: "signNumber",
            params: &[F64],
            ret: F64,
            imp: sign,
        },
        NativeFn {
            name: "minNumber",
            params: &[F64, F64],
            ret: F64,
            imp: min,
        },
        NativeFn {
            name: "maxNumber",
            params: &[F64, F64],
            ret: F64,
            imp: max,
        },
        NativeFn {
            name: "floor",
            params: &[F64],
            ret: F64,
            imp: floor,
        },
        NativeFn {
            name: "ceil",
            params: &[F64],
            ret: F64,
            imp: ceil,
        },
        NativeFn {
            name: "round",
            params: &[F64],
            ret: F64,
            imp: round,
        },
        NativeFn {
            name: "floorNumber",
            params: &[F64],
            ret: F64,
            imp: floor,
        },
        NativeFn {
            name: "ceilNumber",
            params: &[F64],
            ret: F64,
            imp: ceil,
        },
        NativeFn {
            name: "roundNumber",
            params: &[F64],
            ret: F64,
            imp: round,
        },
        NativeFn {
            name: "truncNumber",
            params: &[F64],
            ret: F64,
            imp: trunc,
        },
        NativeFn {
            name: "receiverValue",
            params: &[Opaque, F64],
            ret: F64,
            imp: receiver_value,
        },
        NativeFn {
            name: "powNumber",
            params: &[F64, F64],
            ret: F64,
            imp: pow,
        },
        NativeFn {
            name: "sinNumber",
            params: &[F64],
            ret: F64,
            imp: sin,
        },
        NativeFn {
            name: "cosNumber",
            params: &[F64],
            ret: F64,
            imp: cos,
        },
        NativeFn {
            name: "expNumber",
            params: &[F64],
            ret: F64,
            imp: exp,
        },
        NativeFn {
            name: "logNumber",
            params: &[F64],
            ret: F64,
            imp: log,
        },
        NativeFn {
            name: "log10Number",
            params: &[F64],
            ret: F64,
            imp: log10,
        },
        NativeFn {
            name: "log2Number",
            params: &[F64],
            ret: F64,
            imp: log2,
        },
        NativeFn {
            name: "expm1Number",
            params: &[F64],
            ret: F64,
            imp: expm1,
        },
        NativeFn {
            name: "log1pNumber",
            params: &[F64],
            ret: F64,
            imp: log1p,
        },
        NativeFn {
            name: "sinhNumber",
            params: &[F64],
            ret: F64,
            imp: sinh,
        },
        NativeFn {
            name: "coshNumber",
            params: &[F64],
            ret: F64,
            imp: cosh,
        },
        NativeFn {
            name: "tanhNumber",
            params: &[F64],
            ret: F64,
            imp: tanh,
        },
        NativeFn {
            name: "asinhNumber",
            params: &[F64],
            ret: F64,
            imp: asinh,
        },
        NativeFn {
            name: "acoshNumber",
            params: &[F64],
            ret: F64,
            imp: acosh,
        },
        NativeFn {
            name: "atanhNumber",
            params: &[F64],
            ret: F64,
            imp: atanh,
        },
        NativeFn {
            name: "tanNumber",
            params: &[F64],
            ret: F64,
            imp: tan,
        },
        NativeFn {
            name: "atan2Number",
            params: &[F64, F64],
            ret: F64,
            imp: atan2,
        },
        NativeFn {
            name: "hypotNumber",
            params: &[F64, F64],
            ret: F64,
            imp: hypot,
        },
    ],
};
