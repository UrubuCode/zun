mod common;
use common::*;

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
fn array_to_sorted_returns_copy_and_leaves_original() {
    let out = run_source(
        "function main() { \
            let a = new Array(3); a[0]=3.0; a[1]=1.0; a[2]=2.0; \
            let s = a.toSorted(); \
            print(s[0] as number); print(s[1] as number); print(s[2] as number); \
            print(a[0] as number); print(a[1] as number); print(a[2] as number); }",
    );
    // Default string order sorts [3,1,2] -> [1,2,3]; original stays [3,1,2].
    assert_eq!(out, ["1", "2", "3", "3", "1", "2"]);
}

#[test]
fn array_to_sorted_with_comparator() {
    let out = run_source(
        "function main() { \
            let a = new Array(3); a[0]=1.0; a[1]=2.0; a[2]=3.0; \
            let s = a.toSorted((x: any, y: any) => (y as number) - (x as number)); \
            print(s[0] as number); print(s[1] as number); print(s[2] as number); }",
    );
    // Descending comparator -> [3,2,1].
    assert_eq!(out, ["3", "2", "1"]);
}

#[test]
fn array_to_reversed_returns_copy() {
    let out = run_source(&format!(
        "function main() {{ {} \
            let r = a.toReversed(); \
            print(r[0] as number); print(r[1] as number); print(r[2] as number); \
            print(a[0] as number); }}",
        array_123()
    ));
    // [1,2,3] -> [3,2,1]; original first element unchanged.
    assert_eq!(out, ["3", "2", "1", "1"]);
}

#[test]
fn array_with_replaces_index_including_negative() {
    let out = run_source(&format!(
        "function main() {{ {} \
            let b = a.with(1, 9.0); \
            print(b[0] as number); print(b[1] as number); print(b[2] as number); \
            let c = a.with(-1, 9.0); \
            print(c[2] as number); print(a[1] as number); }}",
        array_123()
    ));
    // with(1,9) -> [1,9,3]; with(-1,9) replaces last -> c[2]=9; original a[1] stays 2.
    assert_eq!(out, ["1", "9", "3", "9", "2"]);
}

#[test]
fn array_with_out_of_range_throws() {
    let error = run_source_expect_error(&format!(
        "function main() {{ {} \
            let b = a.with(10, 9.0); print(b[0] as number); }}",
        array_123()
    ));
    assert!(error.contains("out of range"), "unexpected error: {error}");
}

#[test]
fn array_to_spliced_returns_copy() {
    let out = run_source(
        "function main() { \
            let a = new Array(4); a[0]=1.0; a[1]=2.0; a[2]=3.0; a[3]=4.0; \
            let b = a.toSpliced(1 as u64, 2 as u64, \"x\"); \
            print(b.length); print(b[0] as number); print(b[1] as string); \
            print(b[2] as number); print(a.length); }",
    );
    // [1,2,3,4] splice(1,2,'x') -> [1,'x',4]; original length unchanged.
    assert_eq!(out, ["3", "1", "x", "4", "4"]);
}

#[test]
fn array_keys_values_entries() {
    let out = run_source(
        "function main() { \
            let a = new Array(2); a[0] = 10.0; a[1] = 20.0; \
            for (let k of a.keys()) { print(k as number); } \
            for (let v of a.values()) { print(v as number); } \
            for (let e of a.entries()) { let p = e as Array; \
                print(p[0] as number); print(p[1] as number); } }",
    );
    // keys 0,1; values 10,20; entries [0,10] then [1,20].
    assert_eq!(out, ["0", "1", "10", "20", "0", "10", "1", "20"]);
}

#[test]
fn array_values_returns_a_real_iterator() {
    // `values()` returns a true iterator object: `next()` yields
    // `{ value, done }`, ending in `{ value: undefined, done: true }`, and the
    // iterator is itself for-of-able.
    let out = run_source(
        "function main() { \
            let a = new Array(2); a[0] = 10.0; a[1] = 20.0; \
            let it = a.values(); \
            let r1 = it.next(); print(r1.value as number); print(r1.done as boolean); \
            let r2 = it.next(); print(r2.value as number); print(r2.done as boolean); \
            let r3 = it.next(); print(r3.done as boolean); \
            let s = 0.0; for (let x of a.values()) { s = s + (x as number); } print(s); }",
    );
    // next() yields {10,false},{20,false}, then terminal done=true; the
    // iterator is itself for-of-able (summing 10+20 = 30).
    assert_eq!(out, ["10", "false", "20", "false", "true", "30"]);
}

