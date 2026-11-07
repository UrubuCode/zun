mod common;
use common::*;

use zun_interp::value::Value;
use zun_interp::CaptureOutput;

#[test]
fn regexp_methods_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): int { let pattern = new RegExp(\"zun-[0-9]+\"); if (pattern.isMatch(\"zun-42\")) { return 1; } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "RegExp requires Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn managed_collection_constructors_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): u64 { let map = new Map(); map.set(7.0, 42.0); if (map.has(7.0)) { return map.size(); } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "managed constructors require Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn managed_promise_intrinsics_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "import { promiseNew, promiseResolve, promiseState } from \"zun:core\"; function main(): int { let promise = promiseNew(); if (promiseState(promise) != 0) { return 1; } promiseResolve(promise, 42.0); if (promiseState(promise) == 1) { return 0; } return 2; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "Promise intrinsics require Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Int(0, _)));
}

#[test]
fn managed_promise_class_methods_use_interpreter_fallback() {
    let module = compile(
        "function main(): int { let promise = new Promise(); if (promise.isReady()) { return 1; } if (promise.state() != 0) { return 2; } promise.resolve(42.0); if (promise.isReady()) { let value = promise.result(); return 0; } return 3; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "Promise methods require Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Int(0, _)));
}

#[test]
fn async_function_without_await_returns_a_promise_through_tier0() {
    let module = compile(
        "async function immediate(): Promise { return new Promise(); } function main(): int { let promise = immediate(); return promise.state() as int; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "managed Promise requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Int(0, _)));
}

#[test]
fn async_await_preserves_the_interpreter_fallback_boundary() {
    let module = compile(
        // A top-level `await` parks the entry frame itself (an `async function` is a
        // coroutine driven by the scheduler, so its entry returns a Promise).
        "let p = new Promise(); await p;",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "Await requires Tier 0 state-machine execution"
    );
    let mut output = CaptureOutput::default();
    let mut session =
        zun_interp::ResumableSession::new(&module, entry, Vec::new(), &mut output, None, None)
            .expect("create resumable session");
    assert!(matches!(
        session.poll().expect("initial poll"),
        zun_interp::ResumeResult::Suspended
    ));
    let promise = session.awaited_promise().expect("awaited promise");
    let boxed = Value::Int(1, zun_il::NumKind::I32)
        .box_any(session.heap_mut())
        .expect("box result");
    assert_eq!(
        session.heap_mut().promise_resolve(promise, boxed),
        Some(true)
    );
    assert!(matches!(
        session.poll().expect("resumed poll"),
        zun_interp::ResumeResult::Complete(Value::Void)
    ));
}

