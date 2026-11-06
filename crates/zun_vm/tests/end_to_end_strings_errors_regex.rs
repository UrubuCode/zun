mod common;
use common::*;

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
            print(s.add(a) === s);
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
            print("héllo".codePointAt(1) as number);
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

// --- Error.cause + any-field carrier + Error as a realm global (NUCLEUS #22) ---

#[test]
fn error_cause_carries_the_options_value() {
    // `new Error(message, { cause })` records `cause`; a numeric cause reads
    // back through the frozen-layout `any`-field carrier.
    let out = run_with_std(
        r#"function main() {
            let e = new Error("boom", { cause: 42 });
            print(e.message as string);
            if ((e.cause as number) === 42) { print("cause-42"); }
        }"#,
    );
    assert_eq!(out, ["boom", "cause-42"]);
}

#[test]
fn error_cause_absent_is_undefined() {
    // With no options object, `cause` is absent (JS `undefined`), distinct from
    // a present value.
    let out = run_with_std(
        r#"function main() {
            let e = new Error("boom");
            print(e.cause);
            print(typeof e.cause);
        }"#,
    );
    assert_eq!(out, ["undefined", "undefined"]);
}

#[test]
fn error_cause_carries_a_non_number() {
    // The `any` carrier holds any type: a string cause and an object cause both
    // round-trip (the object through its boxed reference).
    let out = run_with_std(
        r#"function main() {
            let s = new Error("boom", { cause: "root" });
            print(s.cause);
            let inner = new Error("inner");
            let o = new Error("outer", { cause: inner });
            let c: any = o.cause;
            print((c as Error).message);
        }"#,
    );
    assert_eq!(out, ["root", "inner"]);
}

#[test]
fn error_cause_can_be_assigned_after_construction() {
    // `err.cause = x` stores through the declared `any` field on a typed Error
    // instance, independent of the `new Error(msg, { cause })` form (NUCLEUS #22).
    let out = run_with_std(
        r#"function main() {
            let e = new Error("boom");
            e.cause = 99;
            if ((e.cause as number) === 99) { print("num"); }
            e.cause = "root";
            print(e.cause);
        }"#,
    );
    assert_eq!(out, ["num", "root"]);
}

#[test]
fn error_cause_on_a_subclass() {
    // Subclasses inherit the `message`/`stackFrames`/`cause` layout, so the
    // options-object cause works for them too.
    let out = run_with_std(
        r#"function main() {
            let e = new TypeError("bad", { cause: 7 });
            print(e.name);
            if ((e.cause as number) === 7) { print("cause-7"); }
        }"#,
    );
    assert_eq!(out, ["TypeError", "cause-7"]);
}

#[test]
fn error_is_a_realm_global_and_throws() {
    // `Error` resolves as a realm global: `throw new Error(...)` unwinds to the
    // catch with the message intact, and `globalThis.Error` resolves to its
    // descriptor.
    let out = run_with_std(
        r#"function main() {
            try {
                throw new Error("nope");
            } catch (e: Error) {
                print(e.message as string);
            }
            if (typeof globalThis.Error !== "undefined") { print("global-error"); }
        }"#,
    );
    assert_eq!(out, ["nope", "global-error"]);
}

#[test]
fn error_subclass_throws_as_realm_global() {
    let out = run_with_std(
        r#"function main() {
            try {
                throw new TypeError("wrong type");
            } catch (e: TypeError) {
                print(e.name);
                print(e.message as string);
            }
        }"#,
    );
    assert_eq!(out, ["TypeError", "wrong type"]);
}

// --- Static-regex lowering (the `[GeneratedRegex]` analog) ------------------


#[test]
fn static_regex_literal_test_matches_js_semantics() {
    // A regex literal used directly by `.test` lowers to the synthesized ZIL
    // matcher. `.test` is unanchored, so a mid-string match counts.
    let out = run_with_std(
        r#"function main(): void {
            if (/abc/.test("xabcx")) { print("hit-mid"); }
            if (!/abc/.test("ab")) { print("no-partial"); }
            if (/a|b/.test("zzb")) { print("alt"); }
            if (/[0-9]x/.test("q9x")) { print("class"); }
            if (!/zzz/.test("")) { print("empty-false"); }
        }"#,
    );
    assert_eq!(out, ["hit-mid", "no-partial", "alt", "class", "empty-false"]);
}

#[test]
fn static_new_regexp_const_test_matches_js_semantics() {
    // `new RegExp("const", "flags")` with literal arguments is also static.
    let out = run_with_std(
        r#"function main(): void {
            if (new RegExp("a.c", "s").test("a\nc")) { print("dotall"); }
            if (new RegExp("foo|bar").test("xbarx")) { print("alt2"); }
        }"#,
    );
    assert_eq!(out, ["dotall", "alt2"]);
}

