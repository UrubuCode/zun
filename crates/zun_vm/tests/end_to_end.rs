//! End-to-end pipeline tests: TypeScript source all the way to program output,
//! through the real `.zm` serialization round-trip.

use zun_interp::CaptureOutput;

/// Compile `src` with the std prelude, run it, and return the output lines.
fn run_with_std(user: &str) -> Vec<String> {
    run_source(&zun_prelude::with_prelude(user))
}

/// Compile `src` to a module, serialize it, and run the bytes, returning the
/// captured output lines.
fn run_source(src: &str) -> Vec<String> {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse errors: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);

    // Go through the on-disk format so the test also exercises serialization.
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

/// Compile and run `src`, expecting a runtime failure; return the error text.
fn run_source_expect_error(src: &str) -> String {
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

#[test]
fn boolean_coercion_matches_js_truthiness() {
    // `Boolean(x)` returns the JS truthiness of `x` as a `boolean` primitive,
    // consistent with the truthiness used for `if`/`&&` conditions.
    let out = run_source(
        r#"function main() {
            if (!Boolean(0.0)) { print("zero-false"); }
            if (Boolean(1.0)) { print("one-true"); }
            if (!Boolean("")) { print("empty-false"); }
            if (Boolean("x")) { print("nonempty-true"); }
            if (!Boolean(box_any(null))) { print("null-false"); }
            if (Boolean(true)) { print("bool-passthrough"); }
        }"#,
    );
    assert_eq!(
        out,
        [
            "zero-false",
            "one-true",
            "empty-false",
            "nonempty-true",
            "null-false",
            "bool-passthrough",
        ]
    );
}

#[test]
fn boolean_primitive_to_string() {
    let out = run_source(
        r#"function main() {
            print(true.toString());
            print(false.toString());
        }"#,
    );
    assert_eq!(out, ["true", "false"]);
}

#[test]
fn global_this_boolean_resolves() {
    // `Boolean` is a realm global; reading it through `globalThis` yields its
    // class descriptor (a stable token), proving the binding resolves.
    let out = run_source(
        r#"import { classDescriptorToken } from "zun:core";
        function main() {
            let token: number = classDescriptorToken(dynamic_get(globalThis, "Boolean"));
            if (token >= 0.0) { print("boolean-resolves"); }
        }"#,
    );
    assert_eq!(out, ["boolean-resolves"]);
}

#[test]
fn object_keys_values_entries_on_a_dynamic_object_follow_insertion_order() {
    let out = run_source(
        r#"function main() {
            let o = dynamic_new();
            dynamic_set(o, "b", box_any(2));
            dynamic_set(o, "a", box_any(1));
            dynamic_set(o, "c", box_any(3));
            let keys = Object.keys(o);
            print(keys.length);
            print(keys[0] as string);
            print(keys[1] as string);
            print(keys[2] as string);
            let values = Object.values(o);
            print(values[0] as number);
            print(values[1] as number);
            print(values[2] as number);
            let entries = Object.entries(o);
            let first = entries[0] as Array;
            print(first[0] as string);
            print(first[1] as number);
        }"#,
    );
    assert_eq!(out, ["3", "b", "a", "c", "2", "1", "3", "b", "2"]);
}

#[test]
fn a_decorated_class_runs_identically_to_the_undecorated_version() {
    // Phase 1 decorators are syntax only: they are parsed and carried into the
    // AST/HIR but apply no semantics. A decorated class must compile and run
    // exactly as if undecorated. Decorator callees (`sealed`, `trace`) need no
    // definition because nothing resolves or executes them this phase.
    let decorated = run_source(
        "@sealed\n\
         class Counter { value: number;\n\
            @trace\n\
            bump(): number { return this.value + 1.0; }\n\
         }\n\
         function main() {\n\
            let c = new Counter(41.0);\n\
            print(c.bump());\n\
         }",
    );
    let undecorated = run_source(
        "class Counter { value: number;\n\
            bump(): number { return this.value + 1.0; }\n\
         }\n\
         function main() {\n\
            let c = new Counter(41.0);\n\
            print(c.bump());\n\
         }",
    );
    assert_eq!(decorated, ["42"]);
    assert_eq!(decorated, undecorated);
}

#[test]
fn object_keys_and_values_on_a_typed_object_use_declared_fields() {
    let out = run_source(
        "class Point { x: number; y: number; }\n\
         function main() {\n\
            let p = new Point(10.0, 20.0);\n\
            let keys = Object.keys(p);\n\
            print(keys.length);\n\
            print(keys[0] as string);\n\
            print(keys[1] as string);\n\
            let values = Object.values(p);\n\
            print(values[0] as number);\n\
            print(values[1] as number);\n\
         }",
    );
    assert_eq!(out, ["2", "x", "y", "10", "20"]);
}

#[test]
fn object_assign_merges_sources_and_returns_the_target() {
    let out = run_source(
        r#"function main() {
            let target = dynamic_new();
            dynamic_set(target, "a", box_any(1));
            let source = dynamic_new();
            dynamic_set(source, "b", box_any(2));
            let result = Object.assign(target, source);
            let keys = Object.keys(result);
            print(keys.length);
            print(keys[0] as string);
            print(keys[1] as string);
            print(dynamic_get(target, "b") as number);
        }"#,
    );
    assert_eq!(out, ["2", "a", "b", "2"]);
}

#[test]
fn object_freeze_blocks_dynamic_mutation_and_is_frozen_reports_it() {
    let out = run_source(
        r#"function main() {
            let o = dynamic_new();
            dynamic_set(o, "a", box_any(1));
            if (Object.isFrozen(o)) { print("before-frozen"); } else { print("before-open"); }
            Object.freeze(o);
            if (Object.isFrozen(o)) { print("after-frozen"); } else { print("after-open"); }
            dynamic_set(o, "b", box_any(2));
            print(Object.keys(o).length);
        }"#,
    );
    assert_eq!(out, ["before-open", "after-frozen", "1"]);
}

#[test]
fn object_is_implements_same_value_including_nan_and_negative_zero() {
    let out = run_source(
        r#"function main() {
            let nan = 0.0 / 0.0;
            if (Object.is(nan, nan)) { print("nan-equal"); }
            let zero = 0.0;
            let negative = -0.0;
            if (Object.is(zero, negative)) { print("zero-equal"); } else { print("zero-distinct"); }
            if (Object.is(negative, negative)) { print("neg-equal"); }
            if (Object.is(2.0, 2.0)) { print("two-equal"); }
        }"#,
    );
    assert_eq!(out, ["nan-equal", "zero-distinct", "neg-equal", "two-equal"]);
}

