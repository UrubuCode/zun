// Date construction, epoch arithmetic, ISO 8601 parsing and serialization.
// Every instant is a fixed UTC value: no clock, no timezone, no locale.

function iso(ms: number): string {
  return new Date(ms).toISOString();
}

// ---------------------------------------------------------------- epoch
const epoch = new Date(0);
console.log("epoch_time=" + epoch.getTime());
console.log("epoch_valueOf=" + epoch.valueOf());
console.log("epoch_iso=" + epoch.toISOString());
console.log("epoch_json=" + epoch.toJSON());
console.log("epoch_parse=" + Date.parse("1970-01-01T00:00:00.000Z"));
console.log("epoch_utc=" + Date.UTC(1970, 0, 1));
console.log("pre_epoch_iso=" + iso(-1));
console.log("pre_epoch_parse=" + Date.parse("1969-12-31T23:59:59.999Z"));
console.log("one_day_before=" + iso(-86400000));
console.log("y2k_ms=" + Date.UTC(2000, 0, 1));
console.log("y2k_iso=" + iso(946684800000));
console.log("leap_second_day=" + iso(Date.UTC(2016, 11, 31, 23, 59, 59, 999)));

// ------------------------------------------------------------ Date.UTC
console.log("utc_y=" + Date.UTC(2024));
console.log("utc_ym=" + Date.UTC(2024, 5));
console.log("utc_ymd=" + Date.UTC(2024, 5, 10));
console.log("utc_ymdh=" + Date.UTC(2024, 5, 10, 7));
console.log("utc_ymdhm=" + Date.UTC(2024, 5, 10, 7, 8));
console.log("utc_ymdhms=" + Date.UTC(2024, 5, 10, 7, 8, 9));
console.log("utc_full=" + Date.UTC(2024, 5, 10, 7, 8, 9, 10));
console.log("utc_defaults_equal=" + (Date.UTC(2024) === Date.UTC(2024, 0, 1, 0, 0, 0, 0)));
console.log("utc_zero_args_nan=" + Number.isNaN(Date.UTC()));
console.log("utc_extra_ignored=" + ((Date.UTC as any)(2024, 0, 1, 0, 0, 0, 0, 99) === Date.UTC(2024, 0, 1)));
console.log("utc_frac_trunc=" + (Date.UTC(2024, 0, 1, 5.9) === Date.UTC(2024, 0, 1, 5)));
console.log("utc_frac_year_trunc=" + new Date(Date.UTC(95.9, 0, 1)).getUTCFullYear());
console.log("utc_nan_month=" + Number.isNaN(Date.UTC(2024, NaN)));
console.log("utc_nan_day=" + Number.isNaN(Date.UTC(2024, 0, NaN)));
console.log("utc_nan_ms=" + Number.isNaN(Date.UTC(2024, 0, 1, 0, 0, 0, NaN)));
console.log("utc_infinity_nan=" + Number.isNaN(Date.UTC(Infinity, 0)));

// Years 0..99 are mapped to 1900..1999 by Date.UTC only; 100 and negatives are not.
console.log("y0=" + iso(Date.UTC(0, 0, 1)));
console.log("y70=" + iso(Date.UTC(70, 0, 1)));
console.log("y99=" + iso(Date.UTC(99, 11, 31)));
console.log("y100=" + iso(Date.UTC(100, 0, 1)));
console.log("y_neg1=" + iso(Date.UTC(-1, 0, 1)));

// ------------------------------------------------- constructor rollover
console.log("m12=" + iso(Date.UTC(2020, 12, 1)));
console.log("m24=" + iso(Date.UTC(2020, 24, 1)));
console.log("m_neg1=" + iso(Date.UTC(2020, -1, 1)));
console.log("m_neg13=" + iso(Date.UTC(2020, -13, 1)));
console.log("d0_jan=" + iso(Date.UTC(2020, 0, 0)));
console.log("d0_mar=" + iso(Date.UTC(2020, 2, 0)));
console.log("d_neg1=" + iso(Date.UTC(2020, 0, -1)));
console.log("jan32=" + iso(Date.UTC(2020, 0, 32)));
console.log("feb30_leap=" + iso(Date.UTC(2020, 1, 30)));
console.log("feb30_nonleap=" + iso(Date.UTC(2021, 1, 30)));
console.log("apr31=" + iso(Date.UTC(2020, 3, 31)));
console.log("d366=" + iso(Date.UTC(2020, 0, 366)));
console.log("d367=" + iso(Date.UTC(2020, 0, 367)));
console.log("h24=" + iso(Date.UTC(2020, 0, 1, 24)));
console.log("h_neg1=" + iso(Date.UTC(2020, 0, 1, -1)));
console.log("min60=" + iso(Date.UTC(2020, 0, 1, 0, 60)));
console.log("sec60=" + iso(Date.UTC(2020, 0, 1, 0, 0, 60)));
console.log("ms1000=" + iso(Date.UTC(2020, 0, 1, 0, 0, 0, 1000)));
console.log("ms_neg1=" + iso(Date.UTC(2020, 0, 1, 0, 0, 0, -1)));
console.log("cascade=" + iso(Date.UTC(2020, 11, 31, 23, 59, 59, 1000)));
console.log("leap1900_is_mar1=" + iso(Date.UTC(1900, 1, 29)));
console.log("leap2000=" + iso(Date.UTC(2000, 1, 29)));
console.log("leap2020=" + iso(Date.UTC(2020, 1, 29)));