#[test]
fn out_of_subset_static_regex_still_works_via_runtime_engine() {
    // `a+` is outside the compilable subset, so it stays on the runtime engine —
    // observably identical, never an error.
    let out = run_with_std(
        r#"function main(): void {
            if (/a+/.test("baaa")) { print("plus"); }
            if (/^ab/.test("abc")) { print("anchor"); }
            if (!/^ab/.test("xab")) { print("anchor-no"); }
        }"#,
    );
    assert_eq!(out, ["plus", "anchor", "anchor-no"]);
}

#[test]
fn static_regex_injects_jit_eligible_matcher_method() {
    // The literal `/abc/.test(...)` must synthesize `matchAt` + `isMatch`
    // methods in the module, and `matchAt` must be JIT-eligible (it tiers up to
    // native like any numeric loop).
    let module = compile_with_std(
        r#"function main(): void {
            if (/abc/.test("abc")) { print("ok"); }
        }"#,
    );
    let names: Vec<&str> = module
        .tables
        .method_defs
        .iter()
        .map(|m| module.strings.get(m.name))
        .collect();
    assert!(
        names.iter().any(|n| n.starts_with("matchAt$")),
        "matchAt method must be injected: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.starts_with("isMatch$")),
        "isMatch method must be injected: {names:?}"
    );

    // The injected `matchAt` is JIT-eligible.
    let eligible = zun_codegen::eligible_methods(&module);
    let match_rid = module
        .tables
        .method_defs
        .iter()
        .position(|m| module.strings.get(m.name).starts_with("matchAt$"))
        .map(|i| i as u32 + 1)
        .expect("matchAt present");
    assert!(eligible.contains(&match_rid), "synthesized matchAt must be JIT-eligible");
}

#[test]
fn a_concrete_stored_function_value_is_jit_eligible_and_runs() {
    // `let f = inc; return f(n);` — a concrete function stored in a local and
    // called indirectly. The local is carried as a one-word `Function` rid, so
    // the method is JIT-eligible under the bridge: it is provably closure-free
    // (the value is a bare `LoadFunction`, there is no function-typed parameter,
    // and no call returns a function). This is the stored-function slice built
    // on top of slice 1's transient `(expr)(args)` form.
    let user = "function inc(n: int): int { return n + 1; } \
                function choose(n: int): int { let f = inc; return f(n); } \
                function main(): void { print(choose(10)); }";
    let module = compile_with_std(user);
    let choose_rid = module
        .tables
        .method_defs
        .iter()
        .position(|m| module.strings.get(m.name) == "choose")
        .map(|i| i as u32 + 1)
        .expect("choose present");
    let eligible = zun_codegen::eligible_methods_bridged(&module, true);
    assert!(
        eligible.contains(&choose_rid),
        "a concrete stored-function method must be JIT-eligible under the bridge"
    );
    assert_eq!(run_with_std(user), ["11"]);
}

// A closure that captures a local, is returned, and is later called. This runs
// on the interpreter today (the returning method uses `MakeClosure`, so it is
// JIT-ineligible). It is the behavioral equivalence baseline the JIT closure
// carrier must reproduce once it lands (see docs/design/jit-closure-carrier.md):
// the same program must print the same lines whether `makeAdder` runs on Tier 0
// or native. Kept as a regression guard for that work.
#[test]
fn closure_captures_a_local_is_returned_and_called() {
    let user = "function makeAdder(n: number): (x: number) => number { return (x: number) => x + n; } \
         function main(): void { let add5 = makeAdder(5.0); let add1 = makeAdder(1.0); \
                                 print(add5(10.0)); print(add5(2.0)); print(add1(10.0)); }";
    // Each call closes over a distinct captured `n`, so the two closures stay
    // independent — a soundness property the carrier's GC-traced environment
    // must preserve.
    assert_eq!(run_with_std(user), ["15", "7", "11"]);

    // Today `makeAdder` is JIT-ineligible (it builds a closure). When the
    // carrier lands and relaxes eligibility, this assertion flips to
    // `contains`; until then it documents the current Tier-0 boundary.
    let module = compile_with_std(user);
    let make_rid = module
        .tables
        .method_defs
        .iter()
        .position(|m| module.strings.get(m.name) == "makeAdder")
        .map(|i| i as u32 + 1)
        .expect("makeAdder present");
    assert!(
        !zun_codegen::eligible_methods_bridged(&module, true).contains(&make_rid),
        "a closure-building method is on Tier 0 until the closure carrier lands"
    );
}

// ---------------------------------------------------------------------------
// Operator tokens: `**`, bitwise `& | ^ ~`, shifts `<< >> >>>`, and `void`.
// ---------------------------------------------------------------------------

