// The proleptic Gregorian calendar as the Date object implements it (weekdays,
// leap years, month lengths, 400-year cycle), plus how Dates compare, coerce
// to primitives and serialize in UTC. No local-time accessors, no clock.

function weekday(y: number, m: number, d: number): number {
  return new Date(Date.UTC(y, m, d)).getUTCDay();
}

function isLeap(y: number): boolean {
  return new Date(Date.UTC(y, 1, 29)).getUTCMonth() === 1;
}

function isoDay(y: number, m: number, d: number): string {
  return new Date(Date.UTC(y, m, d)).toISOString().slice(0, 10);
}

function daysInYear(y: number): number {
  return (Date.UTC(y + 1, 0, 1) - Date.UTC(y, 0, 1)) / 86400000;
}

function dayOfYear(y: number, m: number, d: number): number {
  return (Date.UTC(y, m, d) - Date.UTC(y, 0, 1)) / 86400000 + 1;
}

// ------------------------------------------------------------ weekdays
console.log("week_of_epoch=" + [1, 2, 3, 4, 5, 6, 7].map((d) => weekday(1970, 0, d)).join(","));
console.log("epoch_day=" + new Date(0).getUTCDay());
console.log("y2000_jan1=" + weekday(2000, 0, 1));
console.log("y2024_jan1=" + weekday(2024, 0, 1));
console.log("y2024_feb29=" + weekday(2024, 1, 29));
console.log("y1969_jul20=" + weekday(1969, 6, 20));
console.log("y1900_jan1=" + weekday(1900, 0, 1));
console.log("y1800_jan1=" + weekday(1800, 0, 1));
const cycle: number[] = [];
for (let i = 0; i < 9; i++) cycle.push(weekday(2024, 0, 1 + i));
console.log("cycle=" + cycle.join(","));
console.log("period7=" + (weekday(2024, 0, 1) === weekday(2024, 0, 8)));
console.log("pre_epoch_walk=" + [-3, -2, -1, 0, 1].map((n) => new Date(n * 86400000).getUTCDay()).join(","));

// ---------------------------------------------------------- leap years
console.log("leap_2024=" + isLeap(2024));
console.log("leap_2023=" + isLeap(2023));
console.log("leap_2000=" + isLeap(2000));
console.log("leap_1900=" + isLeap(1900));
console.log("leap_2100=" + isLeap(2100));
console.log("leap_2400=" + isLeap(2400));
console.log("leap_1600=" + isLeap(1600));
console.log("feb29_1900=" + isoDay(1900, 1, 29));
console.log("feb29_2100=" + isoDay(2100, 1, 29));
console.log("feb29_2000=" + isoDay(2000, 1, 29));

// Month lengths: day 0 of the following month is the last day of this one.
const lengths: number[] = [];
const lengths23: number[] = [];
for (let m = 0; m < 12; m++) {
  lengths.push(new Date(Date.UTC(2024, m + 1, 0)).getUTCDate());
  lengths23.push(new Date(Date.UTC(2023, m + 1, 0)).getUTCDate());
}
console.log("lengths_2024=" + lengths.join(","));
console.log("lengths_2023=" + lengths23.join(","));

console.log("days_2024=" + daysInYear(2024));
console.log("days_2023=" + daysInYear(2023));
console.log("days_1900=" + daysInYear(1900));
console.log("days_2000=" + daysInYear(2000));
console.log("doy_2024_dec31=" + dayOfYear(2024, 11, 31));
console.log("doy_2023_dec31=" + dayOfYear(2023, 11, 31));
console.log("doy_2024_mar1=" + dayOfYear(2024, 2, 1));
console.log("doy_2023_mar1=" + dayOfYear(2023, 2, 1));

// A 400-year cycle is a whole number of weeks, so the weekday pattern repeats.
const span400 = (Date.UTC(2400, 0, 1) - Date.UTC(2000, 0, 1)) / 86400000;
console.log("cycle400_days=" + span400);
console.log("cycle400_mod7=" + span400 % 7);
console.log("same_weekday_400=" + (weekday(2000, 0, 1) === weekday(2400, 0, 1)));

let fridays13 = 0;
for (let m = 0; m < 12; m++) if (weekday(2024, m, 13) === 5) fridays13 += 1;
console.log("friday13_in_2024=" + fridays13);

// Walk across a leap day and across a non-leap February.
const leapWalk: string[] = [];
const cursor = new Date(Date.UTC(2024, 1, 27));
for (let i = 0; i < 5; i++) {
  leapWalk.push(cursor.toISOString().slice(5, 10) + "(" + cursor.getUTCDay() + ")");
  cursor.setUTCDate(cursor.getUTCDate() + 1);
}
console.log("leap_walk=" + leapWalk.join(","));

