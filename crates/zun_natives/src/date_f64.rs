//! `zun:date` natives over f64 time values. Invalid Date is NaN, and every
//! function here propagates it. These back the ECMAScript `Date` surface in
//! `crates/zun_prelude/builtin/date.zun`.

use crate::date::{local_offset_minutes_at, zone_of};
use crate::{NativeArg, NativeError, NativeValue, Sink};

const MS_PER_DAY: f64 = 86_400_000.0;
const MAX_TIME: f64 = 8.64e15;

fn num(args: &[NativeArg], index: usize) -> f64 {
    match args.get(index) {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => f64::NAN,
    }
}

fn int(args: &[NativeArg], index: usize) -> i64 {
    match args.get(index) {
        Some(NativeArg::Int(value)) => *value,
        Some(NativeArg::Float(value)) => *value as i64,
        _ => 0,
    }
}

/// ECMAScript TimeClip: NaN outside +-8.64e15 ms, otherwise the truncated value (never -0).
fn time_clip(t: f64) -> f64 {
    if t.is_finite() && t.abs() <= MAX_TIME {
        t.trunc() + 0.0
    } else {
        f64::NAN
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// ECMAScript MakeDay (month is zero-based).
fn make_day(year: f64, month: f64, date: f64) -> f64 {
    if !(year.is_finite() && month.is_finite() && date.is_finite()) {
        return f64::NAN;
    }
    let (year, month, date) = (year.trunc(), month.trunc(), date.trunc());
    let ym = year + (month / 12.0).floor();
    if ym.abs() > 400_000.0 {
        return f64::NAN;
    }
    let mn = month.rem_euclid(12.0);
    days_from_civil(ym as i64, mn as i64 + 1, 1) as f64 + date - 1.0
}

/// ECMAScript MakeTime.
fn make_time(hour: f64, minute: f64, second: f64, milli: f64) -> f64 {
    if !(hour.is_finite() && minute.is_finite() && second.is_finite() && milli.is_finite()) {
        return f64::NAN;
    }
    hour.trunc() * 3_600_000.0
        + minute.trunc() * 60_000.0
        + second.trunc() * 1_000.0
        + milli.trunc()
}

/// ECMAScript MakeDate.
fn make_date(day: f64, time: f64) -> f64 {
    let value = day * MS_PER_DAY + time;
    if day.is_finite() && time.is_finite() && value.is_finite() {
        value
    } else {
        f64::NAN
    }
}

/// Broken-down fields of a valid time value: year, month0, date, h, m, s, ms, weekday0.
fn fields_of(t: f64) -> Option<[f64; 8]> {
    if !t.is_finite() {
        return None;
    }
    let value = zun_core::DateValue::from_millis(t as i64);
    let (y, mo, d) = value.utc_date()?;
    let (h, mi, s, ms) = value.utc_time()?;
    let weekday = value.weekday()? % 7;
    Some([
        f64::from(y),
        f64::from(mo) - 1.0,
        f64::from(d),
        f64::from(h),
        f64::from(mi),
        f64::from(s),
        f64::from(ms),
        f64::from(weekday),
    ])
}

pub(crate) fn f_valid(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Bool(num(args, 0).is_finite()))
}

pub(crate) fn f_clip(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Float(time_clip(num(args, 0))))
}

pub(crate) fn f_field(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let which = int(args, 1).clamp(0, 7) as usize;
    Ok(NativeValue::Float(
        fields_of(num(args, 0)).map_or(f64::NAN, |f| f[which]),
    ))
}

/// `Date.UTC` / constructor-from-parts core. The 0-99 year remap applies when the flag is set.
pub(crate) fn f_make_utc(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let mut year = num(args, 0);
    let remap = matches!(args.get(7), Some(NativeArg::Bool(true)));
    if remap && !year.is_nan() {
        let truncated = year.trunc();
        if (0.0..=99.0).contains(&truncated) {
            year = 1900.0 + truncated;
        }
    }
    let day = make_day(year, num(args, 1), num(args, 2));
    let time = make_time(num(args, 3), num(args, 4), num(args, 5), num(args, 6));
    Ok(NativeValue::Float(time_clip(make_date(day, time))))
}

