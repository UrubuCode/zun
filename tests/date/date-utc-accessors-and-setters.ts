// UTC accessors and mutators: every getUTC* reads one broken-down field of the
// same instant, every setUTC* writes fields with arity-dependent scope, rolls
// out-of-range values forward, and treats a NaN argument as poisoning the date.

function iso(d: Date): string {
  return Number.isNaN(d.getTime()) ? "Invalid" : d.toISOString();
}

function parts(d: Date): string {
  return [
    d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate(), d.getUTCDay(),
    d.getUTCHours(), d.getUTCMinutes(), d.getUTCSeconds(), d.getUTCMilliseconds(),
  ].join("/");
}

// ------------------------------------------------------------ accessors
const d = new Date(Date.UTC(2024, 0, 15, 10, 30, 45, 123));
console.log("utc_year=" + d.getUTCFullYear());
console.log("utc_month=" + d.getUTCMonth());
console.log("utc_date=" + d.getUTCDate());
console.log("utc_day=" + d.getUTCDay());
console.log("utc_hours=" + d.getUTCHours());
console.log("utc_minutes=" + d.getUTCMinutes());
console.log("utc_seconds=" + d.getUTCSeconds());
console.log("utc_ms=" + d.getUTCMilliseconds());
console.log("parts=" + parts(d));

// Conventions: month is zero-based, date is one-based, weekday is zero-based from Sunday.
const conv = new Date(Date.UTC(2026, 7, 16, 0, 0, 0));
console.log("conventions=" + conv.getUTCMonth() + "/" + conv.getUTCDate() + "/" + conv.getUTCDay() + " iso=" + iso(conv));

// Accessors of the same instant agree with what the constructor was given.
const probes: [number, number, number, number, number, number, number][] = [
  [1970, 0, 1, 0, 0, 0, 0],
  [1969, 11, 31, 23, 59, 59, 999],
  [2000, 0, 1, 0, 0, 0, 0],
  [2024, 1, 29, 12, 34, 56, 789],
  [2023, 1, 28, 23, 59, 59, 999],
  [1999, 11, 31, 23, 59, 59, 999],
  [-1, 0, 1, 0, 0, 0, 0],
  [275760, 8, 13, 0, 0, 0, 0],
];
for (const p of probes) {
  const x = new Date(Date.UTC(p[0], p[1], p[2], p[3], p[4], p[5], p[6]));
  const same =
    x.getUTCFullYear() === p[0] && x.getUTCMonth() === p[1] && x.getUTCDate() === p[2] &&
    x.getUTCHours() === p[3] && x.getUTCMinutes() === p[4] && x.getUTCSeconds() === p[5] &&
    x.getUTCMilliseconds() === p[6];
  console.log("probe_" + p[0] + "_" + p[1] + "_" + p[2] + "=" + same + " day=" + x.getUTCDay());
}

// The weekday cycle is continuous across the epoch.
const run: number[] = [];
for (let i = -3; i <= 3; i++) run.push(new Date(i * 86400000).getUTCDay());
console.log("weekday_run=" + run.join(","));

// ---------------------------------------------------- setUTC* arity
function after(label: string, f: (x: Date) => number): void {
  const x = new Date(Date.UTC(2024, 5, 15, 10, 20, 30, 400));
  const r = f(x);
  console.log(label + " ret=" + (Number.isNaN(r) ? "NaN" : String(r)) + " iso=" + iso(x));
}

after("set_year_1", (x) => x.setUTCFullYear(2030));
after("set_year_2", (x) => x.setUTCFullYear(2030, 0));
after("set_year_3", (x) => x.setUTCFullYear(2030, 0, 2));
after("set_month_1", (x) => x.setUTCMonth(0));
after("set_month_2", (x) => x.setUTCMonth(0, 31));
after("set_date_1", (x) => x.setUTCDate(1));
after("set_hours_1", (x) => x.setUTCHours(1));
after("set_hours_4", (x) => x.setUTCHours(1, 2, 3, 4));
after("set_minutes_3", (x) => x.setUTCMinutes(1, 2, 3));
after("set_seconds_2", (x) => x.setUTCSeconds(1, 2));
after("set_ms", (x) => x.setUTCMilliseconds(999));
after("set_year_extra_ignored", (x) => (x.setUTCFullYear as any)(2030, 0, 2, 23));
after("set_hours_extra_ignored", (x) => (x.setUTCHours as any)(1, 2, 3, 4, 5));

