//! `Date`'s JS-facing surface, end to end.
//!
//! The getters named after JS must answer what JS answers. Two did not:
//! `getUTCMonth()` returned the 1-based calendar month where JS is 0-based,
//! and `getUTCDay()` returned the ISO weekday (1 = Monday, 7 = Sunday) where
//! JS is 0-based from Sunday. Both were wrong answers rather than missing
//! ones, so ported code computed the wrong date silently.
//!
//! The reference values below are what JS produces for the same instants.

use zun_interp::CaptureOutput;

fn run_source(user: &str) -> Vec<String> {
    // `Date` is a prelude class, so the prelude has to be here; checking the
    // user source alone leaves it undefined.
    let src = &zun_prelude::with_prelude(user);
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

#[test]
fn the_js_named_getters_use_js_conventions() {
    // 2021-01-03 is a Sunday. JS: year 2021, month 0, date 3, day 0.
    let out = run_source(
        r#"function main() {
            let d = Date.fromISO("2021-01-03T00:00:00.000Z");
            print(d.getUTCFullYear());
            print(d.getUTCMonth());
            print(d.getUTCDate());
            print(d.getUTCDay());
        }"#,
    );
    assert_eq!(out, vec!["2021", "0", "3", "0"]);
}

#[test]
fn the_weekday_is_zero_based_from_sunday_across_a_week() {
    // 2021-01-03 Sunday through 2021-01-09 Saturday: JS gives 0..6.
    let out = run_source(
        r#"function main() {
            print(Date.fromISO("2021-01-03T00:00:00.000Z").getUTCDay());
            print(Date.fromISO("2021-01-04T00:00:00.000Z").getUTCDay());
            print(Date.fromISO("2021-01-07T00:00:00.000Z").getUTCDay());
            print(Date.fromISO("2021-01-09T00:00:00.000Z").getUTCDay());
        }"#,
    );
    assert_eq!(out, vec!["0", "1", "4", "6"]);
}

#[test]
fn the_calendar_accessors_keep_their_one_based_convention() {
    // `utcMonth`/`utcWeekday` are Zun's own names and stay 1-based and ISO;
    // only the JS-named getters follow JS.
    let out = run_source(
        r#"function main() {
            let d = Date.fromISO("2021-01-03T00:00:00.000Z");
            print(d.utcMonth());
            print(d.utcWeekday());
        }"#,
    );
    assert_eq!(out, vec!["1", "7"]);
}

#[test]
fn utc_builds_a_timestamp_from_a_zero_based_month() {
    // Date.UTC(2020, 0, 1) is 2020-01-01T00:00:00Z.
    let out = run_source(
        r#"function main() {
            print(Date.UTC(2020, 0, 1, 0, 0, 0, 0));
            print(Date.UTC(1970, 0, 1, 0, 0, 0, 0));
        }"#,
    );
    assert_eq!(out, vec!["1577836800000", "0"]);
}

#[test]
fn utc_round_trips_through_the_getters() {
    let out = run_source(
        r#"function main() {
            let d = Date.fromMillis(Date.UTC(1999, 11, 25, 1, 2, 3, 4));
            print(d.toISOString());
        }"#,
    );
    assert_eq!(out, vec!["1999-12-25T01:02:03.004Z"]);
}

#[test]
fn each_setter_replaces_only_its_component() {
    let out = run_source(
        r#"function main() {
            let d = Date.fromISO("2021-01-03T10:20:30.400Z");
            d.setUTCFullYear(1999);
            print(d.toISOString());
            d.setUTCMonth(11);
            print(d.toISOString());
            d.setUTCDate(25);
            print(d.toISOString());
            d.setUTCHours(1);
            d.setUTCMinutes(2);
            d.setUTCSeconds(3);
            d.setUTCMilliseconds(4);
            print(d.toISOString());
        }"#,
    );
    assert_eq!(
        out,
        vec![
            "1999-01-03T10:20:30.400Z",
            "1999-12-03T10:20:30.400Z",
            "1999-12-25T10:20:30.400Z",
            "1999-12-25T01:02:03.004Z",
        ]
    );
}

#[test]
fn a_setter_returns_the_new_timestamp() {
    let out = run_source(
        r#"function main() {
            let d = Date.fromMillis(0);
            print(d.setUTCFullYear(1971));
            print(d.getTime());
        }"#,
    );
    // 1971-01-01T00:00:00Z, one non-leap year after the epoch.
    assert_eq!(out, vec!["31536000000", "31536000000"]);
}

#[test]
fn out_of_range_components_roll_over_as_they_do_in_js() {
    let out = run_source(
        r#"function main() {
            let d = Date.fromISO("2020-01-15T00:00:00.000Z");
            d.setUTCMonth(12);
            print(d.toISOString());
            let e = Date.fromISO("2020-01-31T00:00:00.000Z");
            e.setUTCDate(32);
            print(e.toISOString());
            let f = Date.fromISO("2020-01-01T00:00:00.000Z");
            f.setUTCHours(25);
            print(f.toISOString());
        }"#,
    );
    assert_eq!(
        out,
        vec![
            // Month 12 (0-based) is January of the next year.
            "2021-01-15T00:00:00.000Z",
            // Day 32 of January is the 1st of February.
            "2020-02-01T00:00:00.000Z",
            // Hour 25 is 1am the next day.
            "2020-01-02T01:00:00.000Z",
        ]
    );
}