#[test]
fn map_and_set_values_are_exposed_as_instance_methods() {
    let module = compile(
        "function main(): int { let map = new Map(); map.set(7.0, 42.0); let set = new Set(); set.add(9.0); let mr = map.values().next(); let sr = set.values().next(); if ((mr.value as number) == 42.0) { if ((sr.value as number) == 9.0) { return 1; } } return 0; }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn map_keys_and_entries_are_exposed_as_instance_methods() {
    let module = compile(
        "function main(): int { let map = new Map(); map.set(7.0, 42.0); let kr = map.keys().next(); let er = map.entries().next(); let pair: Array = er.value as Array; if ((kr.value as number) == 7.0) { if ((pair[1] as number) == 42.0) { return 1; } } return 0; }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn set_entries_are_exposed_as_instance_methods() {
    let module = compile(
        "function main(): int { let set = new Set(); set.add(9.0); let er = set.entries().next(); let pair: Array = er.value as Array; if ((pair[0] as number) == 9.0) { if ((pair[1] as number) == 9.0) { return 1; } } return 0; }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn set_keys_alias_values() {
    let module = compile(
        "function main(): int { let set = new Set(); set.add(9.0); let kr = set.keys().next(); if ((kr.value as number) == 9.0) { return 1; } return 0; }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn parenthesized_function_values_use_indirect_call_equivalence() {
    let module = compile(
        "function add(value: int): int { return value + 1; } function main(): int { let callback = add; return (callback)(7); }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(8, _)));
}

#[test]
fn parenthesized_dynamic_function_values_use_indirect_call_equivalence() {
    let module = compile(
        "function add(value: number): number { return value + 1.0; } function main(): int { let callback: Function = add; return ((callback)(7.0) as int); }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(8, _)));
}

#[test]
fn parenthesized_rest_function_values_pack_arguments() {
    let module = compile(
        "function count(first: int, ...rest: Array): int { return first + (rest[0] as int); } function main(): int { let callback = count; return ((callback)(7, 1, 2) as int); }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(8, _)));
}

#[test]
fn labeled_loop_control_is_interpreter_jit_equivalent() {
    assert_equivalent(
        "function main(): int { let total: int = 0; outer: for (let i: int = 0; i < 4; i = i + 1) { for (let j: int = 0; j < 4; j = j + 1) { if (j == 2) { continue outer; } total = total + 1; } } return total; }",
    );
}

#[test]
fn labeled_break_exits_the_named_loop() {
    let module = compile(
        "function main(): int { let total: int = 0; outer: for (let i: int = 0; i < 4; i = i + 1) { for (let j: int = 0; j < 4; j = j + 1) { total = total + 1; if (i == 1) { if (j == 1) { break outer; } } } } return total; }",
    );
    assert!(matches!(interp_entry(&module), Value::Int(6, _)));
}

#[test]
fn virtual_rest_methods_match_interpreter_and_jit_fallback() {
    let module = compile(
        "class Counter { count(first: int, ...rest: Array): int { return first + (rest[0] as int); } } function main(): int { let counter = new Counter(); return counter.count(7, 1, 2); }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(8, _)));
}

#[test]
fn virtual_rest_methods_accept_array_spread() {
    let module = compile(
        "class Counter { count(first: int, ...rest: Array): int { return first + (rest[0] as int); } } function main(): int { let counter = new Counter(); let values: Array = new Array(2); values[0] = 1; values[1] = 2; return counter.count(7, ...values); }",
    );
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(8, _)));
}

#[test]
fn weak_map_class_methods_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "class Key {} function main(): int { let key = new Key(); let value = new Key(); let weak = new WeakMap(); weak.set(key, value); if (weak.has(key)) { let fetched: any = weak.get(key); if (weak.delete(key)) { if (!weak.has(key)) { return 1; } } } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "WeakMap requires Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
}

#[test]
fn finalization_registry_preserves_interpreter_fallback_boundary() {
    let module = compile(
        "function cleanup(value: any): void { } class Key {} function main(): boolean { let registry = new FinalizationRegistry(cleanup); let key = new Key(); let token = new Key(); registry.register(key, 7, token); return registry.unregister(token); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "FinalizationRegistry requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn reflect_dynamic_keys_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): any { let object = dynamic_new(); let key = \"answer\"; reflect_set(object, key, box_any(41)); return reflect_get(object, key); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "dynamic property keys require Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Any(value) if value.as_float() == Some(41.0)));
}

#[test]
fn proxy_construction_preserves_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): boolean { let target = dynamic_new(); let handler = dynamic_new(); let proxy = new Proxy(target, handler); return true; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "Proxy requires Tier 0 fallback");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn proxy_get_preserves_interpreter_fallback_boundary() {
    let module = compile(
        "import { proxyGet } from \"zun:core\"; function main(): any { let target = dynamic_new(); dynamic_set(target, \"answer\", box_any(41)); let handler = dynamic_new(); let proxy = new Proxy(target, handler); return proxyGet(proxy, \"answer\"); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "Proxy get requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Any(value) if value.as_float() == Some(41.0)));
}

#[test]
fn proxy_set_preserves_interpreter_fallback_boundary() {
    let module = compile(
        "import { proxySet, proxyGet } from \"zun:core\"; function main(): any { let target = dynamic_new(); let handler = dynamic_new(); let proxy = new Proxy(target, handler); proxySet(proxy, \"answer\", box_any(41)); return proxyGet(proxy, \"answer\"); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "Proxy set requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Any(value) if value.as_float() == Some(41.0)));
}

#[test]
fn proxy_has_and_delete_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "import { proxyHas, proxySet, proxyDelete } from \"zun:core\"; function main(): boolean { let target = dynamic_new(); let handler = dynamic_new(); let proxy = new Proxy(target, handler); proxySet(proxy, \"answer\", box_any(41)); return proxyHas(proxy, \"answer\") && proxyDelete(proxy, \"answer\") && !proxyHas(proxy, \"answer\"); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "Proxy property traps require Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn proxy_get_trap_preserves_interpreter_fallback_boundary() {
    let module = compile(
        "import { dynamicSetFunction, proxyGet } from \"zun:core\"; function getTrap(target: any, key: string): any { return box_any(99); } function main(): any { let target = dynamic_new(); let handler = dynamic_new(); dynamicSetFunction(handler, \"get\", getTrap); let proxy = new Proxy(target, handler); return proxyGet(proxy, \"answer\"); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "Proxy trap invocation requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Any(value) if value.as_float() == Some(99.0)));
}