#[test]
fn number_strict_predicates_do_not_coerce() {
    let out = run_source(
        r#"function main() {
            if (Number.isNaN(Number.NaN)) { print("nan-true"); }
            if (!Number.isNaN(42.0)) { print("nan-false-number"); }
            if (!Number.isNaN(box_any("x"))) { print("nan-false-string"); }
            if (Number.isInteger(4.0)) { print("int-true"); }
            if (!Number.isInteger(4.5)) { print("int-false"); }
            if (!Number.isFinite(Number.POSITIVE_INFINITY)) { print("finite-false"); }
            if (Number.isFinite(3.0)) { print("finite-true"); }
            if (!Number.isSafeInteger(9007199254740992.0)) { print("safe-false"); }
            if (Number.isSafeInteger(9007199254740991.0)) { print("safe-true"); }
        }"#,
    );
    assert_eq!(
        out,
        [
            "nan-true",
            "nan-false-number",
            "nan-false-string",
            "int-true",
            "int-false",
            "finite-false",
            "finite-true",
            "safe-false",
            "safe-true",
        ]
    );
}

#[test]
fn number_parse_statics_and_constants() {
    let out = run_source(
        r#"function main() {
            print(Number.parseInt("42"));
            print(Number.parseInt("ff", 16));
            print(Number.parseFloat("3.14"));
            print(Number.MAX_SAFE_INTEGER);
            if (Number.EPSILON > 0.0) { print("epsilon-positive"); }
        }"#,
    );
    assert_eq!(out, ["42", "255", "3.14", "9007199254740991", "epsilon-positive"]);
}

#[test]
fn number_to_string_radix() {
    let out = run_source(
        r#"function main() {
            print((255.0).toString(16));
            print((10.0).toString(2));
            print((255.0).toString());
        }"#,
    );
    assert_eq!(out, ["ff", "1010", "255"]);
}

#[test]
fn object_from_entries_round_trips_object_entries() {
    let out = run_source(
        r#"function main() {
            let o = dynamic_new();
            dynamic_set(o, "a", box_any(1));
            dynamic_set(o, "b", box_any(2));
            let rebuilt = Object.fromEntries(Object.entries(o));
            let keys = Object.keys(rebuilt);
            print(keys.length);
            print(keys[0] as string);
            print(keys[1] as string);
            print(dynamic_get(rebuilt, "a") as number);
            print(dynamic_get(rebuilt, "b") as number);
        }"#,
    );
    assert_eq!(out, ["2", "a", "b", "1", "2"]);
}

#[test]
fn object_has_own_property_on_dynamic_and_typed_objects() {
    let out = run_source(
        "class Point { x: number; y: number; }\n\
         function main() {\n\
            let o = dynamic_new();\n\
            dynamic_set(o, \"a\", box_any(1));\n\
            if (o.hasOwnProperty(\"a\")) { print(\"dyn-has-a\"); }\n\
            if (!o.hasOwnProperty(\"b\")) { print(\"dyn-no-b\"); }\n\
            let p = new Point(1.0, 2.0);\n\
            if (p.hasOwnProperty(\"x\")) { print(\"typed-has-x\"); }\n\
            if (!p.hasOwnProperty(\"z\")) { print(\"typed-no-z\"); }\n\
         }",
    );
    assert_eq!(out, ["dyn-has-a", "dyn-no-b", "typed-has-x", "typed-no-z"]);
}

#[test]
fn object_create_sets_the_prototype_chain_for_is_prototype_of() {
    let out = run_source(
        r#"function main() {
            let proto = dynamic_new();
            dynamic_set(proto, "kind", box_any(7));
            let child = Object.create(proto);
            if (proto.isPrototypeOf(child)) { print("proto-of-child"); }
            let other = dynamic_new();
            if (!other.isPrototypeOf(child)) { print("not-proto-of-child"); }
        }"#,
    );
    assert_eq!(out, ["proto-of-child", "not-proto-of-child"]);
}

#[test]
fn function_call_threads_this_as_the_leading_argument() {
    // `f.call(thisArg, x)` invokes `f` with `thisArg` bound to its first
    // parameter and `x` to the second.
    let out = run_source(
        "function greet(base: number, x: number): number { return base + x; }\n\
         function main() { print(greet.call(10.0, 5.0)); }",
    );
    assert_eq!(out, ["15"]);
}

/// Compile `user` together with the `Array` class source (so its Zun-extension
/// methods resolve) and run it.
fn run_with_array_class(user: &str) -> Vec<String> {
    let src = format!("{user}\n{}", include_str!("../../../builtin/array.zun"));
    run_source(&src)
}

#[test]
fn array_custom_extensions_are_preserved() {
    // `chunk` and the renamed numeric `sortNumeric` are Zun extensions beyond
    // ECMAScript; they must keep working alongside the standard native methods.
    let out = run_with_array_class(
        "function main() { \
            let a = new Array(4); a[0] = 3.0; a[1] = 1.0; a[2] = 4.0; a[3] = 2.0; \
            a.sortNumeric(); \
            print(a[0] as number); print(a[3] as number); \
            let groups = a.chunk(2 as u64); \
            print(groups.length); \
            let first = groups[0] as Array; \
            print(first[0] as number); print(first[1] as number); }",
    );
    assert_eq!(out, ["1", "4", "2", "1", "2"]);
}

#[test]
fn array_literal_length_and_indexing() {
    let out = run_source(
        "function main() { \
            print([1.0, 2.0, 3.0].length); \
            print([1.0, 2.0, 3.0][1] as number); }",
    );
    assert_eq!(out, ["3", "2"]);
}

#[test]
fn array_literal_empty_and_nested() {
    // Array elements are `Any`, so a nested access casts the middle value back
    // to `Array` before indexing it again.
    let out = run_source(
        "function main() { \
            let e = []; \
            print(e.length); \
            let inner = [[1.0], [2.0]][0] as Array; \
            print(inner[0] as number); }",
    );
    assert_eq!(out, ["0", "1"]);
}

#[test]
fn array_literal_composes_with_map_and_sort() {
    let out = run_source(
        "function main() { \
            let doubled = [1.0, 2.0, 3.0].map((x: any, i: any, a: any) => (x as number) * 2.0); \
            print(doubled[0] as number); print(doubled[2] as number); \
            let sorted = [3.0, 1.0, 2.0].sort(); \
            print(sorted[0] as number); print(sorted[2] as number); }",
    );
    assert_eq!(out, ["2", "6", "1", "3"]);
}