#[test]
fn the_timezone_offset_is_zero_because_the_calendar_is_utc() {
    // Not a stub: with a UTC-only calendar, local time is UTC.
    let out = run_source(
        r#"function main() { print(Date.fromMillis(0).getTimezoneOffset()); }"#,
    );
    assert_eq!(out, vec!["0"]);
}

#[test]
fn a_leap_day_survives_a_round_trip() {
    let out = run_source(
        r#"function main() {
            let d = Date.fromMillis(Date.UTC(2020, 1, 29, 12, 0, 0, 0));
            print(d.toISOString());
            print(d.getUTCMonth());
            print(d.getUTCDate());
        }"#,
    );
    assert_eq!(out, vec!["2020-02-29T12:00:00.000Z", "1", "29"]);
}

#[test]
fn no_arg_construction_is_the_current_time() {
    // `new Date()` builds from the `millis = time.nowMillis()` default, so its
    // timestamp is positive and tracks `Date.now()`. `new Date(0)` still binds
    // the epoch, proving the one-argument form is unaffected by the default.
    let out = run_source(
        r#"function main() {
            let now = Date.now();
            let d = new Date();
            print(now > 0);
            print(d.getTime() > 0);
            print(d.getTime() >= now);
            print(new Date(0).getTime());
        }"#,
    );
    assert_eq!(out, vec!["true", "true", "true", "0"]);
}

#[test]
fn local_getters_equal_utc_because_the_calendar_is_utc() {
    // Local time is UTC (no platform timezone), so every local getter returns
    // its UTC counterpart for a known instant: 2021-01-03T10:20:30.400Z, a
    // Sunday (day 0), January (month 0).
    let out = run_source(
        r#"function main() {
            let d = Date.fromISO("2021-01-03T10:20:30.400Z");
            print(d.getFullYear());
            print(d.getMonth());
            print(d.getDate());
            print(d.getDay());
            print(d.getHours());
            print(d.getMinutes());
            print(d.getSeconds());
            print(d.getMilliseconds());
        }"#,
    );
    assert_eq!(out, vec!["2021", "0", "3", "0", "10", "20", "30", "400"]);
}

#[test]
fn local_setters_change_the_right_component_and_return_the_new_time() {
    let out = run_source(
        r#"function main() {
            let d = Date.fromISO("2021-01-03T10:20:30.400Z");
            print(d.setFullYear(1999));
            print(d.toISOString());
            print(d.setMonth(11));
            print(d.toISOString());
            print(d.setDate(25));
            print(d.toISOString());
            print(d.setHours(1));
            print(d.setMinutes(2));
            print(d.setSeconds(3));
            print(d.setMilliseconds(4));
            print(d.toISOString());
        }"#,
    );
    assert_eq!(
        out,
        vec![
            "915358830400",
            "1999-01-03T10:20:30.400Z",
            "944216430400",
            "1999-12-03T10:20:30.400Z",
            "946117230400",
            "1999-12-25T10:20:30.400Z",
            "946084830400",
            "946083750400",
            "946083723400",
            "946083723004",
            "1999-12-25T01:02:03.004Z",
        ]
    );
}

#[test]
fn local_setters_equal_their_utc_counterparts_on_the_same_input() {
    // Local == UTC in this UTC-only calendar, so a local setter must land on the
    // exact same timestamp as its `setUTC*` counterpart from the same instant.
    let out = run_source(
        r#"function main() {
            let a = Date.fromISO("2021-01-03T10:20:30.400Z");
            let b = Date.fromISO("2021-01-03T10:20:30.400Z");
            print(a.setFullYear(1999) == b.setUTCFullYear(1999));
            print(a.setMonth(11) == b.setUTCMonth(11));
            print(a.setDate(25) == b.setUTCDate(25));
            print(a.setHours(1) == b.setUTCHours(1));
            print(a.setMinutes(2) == b.setUTCMinutes(2));
            print(a.setSeconds(3) == b.setUTCSeconds(3));
            print(a.setMilliseconds(4) == b.setUTCMilliseconds(4));
        }"#,
    );
    assert_eq!(out, vec!["true", "true", "true", "true", "true", "true", "true"]);
}

#[test]
fn local_setters_accept_optional_trailing_arguments() {
    // `setFullYear(y, month, date)` and `setHours(h, min, sec, ms)` replace all
    // supplied components at once; omitted trailing components keep their value.
    let out = run_source(
        r#"function main() {
            let d = new Date(0);
            d.setFullYear(2020, 5, 15);
            print(d.toISOString());
            d.setHours(8, 9, 10, 11);
            print(d.toISOString());
            let e = new Date(0);
            e.setFullYear(2020);
            print(e.getFullYear());
        }"#,
    );
    assert_eq!(
        out,
        vec!["2020-06-15T00:00:00.000Z", "2020-06-15T08:09:10.011Z", "2020"]
    );
}

#[test]
fn set_full_year_on_the_epoch_then_read_it_back() {
    let out = run_source(
        r#"function main() {
            let d = new Date(0);
            d.setFullYear(2020);
            print(d.getFullYear());
        }"#,
    );
    assert_eq!(out, vec!["2020"]);
}

#[test]
fn dates_before_the_epoch_work() {
    let out = run_source(
        r#"function main() {
            let d = Date.fromMillis(Date.UTC(1969, 6, 20, 20, 17, 40, 0));
            print(d.toISOString());
            print(d.getUTCFullYear());
            print(d.getUTCMonth());
        }"#,
    );
    assert_eq!(out, vec!["1969-07-20T20:17:40.000Z", "1969", "6"]);
}