#[test]
fn proxy_property_traps_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "import { dynamicSetFunction, proxySet, proxyHas, proxyDelete } from \"zun:core\"; function setTrap(target: any, key: string, value: any): boolean { return true; } function hasTrap(target: any, key: string): boolean { return true; } function deleteTrap(target: any, key: string): boolean { return true; } function main(): boolean { let target = dynamic_new(); let handler = dynamic_new(); dynamicSetFunction(handler, \"set\", setTrap); dynamicSetFunction(handler, \"has\", hasTrap); dynamicSetFunction(handler, \"deleteProperty\", deleteTrap); let proxy = new Proxy(target, handler); return proxySet(proxy, \"answer\", box_any(41)) && proxyHas(proxy, \"answer\") && proxyDelete(proxy, \"answer\"); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "Proxy property traps require Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn map_and_set_for_of_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): number { let map = new Map(); map.set(1.0, 10.0); let set = new Set(); set.add(2.0); for (let value of map) { print(value); } for (let value of set) { print(value); } return 42.0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "collection iteration requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Float(42.0, _)));
}

#[test]
fn symbol_iterator_calls_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "import { arrayIteratorNext } from \"zun:core\"; function main(): number { let values = new Array(1); values[0] = 42.0; let iterator = values[Symbol.iterator](); return arrayIteratorNext(iterator) as number; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "Symbol.iterator requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Float(42.0, _)));
}

#[test]
fn typed_view_instance_accessors_preserve_interpreter_fallback_boundary() {
    let module = compile(
        "import { bufferNew } from \"zun:core\"; function main(): u64 { let buffer = bufferNew(4); let values = new Uint8Array(buffer, 0, 4); values.set(1, 255); return values.get(1) as u64; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "typed views require Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Int(255, _)));
}

#[test]
fn switch_lowering_is_jit_equivalent() {
    // `switch` stays on the interpreter fallback until verifier joins are
    // lowered: the method is not JIT-compiled (the module still builds) and
    // the interpreter produces the right result.
    let module = compile(
        "function main(): u64 { let value: u64 = 2; switch (value) { case 1: return 10; case 2: return 20; default: return 30; } }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "switch must remain on the interpreter fallback until verifier joins are lowered"
    );
    assert!(matches!(interp_entry(&module), Value::Int(20, _)));
}

#[test]
fn do_while_preserves_interpreter_fallback_boundary() {
    assert_equivalent(
        "function main(): int { let count: int = 0; do { count = count + 1; } while (false); return count; }",
    );
}

#[test]
fn c_style_for_is_jit_equivalent() {
    assert_equivalent(
        "function main(): int { let total: int = 0; for (let i: int = 0; i < 4; i = i + 1) { total = total + i; } return total; }",
    );
}

