mod common;
use common::*;

#[test]
fn dynamic_object_accepts_any_and_null_values() {
    // Regression: storing an `any`-typed value into a dynamic object slot
    // (computed `o[k]` or dotted `o.q`), and a `null` object-literal property,
    // used to throw `BadToken` because the store path required an already-boxed
    // `Any`. The store now boxes any non-function value, like `StoreArray`.
    let out = run_with_std(
        r#"
        function main(): void {
            let o: any = {};
            let v: any = 5.0;
            o["k"] = v;
            o.q = "hi";
            let lit: any = { nothing: null, n: 7.0 };
            print((o["k"] as number) + 1.0);
            print(o.q as string);
            print(lit.n as number);
            print(typeof lit.nothing);
        }
        "#,
    );
    assert_eq!(out, vec!["6", "hi", "7", "object"]);
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
fn a_runtime_method_decorator_runs_at_definition_time_with_context() {
    // PLAN §3.5 Stage 3 (runtime TC39 flavor): a `@decorator` whose callee is
    // an ordinary top-level function (not a compile-time macro) is invoked once
    // at class-definition time with `(value, context)`. The decorator records a
    // side effect (printing) before `main` runs, and `context` carries the
    // element kind, name, and static flag. The method itself is unchanged
    // (return-value replacement is deferred), so `bump()` still returns 42.
    let out = run_source(
        "function record(value: any, context: any): void {\n\
            print(context.kind as string);\n\
            print(context.name as string);\n\
            print((context[\"static\"] as boolean) ? \"static\" : \"instance\");\n\
         }\n\
         class Counter { value: number;\n\
            @record\n\
            bump(): number { return this.value + 1.0; }\n\
         }\n\
         function main() {\n\
            let c = new Counter(41.0);\n\
            print(c.bump());\n\
         }",
    );
    // Decorator side effect precedes `main` (the module initializer runs first).
    assert_eq!(out, ["method", "bump", "instance", "42"]);
}

#[test]
fn a_runtime_class_decorator_runs_at_definition_time() {
    // A class decorator is called with `context.kind === "class"` and the class
    // name. There is no first-class runtime class value in the typed-class
    // model, so `value` is `null` and return-value replacement is deferred; the
    // call still runs for its observable side effect.
    let out = run_source(
        "function register(value: any, context: any): void {\n\
            print(context.kind as string);\n\
            print(context.name as string);\n\
         }\n\
         @register\n\
         class Widget { id: number; }\n\
         function main() {\n\
            let w = new Widget(7.0);\n\
            print(w.id);\n\
         }",
    );
    assert_eq!(out, ["class", "Widget", "7"]);
}

#[test]
fn an_undefined_runtime_decorator_stays_inert() {
    // A decorator whose callee resolves to neither a macro nor a top-level
    // function stays inert (Phase-1 behavior), so the class runs normally.
    let out = run_source(
        "@sealed\n\
         class Box { n: number;\n\
            @trace\n\
            get(): number { return this.n; }\n\
         }\n\
         function main() {\n\
            let b = new Box(5.0);\n\
            print(b.get());\n\
         }",
    );
    assert_eq!(out, ["5"]);
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
fn object_literal_method_shorthand_is_callable() {
    // `{ m() {} }` stores the method in the object's function slot
    // (dynamicSetFunction); `o.m()` dispatches through it.
    let out = run_source(
        r#"function main() {
            let o = { m() { return 1.0; } };
            print(o.m() as number);
        }"#,
    );
    assert_eq!(out, ["1"]);
}

#[test]
fn object_literal_computed_symbol_method_backs_has_instance() {
    // A computed Symbol method key routes to the symbol-function slot
    // (dynamicSetSymbolFunction); a user `[Symbol.hasInstance]` is then
    // consulted by `instanceof` (NUCLEUS #5 enabler).
    let out = run_source(
        r#"function main() {
            let matcher = { [Symbol.hasInstance](value) { return true; } };
            if (5.0 instanceof matcher) { print("yes"); } else { print("no"); }
        }"#,
    );
    assert_eq!(out, ["yes"]);
}

#[test]
fn object_literal_getter_is_invoked_on_read() {
    // `{ get x() {} }` installs an accessor property; reading `o.x` invokes it.
    let out = run_source(
        r#"function main() {
            let o = { get x() { return 42.0; } };
            print(o.x as number);
        }"#,
    );
    assert_eq!(out, ["42"]);
}

#[test]
fn object_literal_setter_is_invoked_on_write() {
    // `{ set x(v) {} }` installs an accessor; assigning `o.x = v` invokes it.
    let out = run_source(
        r#"function main() {
            let sink = new Array(1);
            let o = { set x(v) { sink[0] = v; } };
            o.x = 7.0;
            print(sink[0] as number);
        }"#,
    );
    assert_eq!(out, ["7"]);
}

