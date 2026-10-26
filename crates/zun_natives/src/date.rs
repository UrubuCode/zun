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
    Ok(NativeValue::Int(
        date(args, "year")?
            .utc_date()
            .ok_or_else(|| NativeError("year: invalid date".into()))?
            .0 as i64,
    ))
}

fn month(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(
        date(args, "month")?
            .utc_date()
            .ok_or_else(|| NativeError("month: invalid date".into()))?
            .1 as i64,
    ))
}

fn day(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(
        date(args, "day")?
            .utc_date()
            .ok_or_else(|| NativeError("day: invalid date".into()))?
            .2 as i64,
    ))
}

fn hour(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(
        date(args, "hour")?
            .utc_time()
            .ok_or_else(|| NativeError("hour: invalid date".into()))?
            .0 as i64,
    ))
}

fn minute(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(
        date(args, "minute")?
            .utc_time()
            .ok_or_else(|| NativeError("minute: invalid date".into()))?
            .1 as i64,
    ))
}

fn second(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(
        date(args, "second")?
            .utc_time()
            .ok_or_else(|| NativeError("second: invalid date".into()))?
            .2 as i64,
    ))
}

fn millisecond(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(
        date(args, "millisecond")?
            .utc_time()
            .ok_or_else(|| NativeError("millisecond: invalid date".into()))?
            .3 as i64,
    ))
}

fn weekday(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(
        date(args, "weekday")?
            .weekday()
            .ok_or_else(|| NativeError("weekday: invalid date".into()))? as i64,
    ))
}

fn ordinal(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Int(
        date(args, "dayOfYear")?
            .ordinal()
            .ok_or_else(|| NativeError("dayOfYear: invalid date".into()))? as i64,
    ))
}

fn iso(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    date(args, "iso")?
        .iso_date()
        .map(NativeValue::Str)
        .ok_or_else(|| NativeError("iso: invalid date".into()))
}