#[test]
fn short_circuit_logical_operators_are_jit_equivalent() {
    // The short-circuit join leaves its result on the operand stack across a
    // block boundary; the translator now threads that value as a block
    // parameter, so these functions JIT-compile and match the interpreter.
    assert_equivalent("function main(): boolean { return true || false && false; }");
    assert_equivalent("function main(): boolean { let a: int = 50; return a >= 48 && a <= 57; }");
    assert_equivalent("function main(): boolean { let a: int = 5; return a < 1 || a > 3; }");
}

#[test]
fn conditional_expression_is_jit_equivalent() {
    // The ternary join is threaded as a block parameter (see the short-circuit
    // test), so it JIT-compiles and matches the interpreter.
    assert_equivalent(
        "function main(): number { let choose: boolean = false; return choose ? 1.0 : 2.0; }",
    );
    assert_equivalent(
        "function main(): number { let choose: boolean = true; return choose ? 1.0 : 2.0; }",
    );
}

#[test]
fn null_literal_uses_the_interpreter_fallback_boundary() {
    let module = compile("function main(): any { return null; }");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "null currently requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Null));
}

#[test]
fn nullish_coalescing_uses_the_interpreter_fallback_boundary() {
    let module =
        compile("function main(): number { let value: any = null; return value ?? 42.0; }");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "nullish coalescing requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Float(42.0, _)));
}

#[test]
fn optional_dynamic_access_uses_the_interpreter_fallback_boundary() {
    let module = compile("function main(): any { let value: any = null; return value?.answer; }");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "optional access requires Tier 0 fallback"
    );
    // `null?.answer` short-circuits to `undefined` (JS semantics).
    assert!(matches!(interp_entry(&module), Value::Void));
}

#[test]
fn optional_managed_method_uses_the_interpreter_fallback_boundary() {
    let module = compile("function main(): u64 { let value = new Map(); return value?.size(); }");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "optional managed method requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Int(0, _)));
}

#[test]
fn default_parameter_uses_the_interpreter_fallback_boundary() {
    let module = compile(
        "function add(value: number = 2.0): number { return value; } function main(): number { return add(); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "default parameter requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Float(2.0, _)));
}

#[test]
fn rest_parameter_uses_the_interpreter_fallback_boundary() {
    let module = compile(
        "function count(...values: Array): u64 { return values.length; } function main(): u64 { return count(1, 2, 3); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "rest calls require Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Int(3, _)));
}

#[test]
fn spread_array_uses_the_interpreter_fallback_boundary() {
    let module = compile(
        "function count(...values: Array): u64 { return values.length; } function main(): u64 { return count(...new Array(3)); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "spread calls require Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Int(3, _)));
}

#[test]
fn fixed_spread_array_uses_the_interpreter_fallback_boundary() {
    let module = compile(
        "function count(prefix: u64, ...values: Array): u64 { return values.length; } function main(): u64 { return count(7, ...new Array(3)); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "fixed spread calls require Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Int(3, _)));
}

#[test]
fn array_destructuring_uses_the_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): number { let values = new Array(2); values[0] = 4; values[1] = 8; let [first, second] = values; return first as number + second as number; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "destructuring requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Float(12.0, _)));
}

#[test]
fn object_destructuring_uses_the_interpreter_fallback_boundary() {
    let module = compile(
        "function main(): number { let object = dynamic_new(); dynamic_set(object, \"answer\", box_any(41)); let {answer} = object; return answer as number; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "object destructuring requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Float(41.0, _)));
}

#[test]
fn method_default_parameter_uses_the_interpreter_fallback_boundary() {
    let module = compile(
        "class Box { value(factor: number = 3.0): number { return factor; } } function main(): number { let box_value = new Box(); return box_value.value(); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(
        !jit.is_compiled(entry),
        "default method parameter requires Tier 0 fallback"
    );
    assert!(matches!(interp_entry(&module), Value::Float(3.0, _)));
}