// ------------------------------------------- extended-year ISO strings
// setUTCFullYear does no 0..99 remapping, so it is the way to reach small years.
function yearIso(y: number): string {
  const d = new Date(0);
  d.setUTCFullYear(y, 0, 1);
  d.setUTCHours(0, 0, 0, 0);
  return d.toISOString();
}
console.log("ext_y1=" + yearIso(1));
console.log("ext_y12=" + yearIso(12));
console.log("ext_y123=" + yearIso(123));
console.log("ext_y1234=" + yearIso(1234));
console.log("ext_y9999=" + yearIso(9999));
console.log("ext_y0=" + yearIso(0));
console.log("ext_y_neg1=" + yearIso(-1));
console.log("ext_y10000=" + yearIso(10000));
console.log("ext_y123456=" + yearIso(123456));
console.log("ext_utc_remaps=" + iso(Date.UTC(1, 0, 1)));

const neg = new Date(0);
neg.setUTCFullYear(-12345, 5, 6);
neg.setUTCHours(1, 2, 3, 4);
console.log("ext_neg12345=" + neg.toISOString());
console.log("ext_neg12345_year=" + neg.getUTCFullYear());

// Range: +-8.64e15 ms is the last valid instant; one more is Invalid Date.
console.log("max_iso=" + iso(8.64e15));
console.log("min_iso=" + iso(-8.64e15));
console.log("max_time=" + new Date(8.64e15).getTime());
console.log("past_max_nan=" + Number.isNaN(new Date(8.64e15 + 1).getTime()));
console.log("past_min_nan=" + Number.isNaN(new Date(-8.64e15 - 1).getTime()));
console.log("setTime_past_max_nan=" + Number.isNaN(new Date(0).setTime(8.64e15 + 1)));
console.log("setTime_at_max=" + new Date(0).setTime(8.64e15));

// --------------------------------------------- numeric coercion of Date
const d = new Date(Date.UTC(2024, 5, 10, 7, 8, 9, 10));
console.log("getTime=" + d.getTime());
console.log("valueOf=" + d.valueOf());
console.log("Number=" + Number(d));
console.log("unary_plus=" + +d);
console.log("agree_all=" + (d.getTime() === d.valueOf() && Number(d) === d.getTime() && +d === d.getTime()));
console.log("times_one=" + (d as any) * 1);
console.log("div_1000=" + (d as any) / 1000);
console.log("math_floor_s=" + Math.floor((d as any) / 1000));
console.log("is_integer=" + Number.isInteger(Number(d)));
console.log("frac_ms_trunc=" + new Date(1.9).getTime());
console.log("frac_neg_ms_trunc=" + new Date(-1.9).getTime());
console.log("copy_from_time=" + (new Date(d.getTime()).getTime() === d.getTime()));
console.log("roundtrip_number=" + (new Date(Number(d)).getTime() === d.getTime()));
console.log("copy_from_date=" + (new Date(d).getTime() === d.getTime()));

