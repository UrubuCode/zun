//! Local time zone support for `Date`: fixed-zone offsets (DST-aware) via the
//! `date.localOffsetMinutes` native, and host-zone invariants that hold in any zone.

use zun_interp::CaptureOutput;

fn run_source(user: &str) -> Vec<String> {
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
fn fixed_zone_offsets_are_dst_aware() {
    let out = run_source(
        r#"import { date } from "zun:core";
        function main() {
            let summer = Date.UTC(2021, 6, 1, 12, 0, 0, 0);
            let winter = Date.UTC(2021, 0, 1, 12, 0, 0, 0);
            print(date.localOffsetMinutes(summer, "America/New_York"));
            print(date.localOffsetMinutes(winter, "America/New_York"));
            print(date.localOffsetMinutes(summer, "Europe/Berlin"));
            print(date.localOffsetMinutes(winter, "Europe/Berlin"));
            print(date.localOffsetMinutes(summer, "UTC"));
        }"#,
    );
    assert_eq!(out, ["-240", "-300", "120", "60", "0"]);
}

#[test]
fn fixed_zone_local_to_utc_round_trips() {
    // 2021-07-01 08:00 wall clock in New York is 12:00 UTC.
    let out = run_source(
        r#"import { date } from "zun:core";
        function main() {
            let wall = Date.UTC(2021, 6, 1, 8, 0, 0, 0);
            print(date.localToUtc(wall, "America/New_York") == Date.UTC(2021, 6, 1, 12, 0, 0, 0));
        }"#,
    );
    assert_eq!(out, ["true"]);
}

#[test]
fn host_zone_getters_are_consistent_with_the_offset() {
    // local minutes-of-day - UTC minutes-of-day == -getTimezoneOffset (mod a day).
    let out = run_source(
        r#"function main() {
            let d = Date.fromISO("2021-07-01T12:34:56.789Z");
            let diff = (d.getHours() * 60 + d.getMinutes()) - (d.getUTCHours() * 60 + d.getUTCMinutes());
            let expect = 0 - d.getTimezoneOffset();
            print(((diff - expect) % 1440 + 1440) % 1440 == 0);
            print(d.getSeconds() == d.getUTCSeconds());
            print(d.getMilliseconds() == d.getUTCMilliseconds());
        }"#,
    );
    assert_eq!(out, ["true", "true", "true"]);
}

#[test]
fn host_zone_setters_round_trip_with_getters() {
    let out = run_source(
        r#"function main() {
            let d = Date.fromISO("2021-07-01T12:34:56.789Z");
            d.setHours(5, 6, 7, 8);
            print(d.getHours());
            print(d.getMinutes());
            print(d.getSeconds());
            print(d.getMilliseconds());
        }"#,
    );
    assert_eq!(out, ["5", "6", "7", "8"]);
}