#[test]
fn array_literal_feeds_set_and_map_constructors() {
    let out = run_source(
        r#"function main() {
            let s: Set = new Set([1.0, 2.0, 2.0, 3.0]);
            print(s.size());
            let m: Map = new Map([["a", 1.0]]);
            print(m.get(box_any("a")) as number);
        }"#,
    );
    assert_eq!(out, ["3", "1"]);
}

/// Build `a` as a fresh `[1.0, 2.0, 3.0]` array in a program preamble.
fn array_123() -> &'static str {
    "let a = new Array(3); a[0] = 1.0; a[1] = 2.0; a[2] = 3.0;"
}

#[test]
fn array_map_passes_value_index_array_and_builds_a_new_array() {
    let out = run_source(&format!(
        "function main() {{ {} \
            let b = a.map((v: any, i: any, arr: any) => (v as number) + (i as number)); \
            print(b[0] as number); print(b[1] as number); print(b[2] as number); \
            print(a.length); }}",
        array_123()
    ));
    // (1+0), (2+1), (3+2); source length unchanged.
    assert_eq!(out, ["1", "3", "5", "3"]);
}

#[test]
fn array_for_each_runs_in_order_with_side_effects() {
    let out = run_source(&format!(
        "function main() {{ {} \
            let sink = new Array(0); \
            a.forEach((v: any, i: any, arr: any) => {{ sink.push(v); }}); \
            print(sink.length); print(sink[0] as number); print(sink[2] as number); }}",
        array_123()
    ));
    assert_eq!(out, ["3", "1", "3"]);
}

#[test]
fn array_filter_keeps_truthy_predicate_elements() {
    let out = run_source(&format!(
        "function main() {{ {} \
            let b = a.filter((v: any, i: any, arr: any) => (v as number) > 1.0); \
            print(b.length); print(b[0] as number); print(b[1] as number); }}",
        array_123()
    ));
    assert_eq!(out, ["2", "2", "3"]);
}

#[test]
fn array_reduce_and_reduce_right_thread_the_accumulator() {
    let out = run_source(&format!(
        "function main() {{ {} \
            let sum = a.reduce((acc: any, v: any, i: any, arr: any) => (acc as number) + (v as number), 10.0); \
            print(sum as number); \
            let order = a.reduceRight((acc: any, v: any, i: any, arr: any) => (acc as number) * 10.0 + (v as number), 0.0); \
            print(order as number); }}",
        array_123()
    ));
    // reduce: 10+1+2+3 = 16; reduceRight digit-builds right-to-left -> 321.
    assert_eq!(out, ["16", "321"]);
}

#[test]
fn array_find_find_index_some_every() {
    let out = run_source(&format!(
        "function main() {{ {} \
            print(a.find((v: any, i: any, arr: any) => (v as number) > 1.0) as number); \
            print(a.findIndex((v: any, i: any, arr: any) => (v as number) > 1.0)); \
            print(a.some((v: any, i: any, arr: any) => (v as number) > 2.0)); \
            print(a.every((v: any, i: any, arr: any) => (v as number) > 0.0)); \
            print(a.every((v: any, i: any, arr: any) => (v as number) > 1.0)); }}",
        array_123()
    ));
    assert_eq!(out, ["2", "1", "true", "true", "false"]);
}

#[test]
fn array_join_with_separator() {
    let out = run_source(&format!(
        "function main() {{ {} print(a.join(\"-\")); }}",
        array_123()
    ));
    assert_eq!(out, ["1-2-3"]);
}

#[test]
fn array_shift_unshift_splice_mutate_in_place() {
    let out = run_source(&format!(
        "function main() {{ {} \
            let first = a.shift(); print(first as number); print(a.length); \
            let len = a.unshift(9.0); print(len); print(a[0] as number); \
            let removed = a.splice(1 as u64, 1 as u64); \
            print(removed.length); print(removed[0] as number); print(a.length); }}",
        array_123()
    ));
    // shift -> 1, len 2 ([2,3]); unshift 9 -> len 3 ([9,2,3]); splice(1,1) removes [2] -> len 2.
    assert_eq!(out, ["1", "2", "3", "9", "1", "2", "2"]);
}

#[test]
fn array_find_last_and_find_last_index() {
    let out = run_source(
        "function main() { \
            let a = new Array(4); a[0]=1.0; a[1]=2.0; a[2]=3.0; a[3]=2.0; \
            print(a.findLast((v: any, i: any, arr: any) => (v as number) == 2.0) as number); \
            print(a.findLastIndex((v: any, i: any, arr: any) => (v as number) == 2.0)); \
            print(a.findLast((v: any, i: any, arr: any) => (v as number) == 9.0)); \
            print(a.findLastIndex((v: any, i: any, arr: any) => (v as number) == 9.0)); }",
    );
    // Last 2 is at index 3; a miss yields undefined / -1.
    assert_eq!(out, ["2", "3", "undefined", "-1"]);
}

#[test]
fn array_flat_depth_default_and_explicit() {
    let out = run_source(
        "function main() { \
            let inner = new Array(1); inner[0] = 3.0; \
            let mid = new Array(2); mid[0] = 2.0; mid[1] = inner; \
            let a = new Array(2); a[0] = 1.0; a[1] = mid; \
            let one = a.flat(); \
            print(one.length); print(one[0] as number); print(one[1] as number); \
            let two = a.flat(2.0); \
            print(two.length); print(two[2] as number); }",
    );
    // flat(): [1, 2, [3]] -> length 3; flat(2): [1, 2, 3] -> length 3, last 3.
    assert_eq!(out, ["3", "1", "2", "3", "3"]);
}

#[test]
fn array_flat_map_maps_then_flattens_one_level() {
    let out = run_source(
        "function main() { \
            let a = new Array(2); a[0] = 1.0; a[1] = 2.0; \
            let b = a.flatMap((x: any, i: any, arr: any) => { \
                let pair = new Array(2); pair[0] = x; pair[1] = x; return pair; }); \
            print(b.length); print(b[0] as number); print(b[1] as number); \
            print(b[2] as number); print(b[3] as number); }",
    );
    // [1,1,2,2]
    assert_eq!(out, ["4", "1", "1", "2", "2"]);
}

#[test]
fn array_at_relative_index() {
    let out = run_source(
        "function main() { \
            let a = new Array(3); a[0]=1.0; a[1]=2.0; a[2]=3.0; \
            print(a.at(-1.0) as number); print(a.at(0.0) as number); \
            print(a.at(-5.0)); print(a.at(10.0)); }",
    );
    assert_eq!(out, ["3", "1", "undefined", "undefined"]);
}