/// Replace the `count` fields starting at `start` (0 year .. 6 ms) of `t` and
/// recompose. A NaN time value is treated as +0 only when the year is set.
pub(crate) fn f_set_fields(
    args: &[NativeArg],
    _: &mut dyn Sink,
) -> Result<NativeValue, NativeError> {
    let t = num(args, 0);
    let start = int(args, 1).clamp(0, 6) as usize;
    let count = int(args, 2).clamp(0, 4) as usize;
    let base = if t.is_nan() {
        if start == 0 {
            0.0
        } else {
            return Ok(NativeValue::Float(f64::NAN));
        }
    } else {
        t
    };
    let Some(mut f) = fields_of(base) else {
        return Ok(NativeValue::Float(f64::NAN));
    };
    for i in 0..count {
        if start + i <= 6 {
            f[start + i] = num(args, 3 + i);
        }
    }
    let day = make_day(f[0], f[1], f[2]);
    let time = make_time(f[3], f[4], f[5], f[6]);
    Ok(NativeValue::Float(time_clip(make_date(day, time))))
}

fn year_text(year: i64) -> String {
    if (0..=9999).contains(&year) {
        format!("{year:04}")
    } else if year < 0 {
        format!("-{:06}", -year)
    } else {
        format!("+{year:06}")
    }
}

pub(crate) fn f_iso_string(
    args: &[NativeArg],
    _: &mut dyn Sink,
) -> Result<NativeValue, NativeError> {
    let f = fields_of(num(args, 0)).ok_or_else(|| NativeError("isoString: invalid date".into()))?;
    Ok(NativeValue::Str(format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year_text(f[0] as i64),
        f[1] as i64 + 1,
        f[2] as i64,
        f[3] as i64,
        f[4] as i64,
        f[5] as i64,
        f[6] as i64
    )))
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn signed_year(year: i64) -> String {
    if year < 0 {
        format!("-{:04}", -year)
    } else {
        format!("{year:04}")
    }
}

pub(crate) fn f_utc_string(
    args: &[NativeArg],
    _: &mut dyn Sink,
) -> Result<NativeValue, NativeError> {
    let f = fields_of(num(args, 0)).ok_or_else(|| NativeError("utcString: invalid date".into()))?;
    Ok(NativeValue::Str(format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        WEEKDAYS[f[7] as usize],
        f[2] as i64,
        MONTHS[f[1] as usize],
        signed_year(f[0] as i64),
        f[3] as i64,
        f[4] as i64,
        f[5] as i64
    )))
}

fn offset_minutes(t: f64) -> i64 {
    if !t.is_finite() {
        return 0;
    }
    i64::from(local_offset_minutes_at(t as i64, &zone_of(&[])))
}

/// `toString` (kind 0), `toDateString` (1), `toTimeString` (2) in the host zone.
pub(crate) fn f_date_string(
    args: &[NativeArg],
    _: &mut dyn Sink,
) -> Result<NativeValue, NativeError> {
    let t = num(args, 0);
    let offset = offset_minutes(t);
    let f = fields_of(t + (offset * 60_000) as f64)
        .ok_or_else(|| NativeError("dateString: invalid date".into()))?;
    let date = format!(
        "{} {} {:02} {}",
        WEEKDAYS[f[7] as usize],
        MONTHS[f[1] as usize],
        f[2] as i64,
        signed_year(f[0] as i64)
    );
    let sign = if offset < 0 { '-' } else { '+' };
    let zone = if offset == 0 {
        " (Coordinated Universal Time)"
    } else {
        ""
    };
    let time = format!(
        "{:02}:{:02}:{:02} GMT{}{:02}{:02}{}",
        f[3] as i64,
        f[4] as i64,
        f[5] as i64,
        sign,
        offset.abs() / 60,
        offset.abs() % 60,
        zone
    );
    Ok(NativeValue::Str(match int(args, 1) {
        1 => date,
        2 => time,
        _ => format!("{date} {time}"),
    }))
}

pub(crate) fn f_local_time(
    args: &[NativeArg],
    _: &mut dyn Sink,
) -> Result<NativeValue, NativeError> {
    let t = num(args, 0);
    Ok(NativeValue::Float(if t.is_finite() {
        t + (offset_minutes(t) * 60_000) as f64
    } else {
        f64::NAN
    }))
}

fn utc_from_local(local: f64) -> f64 {
    if !local.is_finite() || local.abs() > MAX_TIME + MS_PER_DAY * 2.0 {
        return time_clip(local);
    }
    let converted = jiff::Timestamp::from_millisecond(local as i64)
        .ok()
        .and_then(|ts| {
            let civil = ts.to_zoned(jiff::tz::TimeZone::UTC).datetime();
            zone_of(&[])
                .to_zoned(civil)
                .ok()
                .map(|z| z.timestamp().as_millisecond() as f64)
        });
    time_clip(converted.unwrap_or(local))
}