// ----------------------------------------------------------- Date.parse
console.log("parse_date_only=" + Date.parse("2024-01-15"));
console.log("parse_year_month=" + Date.parse("2024-01"));
console.log("parse_year_only=" + Date.parse("2024"));
console.log("parse_z=" + Date.parse("2024-01-15T10:30:00Z"));
console.log("parse_z_ms=" + Date.parse("2024-01-15T10:30:00.123Z"));
console.log("parse_z_no_sec=" + Date.parse("2024-01-15T10:30Z"));
console.log("parse_frac_1digit=" + iso(Date.parse("2024-01-15T10:30:00.1Z")));
console.log("parse_frac_2digit=" + iso(Date.parse("2024-01-15T10:30:00.12Z")));
console.log("parse_plus0_eq_z=" + (Date.parse("2024-01-15T10:30:00+00:00") === Date.parse("2024-01-15T10:30:00Z")));
console.log("parse_minus0_eq_z=" + (Date.parse("2024-01-15T10:30:00-00:00") === Date.parse("2024-01-15T10:30:00Z")));
console.log("parse_plus1=" + iso(Date.parse("2024-01-15T10:30:00+01:00")));
console.log("parse_plus0530=" + iso(Date.parse("2024-01-15T10:30:00+05:30")));
console.log("parse_plus14=" + iso(Date.parse("2024-01-15T10:30:00+14:00")));
console.log("parse_minus3=" + iso(Date.parse("2024-01-15T10:30:00-03:00")));
console.log("parse_minus0930=" + iso(Date.parse("2024-01-15T10:30:00-09:30")));
console.log("parse_offset_delta=" + (Date.parse("2024-01-15T10:30:00Z") - Date.parse("2024-01-15T10:30:00+01:00")));
console.log("parse_roll_next_day=" + iso(Date.parse("2024-01-15T23:30:00-02:00")));
console.log("parse_roll_prev_day=" + iso(Date.parse("2024-01-15T00:30:00+02:00")));
console.log("parse_leap_day=" + iso(Date.parse("2020-02-29T12:00:00Z")));
console.log("parse_expanded_plus=" + Date.parse("+010000-01-01T00:00:00.000Z"));
console.log("parse_expanded_minus=" + Date.parse("-000001-01-01T00:00:00.000Z"));
console.log("parse_expanded_zero=" + Date.parse("+000000-01-01T00:00:00.000Z"));
console.log("parse_minus_zero_year_nan=" + Number.isNaN(Date.parse("-000000-01-01T00:00:00.000Z")));
console.log("parse_max=" + (Date.parse("+275760-09-13T00:00:00.000Z") === 8.64e15));
console.log("parse_min=" + (Date.parse("-271821-04-20T00:00:00.000Z") === -8.64e15));
console.log("parse_past_max_nan=" + Number.isNaN(Date.parse("+275760-09-14T00:00:00.000Z")));
console.log("ctor_eq_parse=" + (new Date("2024-01-15T10:30:00Z").getTime() === Date.parse("2024-01-15T10:30:00Z")));

console.log("bad_garbage_nan=" + Number.isNaN(Date.parse("not-a-date")));
console.log("bad_empty_nan=" + Number.isNaN(Date.parse("")));
console.log("bad_month_nan=" + Number.isNaN(Date.parse("2024-13-01T00:00:00Z")));
console.log("bad_day_nan=" + Number.isNaN(Date.parse("2024-01-32T00:00:00Z")));
console.log("bad_hour_nan=" + Number.isNaN(Date.parse("2024-01-15T25:00:00Z")));
console.log("bad_nonleap_feb29_nan=" + Number.isNaN(Date.parse("2021-02-29T00:00:00Z")));

// ------------------------------------------------ serialization forms
const stamp = new Date(Date.UTC(2024, 0, 15, 10, 30, 45, 123));
console.log("iso=" + stamp.toISOString());
console.log("json=" + stamp.toJSON());
console.log("json_eq_iso=" + (stamp.toJSON() === stamp.toISOString()));
console.log("stringify=" + JSON.stringify(stamp));
console.log("stringify_nested=" + JSON.stringify({ at: stamp }));
console.log("stringify_array=" + JSON.stringify([stamp, stamp]));
console.log("iso_roundtrip=" + (Date.parse(stamp.toISOString()) === stamp.getTime()));
console.log("iso_2000=" + iso(Date.UTC(2000, 11, 31, 23, 59, 59, 999)));
console.log("iso_2024_leap=" + iso(Date.UTC(2024, 1, 29, 12, 34, 56, 789)));

const round = [neg, new Date(0), new Date(8.64e15), new Date(-8.64e15)];
console.log("rt_extended=" + round.map((x) => Date.parse(x.toISOString()) === x.getTime()).join(","));
console.log("json_max=" + new Date(8.64e15).toJSON());
console.log("json_past_max=" + new Date(8.64e15 + 1).toJSON());
console.log("json_neg=" + JSON.stringify({ at: new Date(Date.UTC(-1, 0, 1)) }));

// Invalid Date: toJSON is null, toISOString throws RangeError.
const bad = new Date(NaN);
console.log("invalid_json=" + bad.toJSON());
console.log("invalid_json_null=" + (bad.toJSON() === null));
console.log("invalid_stringify=" + JSON.stringify(bad));
console.log("invalid_in_array=" + JSON.stringify([bad]));
try {
  bad.toISOString();
  console.log("invalid_iso_threw=false");
} catch (e) {
  console.log("invalid_iso_threw=true");
  console.log("invalid_iso_name=" + (e as Error).name);
  console.log("invalid_iso_is_range=" + (e instanceof RangeError));
}
const fromBad = new Date("not-a-date");
console.log("bad_string_json_null=" + (fromBad.toJSON() === null));
console.log("bad_string_time_nan=" + Number.isNaN(fromBad.getTime()));