#[test]
fn array_keys_values_entries() {
    let out = run_source(
        "function main() { \
            let a = new Array(2); a[0] = 10.0; a[1] = 20.0; \
            let k = a.keys(); print(k[0] as number); print(k[1] as number); \
            let v = a.values(); print(v[0] as number); print(v[1] as number); \
            let e = a.entries(); let p = e[1] as Array; \
            print(p[0] as number); print(p[1] as number); }",
    );
    // keys 0,1; values 10,20; entries[1] = [1, 20].
    assert_eq!(out, ["0", "1", "10", "20", "1", "20"]);
}

#[test]
fn array_is_array_static() {
    let out = run_source(
        "function main() { \
            let a = new Array(0); \
            print(Array.isArray(a)); print(Array.isArray(5.0)); print(Array.isArray(\"x\")); }",
    );
    assert_eq!(out, ["true", "false", "false"]);
}

#[test]
fn array_of_static_builds_array() {
    let out = run_source(
        "function main() { \
            let a = Array.of(1.0, 2.0, 3.0); \
            print(a.length); print(a[0] as number); print(a[2] as number); }",
    );
    assert_eq!(out, ["3", "1", "3"]);
}

#[test]
fn array_from_string_and_mapped() {
    let out = run_source(
        "function main() { \
            let chars = Array.from(\"ab\"); \
            print(chars.length); print(chars[0] as string); print(chars[1] as string); \
            let src = new Array(2); src[0] = 1.0; src[1] = 2.0; \
            let doubled = Array.from(src, (x: any, i: any) => (x as number) * 2.0); \
            print(doubled[0] as number); print(doubled[1] as number); }",
    );
    assert_eq!(out, ["2", "a", "b", "2", "4"]);
}

#[test]
fn array_splice_inserts_variadic_items() {
    let out = run_source(
        "function main() { \
            let a = new Array(2); a[0] = \"a\"; a[1] = \"b\"; \
            let removed = a.splice(1 as u64, 0 as u64, \"x\"); \
            print(removed.length); print(a.length); \
            print(a[0] as string); print(a[1] as string); print(a[2] as string); }",
    );
    // splice(1,0,"x") inserts "x" at index 1: ["a","x","b"], nothing removed.
    assert_eq!(out, ["0", "3", "a", "x", "b"]);
}

#[test]
fn array_reduce_without_initial_value() {
    let out = run_source(
        "function main() { \
            let a = new Array(3); a[0]=1.0; a[1]=2.0; a[2]=3.0; \
            print(a.reduce((acc: any, v: any, i: any, arr: any) => (acc as number) + (v as number)) as number); \
            print(a.reduceRight((acc: any, v: any, i: any, arr: any) => (acc as number) * 10.0 + (v as number)) as number); }",
    );
    // reduce no-init: 1+2+3 = 6; reduceRight no-init seeds with 3 -> 3*10+2=32, *10+1=321.
    assert_eq!(out, ["6", "321"]);
}

#[test]
fn array_reduce_empty_no_initial_throws() {
    let error = run_source_expect_error(
        "function main(): number { \
            let a = new Array(0); \
            return a.reduce((acc: any, v: any, i: any, arr: any) => (acc as number) + (v as number)) as number; }",
    );
    assert!(error.contains("Reduce of empty array"), "unexpected error: {error}");
}

#[test]
fn array_sort_default_is_string_ascending() {
    let out = run_source(
        "function main() { \
            let a = new Array(3); a[0] = 10.0; a[1] = 2.0; a[2] = 1.0; \
            a.sort(); \
            print(a[0] as number); print(a[1] as number); print(a[2] as number); }",
    );
    // String comparison: "1" < "10" < "2".
    assert_eq!(out, ["1", "10", "2"]);
}

#[test]
fn array_sort_with_numeric_comparator() {
    let out = run_source(
        "function main() { \
            let a = new Array(3); a[0] = 10.0; a[1] = 2.0; a[2] = 1.0; \
            a.sort((x: any, y: any) => (x as number) - (y as number)); \
            print(a[0] as number); print(a[1] as number); print(a[2] as number); }",
    );
    assert_eq!(out, ["1", "2", "10"]);
}

#[test]
fn array_map_with_a_capturing_closure_survives_allocation() {
    // The closure captures `bias`; `map` allocates a result array and re-enters
    // the interpreter per element, exercising native rooting of the closure env.
    let out = run_source(&format!(
        "function main() {{ {} \
            let bias: number = 100.0; \
            let b = a.map((v: any, i: any, arr: any) => (v as number) + bias); \
            print(b[0] as number); print(b[2] as number); }}",
        array_123()
    ));
    assert_eq!(out, ["101", "103"]);
}

#[test]
fn function_apply_spreads_an_array_and_equals_call() {
    let out = run_source(
        "function greet(base: number, x: number): number { return base + x; }\n\
         function main() {\n\
            let a = new Array(1);\n\
            a[0] = 5.0;\n\
            print((greet.apply(10.0, a)) as number);\n\
            print(greet.call(10.0, 5.0));\n\
         }",
    );
    assert_eq!(out, ["15", "15"]);
}

#[test]
fn function_bind_fixes_this_and_partial_arguments() {
    // `const g = f.bind(1, 2); g(3)` equals `f.call(1, 2, 3)`.
    let out = run_source(
        "function add3(a: number, b: number, c: number): number { return a + b + c; }\n\
         function main() {\n\
            let g = add3.bind(1.0, 2.0);\n\
            print((g(3.0)) as number);\n\
            print(add3.call(1.0, 2.0, 3.0));\n\
         }",
    );
    assert_eq!(out, ["6", "6"]);
}

#[test]
fn bound_function_is_a_first_class_value() {
    // A bound function can be stored and passed, then called elsewhere.
    let out = run_source(
        "function add3(a: number, b: number, c: number): number { return a + b + c; }\n\
         function invokeWith(fn: Function, x: number): number { return (fn(x)) as number; }\n\
         function main() {\n\
            let g = add3.bind(1.0, 2.0);\n\
            print(invokeWith(g, 3.0));\n\
         }",
    );
    assert_eq!(out, ["6"]);
}

#[test]
fn function_name_and_length_resolve_from_the_signature() {
    let out = run_source(
        "function add3(a: number, b: number, c: number): number { return a + b + c; }\n\
         function main() {\n\
            print(add3.name);\n\
            print(add3.length as number);\n\
         }",
    );
    assert_eq!(out, ["add3", "3"]);
}

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
    // its reassignment is silently rejected (freeze-consistent, non-throwing).
    assert_eq!(
        run_source(
            r#"import { global } from "zun:core";
            function main() {
                global.set("Foo", box_any(7), true);
                print(dynamic_get(globalThis, "Foo") as number);
                dynamic_set(globalThis, "Foo", box_any(9));
                print(dynamic_get(global, "Foo") as number);
            }"#,
        ),
        ["7", "7"],
    );
}

