//! `zun:date` — UTC calendar operations over millisecond timestamps.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn millis(args: &[NativeArg], name: &str) -> Result<i64, NativeError> {
    match args.first() {
        Some(NativeArg::Int(value)) => Ok(*value),
        _ => Err(NativeError(format!("{name}: expected an i64 timestamp"))),
    }
}

fn date(args: &[NativeArg], name: &str) -> Result<zun_core::DateValue, NativeError> {
    Ok(zun_core::DateValue::from_millis(millis(args, name)?))
}

fn year(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(date(args, "year")?.utc_date().ok_or_else(|| NativeError("year: invalid date".into()))?.0 as i64))
}

fn month(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(date(args, "month")?.utc_date().ok_or_else(|| NativeError("month: invalid date".into()))?.1 as i64))
}

fn day(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(date(args, "day")?.utc_date().ok_or_else(|| NativeError("day: invalid date".into()))?.2 as i64))
}

fn hour(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(date(args, "hour")?.utc_time().ok_or_else(|| NativeError("hour: invalid date".into()))?.0 as i64))
}

fn weekday(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(date(args, "weekday")?.weekday().ok_or_else(|| NativeError("weekday: invalid date".into()))? as i64))
}

fn iso(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    date(args, "iso")?.iso_date().map(NativeValue::Str).ok_or_else(|| NativeError("iso: invalid date".into()))
}

pub const MODULE: crate::Module = crate::Module {
    name: "zun:date",
    functions: &[
        NativeFn { name: "year", params: &[I64], ret: I32, imp: year },
        NativeFn { name: "month", params: &[I64], ret: I32, imp: month },
        NativeFn { name: "day", params: &[I64], ret: I32, imp: day },
        NativeFn { name: "hour", params: &[I64], ret: I32, imp: hour },
        NativeFn { name: "weekday", params: &[I64], ret: I32, imp: weekday },
        NativeFn { name: "isoDate", params: &[I64], ret: Str, imp: iso },
    ],
};
