//! End-to-end round-trip tests for the binary ser/de PRIMITIVE
//! (`serialize`/`deserialize`, imported from `"zun:core"`).

use zun_interp::CaptureOutput;

/// Compile `src` (hand parser + sema + ilgen), run through the `.zm`
/// round-trip, and return the captured output lines.
fn run(src: &str) -> Vec<String> {
    let src = zun_prelude::with_prelude(src);
    let src = src.as_str();
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

/// Compile and run `src`, expecting a clean runtime failure; return its text.
fn run_expect_error(src: &str) -> String {
    let src = zun_prelude::with_prelude(src);
    let src = src.as_str();
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    match zun_vm::execute_with(&bytes, &mut out) {
        Ok(_) => panic!("program should have failed at runtime"),
        Err(error) => error.to_string(),
    }
}

/// A Zun helper (prepended to a program) that re-serializes a deserialized
/// value and prints whether the bytes are byte-for-byte identical to the
/// original — a full-structure round-trip check (fields, order, memo/refs,
/// cycles) that does not need to introspect the reconstructed `any`.
const STABLE_HELPER: &str = r#"
import { serialize, deserialize } from "zun:core";
function stable(first: Uint8Array): boolean {
    let again: Uint8Array = serialize(deserialize(first));
    let len: i32 = first.length as i32;
    if ((again.length as i32) !== len) { return false; }
    let i: i32 = 0;
    while (i < len) {
        if ((again[i] as number) !== (first[i] as number)) { return false; }
        i = i + 1;
    }
    return true;
}
"#;

fn run_stable(body: &str) -> Vec<String> {
    run(&format!("{STABLE_HELPER}\nfunction main() {{\n{body}\n}}"))
}

#[test]
fn primitives_round_trip() {
    let out = run(&format!(
        "{STABLE_HELPER}\nfunction main() {{\n\
            print(deserialize(serialize(42.5)) as number);\n\
            print(deserialize(serialize(true)) as boolean);\n\
            print(deserialize(serialize(false)) as boolean);\n\
            print(deserialize(serialize(\"zun\")) as string);\n\
            if (!Boolean(deserialize(serialize(box_any(null))))) {{ print(\"null-ok\"); }}\n\
         }}"
    ));
    assert_eq!(out, ["42.5", "true", "false", "zun", "null-ok"]);
}

#[test]
fn integer_kind_round_trips() {
    let out = run(&format!(
        "{STABLE_HELPER}\nfunction main() {{\n\
            let n: i32 = 7;\n\
            print(deserialize(serialize(n)) as i32);\n\
         }}"
    ));
    assert_eq!(out, ["7"]);
}

#[test]
fn bigint_round_trips() {
    let out = run(&format!(
        "{STABLE_HELPER}\nfunction main() {{\n\
            let big = 170141183460469231731687303715884105727n;\n\
            print(stable(serialize(big)));\n\
         }}"
    ));
    assert_eq!(out, ["true"]);
}

#[test]
fn nested_array_round_trips() {
    let out = run_stable(r#"print(stable(serialize([1.0, [2.0, 3.0], [[4.0]]])));"#);
    assert_eq!(out, ["true"]);
}

#[test]
fn dynamic_object_with_string_keys_round_trips() {
    let out = run_stable(
        r#"let o = dynamic_new();
           dynamic_set(o, "a", box_any(1));
           dynamic_set(o, "b", box_any("two"));
           print(stable(serialize(box_any(o))));"#,
    );
    assert_eq!(out, ["true"]);
}

#[test]
fn map_and_set_round_trip() {
    let out = run_stable(
        r#"let m = new Map(); m.set("a", 1.0); m.set("b", 2.0);
           print(stable(serialize(box_any(m))));
           let s = new Set(); s.add(5.0); s.add(6.0);
           print(stable(serialize(box_any(s))));"#,
    );
    assert_eq!(out, ["true", "true"]);
}

#[test]
fn date_round_trips() {
    let out = run_stable(r#"print(stable(serialize(box_any(Date.fromMillis(1000)))));"#);
    assert_eq!(out, ["true"]);
}

#[test]
fn typed_class_instance_round_trips_with_exact_type() {
    // The type token + declared fields round-trip; a double round-trip yields
    // identical bytes, which proves the type reconstructed and the fields were
    // restored in layout order.
    let out = run(&format!(
        "{STABLE_HELPER}\n\
         class Point {{ x: number; y: number; }}\n\
         function main() {{\n\
            let p = new Point(10.0, 20.0);\n\
            print(stable(serialize(box_any(p))));\n\
            let r = deserialize(serialize(box_any(p)));\n\
            print((r as Point).x as number);\n\
            print((r as Point).y as number);\n\
         }}"
    ));
    assert_eq!(out, ["true", "10", "20"]);
}

#[test]
fn array_buffer_and_typed_array_preserve_bytes() {
    let out = run_stable(
        r#"let buf = new ArrayBuffer(4);
           print(stable(serialize(box_any(buf))));
           let view = new Uint8Array(new ArrayBuffer(3), 0, 3);
           view.set(0, 7);
           view.set(1, 8);
           view.set(2, 9);
           print(stable(serialize(box_any(view))));"#,
    );
    assert_eq!(out, ["true", "true"]);
}

#[test]
fn shared_reference_identity_is_preserved() {
    // `o.x` and `o.y` are the SAME object; after a round-trip they are still
    // the same object (memo/ref), observed by mutating through one and reading
    // the other. The double round-trip stays byte-identical too.
    let out = run_stable(
        r#"let a = dynamic_new();
           dynamic_set(a, "n", box_any(1));
           let o = dynamic_new();
           dynamic_set(o, "x", box_any(a));
           dynamic_set(o, "y", box_any(a));
           print(stable(serialize(box_any(o))));"#,
    );
    assert_eq!(out, ["true"]);
}

#[test]
fn self_cycle_round_trips_without_infinite_recursion() {
    let out = run_stable(
        r#"let o = dynamic_new();
           dynamic_set(o, "self", box_any(o));
           print(stable(serialize(box_any(o))));"#,
    );
    assert_eq!(out, ["true"]);
}

#[test]
fn deserialize_of_garbage_bytes_is_a_clean_error() {
    let error = run_expect_error(
        r#"import { deserialize } from "zun:core";
           function main() {
               let bad = new Uint8Array(new ArrayBuffer(3), 0, 3);
               bad.set(0, 200);
               bad.set(1, 200);
               bad.set(2, 200);
               deserialize(bad);
           }"#,
    );
    assert!(
        error.contains("deserialize"),
        "expected a clean deserialize error, got: {error}"
    );
}

#[test]
fn serialize_of_a_function_is_a_clean_error() {
    let error = run_expect_error(
        r#"import { serialize } from "zun:core";
           function helper(): number { return 1.0; }
           function main() {
               serialize(helper);
           }"#,

    );
    assert!(
        error.contains("function"),
        "expected a function-not-serializable error, got: {error}"
    );
}

/// Registered `Symbol.for` keys round-trip; a unique `Symbol()` key is dropped.
#[test]
fn symbol_keyed_entries_registered_round_trip_unique_dropped() {
    let out = run(
        r#"import { serialize, deserialize } from "zun:core";
function main() {
    let k = Symbol.for("k");
    let u = Symbol("u");
    let o = dynamic_new();
    dynamic_set(o, "a", box_any(1));
    o[k] = box_any(7);
    o[u] = box_any(9);
    let back = deserialize(serialize(box_any(o)));
    let syms = Object.getOwnPropertySymbols(back);
    print(syms.length);
    print(back[Symbol.for("k")] as number);
}"#,
    );
    assert_eq!(out, ["1", "7"]);
}

/// A `DataView` round-trips: the deserialized view re-serializes to identical
/// bytes (buffer contents, offset, length and the length-tracking flag all
/// survive), and the wire form carries the written bytes.
#[test]
fn data_view_round_trips_bytes_offset_and_length() {
    let out = run_stable(
        r#"let v = new DataView(new ArrayBuffer(8), 2, 4);
           v.setUint8(0, 11);
           v.setUint8(1, 22);
           v.setUint8(3, 44);
           let s = serialize(box_any(v));
           print(stable(s));
           print(s[5] as number);
           print(s[6] as number);
           print(s[8] as number);
           print(s[11] as number);
           print(s[12] as number);
           print(s[13] as number);
           let t = new DataView(new ArrayBuffer(6), 1);
           let ts = serialize(box_any(t));
           print(stable(ts));
           print(ts[11] as number);"#,
    );
    assert_eq!(out, ["true", "11", "22", "44", "2", "4", "0", "true", "1"]);
}
