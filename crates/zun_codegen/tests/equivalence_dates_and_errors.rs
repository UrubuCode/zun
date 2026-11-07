mod common;
use common::*;

use zun_codegen::RetVal;
use zun_interp::value::Value;

#[test]
fn f64_arithmetic_and_recursion() {
    assert_equivalent(
        "function fib(n: number): number {\n\
            if (n < 2) { return n; }\n\
            return fib(n - 1) + fib(n - 2);\n\
         }\n\
         function main(): number { return fib(15); }",
    );
}

#[test]
fn int_arithmetic() {
    assert_equivalent(
        "function main(): int { let a: int = 10; let b: int = 3; return a * b + a / b; }",
    );
}

#[test]
fn managed_error_hierarchy_constructs_and_preserves_fields() {
    let module = compile(
        "class Error { name: string; message: string; }\n\
         class TypeError extends Error {}\n\
         function main(): int { let error: TypeError = new TypeError(\"TypeError\", \"bad value\"); if (error.name == \"TypeError\") { if (error.message == \"bad value\") { return 1; } } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "string-field error objects stay interpreter-only"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn managed_error_to_string_uses_name_and_message() {
    let module = compile(
        "class Error { name: string; message: string; toString(): string { return this.name + \": \" + this.message; } }\n\
         class RangeError extends Error {}\n\
         function main(): int { let error: RangeError = new RangeError(\"RangeError\", \"outside\"); let text: string = error.toString(); if (text == \"RangeError: outside\") { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "string operations stay interpreter-only"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn virtual_override_dispatch_is_jit_equivalent() {
    let module = compile(
        "class Base { value(): int { return 1; } }\n\
         class Derived extends Base { value(): int { return 2; } }\n\
         function main(): int { let item: Base = new Derived(); return item.value(); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles virtual override");
    assert!(jit.is_compiled(entry));
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("run JIT virtual override")
    };
    assert!(matches!(jit_result, RetVal::Int(2)));
    assert!(matches!(interp_entry(&module), Value::Int(2, _)));
}

#[test]
fn date_surface_decomposes_utc_timestamps() {
    let module = compile(
        "import { year, month, day } from \"zun:core\";\n\
         class Date { millis: i64; utcYear(): i32 { return year(this.millis); } utcMonth(): i32 { return month(this.millis); } utcDay(): i32 { return day(this.millis); } }\n\
         function main(): int { let value: Date = new Date(0); if (value.utcYear() == 1970) { if (value.utcMonth() == 1) { if (value.utcDay() == 1) { return 1; } } } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    assert!(matches!((jit_result, interp_result),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1));
}

#[test]
fn date_validity_is_jit_equivalent() {
    assert_equivalent(
        "import { isValid } from \"zun:core\"; function main(): boolean { return isValid(0); }",
    );
}

#[test]
fn date_numeric_intrinsics_are_equivalent_in_jit() {
    let module = compile(
        "import { year, month, day, hour, minute, second, millisecond, weekday, dayOfYear } from \"zun:core\";\n\
         class Date { millis: i64; all(): int { return year(this.millis) + month(this.millis) + day(this.millis) + hour(this.millis) + minute(this.millis) + second(this.millis) + millisecond(this.millis) + weekday(this.millis) + dayOfYear(this.millis); } }\n\
         function main(): int { return new Date(3723004).all(); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    assert!(matches!((jit_result, interp_entry(&module)),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1987));
}

#[test]
fn date_numeric_intrinsics_handle_pre_epoch_timestamps() {
    let module = compile(
        "import { year, month, day, hour, minute, second, millisecond } from \"zun:core\";\n\
         class Date { millis: i64; signature(): int { return year(this.millis) + month(this.millis) + day(this.millis) + hour(this.millis) + minute(this.millis) + second(this.millis) + millisecond(this.millis); } }\n\
         function main(): int { return new Date(-1).signature(); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    assert!(matches!((jit_result, interp_entry(&module)),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1969 + 12 + 31 + 23 + 59 + 59 + 999));
}

#[test]
fn date_surface_exposes_time_components_and_string_form() {
    let module = compile(
        "import { hour, minute, second, millisecond, isoDateTime } from \"zun:core\";\n\
         class Date { millis: i64; utcHour(): i32 { return hour(this.millis); } utcMinute(): i32 { return minute(this.millis); } utcSecond(): i32 { return second(this.millis); } utcMillisecond(): i32 { return millisecond(this.millis); } isoDateTime(): string { return isoDateTime(this.millis); } toString(): string { return this.isoDateTime(); } }\n\
         function main(): int { let value: Date = new Date(3723004); if (value.utcHour() == 1) { if (value.utcMinute() == 2) { if (value.utcSecond() == 3) { if (value.utcMillisecond() == 4) { if (value.toString() == \"1970-01-01T01:02:03.004Z\") { return 1; } } } } } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "date string operations remain interpreter-only"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn date_surface_updates_and_returns_timestamp_values() {
    let module = compile(
        "class Date { millis: i64; setMillis(value: i64): Date { this.millis = value; return this; } valueOf(): i64 { return this.millis; } }\n\
         function main(): int { let value: Date = new Date(0); let same: Date = value.setMillis(42); if (same.valueOf() == 42) { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    assert!(matches!((jit_result, interp_result),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1));
}

#[test]
fn date_surface_supports_time_aliases_and_comparisons() {
    let module = compile(
        "class Date { millis: i64; getTime(): i64 { return this.millis; } setTime(value: i64): i64 { this.millis = value; return this.millis; } isBefore(other: Date): boolean { return this.millis < other.millis; } isAfter(other: Date): boolean { return this.millis > other.millis; } isEqual(other: Date): boolean { return this.millis == other.millis; } }\n\
         function main(): int { let first: Date = new Date(10); let second: Date = new Date(20); first.setTime(15); if (first.getTime() == 15) { if (first.isBefore(second)) { if (second.isAfter(first)) { if (!first.isEqual(second)) { return 1; } } } } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    assert!(matches!((jit_result, interp_result),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1));
}

#[test]
fn date_surface_exposes_ecmascript_utc_aliases() {
    let module = compile(
        "import { year, month, day, hour, minute, second, millisecond, isoDateTime } from \"zun:core\";\n\
         class Date { millis: i64; utcYear(): i32 { return year(this.millis); } utcMonth(): i32 { return month(this.millis); } utcDay(): i32 { return day(this.millis); } utcHour(): i32 { return hour(this.millis); } utcMinute(): i32 { return minute(this.millis); } utcSecond(): i32 { return second(this.millis); } utcMillisecond(): i32 { return millisecond(this.millis); } isoDateTime(): string { return isoDateTime(this.millis); } getUTCFullYear(): i32 { return this.utcYear(); } getUTCMonth(): i32 { return this.utcMonth(); } getUTCDate(): i32 { return this.utcDay(); } getUTCHours(): i32 { return this.utcHour(); } getUTCMinutes(): i32 { return this.utcMinute(); } getUTCSeconds(): i32 { return this.utcSecond(); } getUTCMilliseconds(): i32 { return this.utcMillisecond(); } toISOString(): string { return this.isoDateTime(); } }\n\
         function main(): int { let value: Date = new Date(3723004); if (value.getUTCFullYear() == 1970) { if (value.getUTCMonth() == 1) { if (value.getUTCDate() == 1) { if (value.getUTCHours() == 1) { if (value.getUTCMinutes() == 2) { if (value.getUTCSeconds() == 3) { if (value.getUTCMilliseconds() == 4) { if (value.toISOString() == \"1970-01-01T01:02:03.004Z\") { return 1; } } } } } } } } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "date string aliases remain interpreter-only"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn date_surface_provides_timestamp_factories() {
    let module = compile(
        "\n\
         class Date { millis: i64; valueOf(): i64 { return this.millis; } }\n\
         function dateFromMillis(value: i64): Date { return new Date(value); }\n\
         function main(): int { let fixed: Date = dateFromMillis(42); if (fixed.valueOf() == 42) { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    assert!(matches!((jit_result, interp_result),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1));
}

#[test]
fn date_surface_parses_iso_datetime_values() {
    let module = compile(
        "import { parseIsoDateTime } from \"zun:core\";\n\
         class Date { millis: i64; valueOf(): i64 { return this.millis; } }\n\
         function dateParse(value: string): Date { return new Date(parseIsoDateTime(value)); }\n\
         function main(): int { let value: Date = dateParse(\"1970-01-01T00:00:00.042Z\"); if (value.valueOf() == 42) { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "ISO parsing remains interpreter-only until native JIT calls are lowered"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn date_surface_json_representation_matches_iso() {
    let module = compile(
        "import { isoDateTime } from \"zun:core\";\n\
         class Date { millis: i64; isoDateTime(): string { return isoDateTime(this.millis); } toISOString(): string { return this.isoDateTime(); } toJSON(): string { return this.toISOString(); } }\n\
         function main(): int { let value: Date = new Date(42); if (value.toJSON() == \"1970-01-01T00:00:00.042Z\") { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "Date JSON strings remain interpreter-only"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn date_surface_exposes_utc_weekday_alias() {
    let module = compile(
        "import { weekday } from \"zun:core\";\n\
         class Date { millis: i64; utcWeekday(): i32 { return weekday(this.millis); } getUTCWeekday(): i32 { return this.utcWeekday(); } }\n\
         function main(): int { let value: Date = new Date(0); if (value.getUTCWeekday() == 4) { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    assert!(matches!((jit_result, interp_entry(&module)),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1));
}

#[test]
fn date_surface_converts_iso_weekday_to_ecmascript_day() {
    let module = compile(
        "import { weekday } from \"zun:core\";\n\
         class Date { millis: i64; utcWeekday(): i32 { return weekday(this.millis); } getUTCDay(): i32 { return this.utcWeekday(); } }\n\
         function main(): int { let value: Date = new Date(0); if (value.getUTCDay() == 4) { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    assert!(matches!((jit_result, interp_entry(&module)),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 1));
}