#[test]
fn global_set_readonly_rejects_computed_store() {
    // The computed-key dynamic store path (stdyn.value) also respects the
    // attribute, not only the string-literal `dynamic_set` form.
    assert_eq!(
        run_source(
            r#"import { global } from "zun:core";
            function main() {
                global.set("Foo", box_any(7), true);
                let key: string = "Foo";
                reflect_set(globalThis, key, box_any(9));
                print(dynamic_get(global, "Foo") as number);
            }"#,
        ),
        ["7"],
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
    // A dotted reassignment of a readonly global is silently rejected, exactly
    // like the explicit `dynamic_set` store path (freeze-consistent).
    assert_eq!(
        run_source(
            r#"import { global } from "zun:core";
            function main() {
                global.set("Foo", box_any(7), true);
                globalThis.Foo = 9;
                print(globalThis.Foo as number);
            }"#,
        ),
        ["7"],
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
                dynamic_set(globalThis, "Map", box_any(9));
                print(classDescriptorToken(dynamic_get(global, "Map")));
            }"#,
        ),
        ["0", "0"],
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
    assert!(error.contains("division by zero"), "unexpected error: {error}");
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

#[test]
fn arithmetic_and_locals() {
    let out = run_source("function main() { let a = 6; let b = 7; print(a * b); }");
    assert_eq!(out, ["42"]);
}

#[test]
fn user_function_call_with_args() {
    let out = run_source(
        "function add(a: number, b: number): number { return a + b; }\n\
         function main() { print(add(2, 40)); }",
    );
    assert_eq!(out, ["42"]);
}

#[test]
fn tiering_calls_a_heap_eligible_array_reader() {
    let out = run_source(
        "function read(values: Array): u64 { return values[0] as u64; }\n\
         function main() { let values = new Array(1); values[0] = 41 as u64; print(read(values)); print(read(values)); }",
    );
    assert_eq!(out, ["41", "41"]);
}

#[test]
fn tiering_calls_a_heap_eligible_f64_array_reader() {
    let out = run_source(
        "function read(values: Array): number { return values[0] as number; }\n\
         function main() { let values = new Array(1); values[0] = 41.0; print(read(values)); print(read(values)); }",
    );
    assert_eq!(out, ["41", "41"]);
}

#[test]
fn tiering_calls_a_heap_eligible_f64_array_writer() {
    let out = run_source(
        "function write(values: Array, value: number): number { values[0] = value; return value; }\n\
         function main() { let values = new Array(1); print(write(values, 41.0)); print(write(values, 42.0)); print(values[0]); }",
    );
    assert_eq!(out, ["41", "42", "42"]);
}

#[test]
fn tiering_calls_a_heap_eligible_u64_array_writer() {
    let out = run_source(
        "function write(values: Array, value: u64): u64 { values[0] = value; return value; }\n\
         function main() { let values = new Array(1); print(write(values, 41 as u64)); print(write(values, 42 as u64)); print(values[0] as u64); }",
    );
    assert_eq!(out, ["41", "42", "42"]);
}

#[test]
fn tiering_calls_a_heap_eligible_f64_array_push() {
    let out = run_source(
        "function push(values: Array, value: number): number { return values.push(value) as number; }\n\
         function main() { let values = new Array(0); print(push(values, 41.0)); print(push(values, 42.0)); print(values.length); }",
    );
    assert_eq!(out, ["1", "2", "2"]);
}

#[test]
fn tiering_calls_a_heap_eligible_f64_array_pop() {
    let out = run_source(
        "function pop(values: Array): number { return values.pop() as number; }\n\
         function main() { let values = new Array(2); values[0] = 41.0; values[1] = 42.0; print(pop(values)); print(pop(values)); print(values.length); }",
    );
    assert_eq!(out, ["42", "41", "0"]);
}

#[test]
fn tiering_calls_heap_eligible_u64_array_push_and_pop() {
    let out = run_source(
        "function push(values: Array, value: u64): u64 { return values.push(value) as u64; }\n\
         function pop(values: Array): u64 { return values.pop() as u64; }\n\
         function main() { let values = new Array(0); print(push(values, 41 as u64)); print(push(values, 42 as u64)); print(pop(values)); print(pop(values)); }",
    );
    assert_eq!(out, ["1", "2", "42", "41"]);
}

#[test]
fn tiering_calls_a_heap_eligible_object_field_reader() {
    let out = run_source(
        "class Box { value: number; get(): number { return this.value; } }\n\
         function main() { let box = new Box(41.0); print(box.get()); print(box.get()); }",
    );
    assert_eq!(out, ["41", "41"]);
}

#[test]
fn tiering_calls_a_heap_eligible_object_field_writer() {
    let out = run_source(
        "class Box { value: number; set(value: number): number { this.value = value; return this.value; } }\n\
         function main() { let box = new Box(0.0); print(box.set(41.0)); print(box.set(42.0)); print(box.value); }",
    );
    assert_eq!(out, ["41", "42", "42"]);
}

#[test]
fn tiering_calls_a_heap_eligible_non_overridden_virtual_method() {
    let out = run_source(
        "class Box { value: number; get(): number { return this.value; } }\n\
         function main() { let box = new Box(41.0); print(box.get()); print(box.get()); }",
    );
    assert_eq!(out, ["41", "41"]);
}

#[test]
fn tiering_returns_a_jit_allocated_object_through_the_vm() {
    let out = run_source(
        "class Box { value: number; }\n\
         function make(): Box { return new Box(41.0); }\n\
         function main() { let first = make(); let second = make(); print(first.value); print(second.value); }",
    );
    assert_eq!(out, ["41", "41"]);
}

#[test]
fn tiering_returns_a_managed_object_field_through_the_vm() {
    let out = run_source(
        "class Box { value: number; }\n\
         class Holder { box: Box; get(): Box { return this.box; } }\n\
         function main() { let box = new Box(41.0); let holder = new Holder(box); let first = holder.get(); let second = holder.get(); print(first.value); print(second.value); }",
    );
    assert_eq!(out, ["41", "41"]);
}

#[test]
fn tiering_returns_jit_allocated_arrays_through_the_vm() {
    let out = run_source(
        "function make(length: u64): Array { return new Array(length); }\n\
         function main() { let first = make(2 as u64); let second = make(3 as u64); print(first.length); print(second.length); }",
    );
    assert_eq!(out, ["2", "3"]);
}

#[test]
fn while_loop_sums() {
    let out = run_source(
        "function main() {\n\
            let n = 10; let acc = 0;\n\
            while (n > 0) { acc = acc + n; n = n - 1; }\n\
            print(acc);\n\
         }",
    );
    assert_eq!(out, ["55"]); // 10+9+...+1
}

#[test]
fn if_else_picks_branch() {
    let out = run_source(
        "function main() {\n\
            let x = 3;\n\
            if (x > 5) { print(\"big\"); } else { print(\"small\"); }\n\
         }",
    );
    assert_eq!(out, ["small"]);
}

#[test]
fn recursion_factorial() {
    let out = run_source(
        "function fact(n: number): number {\n\
            if (n < 2) { return 1; }\n\
            return n * fact(n - 1);\n\
         }\n\
         function main() { print(fact(5)); }",
    );
    assert_eq!(out, ["120"]);
}

#[test]
fn string_concatenation() {
    let out = run_source(
        "function greet(who: string): string { return \"hi, \" + who; }\n\
         function main() { print(greet(\"zun\")); }",
    );
    assert_eq!(out, ["hi, zun"]);
}

#[test]
fn class_field_read() {
    let out = run_source(
        "class Point { x: number; y: number; }\n\
         function main() { let p = new Point(3, 4); print(p.x); print(p.y); }",
    );
    assert_eq!(out, ["3", "4"]);
}

#[test]
fn class_field_write_and_reference_semantics() {
    // Writing through one binding is visible through an alias (reference type).
    let out = run_source(
        "class Counter { n: number; }\n\
         function bump(c: Counter) { c.n = c.n + 1; }\n\
         function main() {\n\
            let c = new Counter(0);\n\
            bump(c); bump(c); bump(c);\n\
            print(c.n);\n\
         }",
    );
    assert_eq!(out, ["3"]);
}

#[test]
fn nested_objects() {
    let out = run_source(
        "class Inner { v: number; }\n\
         class Outer { inner: Inner; }\n\
         function main() {\n\
            let o = new Outer(new Inner(42));\n\
            print(o.inner.v);\n\
         }",
    );
    assert_eq!(out, ["42"]);
}

#[test]
fn field_assignment_as_expression() {
    let out = run_source(
        "class Box { v: number; }\n\
         function main() { let b = new Box(0); let y = (b.v = 7); print(y); print(b.v); }",
    );
    assert_eq!(out, ["7", "7"]);
}

#[test]
fn instance_methods_with_this() {
    let out = run_source(
        "class Counter {\n\
            n: number;\n\
            inc() { this.n = this.n + 1; }\n\
            get(): number { return this.n; }\n\
         }\n\
         function main() {\n\
            let c = new Counter(0);\n\
            c.inc(); c.inc(); c.inc();\n\
            print(c.get());\n\
         }",
    );
    assert_eq!(out, ["3"]);
}

#[test]
fn method_with_parameters() {
    let out = run_source(
        "class Adder {\n\
            base: number;\n\
            add(x: number): number { return this.base + x; }\n\
         }\n\
         function main() { let a = new Adder(10); print(a.add(5)); }",
    );
    assert_eq!(out, ["15"]);
}

#[test]
fn method_calling_another_method() {
    let out = run_source(
        "class Rect {\n\
            w: number;\n\
            h: number;\n\
            area(): number { return this.w * this.h; }\n\
            describe(): number { return this.area() + this.area(); }\n\
         }\n\
         function main() { let r = new Rect(3, 4); print(r.describe()); }",
    );
    assert_eq!(out, ["24"]); // 12 + 12
}

#[test]
fn inherited_field_and_method() {
    let out = run_source(
        "class Animal { legs: number; legCount(): number { return this.legs; } }\n\
         class Dog extends Animal { goodBoy: number; }\n\
         function main() { let d = new Dog(4, 1); print(d.legCount()); print(d.goodBoy); }",
    );
    assert_eq!(out, ["4", "1"]);
}

#[test]
fn virtual_dispatch_calls_the_override() {
    // speak() is overridden in Dog; calling through a method that only knows
    // the base type must still reach Dog's version at runtime.
    let out = run_source(
        "class Animal {\n\
            id: number;\n\
            speak(): number { return 0; }\n\
         }\n\
         class Dog extends Animal {\n\
            speak(): number { return 1; }\n\
         }\n\
         class Cat extends Animal {\n\
            speak(): number { return 2; }\n\
         }\n\
         function hear(a: Animal): number { return a.speak(); }\n\
         function main() {\n\
            print(hear(new Animal(0)));\n\
            print(hear(new Dog(1)));\n\
            print(hear(new Cat(2)));\n\
         }",
    );
    assert_eq!(out, ["0", "1", "2"]);
}

#[test]
fn override_can_extend_via_shared_field() {
    // A base method reads a field; a subclass overrides another method. Field
    // layout is inherited-first, so the base field slot is stable.
    let out = run_source(
        "class Shape {\n\
            kind: number;\n\
            area(): number { return 0; }\n\
         }\n\
         class Square extends Shape {\n\
            side: number;\n\
            area(): number { return this.side * this.side; }\n\
         }\n\
         function main() {\n\
            let s = new Square(1, 5);\n\
            print(s.area());\n\
            print(s.kind);\n\
         }",
    );
    assert_eq!(out, ["25", "1"]);
}

#[test]
fn int_type_arithmetic() {
    let out = run_source(
        "function main() {\n\
            let a: int = 10;\n\
            let b: int = 3;\n\
            print(a * b);\n\
            print(a / b);\n\
            print(a % b);\n\
         }",
    );
    // Integer division truncates (10/3 = 3), unlike float.
    assert_eq!(out, ["30", "3", "1"]);
}

#[test]
fn int_and_float_division_differ() {
    let out = run_source(
        "function main() {\n\
            let i: int = 7;\n\
            let f: float = 7.0;\n\
            print(i / 2);\n\
            print(f / 2.0);\n\
         }",
    );
    assert_eq!(out, ["3", "3.5"]);
}

#[test]
fn int_through_function_and_recursion() {
    let out = run_source(
        "function sumTo(n: int): int {\n\
            if (n == 0) { return 0; }\n\
            return n + sumTo(n - 1);\n\
         }\n\
         function main() { print(sumTo(5)); }",
    );
    assert_eq!(out, ["15"]);
}

#[test]
fn std_prelude_numeric_helpers() {
    let out = run_with_std(
        "function main() {\n\
            print(abs(-7));\n\
            print(clamp(42, 0, 10));\n\
            print(powi(2, 10));\n\
            print(max(3, 9));\n\
            print(fabs(-2.5));\n\
            print(sign(-100));\n\
         }",
    );
    assert_eq!(out, ["7", "10", "1024", "9", "2.5", "-1"]);
}

#[test]
fn unknown_import_is_a_sema_error() {
    let src = "import { nope } from \"zun:ghost\";\nfunction main() {}";
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    zun_sema::check(src, &ast, &mut diags);
    assert!(diags.iter().any(|d| d.message.contains("unknown module")));
}

#[test]
fn gc_runs_without_corrupting_live_objects() {
    // Allocate far more objects than the GC interval (256) inside a loop while
    // keeping a live accumulator object referenced from a local. Collection
    // fires repeatedly; if roots were imprecise, the live object (or the
    // garbage churn feeding it) would be corrupted and the sum would be wrong.
    let out = run_source(
        "class Cell { v: int; }\n\
         class Acc { total: int; }\n\
         function main() {\n\
            let acc: Acc = new Acc(0);\n\
            let i: int = 0;\n\
            while (i < 5000) {\n\
                let c: Cell = new Cell(i);\n\
                acc.total = acc.total + c.v;\n\
                i = i + 1;\n\
            }\n\
            print(acc.total);\n\
         }",
    );
    // Sum of 0..4999 = 4999*5000/2 = 12497500.
    assert_eq!(out, ["12497500"]);
}

#[test]
fn adaptive_tiering_compiles_hot_methods() {
    use zun_interp::{run_with, CaptureOutput};
    // A numeric function called repeatedly from main; with a low threshold it
    // becomes hot and is compiled, and the output must stay correct whether a
    // given call was interpreted (cold) or native (hot).
    let src = "function sq(n: int): int { return n * n; }\n\
               function main() {\n\
                   let i: int = 0;\n\
                   while (i < 5) { print(sq(i)); i = i + 1; }\n\
               }";
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty());
    let module = zun_ilgen::lower(&hir);
    let bytes = module.write();

    let loaded = zun_vm::load(&bytes).unwrap();
    let jit = zun_codegen::Jit::new(&loaded).unwrap();
    let sq_rid = 1; // sq is the first function
    let hook = zun_vm::JitHook::with_threshold(jit, &loaded, 2);
    let mut out = CaptureOutput::default();
    run_with(&loaded, &mut out, Some(&hook)).unwrap();

    assert_eq!(out.lines, ["0", "1", "4", "9", "16"]);
    // After 5 calls with threshold 2, `sq` must have been compiled.
    assert!(hook.jit_is_defined(sq_rid));
    // The interpreted loop reports its backedge before the hot call path
    // takes over, providing the observation point required by future OSR.
    assert!(hook.total_loop_backedges(2) > 0);
    let hot_headers = hook.hot_loop_headers(2);
    assert!(!hot_headers.is_empty());
    assert!(hot_headers
        .iter()
        .all(|&target| hook.is_loop_hot(2, target)));
}

