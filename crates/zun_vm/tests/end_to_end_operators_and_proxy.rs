mod common;
use common::*;

#[test]
fn number_bitwise_and_shift_operators_follow_js_int32_semantics() {
    // On `number` operands the bitwise/shift operators coerce to int32
    // (uint32 for the `>>>` left operand) and widen the result back to a
    // number, matching JavaScript.
    let out = run_source(
        r#"function main() {
            print(5 & 3);
            print(5 | 2);
            print(6 ^ 3);
            print(1 << 4);
            print(-8 >> 1);
            print(-1 >>> 28);
            print(~5);
            print(5.9 & 3);
            print((-1) >>> 0);
        }"#,
    );
    assert_eq!(
        out,
        ["1", "7", "5", "16", "-4", "15", "-6", "1", "4294967295"]
    );
}

#[test]
fn number_exponent_operator_uses_math_pow() {
    let out = run_source(
        r#"function main() {
            print(2 ** 10);
            print(2.0 ** 0.5);
            print(3 ** 3 ** 2);
        }"#,
    );
    // `**` is right-associative: 3 ** (3 ** 2) == 3 ** 9 == 19683.
    assert_eq!(out, ["1024", "1.4142135623730951", "19683"]);
}

#[test]
fn bigint_exponent_bitwise_and_shift_operators_use_the_managed_floor() {
    // The NUCLEUS #4 payoff: these operators were method-only because the
    // tokens did not exist. They now route to the same bigint natives as
    // `+ - * / %`.
    let out = run_source(
        r#"function main(): int {
            print((2n ** 10n).toString());
            print((7n & 3n).toString());
            print((5n | 2n).toString());
            print((6n ^ 3n).toString());
            print((1n << 4n).toString());
            print((-8n >> 1n).toString());
            print((~7n).toString());
            return 0;
        }"#,
    );
    assert_eq!(out, ["1024", "3", "7", "5", "16", "-4", "-8"]);
}

#[test]
fn bigint_unsigned_right_shift_is_rejected() {
    // ECMAScript: `>>>` has no BigInt form, so `8n >>> 1n` throws a TypeError at
    // run time (not a compile error), matching Node/Bun.
    let src = zun_prelude::with_prelude(
        r#"function main(): int {
            let a: BigInt = 8n;
            let b: BigInt = 1n;
            print((a >>> b).toString());
            return 0;
        }"#,
    );
    let error = run_source_expect_error(&src);
    assert!(
        error.contains("TypeError") || error.to_lowercase().contains("exception"),
        "expected a BigInt `>>>` runtime TypeError, got: {error}"
    );
}

#[test]
fn void_operator_yields_undefined() {
    let out = run_source(
        r#"function main() {
            print(void 0);
            print(void "ignored");
        }"#,
    );
    assert_eq!(out, ["undefined", "undefined"]);
}

#[test]
fn string_raw_tag_keeps_backslashes_literal() {
    // `String.raw` is the built-in template tag: the raw (un-escaped) literal
    // chunks are joined with the stringified substitutions, so `\n` stays a
    // literal backslash-n (NUCLEUS #1).
    let out = run_with_std("function main(): void { print(String.raw`a\n${1.0}b`); }");
    assert_eq!(out, ["a\n1b"]);
}

#[test]
fn decorator_factory_runs_at_definition_time() {
    // `@wrap(2)` evaluates the factory `wrap(2)` to produce the actual
    // decorator, which is then invoked with `(value, context)` before `main`
    // (NUCLEUS #25). The factory argument is recorded at definition time.
    let out = run_with_std(
        r#"function wrap(n: number): Function {
            return (value: any, context: any) => { print(`wrapped-${n}`); };
        }
        class C {
            @wrap(2)
            m(): void {}
        }
        function main(): void { print("main"); }"#,
    );
    assert_eq!(out, ["wrapped-2", "main"]);
}

#[test]
fn typed_class_symbol_has_instance_drives_instanceof() {
    // A typed class carrying `[Symbol.hasInstance]` decides `instanceof`
    // membership itself via the per-type symbol-method side table (NUCLEUS #5).
    let out = run_with_std(
        r#"class Accepting {
            [Symbol.hasInstance](value: any): boolean { return true; }
        }
        class Rejecting {
            [Symbol.hasInstance](value: any): boolean { return false; }
        }
        function main(): void {
            let x: any = box_any(1.0);
            if (x instanceof Accepting) { print("accept-yes"); } else { print("accept-no"); }
            if (x instanceof Rejecting) { print("reject-yes"); } else { print("reject-no"); }
        }"#,
    );
    assert_eq!(out, ["accept-yes", "reject-no"]);
}

// --- NUCLEUS #10: Proxy descriptor traps + apply + Reflect statics ---------

#[test]
fn proxy_get_own_property_descriptor_trap_fires() {
    // `Object.getOwnPropertyDescriptor(proxy, key)` consults the handler's
    // `getOwnPropertyDescriptor` trap; its return value is the descriptor.
    let out = run_source(
        r#"import { dynamicSetFunction } from "zun:core";
        function descTrap(target: any, key: string): any {
            let d = dynamic_new();
            dynamic_set(d, "value", box_any(42.0));
            return box_any(d);
        }
        function main() {
            let target = dynamic_new();
            let handler = dynamic_new();
            dynamicSetFunction(handler, "getOwnPropertyDescriptor", descTrap);
            let proxy = new Proxy(target, handler);
            let d: any = Object.getOwnPropertyDescriptor(proxy, "x");
            print((d.value) as number);
        }"#,
    );
    assert_eq!(out, ["42"]);
}

