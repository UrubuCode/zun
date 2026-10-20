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

fn is_valid(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Bool(date(args, "isValid")?.millis().is_some()))
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

fn minute(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(date(args, "minute")?.utc_time().ok_or_else(|| NativeError("minute: invalid date".into()))?.1 as i64))
}

fn second(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(date(args, "second")?.utc_time().ok_or_else(|| NativeError("second: invalid date".into()))?.2 as i64))
}

fn millisecond(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(date(args, "millisecond")?.utc_time().ok_or_else(|| NativeError("millisecond: invalid date".into()))?.3 as i64))
}

fn weekday(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(date(args, "weekday")?.weekday().ok_or_else(|| NativeError("weekday: invalid date".into()))? as i64))
}

fn ordinal(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(date(args, "dayOfYear")?.ordinal().ok_or_else(|| NativeError("dayOfYear: invalid date".into()))? as i64))
}

fn iso(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    date(args, "iso")?.iso_date().map(NativeValue::Str).ok_or_else(|| NativeError("iso: invalid date".into()))
}

fn iso_datetime(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    date(args, "isoDateTime")?.iso_datetime().map(NativeValue::Str).ok_or_else(|| NativeError("isoDateTime: invalid date".into()))
}

fn parse_iso_datetime(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => *value,
        _ => return Err(NativeError("parseIsoDateTime: expected a string".into())),
    };
    zun_core::DateValue::parse_iso_datetime(value)
        .and_then(zun_core::DateValue::millis)
        .map(NativeValue::Int)
        .ok_or_else(|| NativeError(format!("parseIsoDateTime: invalid ISO datetime `{value}`")))
}

fn parse_iso_date(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => *value,
        _ => return Err(NativeError("parseIsoDate: expected a string".into())),
    };
    zun_core::DateValue::parse_iso_date(value)
        .and_then(zun_core::DateValue::millis)
        .map(NativeValue::Int)
        .ok_or_else(|| NativeError(format!("parseIsoDate: invalid ISO date `{value}`")))
}

pub const MODULE: crate::Module = crate::Module {
    name: "zun:date",
    functions: &[
        NativeFn { name: "isValid", params: &[I64], ret: Bool, imp: is_valid },
        NativeFn { name: "year", params: &[I64], ret: I32, imp: year },
        NativeFn { name: "month", params: &[I64], ret: I32, imp: month },
        NativeFn { name: "day", params: &[I64], ret: I32, imp: day },
        NativeFn { name: "hour", params: &[I64], ret: I32, imp: hour },
        NativeFn { name: "minute", params: &[I64], ret: I32, imp: minute },
        NativeFn { name: "second", params: &[I64], ret: I32, imp: second },
        NativeFn { name: "millisecond", params: &[I64], ret: I32, imp: millisecond },
        NativeFn { name: "weekday", params: &[I64], ret: I32, imp: weekday },
        NativeFn { name: "dayOfYear", params: &[I64], ret: I32, imp: ordinal },
        NativeFn { name: "isoDate", params: &[I64], ret: Str, imp: iso },
        NativeFn { name: "isoDateTime", params: &[I64], ret: Str, imp: iso_datetime },
        NativeFn { name: "parseIsoDateTime", params: &[Str], ret: I64, imp: parse_iso_datetime },
        NativeFn { name: "parseIsoDate", params: &[Str], ret: I64, imp: parse_iso_date },
    ],
};