pub(crate) fn f_utc_from_local(
    args: &[NativeArg],
    _: &mut dyn Sink,
) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Float(utc_from_local(num(args, 0))))
}

pub(crate) fn f_tz_offset(
    args: &[NativeArg],
    _: &mut dyn Sink,
) -> Result<NativeValue, NativeError> {
    let t = num(args, 0);
    Ok(NativeValue::Float(if t.is_finite() {
        (-offset_minutes(t)) as f64 + 0.0
    } else {
        f64::NAN
    }))
}

/// ISO 8601 / ECMAScript date-time string format. Date-only forms are UTC;
/// date-time forms without an offset are local time.
fn parse_iso(text: &str) -> Option<f64> {
    let b = text.trim().as_bytes();
    let mut i = 0usize;
    let digits = |i: &mut usize, n: usize| -> Option<i64> {
        if *i + n > b.len() || !b[*i..*i + n].iter().all(u8::is_ascii_digit) {
            return None;
        }
        let v = std::str::from_utf8(&b[*i..*i + n]).ok()?.parse().ok()?;
        *i += n;
        Some(v)
    };
    let year = match b.first()? {
        b'+' | b'-' => {
            let negative = b[0] == b'-';
            i = 1;
            let v = digits(&mut i, 6)?;
            if negative && v == 0 {
                return None;
            }
            if negative {
                -v
            } else {
                v
            }
        }
        _ => digits(&mut i, 4)?,
    };
    let (mut month, mut day) = (1i64, 1i64);
    if i < b.len() && b[i] == b'-' {
        i += 1;
        month = digits(&mut i, 2)?;
        if i < b.len() && b[i] == b'-' {
            i += 1;
            day = digits(&mut i, 2)?;
        }
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (mut hour, mut minute, mut second, mut milli) = (0i64, 0i64, 0i64, 0i64);
    let mut offset: Option<i64> = None;
    let mut has_time = false;
    if i < b.len() {
        if !matches!(b[i], b'T' | b't' | b' ') {
            return None;
        }
        i += 1;
        has_time = true;
        hour = digits(&mut i, 2)?;
        if i >= b.len() || b[i] != b':' {
            return None;
        }
        i += 1;
        minute = digits(&mut i, 2)?;
        if i < b.len() && b[i] == b':' {
            i += 1;
            second = digits(&mut i, 2)?;
            if i < b.len() && (b[i] == b'.' || b[i] == b',') {
                i += 1;
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i == start {
                    return None;
                }
                let mut frac: String = std::str::from_utf8(&b[start..i])
                    .ok()?
                    .chars()
                    .take(3)
                    .collect();
                while frac.len() < 3 {
                    frac.push('0');
                }
                milli = frac.parse().ok()?;
            }
        }
        if i < b.len() {
            match b[i] {
                b'Z' | b'z' => {
                    offset = Some(0);
                    i += 1;
                }
                b'+' | b'-' => {
                    let sign = if b[i] == b'-' { -1 } else { 1 };
                    i += 1;
                    let oh = digits(&mut i, 2)?;
                    if i < b.len() && b[i] == b':' {
                        i += 1;
                    }
                    let om = digits(&mut i, 2)?;
                    if oh > 23 || om > 59 {
                        return None;
                    }
                    offset = Some(sign * (oh * 60 + om));
                }
                _ => return None,
            }
        }
        if i != b.len() {
            return None;
        }
    }
    if hour > 24 || minute > 59 || second > 59 || (hour == 24 && (minute | second | milli) != 0) {
        return None;
    }
    let days = days_from_civil(year, month, day) as f64;
    let local =
        days * MS_PER_DAY + (hour * 3_600_000 + minute * 60_000 + second * 1_000 + milli) as f64;
    Some(match offset {
        Some(minutes) => time_clip(local - (minutes * 60_000) as f64),
        None if has_time => utc_from_local(local),
        None => time_clip(local),
    })
}

pub(crate) fn f_parse_date(
    args: &[NativeArg],
    _: &mut dyn Sink,
) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => *value,
        _ => return Ok(NativeValue::Float(f64::NAN)),
    };
    Ok(NativeValue::Float(parse_iso(value).unwrap_or(f64::NAN)))
}
