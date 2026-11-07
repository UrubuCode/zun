mod common;
use common::*;

#[test]
fn hello_world() {
    assert_eq!(
        run_source(r#"function main() { print("hello, zun"); }"#),
        ["hello, zun"]
    );
}

#[test]
fn global_set_readonly_registers_and_rejects_reassignment() {
    // A readonly registration is visible through both realm-global aliases and
    // its reassignment is refused: modules are strict, so the store throws a
    // TypeError (the value stays 7).
    assert_eq!(
        run_source(
            r#"import { global } from "zun:core";
            function main() {
                global.set("Foo", box_any(7), true);
                print(dynamic_get(globalThis, "Foo") as number);
                print(dynamic_get(global, "Foo") as number);
            }"#,
        ),
        ["7", "7"],
    );
    let error = run_source_expect_error(
        r#"import { global } from "zun:core";
        function main() {
            global.set("Foo", box_any(7), true);
            dynamic_set(globalThis, "Foo", box_any(9));
        }"#,
    );
    assert!(
        error.contains("Cannot assign to read only property 'Foo'"),
        "{error}"
    );
}

#[test]
fn global_set_readonly_rejects_computed_store() {
    // The computed-key dynamic store path (stdyn.value) also respects the
    // attribute, not only the string-literal `dynamic_set` form.
    // `reflect_set` lowers to the same strict store, so the refused write throws.
    let error = run_source_expect_error(
        r#"import { global } from "zun:core";
        function main() {
            global.set("Foo", box_any(7), true);
            let key: string = "Foo";
            reflect_set(globalThis, key, box_any(9));
        }"#,
    );
    assert!(
        error.contains("Cannot assign to read only property 'Foo'"),
        "{error}"
    );
}

#[test]
fn global_set_writable_can_be_reassigned() {
    assert_eq!(
        run_source(
            r#"import { global } from "zun:core";
            function main() {
                global.set("Bar", box_any(1), false);
                dynamic_set(globalThis, "Bar", box_any(2));
                print(dynamic_get(globalThis, "Bar") as number);
            }"#,
        ),
        ["2"],
    );
}

#[test]
fn dotted_dynamic_store_and_load_round_trip() {
    // Dotted member access/store on a dynamic receiver lowers to the same
    // lddyn/stdyn opcodes as `dynamic_get`/`dynamic_set`.
    assert_eq!(
        run_source(
            r#"function main() {
                globalThis.Foo = 1;
                print(globalThis.Foo as number);
            }"#,
        ),
        ["1"],
    );
}

#[test]
fn dotted_dynamic_store_rejects_readonly_binding() {
    // A dotted reassignment of a readonly global throws a TypeError (strict
    // mode), exactly like the explicit `dynamic_set` store path.
    let error = run_source_expect_error(
        r#"import { global } from "zun:core";
        function main() {
            global.set("Foo", box_any(7), true);
            globalThis.Foo = 9;
        }"#,
    );
    assert!(
        error.contains("Cannot assign to read only property 'Foo'"),
        "{error}"
    );
}

#[test]
fn realm_global_aliases_share_the_same_dynamic_object() {
    assert_eq!(
        run_source(
            r#"function main() {
                dynamic_set(globalThis, "answer", box_any(41));
                print(dynamic_get(global, "answer") as number);
            }"#,
        ),
        ["41"],
    );
}

#[test]
fn realm_map_descriptor_registers_as_readonly_and_reads_back() {
    // The declarative realm directive form (PLAN §3.1): `Map` as a value is the
    // opaque class descriptor, `global.set` registers it readonly, and reading
    // it back through `globalThis` observes the descriptor token (0). A readonly
    // reassignment is silently rejected, so the descriptor survives.
    assert_eq!(
        run_source(
            r#"import { global } from "zun:core";
            import { classDescriptorToken } from "zun:core";
            function main() {
                global.set("Map", Map, true);
                print(classDescriptorToken(dynamic_get(globalThis, "Map")));
                print(classDescriptorToken(dynamic_get(global, "Map")));
            }"#,
        ),
        ["0", "0"],
    );
    // The readonly reassignment is refused with a TypeError (strict mode).
    let error = run_source_expect_error(
        r#"import { global } from "zun:core";
        function main() {
            global.set("Map", Map, true);
            dynamic_set(globalThis, "Map", box_any(9));
        }"#,
    );
    assert!(
        error.contains("Cannot assign to read only property 'Map'"),
        "{error}"
    );
}