#[test]
fn hot_interpreted_backedge_compiles_eligible_method() {
    use zun_interp::{run_with, CaptureOutput};
    let src = "function spin(n: int): int { let i: int = 0; let total: int = 0; while (i < n) { total = total + i; i = i + 1; } return total; }\n\
               function main(): int { return spin(5); }";
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty());
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty());
    let module = zun_ilgen::lower(&hir);
    let loaded = zun_vm::load(&module.write()).unwrap();
    let spin_rid = 1;
    let jit = zun_codegen::Jit::new(&loaded).unwrap();
    let hook = zun_vm::JitHook::with_threshold(jit, &loaded, 2);
    let mut out = CaptureOutput::default();
    let result = run_with(&loaded, &mut out, Some(&hook)).unwrap();
    assert!(matches!(result, zun_interp::value::Value::Int(10, _)));
    // Eligible loop methods are compiled at their normal entry point, so no
    // interpreted backedge or mid-frame OSR request is needed.
    assert_eq!(hook.total_loop_backedges(spin_rid), 0);
    assert!(hook.jit_is_defined(spin_rid));
    assert!(hook.take_osr_requests().is_empty());
    assert!(hook.take_valid_osr_snapshots().is_empty());
    assert!(hook.take_osr_snapshots().is_empty());
}