#[test]
fn object_literal_spread_merges_with_later_keys_overriding() {
    // `{ ...a, y: 2 }` copies `a`'s own enumerable keys in place; a later key
    // overrides a spread-copied one (source order).
    let out = run_source(
        r#"function main() {
            let a = { x: 1.0, y: 1.0 };
            let o = { x: 9.0, ...a, y: 2.0 };
            print(o.x as number);
            print(o.y as number);
        }"#,
    );
    assert_eq!(out, ["1", "2"]);
}

#[test]
fn array_literal_holes_count_toward_length() {
    // `[1, , 3]` has length 3; the hole reads as `undefined` (not `0`).
    let out = run_source(
        r#"function main() {
            let a = [1.0, , 3.0];
            print(a.length);
            print(a[0] as number);
            print(a[2] as number);
        }"#,
    );
    assert_eq!(out, ["3", "1", "3"]);
}

#[test]
fn array_literal_spread_splices_array_and_string_sources() {
    let out = run_source(
        r#"function main() {
            let xs = [1.0, 2.0];
            let a = [0.0, ...xs, 3.0];
            print(a.length);
            print(a[1] as number);
            print(a[3] as number);
            let chars = [..."ab"];
            print(chars.length);
            print(chars[0] as string);
        }"#,
    );
    assert_eq!(out, ["4", "1", "3", "2", "a"]);
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
            print(Object.keys(o).length);
        }"#,
    );
    assert_eq!(out, ["before-open", "after-frozen", "1"]);
    // Adding a key to a frozen object throws a TypeError (strict mode).
    let error = run_source_expect_error(
        r#"function main() {
            let o = dynamic_new();
            dynamic_set(o, "a", box_any(1));
            Object.freeze(o);
            dynamic_set(o, "b", box_any(2));
        }"#,
    );
    assert!(error.contains("object is not extensible"), "{error}");
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
    assert_eq!(
        out,
        ["nan-equal", "zero-distinct", "neg-equal", "two-equal"]
    );
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
    assert_eq!(
        out,
        ["42", "255", "3.14", "9007199254740991", "epsilon-positive"]
    );
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
fn number_to_exponential() {
    let out = run_source(
        r#"function main() {
            print((1234.5).toExponential());
            print((1234.5).toExponential(2));
            print((0.0).toExponential());
            print((0.0).toExponential(2));
            print((-1234.5).toExponential(2));
            print((0.00001234).toExponential(2));
        }"#,
    );
    assert_eq!(
        out,
        [
            "1.2345e+3",
            "1.23e+3",
            "0e+0",
            "0.00e+0",
            "-1.23e+3",
            "1.23e-5"
        ]
    );
}

#[test]
fn number_to_precision() {
    let out = run_source(
        r#"function main() {
            print((123.456).toPrecision(4));
            print((123.456).toPrecision());
            print((1234.5).toPrecision(2));
            print((0.0001).toPrecision(2));
            print((0.00000012).toPrecision(2));
            print((123.0).toPrecision(5));
        }"#,
    );
    assert_eq!(
        out,
        ["123.5", "123.456", "1.2e+3", "0.00010", "1.2e-7", "123.00"]
    );
}

#[test]
fn number_and_string_value_of_are_identity() {
    let out = run_source(
        r#"function main() {
            print((42.5).valueOf());
            print("hi".valueOf());
        }"#,
    );
    assert_eq!(out, ["42.5", "hi"]);
}

#[test]
fn string_match_all_global_regex() {
    let out = run_with_std(
        r#"function main() {
            let all = [..."12 34 56".matchAll(/(\d)(\d)/g)];
            print(all.length);
            print((all[0] as Array)[0] as string);
            print((all[0] as Array)[1] as string);
            print((all[0] as Array)[2] as string);
            print((all[1] as Array)[0] as string);
            print((all[2] as Array)[1] as string);
        }"#,
    );
    assert_eq!(out, ["3", "12", "1", "2", "34", "5"]);
}

#[test]
fn string_match_all_without_global_flag_errors() {
    let error = run_source_expect_error(
        r#"function main() {
            let all = "12".matchAll(/\d/);
            print(all.length);
        }"#,
    );
    assert!(
        error.contains("matchAll called with a non-global RegExp"),
        "unexpected error: {error}"
    );
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
fn object_define_property_data_descriptor_controls_enumerability() {
    // A non-enumerable data property is readable and listed by
    // getOwnPropertyNames, but skipped by Object.keys.
    let out = run_source(
        r#"function main() {
            let o = dynamic_new();
            let d = dynamic_new();
            dynamic_set(d, "value", box_any(1));
            dynamic_set(d, "enumerable", box_any(false));
            Object.defineProperty(o, "x", d);
            print(dynamic_get(o, "x") as number);
            print(Object.keys(o).length);
            print(Object.getOwnPropertyNames(o).length);
            print(Object.getOwnPropertyNames(o)[0] as string);
        }"#,
    );
    assert_eq!(out, ["1", "0", "1", "x"]);
}