const plainWalk: string[] = [];
const cursor23 = new Date(Date.UTC(2023, 1, 27));
for (let i = 0; i < 5; i++) {
  plainWalk.push(cursor23.toISOString().slice(5, 10));
  cursor23.setUTCDate(cursor23.getUTCDate() + 1);
}
console.log("nonleap_walk=" + plainWalk.join(","));

const anniversary = new Date(Date.UTC(2024, 1, 29));
anniversary.setUTCFullYear(2025);
console.log("feb29_plus_year=" + anniversary.toISOString().slice(0, 10));
const four = new Date(Date.UTC(2024, 1, 29));
four.setUTCFullYear(2028);
console.log("feb29_plus_four=" + four.toISOString().slice(0, 10));

// ------------------------------------------------------- comparisons
const a = new Date(Date.UTC(2024, 0, 1));
const b = new Date(Date.UTC(2024, 0, 2));
const aCopy = new Date(Date.UTC(2024, 0, 1));

console.log("lt=" + (a < b) + " gt=" + (a > b) + " lte=" + (a <= b) + " gte=" + (a >= b));
console.log("copy_lte=" + (a <= aCopy) + " copy_gte=" + (a >= aCopy));
console.log("copy_lt=" + (a < aCopy) + " copy_gt=" + (a > aCopy));
console.log("copy_eqeq=" + ((a as any) == aCopy) + " copy_eqeqeq=" + (a === aCopy));
console.log("copy_noteq=" + ((a as any) != aCopy));
console.log("self_eqeqeq=" + (a === a));
console.log("time_eq=" + (a.getTime() === aCopy.getTime()));
console.log("vs_number_lt=" + (a < Date.UTC(2024, 0, 2)));
console.log("vs_number_gte=" + (a >= Date.UTC(2024, 0, 1)));
console.log("eqeq_number=" + ((a as any) == Date.UTC(2024, 0, 1)));

const bad = new Date(NaN);
console.log("bad_lt=" + (bad < a) + " bad_gt=" + (bad > a));
console.log("bad_lte=" + (bad <= a) + " bad_gte=" + (bad >= a));
console.log("bad_lte_self=" + (bad <= bad) + " bad_eqeqeq_self=" + (bad === bad));

const list = [
  new Date(Date.UTC(2024, 0, 3)),
  new Date(Date.UTC(2024, 0, 1)),
  new Date(Date.UTC(2024, 0, 2)),
];
list.sort((x, y) => x.getTime() - y.getTime());
console.log("sorted=" + list.map((x) => x.toISOString().slice(0, 10)).join(","));
console.log("min=" + new Date(Math.min(+a, +b)).toISOString());
console.log("max=" + new Date(Math.max(+a, +b)).toISOString());

// ------------------------------------------------- toPrimitive hints
const stamp = new Date(Date.UTC(2024, 0, 15, 10, 30, 45, 123));
const prim = (stamp as any)[Symbol.toPrimitive];
console.log("has_toPrimitive=" + (typeof prim === "function"));
console.log("own_on_proto=" + Object.prototype.hasOwnProperty.call(Date.prototype, Symbol.toPrimitive));
console.log("toPrimitive_length=" + prim.length + " name=" + prim.name);
console.log("number_hint=" + prim.call(stamp, "number") + " is_time=" + (prim.call(stamp, "number") === stamp.getTime()));
console.log("default_hint_type=" + typeof prim.call(stamp, "default"));
console.log("string_hint_type=" + typeof prim.call(stamp, "string"));
console.log("default_eq_string=" + (prim.call(stamp, "default") === prim.call(stamp, "string")));
console.log("plus_is_number=" + (+stamp === stamp.getTime()));
console.log("minus_numeric=" + (((stamp as any) - 0) === stamp.getTime()));
console.log("concat_is_string=" + (typeof ((stamp as any) + "")));
console.log("invalid_number_hint_nan=" + Number.isNaN(prim.call(bad, "number")));
for (const hint of ["bogus", undefined]) {
  try {
    prim.call(stamp, hint);
    console.log("bad_hint_threw_" + String(hint) + "=false");
  } catch (e) {
    console.log("bad_hint_threw_" + String(hint) + "=true name=" + (e as Error).name);
  }
}

// --------------------------------------------------- UTC string form
console.log("utc_string=" + stamp.toUTCString());
console.log("utc_string_epoch=" + new Date(0).toUTCString());
console.log("utc_string_single_digit=" + new Date(Date.UTC(2024, 0, 5)).toUTCString());
console.log("utc_string_pre_epoch=" + new Date(Date.UTC(1969, 6, 20, 20, 17, 40)).toUTCString());
console.log("utc_string_invalid=" + bad.toUTCString());