#[test]
fn realm_map_is_auto_seeded_by_the_module_initializer() {
    // The module initializer (PLAN §3.1, doc 02 area 10) seeds the realm global
    // before `main` runs, so a program that never calls `global.set` still reads
    // `globalThis.Map` and observes the Map class descriptor (token 0).
    assert_eq!(
        run_source(
            r#"import { classDescriptorToken } from "zun:core";
            function main() {
                print(classDescriptorToken(dynamic_get(globalThis, "Map")));
            }"#,
        ),
        ["0"],
    );
}

#[test]
fn realm_map_new_and_annotation_resolve_through_the_registry() {
    // `new Map()` and the `: Map` annotation resolve via the realm registry yet
    // lower to the same frozen managed-collection representation as before.
    assert_eq!(
        run_source(
            r#"import { mapSet, mapSize } from "zun:core";
            function main() {
                let m: Map = new Map();
                mapSet(m, 1.0, 2.0);
                print(mapSize(m));
            }"#,
        ),
        ["1"],
    );
}

#[test]
fn map_for_each_visits_entries_in_insertion_order_with_value_key_map() {
    // `(value, key, map)` in insertion order. The callback records the key and
    // value of each visit; it also pushes the third (map) argument so the arity
    // is exercised, though the `any` map value is not inspected further.
    let out = run_source(
        r#"function main() {
            let m: Map = new Map();
            m.set(box_any("a"), box_any(1.0));
            m.set(box_any("b"), box_any(2.0));
            let keys = new Array(0);
            let values = new Array(0);
            m.forEach((v: any, k: any, src: any) => {
                values.push(v);
                keys.push(k);
            });
            print(keys[0] as string); print(values[0] as number);
            print(keys[1] as string); print(values[1] as number);
        }"#,
    );
    assert_eq!(out, ["a", "1", "b", "2"]);
}

#[test]
fn set_for_each_passes_the_value_twice_in_insertion_order() {
    let out = run_source(
        r#"function main() {
            let s: Set = new Set();
            s.add(box_any(10.0));
            s.add(box_any(20.0));
            let firsts = new Array(0);
            let seconds = new Array(0);
            s.forEach((v: any, v2: any, src: any) => {
                firsts.push(v);
                seconds.push(v2);
            });
            print(firsts[0] as number); print(seconds[0] as number);
            print(firsts[1] as number); print(seconds[1] as number);
        }"#,
    );
    assert_eq!(out, ["10", "10", "20", "20"]);
}

#[test]
fn map_constructed_from_an_array_of_pairs_holds_the_entries() {
    // `new Map(iterable)` from an array of `[key, value]` pairs (the pairs are
    // built with `new Array` since the language has no array literal).
    let out = run_source(
        r#"function main() {
            let p0 = new Array(2); p0[0] = box_any("a"); p0[1] = box_any(1.0);
            let p1 = new Array(2); p1[0] = box_any("b"); p1[1] = box_any(2.0);
            let pairs = new Array(2); pairs[0] = box_any(p0); pairs[1] = box_any(p1);
            let m: Map = new Map(pairs);
            print(m.size());
            print(m.get(box_any("a")) as number);
            print(m.get(box_any("b")) as number);
            print(m.has(box_any("a")));
        }"#,
    );
    assert_eq!(out, ["2", "1", "2", "true"]);
}