#[test]
fn object_define_property_enforces_writable_and_configurable() {
    // Omitted flags default to false: the property is non-writable and
    // non-configurable, so assignment, redefine, and delete are all rejected.
    let out = run_source(
        r#"function main() {
            let o = dynamic_new();
            let d = dynamic_new();
            dynamic_set(d, "value", box_any(1));
            Object.defineProperty(o, "x", d);
            print(dynamic_get(o, "x") as number);
            print(Object.getOwnPropertyNames(o).length);
        }"#,
    );
    assert_eq!(out, ["1", "1"]);
    // Modules are strict: every refused store, redefine and delete throws.
    let store = run_source_expect_error(
        r#"function main() {
            let o = dynamic_new();
            let d = dynamic_new();
            dynamic_set(d, "value", box_any(1));
            Object.defineProperty(o, "x", d);
            dynamic_set(o, "x", box_any(99));
        }"#,
    );
    assert!(
        store.contains("Cannot assign to read only property 'x'"),
        "{store}"
    );
    let redefine = run_source_expect_error(
        r#"function main() {
            let o = dynamic_new();
            let d = dynamic_new();
            dynamic_set(d, "value", box_any(1));
            Object.defineProperty(o, "x", d);
            let d2 = dynamic_new();
            dynamic_set(d2, "value", box_any(2));
            Object.defineProperty(o, "x", d2);
        }"#,
    );
    assert!(
        redefine.contains("Cannot redefine property: x"),
        "{redefine}"
    );
    let delete = run_source_expect_error(
        r#"function main() {
            let o = dynamic_new();
            let d = dynamic_new();
            dynamic_set(d, "value", box_any(1));
            Object.defineProperty(o, "x", d);
            delete o.x;
        }"#,
    );
    assert!(delete.contains("Cannot delete property 'x'"), "{delete}");
}

#[test]
fn object_get_own_property_descriptor_reports_attributes() {
    // An ordinary assignment creates a writable + enumerable + configurable
    // data property.
    let out = run_source(
        r#"function main() {
            let o = dynamic_new();
            dynamic_set(o, "k", box_any(5));
            let d: any = Object.getOwnPropertyDescriptor(o, "k");
            print((d.value) as number);
            print(Boolean(d.writable));
            print(Boolean(d.enumerable));
            print(Boolean(d.configurable));
        }"#,
    );
    assert_eq!(out, ["5", "true", "true", "true"]);
}

#[test]
fn object_seal_prevents_configuration_but_keeps_values_writable() {
    let out = run_source(
        r#"function main() {
            let o = dynamic_new();
            dynamic_set(o, "a", box_any(1));
            Object.seal(o);
            print(Object.isSealed(o));
            print(Object.isExtensible(o));
            print(Object.getOwnPropertyNames(o).length);
            dynamic_set(o, "a", box_any(9));
            print(dynamic_get(o, "a") as number);
        }"#,
    );
    assert_eq!(out, ["true", "false", "1", "9"]);
    // Adding a key to a sealed object throws a TypeError (strict mode).
    let error = run_source_expect_error(
        r#"function main() {
            let o = dynamic_new();
            dynamic_set(o, "a", box_any(1));
            Object.seal(o);
            dynamic_set(o, "b", box_any(2));
        }"#,
    );
    assert!(error.contains("object is not extensible"), "{error}");
}

#[test]
fn object_prevent_extensions_blocks_new_keys() {
    let out = run_source(
        r#"function main() {
            let o = dynamic_new();
            dynamic_set(o, "a", box_any(1));
            print(Object.isExtensible(o));
            Object.preventExtensions(o);
            print(Object.isExtensible(o));
            print(Object.getOwnPropertyNames(o).length);
        }"#,
    );
    assert_eq!(out, ["true", "false", "1"]);
    // Adding a key to a non-extensible object throws a TypeError (strict mode).
    let error = run_source_expect_error(
        r#"function main() {
            let o = dynamic_new();
            dynamic_set(o, "a", box_any(1));
            Object.preventExtensions(o);
            dynamic_set(o, "b", box_any(2));
        }"#,
    );
    assert!(error.contains("object is not extensible"), "{error}");
}

#[test]
fn ordinary_assignment_creates_enumerable_property() {
    let out = run_source(
        r#"function main() {
            let o = dynamic_new();
            dynamic_set(o, "k", box_any(7));
            print(Object.keys(o).length);
            print(Object.keys(o)[0] as string);
            print(o.propertyIsEnumerable("k"));
        }"#,
    );
    assert_eq!(out, ["1", "k", "true"]);
}

#[test]
fn object_define_property_accessor_getter_is_invoked_on_read() {
    // An accessor descriptor stores its getter in the function slot
    // (dynamicSetFunction); reading the property invokes the getter.
    let out = run_source(
        r#"function main() {
            let o = dynamic_new();
            let d = dynamic_new();
            dynamicSetFunction(d, "get", () => 42.0);
            Object.defineProperty(o, "y", d);
            print(o.y as number);
        }"#,
    );
    assert_eq!(out, ["42"]);
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