#[test]
fn loop_with_exception_region_keeps_tier0_fallback() {
    use zun_interp::{run_with, CaptureOutput};

    let src = "class Error {} function guarded(n: int): int { let i: int = 0; try { while (i < n) { i = i + 1; } } catch (error: Error) { return -1; } return i; }\n\
               function main(): int { return guarded(5); }";
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty());
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty());
    let module = zun_ilgen::lower(&hir);
    let loaded = zun_vm::load(&module.write()).unwrap();
    let guarded_rid = 1;
    let jit = zun_codegen::Jit::new(&loaded).unwrap();
    let hook = zun_vm::JitHook::with_threshold(jit, &loaded, 1);
    let mut out = CaptureOutput::default();
    let result = run_with(&loaded, &mut out, Some(&hook)).unwrap();

    assert!(matches!(result, zun_interp::value::Value::Int(5, _)));
    assert!(!hook.jit_is_defined(guarded_rid));
}

#[test]
fn osr_rejects_snapshots_with_a_live_evaluation_stack() {
    use zun_interp::value::Value;

    let src = "function main(): int { return 1; }";
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    let hir = zun_sema::check(src, &ast, &mut diags);
    let module = zun_ilgen::lower(&hir);
    let loaded = zun_vm::load(&module.write()).unwrap();
    let jit = zun_codegen::Jit::new(&loaded).unwrap();
    let hook = zun_vm::JitHook::with_threshold(jit, &loaded, 1);
    let snapshot = zun_vm::OsrSnapshot {
        method_rid: 1,
        target_pc: 0,
        pc: 0,
        stack: vec![Value::Int(1, zun_il::NumKind::I32)],
        locals: Vec::new(),
    };
    assert!(!hook.is_valid_osr_snapshot(&snapshot));

    let snapshot_without_backedge = zun_vm::OsrSnapshot {
        method_rid: 1,
        target_pc: 0,
        pc: 0,
        stack: Vec::new(),
        locals: Vec::new(),
    };
    assert!(!hook.is_valid_osr_snapshot(&snapshot_without_backedge));
}