#[test]
fn proxy_get_own_property_descriptor_falls_back_to_target() {
    // Without the trap, the descriptor is read from the proxy's target through
    // the NUCLEUS #6 descriptor machinery.
    let out = run_source(
        r#"function main() {
            let target = dynamic_new();
            dynamic_set(target, "k", box_any(5.0));
            let proxy = new Proxy(target, dynamic_new());
            let d: any = Object.getOwnPropertyDescriptor(proxy, "k");
            print((d.value) as number);
        }"#,
    );
    assert_eq!(out, ["5"]);
}

#[test]
fn proxy_define_property_trap_fires() {
    // `Object.defineProperty(proxy, …)` routes to the `defineProperty` trap.
    let out = run_source(
        r#"import { global, dynamicSetFunction } from "zun:core";
        function defTrap(target: any, key: string, desc: any): boolean {
            global.set("defined", box_any(true), false);
            return true;
        }
        function main() {
            let target = dynamic_new();
            let handler = dynamic_new();
            dynamicSetFunction(handler, "defineProperty", defTrap);
            let proxy = new Proxy(target, handler);
            let d = dynamic_new();
            dynamic_set(d, "value", box_any(7.0));
            Object.defineProperty(proxy, "x", d);
            print(Boolean(dynamic_get(global, "defined")));
        }"#,
    );
    assert_eq!(out, ["true"]);
}

#[test]
fn proxy_get_prototype_of_trap_fires() {
    let out = run_source(
        r#"import { dynamicSetFunction } from "zun:core";
        function protoTrap(target: any): any { return box_any(123.0); }
        function main() {
            let handler = dynamic_new();
            dynamicSetFunction(handler, "getPrototypeOf", protoTrap);
            let proxy = new Proxy(dynamic_new(), handler);
            print((Object.getPrototypeOf(proxy)) as number);
        }"#,
    );
    assert_eq!(out, ["123"]);
}

#[test]
fn proxy_is_extensible_trap_fires() {
    let out = run_source(
        r#"import { dynamicSetFunction } from "zun:core";
        function extTrap(target: any): boolean { return false; }
        function main() {
            let handler = dynamic_new();
            dynamicSetFunction(handler, "isExtensible", extTrap);
            let proxy = new Proxy(dynamic_new(), handler);
            print(Object.isExtensible(proxy));
        }"#,
    );
    assert_eq!(out, ["false"]);
}

#[test]
fn proxy_prevent_extensions_trap_fires() {
    let out = run_source(
        r#"import { global, dynamicSetFunction } from "zun:core";
        function prevTrap(target: any): boolean {
            global.set("prevented", box_any(true), false);
            return true;
        }
        function main() {
            let target = dynamic_new();
            let handler = dynamic_new();
            dynamicSetFunction(handler, "preventExtensions", prevTrap);
            let proxy = new Proxy(target, handler);
            Object.preventExtensions(proxy);
            print(Boolean(dynamic_get(global, "prevented")));
        }"#,
    );
    assert_eq!(out, ["true"]);
}

#[test]
fn proxy_apply_trap_intercepts_a_direct_call() {
    // A proxy over a function, called directly, routes through the `apply`
    // trap (NUCLEUS #10).
    let out = run_source(
        r#"import { dynamicSetFunction } from "zun:core";
        function applyTrap(target: any, thisArg: any, args: any): number { return 99.0; }
        function base(a: number, b: number): number { return a + b; }
        function main() {
            let handler = dynamic_new();
            dynamicSetFunction(handler, "apply", applyTrap);
            let proxy = new Proxy(base, handler);
            print((proxy(1.0, 2.0)) as number);
        }"#,
    );
    assert_eq!(out, ["99"]);
}

#[test]
fn proxy_apply_falls_back_to_the_underlying_function() {
    // Without an `apply` trap, a direct call forwards its arguments to the
    // proxy's underlying function target.
    let out = run_source(
        r#"function base(a: number, b: number): number { return a + b; }
        function main() {
            let proxy = new Proxy(base, dynamic_new());
            print((proxy(3.0, 4.0)) as number);
        }"#,
    );
    assert_eq!(out, ["7"]);
}

#[test]
fn reflect_apply_over_a_plain_function() {
    // `Reflect.apply(fn, thisArg, argsArray)` reuses the `functionApply` path
    // (`thisArg` binds to the function's first parameter, per Zun's model).
    let out = run_source(
        r#"function greet(base: number, x: number): number { return base + x; }
        function main() {
            let a = [5.0];
            print((Reflect.apply(greet, 10.0, a)) as number);
        }"#,
    );
    assert_eq!(out, ["15"]);
}

#[test]
fn reflect_apply_over_a_callable_proxy() {
    let out = run_source(
        r#"import { dynamicSetFunction } from "zun:core";
        function applyTrap(target: any, thisArg: any, args: any): number { return 7.0; }
        function base(a: number): number { return a; }
        function main() {
            let handler = dynamic_new();
            dynamicSetFunction(handler, "apply", applyTrap);
            let proxy = new Proxy(base, handler);
            print((Reflect.apply(proxy, 0.0, [1.0])) as number);
        }"#,
    );
    assert_eq!(out, ["7"]);
}

#[test]
fn reflect_get_own_property_descriptor_and_own_keys() {
    let out = run_source(
        r#"function main() {
            let target = dynamic_new();
            dynamic_set(target, "a", box_any(1.0));
            dynamic_set(target, "b", box_any(2.0));
            let proxy = new Proxy(target, dynamic_new());
            let d: any = Reflect.getOwnPropertyDescriptor(proxy, "a");
            print((d.value) as number);
            print(Reflect.ownKeys(proxy).length);
        }"#,
    );
    assert_eq!(out, ["1", "2"]);
}