#[test]
fn set_constructed_from_an_array_deduplicates_elements() {
    let out = run_source(
        r#"function main() {
            let items = new Array(4);
            items[0] = box_any(1.0); items[1] = box_any(2.0);
            items[2] = box_any(2.0); items[3] = box_any(3.0);
            let s: Set = new Set(items);
            print(s.size());
            print(s.has(box_any(2.0)));
            print(s.has(box_any(4.0)));
        }"#,
    );
    assert_eq!(out, ["3", "true", "false"]);
}

#[test]
fn empty_map_and_set_constructors_still_work() {
    let out = run_source(
        r#"function main() {
            let m: Map = new Map();
            let s: Set = new Set();
            m.set(box_any("x"), box_any(7.0));
            s.add(box_any(9.0));
            print(m.size()); print(s.size());
        }"#,
    );
    assert_eq!(out, ["1", "1"]);
}

#[test]
fn map_and_set_size_are_properties() {
    // `.size` is the spec PROPERTY form (`map.size`, not `map.size()`), usable
    // directly and inside an expression.
    let out = run_source(
        r#"function main() {
            let m: Map = new Map();
            let s: Set = new Set();
            m.set(box_any("x"), box_any(7.0));
            m.set(box_any("y"), box_any(8.0));
            s.add(box_any(1.0));
            print(m.size);
            print(s.size);
            print(m.size + s.size);
        }"#,
    );
    assert_eq!(out, ["2", "1", "3"]);
}

#[test]
fn set_constructed_from_a_string_holds_its_code_points() {
    // `new Set("abc")` drains the string by code point (iterable constructor).
    let out = run_source(
        r#"function main() {
            let s: Set = new Set("abc");
            print(s.size);
            print(s.has(box_any("a")));
            print(s.has(box_any("z")));
        }"#,
    );
    assert_eq!(out, ["3", "true", "false"]);
}

#[test]
fn set_constructed_from_another_set_copies_its_elements() {
    let out = run_source(
        r#"function main() {
            let src: Set = new Set();
            src.add(box_any(5.0)); src.add(box_any(6.0));
            let copy: Set = new Set(src);
            print(copy.size);
            print(copy.has(box_any(5.0)));
            print(copy.has(box_any(6.0)));
        }"#,
    );
    assert_eq!(out, ["2", "true", "true"]);
}

#[test]
fn map_constructed_from_another_map_copies_its_entries() {
    let out = run_source(
        r#"function main() {
            let src: Map = new Map();
            src.set(box_any("a"), box_any(1.0));
            src.set(box_any("b"), box_any(2.0));
            let copy: Map = new Map(src);
            print(copy.size);
            print(copy.get(box_any("a")) as number);
            print(copy.get(box_any("b")) as number);
        }"#,
    );
    assert_eq!(out, ["2", "1", "2"]);
}

#[test]
fn bigint_constructor_arithmetic_and_formatting_use_the_managed_floor() {
    let out = run_source(
        r#"function main(): int {
            let left: BigInt = new BigInt("18446744073709551616");
            let right: BigInt = new BigInt("2");
            let sum: BigInt = left.add(right);
            if (sum.toString() == "18446744073709551618") { print(sum.compare(left)); return 0; }
            print(0); return 0;
        }"#,
    );
    assert_eq!(out, ["1"]);
}

#[test]
fn bigint_subtraction_preserves_negative_values() {
    let out = run_source(
        r#"function main(): int {
            let left: BigInt = new BigInt("2");
            let right: BigInt = new BigInt("5");
            print(left.sub(right).toString());
            return 0;
        }"#,
    );
    assert_eq!(out, ["-3"]);
}

#[test]
fn bigint_arithmetic_operators_use_the_managed_floor() {
    let out = run_source(
        r#"function main(): int {
            let left: BigInt = new BigInt("7");
            let right: BigInt = new BigInt("3");
            print((left + right).toString());
            print((left - right).toString());
            print((left * right).toString());
            return 0;
        }"#,
    );
    assert_eq!(out, ["10", "4", "21"]);
}