// ------------------------------------------------------- rollover
after("month12", (x) => x.setUTCMonth(12));
after("month13_date40", (x) => x.setUTCMonth(13, 40));
after("month_neg1", (x) => x.setUTCMonth(-1));
after("date0", (x) => x.setUTCDate(0));
after("date32", (x) => x.setUTCDate(32));
after("date_neg5", (x) => x.setUTCDate(-5));
after("hours25", (x) => x.setUTCHours(25));
after("hours_neg1", (x) => x.setUTCHours(-1));
after("hours_24_60_60_1000", (x) => x.setUTCHours(24, 60, 60, 1000));
after("ms_huge", (x) => x.setUTCMilliseconds(86400000));
after("seconds_neg", (x) => x.setUTCSeconds(-1));
after("minutes_frac_neg", (x) => x.setUTCMinutes(-1.5));
after("hours_fraction_truncates", (x) => x.setUTCHours(3.9, 6.9, 7.9, 8.9));
after("string_date", (x) => x.setUTCDate("15" as any));
after("bool_month", (x) => x.setUTCMonth(true as any));
after("object_month", (x) => x.setUTCMonth({ valueOf: () => 2 } as any));
after("null_month", (x) => x.setUTCMonth(null as any));

// ------------------------------------------------- NaN and undefined poisoning
after("nan_month", (x) => x.setUTCMonth(NaN));
after("nan_then_date", (x) => { x.setUTCMonth(NaN); return x.setUTCDate(1); });
after("nan_then_time", (x) => { x.setUTCMonth(NaN); return x.setTime(0); });
after("nan_then_fullyear", (x) => { x.setUTCMonth(NaN); return x.setUTCFullYear(2000, 0, 1); });
after("undefined_month", (x) => x.setUTCMonth(undefined as any));
after("no_arg_month", (x) => (x.setUTCMonth as any)());
after("explicit_undefined_slot", (x) => (x.setUTCHours as any)(1, undefined));
after("nan_minutes_ret", (x) => x.setUTCMinutes(NaN));

// A poisoned date is revived only by setUTCFullYear (time value treated as +0) and setTime.
const dead = new Date(NaN);
dead.setUTCMonth(5, 10);
console.log("dead_after_month=" + Number.isNaN(dead.getTime()));
dead.setUTCHours(1, 2, 3, 4);
console.log("dead_after_hours=" + Number.isNaN(dead.getTime()));
dead.setUTCFullYear(2020, 1, 29);
console.log("dead_revived=" + iso(dead));
console.log("dead_revived_time=" + dead.getUTCHours() + ":" + dead.getUTCMinutes() + ":" + dead.getUTCSeconds());
const dead2 = new Date(NaN);
console.log("dead2_setTime=" + dead2.setTime(1000));

// Invalid Date answers NaN from every accessor.
const bad = new Date(NaN);
const vals = [bad.getTime(), bad.getUTCFullYear(), bad.getUTCMonth(), bad.getUTCDate(), bad.getUTCDay(), bad.getUTCHours(), bad.getUTCMilliseconds()];
console.log("invalid_all_nan=" + vals.every((v) => Number.isNaN(v)));

// ------------------------------------------- setter arity and return value
console.log("arities=" + [
  Date.prototype.setUTCFullYear.length, Date.prototype.setUTCMonth.length,
  Date.prototype.setUTCDate.length, Date.prototype.setUTCHours.length,
  Date.prototype.setUTCMinutes.length, Date.prototype.setUTCSeconds.length,
  Date.prototype.setUTCMilliseconds.length, Date.prototype.setTime.length,
].join(","));