#[test]
fn osr_rejects_snapshots_inside_exception_regions() {
    use zun_interp::value::Value;

    let src = "class Error {} function main(): int { try { throw new Error(); } catch (error: Error) { return 42; } }";
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty());
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty());
    let module = zun_ilgen::lower(&hir);
    let loaded = zun_vm::load(&module.write()).unwrap();
    let jit = zun_codegen::Jit::new(&loaded).unwrap();
    let hook = zun_vm::JitHook::with_threshold(jit, &loaded, 1);
    let method = loaded.tables.method_defs.first().unwrap();
    let body = zun_module::read_body(&loaded.code, method.body_offset).unwrap();
    let region = body.handlers.first().expect("catch region");
    let snapshot = zun_vm::OsrSnapshot {
        method_rid: 1,
        target_pc: region.handler_start as usize,
        pc: region.handler_start as usize,
        stack: Vec::new(),
        locals: vec![Value::Void; body.local_count as usize],
    };
    assert!(!hook.is_valid_osr_snapshot(&snapshot));
}

#[test]
fn zun_core_object_namespace_short_member_resolves_from_user_code() {
    // The JS-idiomatic SHORT member `object.keys` (native `objectKeys`) is
    // addressed as (namespace, short) and lowers to the same `call.builtin`.
    let out = run_source(
        r#"import { object } from "zun:core";
        function main() {
            let o = dynamic_new();
            dynamic_set(o, "x", box_any(1));
            dynamic_set(o, "y", box_any(2));
            let keys = object.keys(o);
            print(keys.length);
            print(keys[0] as string);
            print(keys[1] as string);
        }"#,
    );
    assert_eq!(out, ["2", "x", "y"]);
}

#[test]
fn weak_map_std_class_round_trips_through_short_members() {
    // The WeakMap std class now calls `weakMap.set`/`weakMap.get`/… (the short
    // members of the `weakMap` namespace); a full set/get/has/delete cycle
    // must behave exactly as before the rename.
    let out = run_with_std(
        r#"class Key {}
        function main() {
            let m = new WeakMap();
            let k = new Key();
            let v = new Key();
            m.set(k, v);
            print(m.has(k));
            let got = m.get(k) as Key;
            print(got == v);
            print(m.delete(k));
            print(m.has(k));
        }"#,
    );
    assert_eq!(out, ["true", "true", "true", "false"]);
}

#[test]
fn weak_set_std_class_add_has_delete_round_trip() {
    // The WeakSet std class mirrors WeakMap over the ephemeron floor: an
    // add/has/delete cycle holds object members weakly and `has` is false for
    // an absent object.
    let out = run_with_std(
        r#"class Key {}
        function main() {
            let s = new WeakSet();
            let a = new Key();
            let b = new Key();
            print(s.add(a));
            print(s.has(a));
            print(s.has(b));
            print(s.delete(a));
            print(s.has(a));
        }"#,
    );
    assert_eq!(out, ["true", "true", "false", "true", "false"]);
}

#[test]
fn weak_ref_std_class_deref_returns_the_live_target() {
    // `new WeakRef(obj).deref()` returns the held object while it is reachable.
    let out = run_with_std(
        r#"class Cell {}
        function main() {
            let target = new Cell();
            let ref = new WeakRef(target);
            let got = ref.deref() as Cell;
            print(got == target);
        }"#,
    );
    assert_eq!(out, ["true"]);
}

#[test]
fn weak_set_and_weak_ref_resolve_as_realm_globals() {
    // `: WeakSet`/`: WeakRef` annotations type-check and the globals resolve
    // through the declarative realm registry, exactly like Map/WeakMap.
    let out = run_with_std(
        r#"class Node {}
        function main() {
            let n = new Node();
            let s: WeakSet = new WeakSet();
            s.add(n);
            print(s.has(n));
            let r: WeakRef = new WeakRef(n);
            let got = r.deref() as Node;
            print(got == n);
        }"#,
    );
    assert_eq!(out, ["true", "true"]);
}

#[test]
fn booleans_and_not() {
    let out = run_source(
        "function main() {\n\
            let ready = false;\n\
            if (!ready) { print(\"waiting\"); }\n\
         }",
    );
    assert_eq!(out, ["waiting"]);
}

// ---------------------------------------------------------------------------
// String method/static surface (NUCLEUS.md item #1). Indices are UTF-16 code
// units; `for...of` and the statics iterate/operate by code point.
// ---------------------------------------------------------------------------

#[test]
fn string_at_slice_concat_and_code_point_at() {
    let out = run_source(
        r#"function main() {
            print("hello".at(-1));
            print("hello".at(0));
            print("héllo".codePointAt(1));
            print("hello".slice(1, 3));
            print("hello".slice(-2));
            print("hello".slice(2));
            print("a".concat("b", "c"));
            print("x".concat());
        }"#,
    );
    assert_eq!(out, ["o", "h", "233", "el", "lo", "llo", "abc", "x"]);
}

#[test]
fn string_split_by_string_and_regexp() {
    let out = run_source(
        r#"function main() {
            let a = "a,b,c".split(",");
            print(a.length);
            print(a[0] as string);
            print(a[1] as string);
            print(a[2] as string);
            let chars = "abc".split("");
            print(chars.length);
            print(chars[0] as string);
            print(chars[2] as string);
            let limited = "a,b,c,d".split(",", 2);
            print(limited.length);
            print(limited[1] as string);
            let re = "a1b2".split(/\d/);
            print(re.length);
            print(re[0] as string);
            print(re[1] as string);
            print(re[2] as string);
        }"#,
    );
    assert_eq!(
        out,
        ["3", "a", "b", "c", "3", "a", "c", "2", "b", "3", "a", "b", ""]
    );
}

#[test]
fn string_replace_all_and_replace_first_by_string() {
    let out = run_source(
        r##"function main() {
            print("aXbXc".replaceAll("X", "-"));
            print("aXb".replace("X", "-"));
            print("aXbXc".replace("X", "-"));
            print("a1b2".replace(/\d/, "#"));
        }"##,
    );
    assert_eq!(out, ["a-b-c", "a-b", "a-bXc", "a#b2"]);
}

#[test]
fn string_from_char_code_and_from_code_point_statics() {
    let out = run_source(
        r#"function main() {
            print(String.fromCharCode(104, 105));
            print(String.fromCodePoint(128512));
            print(String.fromCharCode());
        }"#,
    );
    assert_eq!(out, ["hi", "\u{1F600}", ""]);
}

#[test]
fn string_is_iterable_by_code_point() {
    let out = run_source(
        r#"function main() {
            for (let ch of "a😀b") {
                print(ch);
            }
        }"#,
    );
    assert_eq!(out, ["a", "\u{1F600}", "b"]);
}