#[test]
fn map_and_set_iterators_drive_for_of_via_protocol() {
    let out = run_oxc(
        "function main() { \
            let m = new Map(); m.set(\"a\", 1.0); m.set(\"b\", 2.0); \
            let sum = 0.0; let keys = \"\"; \
            for (const [k, v] of m.entries()) { keys = keys + (k as string); sum = sum + (v as number); } \
            print(keys); print(sum); \
            let set = new Set(); set.add(5.0); set.add(6.0); \
            let ssum = 0.0; for (const x of set.values()) { ssum = ssum + (x as number); } \
            print(ssum); }",
    );
    assert_eq!(out, ["ab", "3", "11"]);
}

#[test]
fn user_object_symbol_iterator_is_consumed_by_for_of() {
    // An arbitrary object literal implementing `[Symbol.iterator]` returning a
    // hand-written iterator with a `next()` method. (The counter-holding
    // closure is built in a top-level helper rather than inline in the object
    // method: an object-literal method that itself creates a capturing closure
    // is miscompiled when reached through the dynamic function slot — a
    // pre-existing closure gap, orthogonal to iteration.)
    let out = run_oxc(
        "function makeCounter(): any { \
            let i = 0; \
            return { next() { \
                if (i < 3) { let v = i; i = i + 1; return { value: v, done: false }; } \
                return { value: 0, done: true }; \
            } }; \
         } \
         function main() { \
            const obj = { [Symbol.iterator]() { return makeCounter(); } }; \
            let sum = 0.0; \
            for (const n of obj) { sum = sum + (n as number); } \
            print(sum); }",
    );
    assert_eq!(out, ["3"]);
}

#[test]
fn typed_class_symbol_iterator_is_consumed_by_for_of() {
    // A typed class's `[Symbol.iterator]` (the stored computed-symbol method) is
    // now driven by for-of through the iteration protocol.
    let out = run_oxc(
        "class Range { \
            [Symbol.iterator](): any { return [100.0, 200.0, 300.0].values(); } \
         } \
         function main() { \
            let r = new Range(); \
            let s = 0.0; \
            for (const x of r) { s = s + (x as number); } \
            print(s); }",
    );
    assert_eq!(out, ["600"]);
}

#[test]
fn set_and_spread_accept_user_iterables() {
    // `new Set(userIterable)` and `[...userIterable]` both route through the
    // same `[Symbol.iterator]().next()` protocol loop.
    let out = run_oxc(
        "function makeCounter(): any { \
            let i = 0; \
            return { next() { \
                if (i < 3) { let v = i; i = i + 1; return { value: v, done: false }; } \
                return { value: 0, done: true }; \
            } }; \
         } \
         function main() { \
            const obj = { [Symbol.iterator]() { return makeCounter(); } }; \
            let set = new Set(obj); let size = set.size(); print(size); \
            let arr = [...obj]; let len = arr.length; print(len); \
            let s = 0.0; for (const x of arr) { s = s + (x as number); } print(s); }",
    );
    assert_eq!(out, ["3", "3", "3"]);
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
    // The thrown value is a real `TypeError` object (JS semantics), so observe
    // its message from inside the program.
    let out = run_with_std(
        "function main() {             let a = new Array(0);             try {                 a.reduce((acc: any, v: any, i: any, arr: any) => (acc as number) + (v as number));                 print(\"no throw\");             } catch (e: any) { print(e.message as string); } }",
    );
    assert_eq!(out.len(), 1);
    assert!(
        out[0].contains("Reduce of empty array"),
        "unexpected: {out:?}"
    );
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
fn function_bind_accepts_a_function_valued_this_argument() {
    // Binding a function as the leading (`this`) argument: `f` is a function
    // parameter, so `apply1.bind(dbl)` fixes `f = dbl`. A function value cannot
    // box into `Any`, so the bound record carries it in a `functionBox` wrapper.
    let out = run_source(
        "function apply1(f: Function, x: number): number { return (f(x)) as number; }\n\
         function dbl(n: number): number { return n * 2.0; }\n\
         function main() {\n\
            let g = apply1.bind(dbl);\n\
            print((g(5.0)) as number);\n\
         }",
    );
    assert_eq!(out, ["10"]);
}

#[test]
fn function_bind_preserves_a_function_valued_partial_argument_position() {
    // A function in a non-leading partial slot keeps its order among the
    // prepended arguments: `combine.bind(1.0, dbl)` fixes `a = 1`, `f = dbl`.
    let out = run_source(
        "function combine(a: number, f: Function, x: number): number { return a + ((f(x)) as number); }\n\
         function dbl(n: number): number { return n * 2.0; }\n\
         function main() {\n\
            let g = combine.bind(1.0, dbl);\n\
            print((g(5.0)) as number);\n\
         }",
    );
    assert_eq!(out, ["11"]);
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