const r = new Date(0);
const ret = r.setUTCFullYear(2001);
console.log("returns_time=" + (ret === r.getTime()) + " v=" + ret);
const r2 = new Date(Date.UTC(2024, 0, 1));
console.log("setTime_returns=" + r2.setTime(0) + " after=" + iso(r2));
console.log("setTime_fraction=" + new Date(0).setTime(1.9));

// ------------------------------------------------ sequential mutation chains
const build = new Date(Date.UTC(2020, 0, 1, 0, 0, 0, 0));
build.setUTCFullYear(2024);
build.setUTCMonth(4);
build.setUTCDate(10);
build.setUTCHours(12);
build.setUTCMinutes(34);
build.setUTCSeconds(56);
build.setUTCMilliseconds(789);
console.log("chain_iso=" + iso(build));
console.log("chain_parts=" + parts(build));

const later = new Date(Date.UTC(2024, 0, 1, 0, 0, 0, 0));
later.setUTCMonth(5);
later.setUTCDate(15);
later.setUTCHours(8);
console.log("chain2=" + iso(later) + " month=" + later.getUTCMonth() + " date=" + later.getUTCDate());

// Month arithmetic rolls over rather than clamping: Jan 31 + 1 month is Mar 2 (2024).
const jan31 = new Date(Date.UTC(2024, 0, 31));
jan31.setUTCMonth(jan31.getUTCMonth() + 1);
console.log("jan31_plus_month=" + iso(jan31));

// Walk backwards across a year boundary.
const back = new Date(Date.UTC(2024, 0, 1));
back.setUTCDate(back.getUTCDate() - 1);
console.log("back_one_day=" + iso(back));
back.setUTCHours(back.getUTCHours() - 25);
console.log("back_25h=" + iso(back));

// ------------------------------------------------------ day arithmetic
const a = new Date(Date.UTC(2024, 0, 1));
const b = new Date(Date.UTC(2024, 0, 2));
console.log("diff_ms=" + (b.getTime() - a.getTime()));
console.log("diff_minus_op=" + ((b as any) - (a as any)));
console.log("diff_days=" + (b.getTime() - a.getTime()) / 86400000);
console.log("diff_reverse=" + (a.getTime() - b.getTime()));

const leapGap = (new Date(Date.UTC(2020, 2, 1)).getTime() - new Date(Date.UTC(2020, 1, 28)).getTime()) / 86400000;
const plainGap = (new Date(Date.UTC(2021, 2, 1)).getTime() - new Date(Date.UTC(2021, 1, 28)).getTime()) / 86400000;
console.log("leap_gap_days=" + leapGap);
console.log("nonleap_gap_days=" + plainGap);
console.log("year2020_days=" + (Date.UTC(2021, 0, 1) - Date.UTC(2020, 0, 1)) / 86400000);
console.log("year2021_days=" + (Date.UTC(2022, 0, 1) - Date.UTC(2021, 0, 1)) / 86400000);

const walk = new Date(Date.UTC(2024, 0, 30));
const seen: string[] = [];
for (let i = 0; i < 4; i++) {
  seen.push(walk.toISOString().slice(0, 10));
  walk.setUTCDate(walk.getUTCDate() + 1);
}
console.log("walk=" + seen.join("|"));

const add45 = new Date(Date.UTC(2024, 0, 1));
add45.setUTCDate(add45.getUTCDate() + 45);
console.log("add45=" + iso(add45));

const sub10 = new Date(Date.UTC(2024, 2, 5));
sub10.setUTCDate(sub10.getUTCDate() - 10);
console.log("sub10=" + iso(sub10));

const hourStep = new Date(Date.UTC(2024, 0, 1));
hourStep.setTime(hourStep.getTime() + 3600000);
console.log("plus_hour=" + iso(hourStep));