#[test]
fn bigint_comparison_operators_use_the_managed_floor() {
    let out = run_source(
        r#"function main(): int {
            let left: BigInt = new BigInt("7");
            let right: BigInt = new BigInt("3");
            if (left == right) { print(0); }
            if (left != right) { print(1); }
            if (left > right) { print(2); }
            if (right <= left) { print(3); }
            return 0;
        }"#,
    );
    assert_eq!(out, ["1", "2", "3"]);
}

#[test]
fn bigint_unary_negation_uses_the_managed_floor() {
    let out = run_source(
        r#"function main(): int {
            let value: BigInt = new BigInt("7");
            print((-value).toString());
            return 0;
        }"#,
    );
    assert_eq!(out, ["-7"]);
}

#[test]
fn bigint_division_and_remainder_use_truncation_toward_zero() {
    let out = run_source(
        r#"function main(): int {
            let left: BigInt = new BigInt("-17");
            let right: BigInt = new BigInt("5");
            print((left / right).toString());
            print((left % right).toString());
            return 0;
        }"#,
    );
    assert_eq!(out, ["-3", "-2"]);
}

#[test]
fn bigint_literals_use_the_managed_constructor_path() {
    let out = run_source(
        r#"function main(): int {
            let value: BigInt = 12345678901234567890n;
            print(value.toString());
            return 0;
        }"#,
    );
    assert_eq!(out, ["12345678901234567890"]);
}

#[test]
fn bigint_division_power_and_negation_methods() {
    let out = run_source(
        r#"function main(): int {
            print((10n / 3n).toString());
            print((10n % 3n).toString());
            print((2n).pow(10n).toString());
            print((5n).neg().toString());
            return 0;
        }"#,
    );
    assert_eq!(out, ["3", "1", "1024", "-5"]);
}

#[test]
fn bigint_bitwise_and_shift_use_twos_complement() {
    let out = run_source(
        r#"function main(): int {
            print((7n).and(3n).toString());
            print((5n).not().toString());
            print((1n).shl(4n).toString());
            print((8n).neg().shr(1n).toString());
            print((12n).or(1n).toString());
            print((6n).xor(3n).toString());
            return 0;
        }"#,
    );
    assert_eq!(out, ["3", "-6", "16", "-4", "13", "5"]);
}

#[test]
fn bigint_coercion_from_number_and_radix_string() {
    let out = run_source(
        r#"function main(): int {
            print(BigInt(42).toString());
            print(BigInt("0xff").toString());
            print(BigInt("0b1010").toString());
            return 0;
        }"#,
    );
    assert_eq!(out, ["42", "255", "10"]);
}

#[test]
fn bigint_coercion_rejects_non_integer_number() {
    let error = run_source_expect_error(
        r#"function main(): int {
            print(BigInt(1.5).toString());
            return 0;
        }"#,
    );
    assert!(error.contains("non-integer"), "unexpected error: {error}");
}

#[test]
fn bigint_division_by_zero_is_a_runtime_error() {
    let error = run_source_expect_error(
        r#"function main(): int {
            print((10n / 0n).toString());
            return 0;
        }"#,
    );
    assert!(
        error.contains("division by zero"),
        "unexpected error: {error}"
    );
}

#[test]
fn bigint_as_int_n_and_as_uint_n_wrap() {
    let out = run_source(
        r#"function main(): int {
            print(BigInt.asIntN(8, 255n).toString());
            print(BigInt.asUintN(8, 256n).toString());
            print(BigInt.asIntN(4, 9n).toString());
            return 0;
        }"#,
    );
    assert_eq!(out, ["-1", "0", "-7"]);
}

#[test]
fn bigint_operator_and_method_forms_agree() {
    let out = run_source(
        r#"function main(): int {
            let a: BigInt = 10n;
            let b: BigInt = 3n;
            if ((a / b).toString() == a.div(b).toString()) { print(1); }
            if ((a % b).toString() == a.rem(b).toString()) { print(2); }
            if ((a * b).toString() == a.mul(b).toString()) { print(3); }
            return 0;
        }"#,
    );
    assert_eq!(out, ["1", "2", "3"]);
}