fn iso_datetime(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    date(args, "isoDateTime")?
        .iso_datetime()
        .map(NativeValue::Str)
        .ok_or_else(|| NativeError("isoDateTime: invalid date".into()))
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

/// Compose a timestamp from UTC components. `month` is 1-based, like the
/// value `month()` returns; `Date.UTC` converts from JS's 0-based form.
fn from_utc(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let at = |index: usize| -> Result<i64, NativeError> {
        match args.get(index) {
            Some(NativeArg::Int(value)) => Ok(*value),
            Some(NativeArg::Float(value)) => Ok(*value as i64),
            _ => Err(NativeError("fromUTC: expected seven integers".into())),
        }
    };
    let year = i32::try_from(at(0)?)
        .map_err(|_| NativeError("fromUTC: year out of range".into()))?;
    let value = zun_core::DateValue::from_utc(year, at(1)?, at(2)?, at(3)?, at(4)?, at(5)?, at(6)?)
        .and_then(|date| date.millis())
        .ok_or_else(|| NativeError("fromUTC: date out of range".into()))?;
    Ok(NativeValue::Int(value))
}

/// `new Intl.DateTimeFormat(locale).format(d)` — runtime-locale ICU4X
/// date+time formatting (medium length, UTC).
fn intl_date_time_format(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    use icu::datetime::{fieldsets, DateTimeFormatter};
    let name = "Intl.DateTimeFormat.format";
    let Some(NativeArg::Str(locale)) = args.first() else {
        return Err(NativeError(format!("{name}: expected (string, date)")));
    };
    let value = date(&args[1..], name)?;
    let invalid = || NativeError(format!("{name}: invalid date"));
    let (y, mo, d) = value.utc_date().ok_or_else(invalid)?;
    let (h, mi, s, ms) = value.utc_time().ok_or_else(invalid)?;
    let input = icu::time::DateTime {
        date: icu::calendar::Date::try_new_iso(y, mo as u8, d as u8)
            .map_err(|e| NativeError(format!("{name}: {e}")))?,
        time: icu::time::Time::try_new(h as u8, mi as u8, s as u8, ms * 1_000_000)
            .map_err(|e| NativeError(format!("{name}: {e}")))?,
    };
    let locale = crate::string::intl_locale(locale);
    let formatter = DateTimeFormatter::try_new((&locale).into(), fieldsets::YMDT::medium())
        .map_err(|e| NativeError(format!("{name}: {e}")))?;
    Ok(NativeValue::Str(formatter.format(&input).to_string()))
}

pub const MODULE: crate::Module = crate::Module {
    name: "date",
    functions: &[
        NativeFn {
            name: "isValid",
            params: &[I64],
            ret: Bool,
            imp: is_valid,
        },
        NativeFn {
            name: "year",
            params: &[I64],
            ret: I32,
            imp: year,
        },
        NativeFn {
            name: "month",
            params: &[I64],
            ret: I32,
            imp: month,
        },
        NativeFn {
            name: "day",
            params: &[I64],
            ret: I32,
            imp: day,
        },
        NativeFn {
            name: "hour",
            params: &[I64],
            ret: I32,
            imp: hour,
        },
        NativeFn {
            name: "minute",
            params: &[I64],
            ret: I32,
            imp: minute,
        },
        NativeFn {
            name: "second",
            params: &[I64],
            ret: I32,
            imp: second,
        },
        NativeFn {
            name: "millisecond",
            params: &[I64],
            ret: I32,
            imp: millisecond,
        },
        NativeFn {
            name: "weekday",
            params: &[I64],
            ret: I32,
            imp: weekday,
        },
        NativeFn {
            name: "dayOfYear",
            params: &[I64],
            ret: I32,
            imp: ordinal,
        },
        NativeFn {
            name: "isoDate",
            params: &[I64],
            ret: Str,
            imp: iso,
        },
        NativeFn {
            name: "isoDateTime",
            params: &[I64],
            ret: Str,
            imp: iso_datetime,
        },
        NativeFn {
            name: "parseIsoDateTime",
            params: &[Str],
            ret: I64,
            imp: parse_iso_datetime,
        },
        NativeFn {
            name: "parseIsoDate",
            params: &[Str],
            ret: I64,
            imp: parse_iso_date,
        },
        NativeFn {
            name: "fromUTC",
            params: &[I64, I64, I64, I64, I64, I64, I64],
            ret: I64,
            imp: from_utc,
        },
        NativeFn { name: "dateToLocaleString", params: &[I64, Str], ret: Str, imp: to_locale_string },
        NativeFn { name: "dateToLocaleDateString", params: &[I64, Str], ret: Str, imp: to_locale_date_string },
        NativeFn { name: "dateToLocaleTimeString", params: &[I64, Str], ret: Str, imp: to_locale_time_string },
        NativeFn { name: "intlDateTimeFormat", params: &[Str, I64], ret: Str, imp: intl_date_time_format },
        NativeFn { name: "localOffsetMinutes", params: &[I64, Str], ret: I32, imp: local_offset_minutes },
        NativeFn { name: "localToUtc", params: &[I64, Str], ret: I64, imp: local_to_utc },
    ],
};

/// Which ICU field set a `toLocale*String` native formats with.
#[derive(Clone, Copy)]
enum LocaleKind {
    DateTime,
    Date,
    Time,
}

fn locale_format(args: &[NativeArg], name: &str, kind: LocaleKind) -> Result<NativeValue, NativeError> {
    use icu::datetime::{fieldsets, DateTimeFormatter};
    let value = date(args, name)?;
    let invalid = || NativeError(format!("{name}: invalid date"));
    let (y, mo, d) = value.utc_date().ok_or_else(invalid)?;
    let (h, mi, s, ms) = value.utc_time().ok_or_else(invalid)?;
    let input = icu::time::DateTime {
        date: icu::calendar::Date::try_new_iso(y, mo as u8, d as u8)
            .map_err(|e| NativeError(format!("{name}: {e}")))?,
        time: icu::time::Time::try_new(h as u8, mi as u8, s as u8, ms * 1_000_000)
            .map_err(|e| NativeError(format!("{name}: {e}")))?,
    };
    let locale = match args.get(1) {
        Some(NativeArg::Str(text)) => crate::string::intl_locale(text),
        _ => icu::locale::Locale::UNKNOWN,
    };
    let prefs = (&locale).into();
    let no_data = |e: icu::datetime::DateTimeFormatterLoadError| NativeError(format!("{name}: {e}"));
    let text = match kind {
        LocaleKind::DateTime => DateTimeFormatter::try_new(prefs, fieldsets::YMDT::medium())
            .map_err(no_data)?
            .format(&input)
            .to_string(),
        LocaleKind::Date => DateTimeFormatter::try_new(prefs, fieldsets::YMD::medium())
            .map_err(no_data)?
            .format(&input)
            .to_string(),
        LocaleKind::Time => DateTimeFormatter::try_new(prefs, fieldsets::T::medium())
            .map_err(no_data)?
            .format(&input)
            .to_string(),
    };
    Ok(NativeValue::Str(text))
}

fn to_locale_string(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    locale_format(args, "dateToLocaleString", LocaleKind::DateTime)
}

fn to_locale_date_string(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    locale_format(args, "dateToLocaleDateString", LocaleKind::Date)
}

fn to_locale_time_string(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    locale_format(args, "dateToLocaleTimeString", LocaleKind::Time)
}

/// Resolve a zone: an IANA id, or the host zone when empty.
fn zone_of(args: &[NativeArg]) -> jiff::tz::TimeZone {
    match args.get(1) {
        Some(NativeArg::Str(id)) if !id.is_empty() => {
            jiff::tz::TimeZone::get(id).unwrap_or(jiff::tz::TimeZone::UTC)
        }
        _ => jiff::tz::TimeZone::system(),
    }
}

/// Signed offset of `zone` from UTC (local - UTC) in minutes at a UTC instant.
/// ECMAScript's `getTimezoneOffset` is the negation of this.
pub fn local_offset_minutes_at(utc_millis: i64, zone: &jiff::tz::TimeZone) -> i32 {
    jiff::Timestamp::from_millisecond(utc_millis)
        .map(|ts| zone.to_offset(ts).seconds() / 60)
        .unwrap_or(0)
}

fn local_offset_minutes(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let utc = millis(args, "localOffsetMinutes")?;
    Ok(NativeValue::Int(local_offset_minutes_at(utc, &zone_of(args)) as i64))
}

/// Interpret `local` (a wall-clock reading encoded as if UTC) in the zone and
/// return the UTC instant (compatible disambiguation for DST gaps/folds).
fn local_to_utc(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let local = millis(args, "localToUtc")?;
    let ts = jiff::Timestamp::from_millisecond(local)
        .map_err(|e| NativeError(format!("localToUtc: {e}")))?;
    let civil = ts.to_zoned(jiff::tz::TimeZone::UTC).datetime();
    let out = zone_of(args)
        .to_zoned(civil)
        .map_err(|e| NativeError(format!("localToUtc: {e}")))?;
    Ok(NativeValue::Int(out.timestamp().as_millisecond()))
}
