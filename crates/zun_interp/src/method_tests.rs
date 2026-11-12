use super::*;
use crate::roots::{collect as collect_roots, collect_frame_roots};

#[test]
fn full_interpreter_gc_compacts_and_rewrites_current_locals() {
    let module = zun_compiler::compile(
            "class Box { value: int; } function main(): int { let live = new Box(7); let i: u64 = 0; while (i < 2300) { let garbage = new Box(1); i = i + 1; } return live.value; }",
        )
        .expect("compile compaction workload");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run compaction workload");
    assert!(matches!(result, Value::Int(value, _) if value == 7));
}

#[test]
fn nested_interpreter_gc_preserves_caller_roots() {
    let module = zun_compiler::compile(
            "class Box { value: int; } function allocate(): int { let i: u64 = 0; while (i < 2300) { let garbage = new Box(1); i = i + 1; } return 0; } function main(): int { let live = new Box(9); allocate(); return live.value; }",
        )
        .expect("compile nested compaction workload");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run nested compaction workload");
    assert!(matches!(result, Value::Int(value, _) if value == 9));
}

#[test]
fn extra_osr_roots_keep_managed_objects_alive() {
    let module = zun_compiler::compile("class Box { value: int; } function main() { }")
        .expect("compile root test module");
    let types = zun_types::TypeSystem::from_module(&module).expect("build type system");
    let mut heap = zun_gc::Heap::new();
    let object = heap.alloc(1, 1);
    let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
    assert_eq!(
        collect_roots(&types, &[], &[], None, &[object]),
        0,
        "an object retained by an OSR snapshot must remain live"
    );
    assert_eq!(
        collect_roots(&types, &[], &[], None, &[]),
        1,
        "the object should be reclaimed after the snapshot root is consumed"
    );
}

#[test]
fn resumable_session_completes_synchronous_async_function() {
    let module = zun_compiler::compile("async function main(): Promise { return new Promise(); }")
        .expect("compile synchronous async function");
    let entry = module.entry_point.expect("entry point");
    let mut output = CaptureOutput::default();
    let mut session =
        ResumableSession::new(&module, entry.rid(), Vec::new(), &mut output, None, None)
            .expect("create resumable session");
    assert!(matches!(
        session.poll().expect("poll session"),
        ResumeResult::Complete(Value::Object(_))
    ));
}

#[test]
fn generator_session_exposes_iterator_result_contract() {
    let module = zun_compiler::compile("function* main(): int { yield 7; return 9; }")
        .expect("compile generator function");
    let entry = module.entry_point.expect("entry point");
    let mut output = CaptureOutput::default();
    let mut generator = GeneratorSession::new(&module, entry.rid(), Vec::new(), &mut output)
        .expect("create generator session");

    let first = generator.next_result().expect("first generator result");
    assert!(matches!(first.value, Value::Float(value, zun_il::NumKind::F64) if value == 7.0));
    assert!(!first.done);

    let second = generator
        .next_result()
        .expect("completion generator result");
    assert!(matches!(second.value, Value::Int(9, zun_il::NumKind::I32)));
    assert!(second.done);

    let terminal = generator.next_result().expect("sticky completion result");
    assert!(matches!(
        terminal.value,
        Value::Int(9, zun_il::NumKind::I32)
    ));
    assert!(terminal.done);
}

fn typeof_of(source: &str) -> String {
    let module = zun_compiler::compile(source).expect("compile typeof program");
    let mut output = CaptureOutput::default();
    match run(&module, &mut output).expect("run typeof program") {
        Value::Str(text) => text.to_string(),
        other => panic!("expected a string result, found {other:?}"),
    }
}

#[test]
fn runtime_typeof_classifies_any_values() {
    assert_eq!(
        typeof_of("function main(): string { let x: any = box_any(42); return typeof x; }"),
        "number"
    );
    assert_eq!(
        typeof_of("function main(): string { let x: any = box_any(\"hi\"); return typeof x; }"),
        "string"
    );
    assert_eq!(
        typeof_of("function main(): string { let x: any = box_any(true); return typeof x; }"),
        "boolean"
    );
    assert_eq!(
        typeof_of("function main(): string { let x: any = 10n; return typeof x; }"),
        "bigint"
    );
    assert_eq!(
        typeof_of("function main(): string { let x: any = Symbol(\"s\"); return typeof x; }"),
        "symbol"
    );
    assert_eq!(
        typeof_of("function main(): string { let x: any = dynamic_new(); return typeof x; }"),
        "object"
    );
}

#[test]
fn runtime_typeof_null_is_object_and_undefined_stays_undefined() {
    // In JS `typeof null === "object"`, while an absent/void value is
    // `"undefined"`. A `null`-initialized `any` reports `object`.
    assert_eq!(
        typeof_of("function main(): string { let x: any = null; return typeof x; }"),
        "object"
    );
}

#[test]
fn runtime_typeof_supports_string_type_guard() {
    // The canonical `typeof x === "string"` narrowing pattern compiles and
    // branches correctly at runtime over an `any` value.
    let source = "function classify(x: any): string { if (typeof x === \"string\") { return \"is-string\"; } return \"other\"; } function main(): string { return classify(box_any(\"hi\")) + \"/\" + classify(box_any(7)); }";
    assert_eq!(typeof_of(source), "is-string/other");
}

#[test]
fn static_typeof_still_folds_to_constants() {
    assert_eq!(
        typeof_of("function main(): string { return typeof 1; }"),
        "number"
    );
    assert_eq!(
        typeof_of("function main(): string { return typeof \"a\"; }"),
        "string"
    );
}

fn bool_of(source: &str) -> bool {
    let module = zun_compiler::compile(source).expect("compile instanceof program");
    let mut output = CaptureOutput::default();
    match run(&module, &mut output).expect("run instanceof program") {
        Value::Bool(flag) => flag,
        other => panic!("expected a boolean result, found {other:?}"),
    }
}

#[test]
fn runtime_instanceof_handles_upcast_instance_in_any() {
    // A `Derived` held through an `any` is still `instanceof` both its own
    // class and its base — the old constant fold answered `false` here.
    let source = "class Base {} class Derived extends Base {} function main(): boolean { let x: any = new Derived(); return (x instanceof Derived) && (x instanceof Base); }";
    assert!(bool_of(source));
    // A sibling class is not matched.
    let negative = "class Base {} class Derived extends Base {} class Other extends Base {} function main(): boolean { let x: any = new Derived(); return x instanceof Other; }";
    assert!(!bool_of(negative));
    // A non-object `any` is never an instance.
    let primitive = "class Base {} function main(): boolean { let x: any = box_any(7); return x instanceof Base; }";
    assert!(!bool_of(primitive));
}

#[test]
fn static_instanceof_still_folds_for_known_instances() {
    let source = "class Base {} class Derived extends Base {} function main(): boolean { let x = new Derived(); return x instanceof Base; }";
    assert!(bool_of(source));
}

#[test]
fn instanceof_dispatches_to_symbol_has_instance_on_dynamic_object() {
    // `C[Symbol.hasInstance]` method decides `x instanceof C` for a dynamic
    // object `C`, overriding the default class check (which a dynamic object
    // could never satisfy). `accept` returns `true`, `reject` returns
    // whether the operand is the number 7.
    let accepting = "import { dynamicSetSymbolFunction } from \"zun:core\";\n\
             function accept(x: any): boolean { return true; }\n\
             function main(): boolean {\n\
               let c = dynamic_new();\n\
               dynamicSetSymbolFunction(c, Symbol.hasInstance, accept);\n\
               let x: any = box_any(5);\n\
               return x instanceof c;\n\
             }";
    assert!(bool_of(accepting));

    // The method's own result drives the answer (ToBoolean of the return).
    let selective = "import { dynamicSetSymbolFunction } from \"zun:core\";\n\
             function isSeven(x: any): boolean { return (x as int) == 7; }\n\
             function main(): boolean {\n\
               let c = dynamic_new();\n\
               dynamicSetSymbolFunction(c, Symbol.hasInstance, isSeven);\n\
               let x: any = box_any(5);\n\
               return x instanceof c;\n\
             }";
    assert!(!bool_of(selective));

    // Without a `[Symbol.hasInstance]` method a dynamic object has no class
    // identity, so the operand is never an instance.
    let bare = "function main(): boolean {\n\
               let c = dynamic_new();\n\
               let x: any = box_any(5);\n\
               return x instanceof c;\n\
             }";
    assert!(!bool_of(bare));
}

#[test]
fn string_coercion_of_primitives() {
    // `String(x)` is the ECMAScript ToString: primitives stringify
    // directly, matching the interpreter's `display`.
    assert_eq!(
        typeof_of("function main(): string { return String(42); }"),
        "42"
    );
    assert_eq!(
        typeof_of("function main(): string { return String(true); }"),
        "true"
    );
    assert_eq!(
        typeof_of("function main(): string { return String(\"hi\"); }"),
        "hi"
    );
}

#[test]
fn string_coercion_invokes_symbol_to_primitive_with_string_hint() {
    // `String(obj)` runs OrdinaryToPrimitive(hint "string"): the stored
    // `[Symbol.toPrimitive]` method is called and its result ToString'd.
    let source = "import { dynamicSetSymbolFunction } from \"zun:core\";\n\
             function toPrim(hint: any): any { return box_any(\"FROM_PRIMITIVE\"); }\n\
             function main(): string {\n\
               let o = dynamic_new();\n\
               dynamicSetSymbolFunction(o, Symbol.toPrimitive, toPrim);\n\
               let x: any = o;\n\
               return String(x);\n\
             }";
    assert_eq!(typeof_of(source), "FROM_PRIMITIVE");
}

#[test]
fn string_coercion_exposes_symbol_to_string_tag() {
    // With no `[Symbol.toPrimitive]`, `String(obj)` surfaces the string
    // `[Symbol.toStringTag]` value as `"[object <Tag>]"`.
    let tagged = "function main(): string {\n\
               let o = dynamic_new();\n\
               o[Symbol.toStringTag] = box_any(\"Widget\");\n\
               let x: any = o;\n\
               return String(x);\n\
             }";
    assert_eq!(typeof_of(tagged), "[object Widget]");

    // Default when neither well-known symbol is present.
    let plain = "function main(): string {\n\
               let o = dynamic_new();\n\
               let x: any = o;\n\
               return String(x);\n\
             }";
    assert_eq!(typeof_of(plain), "[object Object]");
}

#[test]
fn number_coercion_parses_strings_and_primitives() {
    // `Number(x)` (ECMAScript ToNumber): strings parse (NaN on failure),
    // booleans map to 0/1, and the result stringifies like any number.
    assert_eq!(
        typeof_of("function main(): string { return String(Number(\"42\")); }"),
        "42"
    );
    assert_eq!(
        typeof_of("function main(): string { return String(Number(\"  3.5 \")); }"),
        "3.5"
    );
    assert_eq!(
        typeof_of("function main(): string { return String(Number(\"0x10\")); }"),
        "16"
    );
    assert_eq!(
        typeof_of("function main(): string { return String(Number(\"nope\")); }"),
        "NaN"
    );
    assert_eq!(
        typeof_of("function main(): string { return String(Number(true)); }"),
        "1"
    );
    assert_eq!(
        typeof_of("function main(): string { return String(Number(\"\")); }"),
        "0"
    );
}

#[test]
fn number_coercion_invokes_symbol_to_primitive_with_number_hint() {
    // `Number(obj)` runs OrdinaryToPrimitive(hint "number"): the stored
    // `[Symbol.toPrimitive]` method is called and its result ToNumber'd.
    let source = "import { dynamicSetSymbolFunction } from \"zun:core\";\n\
             function toPrim(hint: any): any { return box_any(7); }\n\
             function main(): string {\n\
               let o = dynamic_new();\n\
               dynamicSetSymbolFunction(o, Symbol.toPrimitive, toPrim);\n\
               let x: any = o;\n\
               return String(Number(x));\n\
             }";
    assert_eq!(typeof_of(source), "7");
}

#[test]
fn object_get_own_property_symbols_lists_symbol_keys() {
    // `Object.getOwnPropertySymbols(obj)` returns the object's own symbol
    // keys, which index back into the object.
    let source = "function main(): u64 {\n\
               let a = Symbol(\"a\");\n\
               let b = Symbol(\"b\");\n\
               let o = dynamic_new();\n\
               o[a] = box_any(5);\n\
               o[b] = box_any(9);\n\
               let syms = Object.getOwnPropertySymbols(o);\n\
               let count: u64 = 0;\n\
               if (typeof syms[0] === \"symbol\") { count = count + 1; }\n\
               if (typeof syms[1] === \"symbol\") { count = count + 1; }\n\
               return (syms.length as u64) * 10 + count;\n\
             }";
    let module = zun_compiler::compile(source).expect("compile getOwnPropertySymbols");
    let mut output = CaptureOutput::default();
    match run(&module, &mut output).expect("run getOwnPropertySymbols") {
        Value::Int(value, _) => assert_eq!(value, 22),
        other => panic!("expected an integer result, found {other:?}"),
    }
}

#[test]
fn object_get_own_property_symbols_is_empty_without_symbol_keys() {
    let source = "function main(): u64 {\n\
               let o = dynamic_new();\n\
               o[Symbol(\"only\")] = box_any(1);\n\
               let syms = Object.getOwnPropertySymbols(dynamic_new());\n\
               return syms.length as u64;\n\
             }";
    let module = zun_compiler::compile(source).expect("compile empty getOwnPropertySymbols");
    let mut output = CaptureOutput::default();
    match run(&module, &mut output).expect("run empty getOwnPropertySymbols") {
        Value::Int(value, _) => assert_eq!(value, 0),
        other => panic!("expected an integer result, found {other:?}"),
    }
}

#[test]
fn well_known_symbols_are_distinct_stable_values() {
    // The newly-seeded well-known symbols are distinct identities: using
    // two of them as keys on one object keeps both slots independent.
    let source = "function main(): u64 {\n\
               let o = dynamic_new();\n\
               o[Symbol.match] = box_any(1);\n\
               o[Symbol.replace] = box_any(2);\n\
               o[Symbol.species] = box_any(3);\n\
               return (o[Symbol.match] as u64) * 100 + (o[Symbol.replace] as u64) * 10 + (o[Symbol.species] as u64);\n\
             }";
    let module = zun_compiler::compile(source).expect("compile well-known symbols");
    let mut output = CaptureOutput::default();
    match run(&module, &mut output).expect("run well-known symbols") {
        Value::Int(value, _) => assert_eq!(value, 123),
        other => panic!("expected an integer result, found {other:?}"),
    }
}

#[test]
fn generator_session_contextual_next_uses_owned_state() {
    let module = zun_compiler::compile("function* main(): int { yield 3; return 4; }")
        .expect("compile generator function");
    let entry = module.entry_point.expect("entry point");
    let mut legacy_output = CaptureOutput::default();
    let mut generator = GeneratorSession::new(&module, entry.rid(), Vec::new(), &mut legacy_output)
        .expect("create generator session");
    let mut first_output = CaptureOutput::default();
    assert!(matches!(
        generator
            .next_result_with_context(&mut first_output, None, None)
            .expect("first next"),
        GeneratorIteratorResult { done: false, .. }
    ));
    let mut second_output = CaptureOutput::default();
    assert!(matches!(
        generator
            .next_result_with_context(&mut second_output, None, None)
            .expect("second next"),
        GeneratorIteratorResult { done: true, .. }
    ));
}

#[test]
fn runs_generator_call_through_for_of() {
    let module = zun_compiler::compile(
            "function* values(): int { yield 7; return 9; } function main(): any { for (let value of values()) { return value; } return 0; }",
        )
        .expect("compile generator for-of");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run generator for-of");
    assert!(matches!(result, Value::Float(7.0, zun_il::NumKind::F64)));
}

#[test]
fn resumes_generator_for_each_yield_and_skips_return_value() {
    let module = zun_compiler::compile(
            "function* values(seed: number): int { yield seed; yield seed + 1; return 9; } function main(): any { let total: number = 0.0; for (let value of values(2)) { total = total + (value as number); } return total; }",
        )
        .expect("compile multi-yield generator");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run multi-yield generator");
    assert!(
        matches!(result, Value::Float(5.0, zun_il::NumKind::F64)),
        "{result:?}"
    );
}

#[test]
fn owned_generator_session_injects_context_per_resume() {
    let module = zun_compiler::compile("function* main(): int { yield 7; return 9; }")
        .expect("compile generator function");
    let entry = module.entry_point.expect("entry point");
    let mut session = OwnedGeneratorSession::new(&module, entry.rid(), Vec::new())
        .expect("create owned generator session");
    let mut first_output = CaptureOutput::default();
    assert!(matches!(
        session.resume(&mut first_output, None, None).expect("first resume"),
        ResumeResult::Yielded(Value::Float(value, zun_il::NumKind::F64)) if value == 7.0
    ));
    let mut second_output = CaptureOutput::default();
    assert!(matches!(
        session.resume(&mut second_output, None, None).expect("second resume"),
        ResumeResult::Complete(Value::Int(value, zun_il::NumKind::I32)) if value == 9
    ));
}

#[test]
fn generator_compaction_preserves_pending_object_state() {
    let module = zun_compiler::compile(
            "class Box { value: int; } function* main(): Box { let value = new Box(7); yield value; return value; }",
        )
        .expect("compile generator object state");
    let entry = module.entry_point.expect("entry point");
    let mut session = GeneratorSession::new(
        &module,
        entry.rid(),
        Vec::new(),
        &mut CaptureOutput::default(),
    )
    .expect("create generator session");
    let yielded = session.next().expect("yielded object");
    assert!(matches!(
        yielded,
        GeneratorResult::Yielded(Value::Object(_))
    ));
    let compacted = session
        .compact_pending_frames()
        .expect("pending frame compaction");
    assert!(compacted.moved_objects >= 1, "the parked promise must move");
    assert!(matches!(
        session.next().expect("completed object"),
        GeneratorResult::Complete(Value::Object(_))
    ));
}

#[test]
fn resumable_session_suspends_and_resumes_on_promise_settlement() {
    let module = zun_compiler::compile("let p = new Promise(); await p;")
        .expect("compile async await function");
    let entry = module.entry_point.expect("entry point");
    let mut output = CaptureOutput::default();
    let mut session =
        ResumableSession::new(&module, entry.rid(), Vec::new(), &mut output, None, None)
            .expect("create resumable session");
    assert!(matches!(
        session.poll().expect("initial poll"),
        ResumeResult::Suspended
    ));
    let compacted = session
        .collect_and_compact_pending_frame()
        .expect("compact pending async frame");
    assert!(compacted.moved_objects >= 1, "the parked promise must move");
    let promise = session
        .awaited_promise()
        .expect("relocated awaited promise");
    let value = Value::Int(42, zun_il::NumKind::I32);
    let boxed = value.box_any(session.heap_mut()).expect("box result");
    assert_eq!(
        session.heap_mut().promise_resolve(promise, boxed),
        Some(true)
    );
    assert!(matches!(
        session.poll().expect("resumed poll"),
        ResumeResult::Complete(Value::Void)
    ));
}

#[test]
fn resumable_session_propagates_rejected_await() {
    let module = zun_compiler::compile("let p = new Promise(); await p;")
        .expect("compile async await function");
    let entry = module.entry_point.expect("entry point");
    let mut output = CaptureOutput::default();
    let mut session =
        ResumableSession::new(&module, entry.rid(), Vec::new(), &mut output, None, None)
            .expect("create resumable session");
    assert!(matches!(
        session.poll().expect("initial poll"),
        ResumeResult::Suspended
    ));
    let promise = session.awaited_promise().expect("awaited promise");
    assert_eq!(
        session.heap_mut().promise_reject(promise, "boom"),
        Some(true)
    );
    assert!(matches!(
        session.poll(),
        Err(RunError::Thrown(Value::Str(_)))
    ));
}

#[test]
fn await_of_a_pending_promise_is_gated_by_the_scheduler() {
    // The scheduler, not the host, decides when a parked `await` is
    // runnable: polling again before the promise settles keeps the frame
    // parked rather than resuming it against a still-pending promise.
    let module = zun_compiler::compile("let p = new Promise(); await p;")
        .expect("compile async await function");
    let entry = module.entry_point.expect("entry point");
    let mut output = CaptureOutput::default();
    let mut session =
        ResumableSession::new(&module, entry.rid(), Vec::new(), &mut output, None, None)
            .expect("create resumable session");
    assert!(matches!(
        session.poll().expect("initial poll"),
        ResumeResult::Suspended
    ));
    // No settlement yet: the continuation stays parked, not resumed.
    assert!(matches!(
        session.poll().expect("re-poll"),
        ResumeResult::Suspended
    ));
    let promise = session
        .awaited_promise()
        .expect("still awaiting the promise");
    let boxed = Value::Int(42, zun_il::NumKind::I32)
        .box_any(session.heap_mut())
        .expect("box result");
    assert_eq!(
        session.heap_mut().promise_resolve(promise, boxed),
        Some(true)
    );
    assert!(matches!(
        session.poll().expect("resumed poll"),
        ResumeResult::Complete(Value::Void)
    ));
    // Once resumed and complete, nothing else is runnable.
    assert!(matches!(
        session.poll().expect("drained poll"),
        ResumeResult::Suspended
    ));
}

#[test]
fn await_pending_rejection_runs_an_in_body_finally_on_the_unwinding_path() {
    // A rejection wakes the parked frame through the unwinding path, which
    // now traverses the method's handler table: the `finally` around the
    // `await` runs before the rejection escapes the body.
    let module =
        zun_compiler::compile("let p = new Promise(); try { await p; } finally { print(1.0); }")
            .expect("compile async try/finally await");
    let entry = module.entry_point.expect("entry point");
    let mut output = CaptureOutput::default();
    let mut session =
        ResumableSession::new(&module, entry.rid(), Vec::new(), &mut output, None, None)
            .expect("create resumable session");
    assert!(matches!(
        session.poll().expect("initial poll"),
        ResumeResult::Suspended
    ));
    let promise = session.awaited_promise().expect("awaited promise");
    assert_eq!(
        session.heap_mut().promise_reject(promise, "boom"),
        Some(true)
    );
    // The finally runs, then the rejection continues to escape the body.
    assert!(matches!(
        session.poll(),
        Err(RunError::Thrown(Value::Str(_)))
    ));
    assert_eq!(output.lines, ["1"]);
}

#[test]
fn forced_gc_while_a_frame_is_parked_preserves_the_rejection_path() {
    // A collection while the `await` frame is parked must relocate it (and
    // the promise→id bridge) without corruption; the rejection still
    // unwinds after the move.
    let module = zun_compiler::compile("let p = new Promise(); await p;")
        .expect("compile async await function");
    let entry = module.entry_point.expect("entry point");
    let mut output = CaptureOutput::default();
    let mut session =
        ResumableSession::new(&module, entry.rid(), Vec::new(), &mut output, None, None)
            .expect("create resumable session");
    assert!(matches!(
        session.poll().expect("initial poll"),
        ResumeResult::Suspended
    ));
    let compacted = session
        .collect_and_compact_pending_frame()
        .expect("compact parked async frame");
    assert!(compacted.moved_objects >= 1, "the parked promise must move");
    let promise = session
        .awaited_promise()
        .expect("relocated awaited promise");
    assert_eq!(
        session.heap_mut().promise_reject(promise, "late"),
        Some(true)
    );
    assert!(matches!(
        session.poll(),
        Err(RunError::Thrown(Value::Str(_)))
    ));
}

#[test]
fn resumable_session_resumes_nested_async_callers() {
    let module = zun_compiler::compile(
            "async function child(): Promise { let p = new Promise(); await p; return p; } let q = child(); await q;",
        )
        .expect("compile nested async functions");
    let entry = module.entry_point.expect("entry point");
    let mut output = CaptureOutput::default();
    let mut session =
        ResumableSession::new(&module, entry.rid(), Vec::new(), &mut output, None, None)
            .expect("create resumable session");
    assert!(matches!(
        session.poll().expect("initial poll"),
        ResumeResult::Suspended
    ));
    let promise = session.awaited_promise().expect("child awaited promise");
    let boxed = Value::Int(7, zun_il::NumKind::I32)
        .box_any(session.heap_mut())
        .expect("box child result");
    assert_eq!(
        session.heap_mut().promise_resolve(promise, boxed),
        Some(true)
    );
    assert!(matches!(
        session.poll().expect("resume nested call"),
        ResumeResult::Complete(Value::Void)
    ));
}

#[test]
fn settled_await_reads_the_managed_promise_result() {
    let mut heap = zun_gc::Heap::new();
    let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
    let promise = heap.alloc_promise();
    let value = Value::Int(7, zun_il::NumKind::I32);
    let boxed = value.box_any(&mut heap).expect("box await result");
    assert_eq!(heap.promise_resolve(promise, boxed), Some(true));
    assert_eq!(
        settled_await_value(promise)
            .unwrap()
            .map(|resolved| resolved.as_int()),
        Some(7)
    );
}

#[test]
fn executes_do_while_at_least_once() {
    let module = zun_compiler::compile(
            "function main(): int { let count: int = 0; do { count = count + 1; } while (false); return count; }",
        )
        .expect("compile do-while");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run do-while");
    assert_eq!(value.as_int(), 1);
}

#[test]
fn do_while_continue_checks_the_condition() {
    let module = zun_compiler::compile(
            "function main(): int { let count: int = 0; do { count = count + 1; continue; } while (count < 3); return count; }",
        )
        .expect("compile do-while continue");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run do-while continue");
    assert_eq!(value.as_int(), 3);
}

#[test]
fn executes_c_style_for_loop_and_continue() {
    let module = zun_compiler::compile(
            "function main(): int { let total: int = 0; for (let i: int = 0; i < 4; i = i + 1) { if (i == 2) { continue; } total = total + i; } return total; }",
        )
        .expect("compile c-style for");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run c-style for");
    assert_eq!(value.as_int(), 4);
}

#[test]
fn short_circuit_logical_operators_return_boolean_values() {
    let module =
        zun_compiler::compile("function main(): boolean { return true || false && false; }")
            .expect("compile logical operators");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run logical operators");
    assert!(matches!(value, Value::Bool(true)));
}

#[test]
fn conditional_expression_selects_one_branch() {
    let module = zun_compiler::compile(
        "function main(): number { let choose: boolean = false; return choose ? 1.0 : 2.0; }",
    )
    .expect("compile conditional expression");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run conditional expression");
    assert!(matches!(value, Value::Float(2.0, zun_il::NumKind::F64)));
}

#[test]
fn null_literal_is_preserved_by_tier_zero() {
    let module = zun_compiler::compile("function main(): any { return null; }")
        .expect("compile null literal");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("null should execute");
    assert!(matches!(value, Value::Null));
}

#[test]
fn nullish_coalescing_selects_the_fallback_value() {
    let module = zun_compiler::compile(
        "function main(): number { let value: any = null; return value ?? 42.0; }",
    )
    .expect("compile nullish coalescing");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run nullish coalescing");
    assert!(matches!(value, Value::Float(42.0, zun_il::NumKind::F64)));
}

#[test]
fn optional_dynamic_access_short_circuits_null() {
    let module = zun_compiler::compile(
        "function main(): any { let value: any = null; return value?.answer; }",
    )
    .expect("compile optional access");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run optional access");
    // `null?.answer` short-circuits to `undefined` (JS semantics).
    assert!(
        matches!(value, Value::Void)
            || matches!(value, Value::Any(any) if any.raw_parts() == zun_core::Any::void().raw_parts() || any.is_hole())
    );
}

#[test]
fn optional_dynamic_access_reads_non_null_property() {
    let module = zun_compiler::compile(
            "function main(): any { let object = dynamic_new(); dynamic_set(object, \"answer\", box_any(41)); let value: any = object; return value?.answer; }",
        )
        .expect("compile optional property read");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run optional property read");
    assert!(matches!(value, Value::Any(any) if any.as_float() == Some(41.0)));
}

#[test]
fn optional_managed_method_calls_non_null_receiver() {
    let module = zun_compiler::compile(
            "function main(): u64 { let value = new Map(); value.set(\"answer\", 41); return value?.size(); }",
        )
        .expect("compile optional managed method");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run optional managed method");
    assert!(matches!(value, Value::Int(1, zun_il::NumKind::U64)));
}

#[test]
fn default_parameter_is_used_when_argument_is_omitted() {
    let module = zun_compiler::compile(
            "function add(value: number = 2.0): number { return value; } function main(): number { return add(); }",
        )
        .expect("compile default parameter");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run default parameter");
    assert!(matches!(value, Value::Float(2.0, zun_il::NumKind::F64)));
}

#[test]
fn rest_parameter_materializes_extra_arguments_as_an_array() {
    let module = zun_compiler::compile(
            "function count(...values: Array): u64 { return values.length; } function main(): u64 { return count(1, 2, 3); }",
        )
        .expect("compile rest parameter");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run rest parameter");
    assert!(matches!(value, Value::Int(3, zun_il::NumKind::U64)));
}

#[test]
fn spread_array_feeds_a_zero_fixed_rest_function() {
    let module = zun_compiler::compile(
            "function count(...values: Array): u64 { return values.length; } function main(): u64 { return count(...new Array(3)); }",
        )
        .expect("compile spread call");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run spread call");
    assert!(matches!(value, Value::Int(3, zun_il::NumKind::U64)));
}

#[test]
fn spread_array_follows_fixed_arguments_before_rest() {
    let module = zun_compiler::compile(
            "function count(prefix: u64, ...values: Array): u64 { return values.length; } function main(): u64 { return count(7, ...new Array(3)); }",
        )
        .expect("compile fixed spread call");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run fixed spread call");
    assert!(matches!(value, Value::Int(3, zun_il::NumKind::U64)));
}

#[test]
fn array_destructuring_reads_elements_by_index() {
    let module = zun_compiler::compile(
            "function main(): number { let values = new Array(2); values[0] = 4; values[1] = 8; let [first, second] = values; return first as number + second as number; }",
        )
        .expect("compile array destructuring");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run array destructuring");
    assert!(matches!(value, Value::Float(12.0, zun_il::NumKind::F64)));
}

#[test]
fn object_destructuring_reads_dynamic_properties() {
    let module = zun_compiler::compile(
            "function main(): number { let object = dynamic_new(); dynamic_set(object, \"answer\", box_any(41)); let {answer} = object; return answer as number; }",
        )
        .expect("compile object destructuring");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run object destructuring");
    assert!(matches!(value, Value::Float(41.0, zun_il::NumKind::F64)));
}

#[test]
fn method_default_parameter_preserves_the_receiver() {
    let module = zun_compiler::compile(
            "class Box { value(factor: number = 3.0): number { return factor; } } function main(): number { let box_value = new Box(); return box_value.value(); }",
        )
        .expect("compile method default parameter");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run method default parameter");
    assert!(matches!(value, Value::Float(3.0, zun_il::NumKind::F64)));
}
use zun_il::{encode, Instruction, Operand};
use zun_metadata::{FieldDef, MethodDef, TableKind, Token, TypeDef};
use zun_module::write_body;

#[test]
fn catches_a_thrown_error_in_the_current_frame() {
    let module = zun_compiler::compile(
            "function main(): int { try { throw new Error(\"boom\"); } catch (error: Error) { return 42; } }",
        )
        .expect("compile");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("catch should handle throw");
    assert_eq!(value.as_int(), 42);
}

#[test]
fn catch_all_handles_a_non_object_exception() {
    let module = zun_compiler::compile_with_oxc(
        "function main(): int { try { throw 7; } catch (error) { return 42; } }",
        true,
    )
    .expect("compile catch-all handler");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("catch-all should handle primitive throw");
    assert_eq!(value.as_int(), 42);
}

#[test]
fn finally_runs_before_a_normal_return() {
    let module =
        zun_compiler::compile("function main(): int { try { return 1; } finally { print(2); } }")
            .expect("compile");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("return should succeed");
    assert_eq!(value.as_int(), 1);
    assert_eq!(output.lines, vec!["2"]);
}

#[test]
fn finally_runs_before_an_unhandled_rethrow() {
    let module = zun_compiler::compile(
        "function main() { try { throw new Error(\"boom\"); } finally { print(2); } }",
    )
    .expect("compile");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output);
    assert!(matches!(result, Err(RunError::Thrown(_))));
    assert_eq!(output.lines, vec!["2"]);
}

#[test]
fn catches_an_exception_from_a_called_method() {
    let module = zun_compiler::compile(
            "function fail(): int { throw new Error(\"boom\"); } function main(): int { try { return fail(); } catch (error: Error) { return 7; } }",
        )
        .expect("compile");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("caller should catch child throw");
    assert_eq!(value.as_int(), 7);
}

#[test]
fn nested_catch_can_rethrow_to_an_outer_handler() {
    let module = zun_compiler::compile(
            "function main(): int { try { try { throw new Error(\"boom\"); } catch (inner: Error) { throw inner; } } catch (outer: Error) { return 9; } }",
        )
        .expect("compile");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("outer catch should handle rethrow");
    assert_eq!(value.as_int(), 9);
}

#[test]
fn inner_finally_rethrows_to_an_outer_catch_in_the_same_frame() {
    let module = zun_compiler::compile(
            "function main(): int { try { try { throw new Error(\"boom\"); } finally { print(1); } } catch (outer: Error) { return 42; } }",
        )
        .expect("compile");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("outer catch should receive rethrow");
    assert_eq!(value.as_int(), 42);
    assert_eq!(output.lines, vec!["1"]);
}

#[test]
fn callee_finally_rethrows_to_a_caller_catch() {
    let module = zun_compiler::compile(
            "function fail(): int { try { throw new Error(\"boom\"); } finally { print(1); } } function main(): int { try { return fail(); } catch (outer: Error) { return 42; } }",
        )
        .expect("compile");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("caller catch should receive rethrow");
    assert_eq!(value.as_int(), 42);
    assert_eq!(output.lines, vec!["1"]);
}

#[test]
fn cross_module_member_call_throw_routes_to_caller_catch() {
    // Regression: a thrown exception from a cross-module `MemberRef` call must
    // be routed against the CURRENT method's handler table, not the callee
    // module's body. `app.main` has a try/catch; `core.boom` throws.
    let loader = zun_compiler::compile_linked_modules([
        (
            "core",
            "function boom(): int { throw new Error(\"cross-module boom\"); }",
        ),
        (
            "app",
            "import { boom } from \"core\"; function main(): int { try { return boom(); } catch (error: Error) { print(42); return 7; } }",
        ),
    ])
    .expect("linked compilation");
    let app = loader.get("app").expect("app");
    let main_rid = app.module().entry_point.expect("entry point").rid();
    let mut output = CaptureOutput::default();
    let value = run_method_loaded(app, &loader, main_rid, vec![], &mut output)
        .expect("caller catch should handle the cross-module throw");
    assert_eq!(value.as_int(), 7);
    assert_eq!(output.lines, vec!["42"]);
}

#[test]
fn return_inside_inner_try_runs_finally_before_outer_catch_path() {
    let module = zun_compiler::compile(
            "function main(): int { try { try { return 1; } finally { print(1); } } catch (outer: Error) { return 42; } }",
        )
        .expect("compile");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("normal return should survive cleanup");
    assert_eq!(value.as_int(), 1);
    assert_eq!(output.lines, vec!["1"]);
}

#[test]
fn runs_non_entry_method_with_arguments() {
    let module = zun_compiler::compile(
            "function add_one(value: int): int { return value + 1; } function main(): int { return 0; }",
        )
        .expect("compile");
    let mut output = CaptureOutput::default();
    let result = run_method(
        &module,
        1,
        vec![Value::Int(41, zun_il::NumKind::I32)],
        &mut output,
    )
    .expect("run method");
    assert!(matches!(result, Value::Int(42, zun_il::NumKind::I32)));
}

#[test]
fn executes_indirect_call_for_non_capturing_function() {
    let mut module = zun_module::Module::new();
    let add_body = [
        Instruction::new(Opcode::LoadLocal, Operand::Local(0)),
        Instruction::new(Opcode::ConstF64, Operand::Float64(1.0)),
        Instruction::simple(Opcode::AddF64),
        Instruction::simple(Opcode::Ret),
    ];
    let mut add_code = Vec::new();
    for instruction in add_body {
        encode(&instruction, &mut add_code);
    }
    let add_offset = write_body(
        &mut module.code,
        1,
        1,
        true,
        zun_il::JitKind::Num(zun_il::NumKind::F64).to_byte(),
        &[zun_il::JitKind::Num(zun_il::NumKind::F64).to_byte()],
        &add_code,
    );
    module.tables.add_method_def(MethodDef {
        name: module.strings.intern("add_one"),
        signature: zun_metadata::BlobIndex(0),
        body_offset: add_offset,
        is_static: false,
    });
    let main_body = [
        Instruction::new(
            Opcode::LoadFunction,
            Operand::Token(Token::new(TableKind::MethodDef, 1)),
        ),
        Instruction::new(Opcode::ConstF64, Operand::Float64(41.0)),
        Instruction::new(
            Opcode::CallIndirect,
            Operand::Token(Token::new(TableKind::MethodDef, 1)),
        ),
        Instruction::simple(Opcode::Ret),
    ];
    let mut main_code = Vec::new();
    for instruction in main_body {
        encode(&instruction, &mut main_code);
    }
    let main_offset = write_body(
        &mut module.code,
        0,
        0,
        true,
        zun_il::JitKind::Num(zun_il::NumKind::F64).to_byte(),
        &[],
        &main_code,
    );
    module.tables.add_method_def(MethodDef {
        name: module.strings.intern("main"),
        signature: zun_metadata::BlobIndex(0),
        body_offset: main_offset,
        is_static: false,
    });
    module.entry_point = Some(Token::new(TableKind::MethodDef, 2));
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run indirect call");
    assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_indirect_call_with_closure_environment() {
    let mut module = zun_module::Module::new();
    let environment_field = module.tables.add_field_def(FieldDef {
        name: module.strings.intern("captured"),
        signature: zun_metadata::BlobIndex(0),
        kind: zun_il::JitKind::Num(zun_il::NumKind::F64).to_byte(),
        is_static: false,
        is_private: false,
    });
    let environment_type = module.tables.add_type_def(TypeDef {
        name: module.strings.intern("ClosureEnvironment"),
        namespace: zun_metadata::StringIndex(0),
        base: Token::null(TableKind::TypeDef),
        field_start: environment_field.rid(),
        field_end: environment_field.rid() + 1,
        method_start: 1,
        method_end: 2,
    });
    let scratch_type = module.tables.add_type_def(TypeDef {
        name: module.strings.intern("Scratch"),
        namespace: zun_metadata::StringIndex(0),
        base: Token::null(TableKind::TypeDef),
        field_start: 0,
        field_end: 0,
        method_start: 0,
        method_end: 0,
    });
    let body = [
        Instruction::new(Opcode::LoadLocal, Operand::Local(1)),
        Instruction::new(Opcode::LoadLocal, Operand::Local(0)),
        Instruction::new(Opcode::LoadField, Operand::Token(environment_field)),
        Instruction::new(Opcode::ConstF64, Operand::Float64(1.0)),
        Instruction::simple(Opcode::AddF64),
        Instruction::simple(Opcode::AddF64),
        Instruction::simple(Opcode::Ret),
    ];
    let mut code = Vec::new();
    for instruction in body {
        encode(&instruction, &mut code);
    }
    let target_offset = write_body(
        &mut module.code,
        2,
        2,
        true,
        zun_il::JitKind::Num(zun_il::NumKind::F64).to_byte(),
        &[
            zun_il::JitKind::Object.to_byte(),
            zun_il::JitKind::Num(zun_il::NumKind::F64).to_byte(),
        ],
        &code,
    );
    module.tables.add_method_def(MethodDef {
        name: module.strings.intern("capturedAddOne"),
        signature: zun_metadata::BlobIndex(0),
        body_offset: target_offset,
        is_static: false,
    });
    let mut main_body = vec![
        Instruction::new(Opcode::ConstF64, Operand::Float64(41.0)),
        Instruction::new(Opcode::NewObj, Operand::Token(environment_type)),
        Instruction::new(
            Opcode::MakeClosure,
            Operand::Token(Token::new(TableKind::MethodDef, 1)),
        ),
    ];
    for _ in 0..300 {
        main_body.push(Instruction::new(
            Opcode::NewObj,
            Operand::Token(scratch_type),
        ));
        main_body.push(Instruction::simple(Opcode::Pop));
    }
    main_body.extend([
        Instruction::new(Opcode::ConstF64, Operand::Float64(0.0)),
        Instruction::new(
            Opcode::CallIndirect,
            Operand::Token(Token::null(TableKind::MethodDef)),
        ),
        Instruction::simple(Opcode::Ret),
    ]);
    let mut main_code = Vec::new();
    for instruction in main_body {
        encode(&instruction, &mut main_code);
    }
    let main_offset = write_body(
        &mut module.code,
        0,
        0,
        true,
        zun_il::JitKind::Num(zun_il::NumKind::F64).to_byte(),
        &[],
        &main_code,
    );
    module.tables.add_method_def(MethodDef {
        name: module.strings.intern("main"),
        signature: zun_metadata::BlobIndex(0),
        body_offset: main_offset,
        is_static: false,
    });
    module.entry_point = Some(Token::new(TableKind::MethodDef, 2));
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run closure environment call");
    assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
}

#[test]
fn closure_environment_is_an_interpreter_gc_root() {
    let mut heap = zun_gc::Heap::new();
    let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
    let environment = heap.alloc(1, 0);
    let function = Value::Function(zun_core::FunctionHandle::new(
        1,
        Some(environment.addr() as usize),
    ));
    let mut roots = Vec::new();
    collect_frame_roots(&[function], &[], &mut roots);
    assert_eq!(roots, vec![environment]);
}

#[test]
fn compiles_named_function_reference_and_indirect_call() {
    let module = zun_compiler::compile(
        "function add_one(value: number): number { return value + 1.0; }\n\
             function main(): number { let f = add_one; return f(41.0); }",
    )
    .expect("compile function value source");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run function value source");
    assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
}

#[test]
fn passes_named_function_reference_through_typed_parameter() {
    let module = zun_compiler::compile(
        "function add_one(value: number): number { return value + 1.0; }\n\
             function apply(f: add_one, value: number): number { return f(value); }\n\
             function main(): number { return apply(add_one, 41.0); }",
    )
    .expect("compile typed function parameter");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run typed function parameter");
    assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
}

#[test]
fn resolves_forward_function_type_annotations() {
    let module = zun_compiler::compile(
        "function apply(f: add_one, value: number): number { return f(value); }\n\
             function main(): number { return apply(add_one, 41.0); }\n\
             function add_one(value: number): number { return value + 1.0; }",
    )
    .expect("compile forward function type");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run forward function type");
    assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
}

#[test]
fn invokes_generic_function_callback() {
    let module = zun_compiler::compile(
        "function add_one(value: number): number { return value + 1.0; }\n\
             function apply(f: Function, value: number): any { return f(value); }\n\
             function main(): any { return apply(add_one, 41.0); }",
    )
    .expect("compile generic function callback");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run generic function callback");
    assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
}

#[test]
fn finalization_registry_unregisters_by_token_identity() {
    // `unregister(token)` removes the registration made with that token
    // object and returns true; an unknown token returns false.
    let module = zun_compiler::compile(
        "function cleanup(value: any): void { }\n\
             class Target {}\n\
             function main(): boolean { \
                let registry = new FinalizationRegistry(cleanup); \
                let target = new Target(); \
                let token = new Target(); \
                registry.register(target, 7, token); \
                let removed = registry.unregister(token); \
                let other = new Target(); \
                let missing = registry.unregister(other); \
                return removed && !missing; }",
    )
    .expect("compile finalization registry");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run finalization registry");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn finalization_registry_without_token_cannot_be_unregistered() {
    // A registration made without an unregister token can never be removed,
    // per the ECMAScript spec.
    let module = zun_compiler::compile(
        "function cleanup(value: any): void { }\n\
             class Target {}\n\
             function main(): boolean { \
                let registry = new FinalizationRegistry(cleanup); \
                let target = new Target(); \
                let token = new Target(); \
                registry.register(target, 7); \
                return registry.unregister(token); }",
    )
    .expect("compile finalization registry");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run finalization registry");
    assert!(matches!(result, Value::Bool(false)));
}

#[test]
fn reflect_aliases_use_dynamic_object_storage() {
    let module = zun_compiler::compile(
            "function main(): any { let object = dynamic_new(); let key = \"answer\"; reflect_set(object, key, box_any(41)); return reflect_get(object, key); }",
        )
        .expect("compile reflect aliases");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run reflect aliases");
    assert!(matches!(result, Value::Any(value) if value.as_float() == Some(41.0)));
}

#[test]
fn reflect_has_and_delete_use_dynamic_object_storage() {
    let module = zun_compiler::compile(
            "import { reflectHas, reflectDelete } from \"zun:core\"; function main(): boolean { let object = dynamic_new(); dynamic_set(object, \"answer\", box_any(41)); if (!reflectHas(object, \"answer\")) { return false; } if (!reflectDelete(object, \"answer\")) { return false; } return !reflectHas(object, \"answer\"); }",
        )
        .expect("compile reflect has/delete");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run reflect has/delete");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn constructs_proxy_with_managed_target_and_handler() {
    let module = zun_compiler::compile(
            "function main(): boolean { let target = dynamic_new(); let handler = dynamic_new(); let proxy = new Proxy(target, handler); return true; }",
        )
        .expect("compile proxy construction");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run proxy construction");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn proxy_get_falls_back_to_target_dynamic_properties() {
    let module = zun_compiler::compile(
            "import { proxyGet } from \"zun:core\"; function main(): any { let target = dynamic_new(); dynamic_set(target, \"answer\", box_any(41)); let handler = dynamic_new(); let proxy = new Proxy(target, handler); return proxyGet(proxy, \"answer\"); }",
        )
        .expect("compile proxy get");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run proxy get");
    assert!(matches!(result, Value::Any(value) if value.as_float() == Some(41.0)));
}

#[test]
fn proxy_set_falls_back_to_target_dynamic_properties() {
    let module = zun_compiler::compile(
            "import { proxySet, proxyGet } from \"zun:core\"; function main(): any { let target = dynamic_new(); let handler = dynamic_new(); let proxy = new Proxy(target, handler); proxySet(proxy, \"answer\", box_any(41)); return proxyGet(proxy, \"answer\"); }",
        )
        .expect("compile proxy set");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run proxy set");
    assert!(matches!(result, Value::Any(value) if value.as_float() == Some(41.0)));
}

#[test]
fn proxy_has_and_delete_fall_back_to_target_dynamic_properties() {
    let module = zun_compiler::compile(
            "import { proxyHas, proxySet, proxyDelete } from \"zun:core\"; function main(): boolean { let target = dynamic_new(); let handler = dynamic_new(); let proxy = new Proxy(target, handler); proxySet(proxy, \"answer\", box_any(41)); if (!proxyHas(proxy, \"answer\")) { return false; } return proxyDelete(proxy, \"answer\") && !proxyHas(proxy, \"answer\"); }",
        )
        .expect("compile proxy has/delete");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run proxy has/delete");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn proxy_get_invokes_a_named_handler_trap() {
    let module = zun_compiler::compile(
            "import { dynamicSetFunction } from \"zun:core\"; import { proxyGet } from \"zun:core\"; function getTrap(target: any, key: string): any { return box_any(99); } function main(): any { let target = dynamic_new(); let handler = dynamic_new(); dynamicSetFunction(handler, \"get\", getTrap); let proxy = new Proxy(target, handler); return proxyGet(proxy, \"answer\"); }",
        )
        .expect("compile proxy trap");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run proxy trap");
    assert!(matches!(result, Value::Any(value) if value.as_float() == Some(99.0)));
}

#[test]
fn proxy_property_traps_invoke_named_handlers() {
    let module = zun_compiler::compile(
            "import { dynamicSetFunction, proxySet, proxyHas, proxyDelete } from \"zun:core\"; function setTrap(target: any, key: string, value: any): boolean { return true; } function hasTrap(target: any, key: string): boolean { return true; } function deleteTrap(target: any, key: string): boolean { return true; } function main(): boolean { let target = dynamic_new(); let handler = dynamic_new(); dynamicSetFunction(handler, \"set\", setTrap); dynamicSetFunction(handler, \"has\", hasTrap); dynamicSetFunction(handler, \"deleteProperty\", deleteTrap); let proxy = new Proxy(target, handler); return proxySet(proxy, \"answer\", box_any(41)) && proxyHas(proxy, \"answer\") && proxyDelete(proxy, \"answer\"); }",
        )
        .expect("compile proxy property traps");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run proxy property traps");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn proxy_field_get_syntax_routes_through_handler_trap() {
    let module = zun_compiler::compile(
            "import { dynamicSetFunction } from \"zun:core\"; function getTrap(target: any, key: string): any { return box_any(99); } function main(): any { let target = dynamic_new(); dynamic_set(target, \"answer\", box_any(41)); let handler = dynamic_new(); dynamicSetFunction(handler, \"get\", getTrap); let proxy = new Proxy(target, handler); return proxy.answer; }",
        )
        .expect("compile proxy field get");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run proxy field get");
    assert!(matches!(result, Value::Any(value) if value.as_float() == Some(99.0)));
}

#[test]
fn proxy_field_get_syntax_falls_back_to_target_without_trap() {
    let module = zun_compiler::compile(
            "function main(): any { let target = dynamic_new(); dynamic_set(target, \"answer\", box_any(41)); let handler = dynamic_new(); let proxy = new Proxy(target, handler); return proxy.answer; }",
        )
        .expect("compile proxy field get fallback");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run proxy field get fallback");
    assert!(matches!(result, Value::Any(value) if value.as_float() == Some(41.0)));
}

#[test]
fn proxy_field_set_syntax_routes_through_handler_trap() {
    // A `set` trap that intercepts without writing leaves the target
    // untouched; reading the target directly still sees the original value.
    let module = zun_compiler::compile(
            "import { dynamicSetFunction } from \"zun:core\"; function setTrap(target: any, key: string, value: any): boolean { return true; } function main(): any { let target = dynamic_new(); dynamic_set(target, \"answer\", box_any(1)); let handler = dynamic_new(); dynamicSetFunction(handler, \"set\", setTrap); let proxy = new Proxy(target, handler); proxy.answer = box_any(7); return reflect_get(target, \"answer\"); }",
        )
        .expect("compile proxy field set");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run proxy field set");
    assert!(matches!(result, Value::Any(value) if value.as_float() == Some(1.0)));
}

#[test]
fn proxy_field_set_syntax_falls_back_to_target_without_trap() {
    // No `set` trap: the write routes through to the target, and a
    // trap-less read observes it (round-trip through the proxy).
    let module = zun_compiler::compile(
            "function main(): any { let target = dynamic_new(); let handler = dynamic_new(); let proxy = new Proxy(target, handler); proxy.answer = box_any(7); return proxy.answer; }",
        )
        .expect("compile proxy field set fallback");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run proxy field set fallback");
    assert!(matches!(result, Value::Any(value) if value.as_float() == Some(7.0)));
}

#[test]
fn proxy_index_get_syntax_routes_through_handler_trap() {
    let module = zun_compiler::compile(
            "import { dynamicSetFunction } from \"zun:core\"; function getTrap(target: any, key: string): any { return box_any(123); } function main(): any { let target = dynamic_new(); let handler = dynamic_new(); dynamicSetFunction(handler, \"get\", getTrap); let proxy = new Proxy(target, handler); let key = \"x\"; return proxy[key]; }",
        )
        .expect("compile proxy index get");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run proxy index get");
    assert!(matches!(result, Value::Any(value) if value.as_float() == Some(123.0)));
}

#[test]
fn reflect_get_and_set_operate_on_dynamic_storage() {
    let module = zun_compiler::compile(
            "function main(): any { let obj = dynamic_new(); Reflect.set(obj, \"k\", box_any(5)); return Reflect.get(obj, \"k\"); }",
        )
        .expect("compile Reflect get/set");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Reflect get/set");
    assert!(matches!(result, Value::Any(value) if value.as_float() == Some(5.0)));
}

#[test]
fn reflect_has_and_delete_and_own_keys() {
    let module = zun_compiler::compile(
            "function main(): boolean { let obj = dynamic_new(); dynamic_set(obj, \"k\", box_any(5)); if (!Reflect.has(obj, \"k\")) { return false; } if (Reflect.has(obj, \"nope\")) { return false; } if (Reflect.ownKeys(obj).length != 1) { return false; } if (!Reflect.deleteProperty(obj, \"k\")) { return false; } return !Reflect.has(obj, \"k\"); }",
        )
        .expect("compile Reflect has/delete/ownKeys");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Reflect has/delete/ownKeys");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn global_this_reflect_resolves_to_a_descriptor() {
    let module = zun_compiler::compile("function main(): any { return globalThis.Reflect; }")
        .expect("compile globalThis.Reflect");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run globalThis.Reflect");
    // The realm seeds a class descriptor for every registered base type, so
    // `globalThis.Reflect` resolves to a non-void value.
    assert!(!matches!(result, Value::Void));
}

#[test]
fn host_function_entry_reuses_callable_handle_dispatch() {
    let module = zun_compiler::compile(
        "function add_one(value: number): number { return value + 1.0; }\n\
             function main(): number { return 0.0; }",
    )
    .expect("compile host function callback");
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|method| module.strings.get(method.name) == "add_one")
        .map(|index| index as u32 + 1)
        .expect("add_one method");
    let mut output = CaptureOutput::default();
    let result = run_function(
        &module,
        zun_core::FunctionHandle::new(rid, None),
        vec![Value::Float(41.0, zun_il::NumKind::F64)],
        &mut output,
    )
    .expect("run host callback");
    assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
}

#[test]
fn invokes_a_non_capturing_arrow_closure() {
    let module =
        zun_compiler::compile("function main(): any { let f = x => x * 2.0; return f(21.0); }")
            .expect("compile arrow closure");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run arrow closure");
    assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
}

/// Extract the numeric result of a closure test, whether it comes back as a
/// typed `Float` or boxed in `Any` (untyped closure params are `any` now, so
/// `x + k` takes the dynamic add path and yields an `Any`-wrapped number).
fn closure_result_number(result: &Value) -> f64 {
    match result {
        Value::Float(n, _) => *n,
        Value::Int(n, _) => *n as f64,
        Value::Any(any) => any.as_float().expect("numeric closure result"),
        other => panic!("expected a numeric closure result, found {other:?}"),
    }
}

#[test]
fn invokes_an_arrow_closure_with_a_read_only_capture() {
    let module = zun_compiler::compile(
        "function main(): any { let k = 10.0; let f = x => x + k; return f(32.0); }",
    )
    .expect("compile captured arrow closure");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run captured arrow closure");
    assert_eq!(closure_result_number(&result), 42.0);
}

#[test]
fn mutable_capture_observes_binding_updates() {
    let module = zun_compiler::compile(
        "function main(): any { let k = 10.0; let f = x => x + k; k = 20.0; return f(22.0); }",
    )
    .expect("compile mutable captured arrow closure");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run mutable captured arrow closure");
    assert_eq!(closure_result_number(&result), 42.0);
}

#[test]
fn nested_arrow_closures_share_their_outer_environment() {
    let module = zun_compiler::compile(
            "function main(): any { let k = 10.0; let outer = x => { let inner = y => x + y + k; return inner(1.0); }; return outer(31.0); }",
        )
        .expect("compile nested arrow closures");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run nested arrow closures");
    assert_eq!(closure_result_number(&result), 42.0);
}

#[test]
fn iterates_an_array_with_for_of() {
    let module = zun_compiler::compile(
            "function main(): any { let values = new Array(2); values[0] = 1.0; values[1] = 2.0; for (let value of values) { print(value); } return 42.0; }",
        )
        .expect("compile array for-of");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run array for-of");
    assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    assert_eq!(output.lines, vec!["1", "2"]);
}

#[test]
fn invokes_array_symbol_iterator_from_source() {
    let module = zun_compiler::compile(
            "import { arrayIteratorNext } from \"zun:core\"; function main(): u64 { let values = new Array(1); values[0] = 7.0; let iterator = values[Symbol.iterator](); return arrayIteratorNext(iterator) as u64; }",
        )
        .expect("compile Symbol.iterator call");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Symbol.iterator call");
    assert!(matches!(result, Value::Int(7, zun_il::NumKind::U64)));
}

#[test]
fn invokes_map_and_set_symbol_iterators_from_source() {
    let module = zun_compiler::compile(
            "import { mapIteratorNext, setIteratorNext } from \"zun:core\"; function main(): u64 { let map = new Map(); map.set(1.0, 10.0); let map_iterator = map[Symbol.iterator](); let pair: Array = mapIteratorNext(map_iterator) as Array; let set = new Set(); set.add(32.0); let set_iterator = set[Symbol.iterator](); let value = setIteratorNext(set_iterator); return pair[1] as u64 + value as u64; }",
        )
        .expect("compile collection Symbol.iterator calls");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run collection Symbol.iterator calls");
    assert!(matches!(result, Value::Int(42, zun_il::NumKind::U64)));
}

#[test]
fn symbol_values_are_unique_per_call() {
    // Two `Symbol("x")` are distinct keys: the first slot survives the second
    // write, so reading the first key still yields its own value.
    let module = zun_compiler::compile(
            "function main(): u64 { let a = Symbol(\"x\"); let b = Symbol(\"x\"); let obj = dynamic_new(); obj[a] = box_any(1); obj[b] = box_any(2); return obj[a] as u64; }",
        )
        .expect("compile unique symbols");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run unique symbols");
    assert!(matches!(result, Value::Int(1, zun_il::NumKind::U64)));
}

#[test]
fn symbol_description_is_readable() {
    let module =
        zun_compiler::compile("function main(): any { return Symbol(\"desc\").description; }")
            .expect("compile symbol description");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run symbol description");
    assert!(matches!(result, Value::Str(text) if &*text == "desc"));
}

#[test]
fn object_literal_builds_dynamic_object_with_dotted_access() {
    let module = zun_compiler::compile(
            "function main(): number { let x = { a: 1, b: 2 }; return (x.a as number) + (x.b as number); }",
        )
        .expect("compile object literal");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run object literal");
    assert!(matches!(result, Value::Float(value, _) if value == 3.0));
}

#[test]
fn object_literal_keys_preserve_insertion_order() {
    let module = zun_compiler::compile(
            "function main(): string { let ks = Object.keys({ a: 1, b: 2 }); return (ks[0] as string) + (ks[1] as string); }",
        )
        .expect("compile object literal keys");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run object literal keys");
    assert!(matches!(result, Value::Str(text) if &*text == "ab"));
}

#[test]
fn object_literal_shorthand_property() {
    let module = zun_compiler::compile(
        "function main(): number { let a = 5; let o = { a }; return o.a as number; }",
    )
    .expect("compile object shorthand");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run object shorthand");
    assert!(matches!(result, Value::Float(value, _) if value == 5.0));
}

#[test]
fn object_literal_computed_string_key() {
    let module = zun_compiler::compile(
        "function main(): number { let o = { [\"k\" + \"1\"]: 9 }; return o[\"k1\"] as number; }",
    )
    .expect("compile computed string key");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run computed string key");
    assert!(matches!(result, Value::Float(value, _) if value == 9.0));
}

#[test]
fn object_literal_computed_symbol_key_is_distinct_from_strings() {
    let module = zun_compiler::compile(
        "function main(): u64 { let s = Symbol(\"s\"); let o = { [s]: 7 }; return o[s] as u64; }",
    )
    .expect("compile computed symbol key");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run computed symbol key");
    assert!(matches!(result, Value::Int(7, zun_il::NumKind::U64)));
}

#[test]
fn object_literal_empty_and_nested() {
    let module = zun_compiler::compile(
        "function main(): number { let e = {}; let o = { a: { b: 1 } }; return o.a.b as number; }",
    )
    .expect("compile nested object literal");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run nested object literal");
    assert!(matches!(result, Value::Float(value, _) if value == 1.0));
}

#[test]
fn symbol_for_is_canonical_and_key_for_round_trips() {
    // `Symbol.for("k")` returns the same symbol, so the second lookup reads
    // the value stored under the first.
    let module = zun_compiler::compile(
            "function main(): u64 { let obj = dynamic_new(); obj[Symbol.for(\"k\")] = box_any(5); return obj[Symbol.for(\"k\")] as u64; }",
        )
        .expect("compile Symbol.for");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Symbol.for");
    assert!(matches!(result, Value::Int(5, zun_il::NumKind::U64)));
}

#[test]
fn symbol_key_for_returns_registry_key() {
    let module = zun_compiler::compile(
        "function main(): any { return Symbol.keyFor(Symbol.for(\"hello\")); }",
    )
    .expect("compile Symbol.keyFor");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Symbol.keyFor");
    assert!(matches!(result, Value::Str(text) if &*text == "hello"));
}

#[test]
fn well_known_symbols_have_stable_identity() {
    // `Symbol.iterator` is the same value every time it is evaluated, so a
    // slot keyed by it round-trips.
    let module = zun_compiler::compile(
            "function main(): u64 { let obj = dynamic_new(); obj[Symbol.iterator] = box_any(9); return obj[Symbol.iterator] as u64; }",
        )
        .expect("compile well-known symbol identity");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run well-known symbol identity");
    assert!(matches!(result, Value::Int(9, zun_il::NumKind::U64)));
}

#[test]
fn symbol_keys_are_distinct_from_string_keys() {
    // A string key and a same-named symbol key occupy separate slots.
    let module = zun_compiler::compile(
            "function main(): u64 { let obj = dynamic_new(); obj[\"x\"] = box_any(1); obj[Symbol.for(\"x\")] = box_any(2); return obj[\"x\"] as u64; }",
        )
        .expect("compile symbol vs string keys");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run symbol vs string keys");
    assert!(matches!(result, Value::Int(1, zun_il::NumKind::U64)));
}

#[test]
fn executes_switch_cases_and_default() {
    let module = zun_compiler::compile(
            "function main(): u64 { let value: u64 = 2; switch (value) { case 1: return 10; case 2: return 20; default: return 30; } }",
        )
        .expect("compile switch");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run switch");
    assert!(matches!(result, Value::Int(20, zun_il::NumKind::U64)));
}

#[test]
fn iterates_map_and_set_values_in_insertion_order() {
    let module = zun_compiler::compile(
            "function main(): any { let map = new Map(); map.set(1.0, 10.0); map.set(2.0, 20.0); let set = new Set(); set.add(3.0); set.add(4.0); for (let value of map) { let pair: Array = value as Array; print(pair[1]); } for (let value of set) { print(value); } return 42.0; }",
        )
        .expect("compile managed collection for-of");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run managed collection for-of");
    assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    assert_eq!(output.lines, vec!["10", "20", "3", "4"]);
}

#[test]
fn accesses_gc_backed_byte_buffers_from_source() {
    let module = zun_compiler::compile(
            "import { bufferNew, bufferSetU8, bufferGetU8, bufferLength } from \"zun:core\"; function main(): u64 { let buffer = bufferNew(4); bufferSetU8(buffer, 2, 127); if (bufferLength(buffer) == 4) { return bufferGetU8(buffer, 2) as u64; } return 0; }",
        )
        .expect("compile buffer intrinsics");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run buffer intrinsics");
    assert!(matches!(result, Value::Int(127, zun_il::NumKind::U64)));
}

#[test]
fn resizable_array_buffer_grows_with_zero_filled_tail() {
    let module = zun_compiler::compile(
            "function main(): u64 { let b = new ArrayBuffer(8, { maxByteLength: 16 }); b.resize(16); return b.byteLength() as u64; }",
        )
        .expect("compile resizable grow");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run resizable grow");
    assert!(matches!(result, Value::Int(16, zun_il::NumKind::U64)));
}

#[test]
fn resizable_array_buffer_reports_max_and_resizable() {
    let module = zun_compiler::compile(
            "function main(): u64 { let b = new ArrayBuffer(8, { maxByteLength: 16 }); if (b.resizable()) { return b.maxByteLength() as u64; } return 0; }",
        )
        .expect("compile max/resizable");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run max/resizable");
    assert!(matches!(result, Value::Int(16, zun_il::NumKind::U64)));
}

#[test]
fn fixed_array_buffer_is_not_resizable() {
    let module = zun_compiler::compile(
            "function main(): u64 { let b = new ArrayBuffer(8); if (b.resizable()) { return 1; } if (b.maxByteLength() == 8) { return 0; } return 2; }",
        )
        .expect("compile fixed buffer");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run fixed buffer");
    assert!(matches!(result, Value::Int(0, zun_il::NumKind::U64)));
}

#[test]
fn resize_beyond_max_is_rejected() {
    let module = zun_compiler::compile(
            "function main(): u64 { let b = new ArrayBuffer(8, { maxByteLength: 16 }); b.resize(32); return 0; }",
        )
        .expect("compile over-max resize");
    let mut output = CaptureOutput::default();
    assert!(
        run(&module, &mut output).is_err(),
        "resize beyond max must error"
    );
}

#[test]
fn length_tracking_typed_array_reflects_resize() {
    let module = zun_compiler::compile(
            "function main(): u64 { let b = new ArrayBuffer(4, { maxByteLength: 16 }); let arr = new Uint8Array(b); b.resize(12); return arr.length() as u64; }",
        )
        .expect("compile length-tracking");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run length-tracking");
    assert!(matches!(result, Value::Int(12, zun_il::NumKind::U64)));
}

#[test]
fn fixed_view_that_no_longer_fits_becomes_out_of_bounds() {
    let module = zun_compiler::compile(
            "function main(): u64 { let b = new ArrayBuffer(16, { maxByteLength: 16 }); let arr = new Uint8Array(b, 0, 16); b.resize(4); return arr.length() as u64; }",
        )
        .expect("compile OOB view");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run OOB view");
    assert!(matches!(result, Value::Int(0, zun_il::NumKind::U64)));
}

#[test]
fn resize_preserves_data_and_zero_fills_grown_tail() {
    let module = zun_compiler::compile(
            "function main(): u64 { let b = new ArrayBuffer(4, { maxByteLength: 8 }); let arr = new Uint8Array(b); arr.set(0, 99); b.resize(8); return (arr.get(0) as u64) + (arr.get(7) as u64); }",
        )
        .expect("compile preserve/zero-fill");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run preserve/zero-fill");
    assert!(matches!(result, Value::Int(99, zun_il::NumKind::U64)));
}

#[test]
fn accesses_numeric_values_in_gc_backed_buffers_from_source() {
    let module = zun_compiler::compile(
            "import { bufferNew, bufferWriteU32, bufferReadU32, bufferWriteF64, bufferReadF64 } from \"zun:core\"; function main(): number { let buffer = bufferNew(16); bufferWriteU32(buffer, 0, 16909060); bufferWriteF64(buffer, 8, 2.5); if (bufferReadU32(buffer, 0) == 16909060) { return bufferReadF64(buffer, 8); } return 0.0; }",
        )
        .expect("compile numeric buffer intrinsics");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run numeric buffer intrinsics");
    assert!(matches!(result, Value::Float(2.5, zun_il::NumKind::F64)));
}

#[test]
fn accesses_data_views_over_gc_backed_buffers_from_source() {
    let module = zun_compiler::compile(
            "import { bufferNew, dataViewNew, dataViewSetU32, dataViewGetU32 } from \"zun:core\"; function main(): u64 { let buffer = bufferNew(8); let view = dataViewNew(buffer, 2, 4); dataViewSetU32(view, 0, 16909060, true); return dataViewGetU32(view, 0, true) as u64; }",
        )
        .expect("compile DataView intrinsics");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run DataView intrinsics");
    assert!(matches!(result, Value::Int(16909060, zun_il::NumKind::U64)));
}

#[test]
fn accesses_data_view_u16_and_f64_operations_from_source() {
    let module = zun_compiler::compile(
            "import { bufferNew, dataViewNew, dataViewSetU16, dataViewGetU16, dataViewSetF64, dataViewGetF64 } from \"zun:core\"; function main(): number { let buffer = bufferNew(16); let view = dataViewNew(buffer, 0, 16); dataViewSetU16(view, 0, 4660, false); dataViewSetF64(view, 8, 2.5, true); if (dataViewGetU16(view, 0, false) == 4660) { return dataViewGetF64(view, 8, true); } return 0.0; }",
        )
        .expect("compile extended DataView intrinsics");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run extended DataView intrinsics");
    assert!(matches!(result, Value::Float(2.5, zun_il::NumKind::F64)));
}

#[test]
fn accesses_gc_backed_u32_typed_arrays_from_source() {
    let module = zun_compiler::compile(
            "import { bufferNew, u32ArrayNew, u32ArraySet, u32ArrayGet } from \"zun:core\"; function main(): u64 { let buffer = bufferNew(16); let values = u32ArrayNew(buffer, 0, 4); u32ArraySet(values, 2, 16909060); return u32ArrayGet(values, 2) as u64; }",
        )
        .expect("compile typed-array intrinsics");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run typed-array intrinsics");
    assert!(matches!(result, Value::Int(16909060, zun_il::NumKind::U64)));
}

#[test]
fn accesses_gc_backed_u8_typed_arrays_from_source() {
    let module = zun_compiler::compile(
            "import { bufferNew, u8ArrayNew, u8ArraySet, u8ArrayGet } from \"zun:core\"; function main(): u64 { let buffer = bufferNew(4); let values = u8ArrayNew(buffer, 0, 4); u8ArraySet(values, 1, 255); return u8ArrayGet(values, 1) as u64; }",
        )
        .expect("compile Uint8Array intrinsics");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Uint8Array intrinsics");
    assert!(matches!(result, Value::Int(255, zun_il::NumKind::U64)));
}

#[test]
fn accesses_gc_backed_f64_typed_arrays_from_source() {
    let module = zun_compiler::compile(
            "import { bufferNew, f64ArrayNew, f64ArraySet, f64ArrayGet } from \"zun:core\"; function main(): number { let buffer = bufferNew(16); let values = f64ArrayNew(buffer, 0, 2); f64ArraySet(values, 1, 2.5); return f64ArrayGet(values, 1); }",
        )
        .expect("compile Float64Array intrinsics");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Float64Array intrinsics");
    assert!(matches!(result, Value::Float(2.5, zun_il::NumKind::F64)));
}

#[test]
fn accesses_gc_backed_u16_typed_arrays_from_source() {
    let module = zun_compiler::compile(
            "import { bufferNew, u16ArrayNew, u16ArraySet, u16ArrayGet } from \"zun:core\"; function main(): u64 { let buffer = bufferNew(8); let values = u16ArrayNew(buffer, 0, 4); u16ArraySet(values, 2, 4660); return u16ArrayGet(values, 2) as u64; }",
        )
        .expect("compile Uint16Array intrinsics");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Uint16Array intrinsics");
    assert!(matches!(result, Value::Int(4660, zun_il::NumKind::U64)));
}

#[test]
fn constructs_typed_array_views_from_source() {
    let module = zun_compiler::compile(
            "import { bufferNew, u8ArraySet, u8ArrayGet } from \"zun:core\"; function main(): u64 { let buffer = bufferNew(4); let values = new Uint8Array(buffer, 0, 4); u8ArraySet(values, 1, 255); return u8ArrayGet(values, 1) as u64; }",
        )
        .expect("compile typed-array constructor");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run typed-array constructor");
    assert!(matches!(result, Value::Int(255, zun_il::NumKind::U64)));
}

#[test]
fn typed_array_views_expose_instance_accessors() {
    let module = zun_compiler::compile(
            "import { bufferNew } from \"zun:core\"; function main(): u64 { let buffer = bufferNew(4); let values = new Uint8Array(buffer, 0, 4); values.set(1, 255); return values.get(1) as u64; }",
        )
        .expect("compile typed-array instance accessors");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run typed-array instance accessors");
    assert!(matches!(result, Value::Int(255, zun_il::NumKind::U64)));
}

#[test]
fn constructs_data_views_from_source() {
    let module = zun_compiler::compile(
            "import { bufferNew, dataViewSetU32, dataViewGetU32 } from \"zun:core\"; function main(): u64 { let buffer = bufferNew(8); let view = new DataView(buffer, 0, 8); dataViewSetU32(view, 0, 16909060, true); return dataViewGetU32(view, 0, true) as u64; }",
        )
        .expect("compile DataView constructor");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run DataView constructor");
    assert!(matches!(result, Value::Int(16909060, zun_il::NumKind::U64)));
}

#[test]
fn data_views_expose_instance_accessors() {
    let module = zun_compiler::compile(
            "import { bufferNew } from \"zun:core\"; function main(): u64 { let buffer = bufferNew(8); let view = new DataView(buffer, 0, 8); view.setUint32(0, 16909060, true); return view.getUint32(0, true) as u64; }",
        )
        .expect("compile DataView instance accessors");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run DataView instance accessors");
    assert!(matches!(result, Value::Int(16909060, zun_il::NumKind::U64)));
}

#[test]
fn constructs_array_buffers_from_source() {
    let module = zun_compiler::compile(
            "import { bufferLength } from \"zun:core\"; function main(): u64 { let buffer = new ArrayBuffer(4); return bufferLength(buffer); }",
        )
        .expect("compile ArrayBuffer constructor");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run ArrayBuffer constructor");
    assert!(matches!(result, Value::Int(4, zun_il::NumKind::U64)));
}

#[test]
fn exposes_array_iterator_handles_to_source() {
    let module = zun_compiler::compile(
            "import { arrayIteratorNew, arrayIteratorNext } from \"zun:core\"; function main(): any { let values = new Array(2); values[0] = 7.0; values[1] = 9.0; let iterator = arrayIteratorNew(values); print(arrayIteratorNext(iterator)); print(arrayIteratorNext(iterator)); return arrayIteratorNext(iterator); }",
        )
        .expect("compile array iterator intrinsics");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run array iterator intrinsics");
    assert!(matches!(result, Value::Void));
    assert_eq!(output.lines, vec!["7", "9"]);
}

#[test]
fn exposes_set_iterator_handles_to_source() {
    let module = zun_compiler::compile(
            "import { setIteratorNew, setIteratorNext } from \"zun:core\"; function main(): any { let values = new Set(); values.add(7.0); values.add(9.0); let iterator = setIteratorNew(values); print(setIteratorNext(iterator)); print(setIteratorNext(iterator)); return setIteratorNext(iterator); }",
        )
        .expect("compile set iterator intrinsics");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run set iterator intrinsics");
    assert!(matches!(result, Value::Void));
    assert_eq!(output.lines, vec!["7", "9"]);
}

#[test]
fn exposes_map_iterator_entry_pairs_to_source() {
    let module = zun_compiler::compile(
            "import { mapIteratorNew, mapIteratorNext } from \"zun:core\"; function main(): any { let values = new Map(); values.set(7.0, 70.0); values.set(9.0, 90.0); let iterator = mapIteratorNew(values); print(mapIteratorNext(iterator)); return mapIteratorNext(iterator); }",
        )
        .expect("compile map iterator intrinsics");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run map iterator intrinsics");
    assert!(matches!(result, Value::Object(_)));
    assert_eq!(output.lines, vec!["[any:Object]"]);
}

#[test]
fn uses_runtime_backed_map_and_set_intrinsics() {
    let module = zun_compiler::compile(
            "import { mapNew, mapSet, mapHas, mapSize, setNew, setAdd, setHas, setSize } from \"zun:core\"; function main(): any { let map = mapNew(); mapSet(map, 7.0, 42.0); let set = setNew(); setAdd(set, 7.0); if (mapHas(map, 7.0)) { if (setHas(set, 7.0)) { return mapSize(map) + setSize(set); } } return 0.0; }",
        )
        .expect("compile collection intrinsics");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run collection intrinsics");
    assert!(matches!(result, Value::Int(2, _)));
}

#[test]
fn constructs_managed_collections_from_source() {
    let module = zun_compiler::compile(
            "import { mapSet, mapHas, setAdd, setHas, weakMapSet, weakMapHas } from \"zun:core\"; class Key {} function main(): int { let map = new Map(); mapSet(map, 1.0, 2.0); let set = new Set(); setAdd(set, 1.0); let key = new Key(); let value = new Key(); let weak = new WeakMap(); weakMapSet(weak, key, value); if (mapHas(map, 1.0)) { if (setHas(set, 1.0)) { if (weakMapHas(weak, key)) { return 1; } } } return 0; }",
        )
        .expect("compile managed collection constructors");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run managed collection constructors");
    assert!(matches!(result, Value::Int(1, _)));
}

#[test]
fn calls_managed_collection_methods_with_a_receiver() {
    let module = zun_compiler::compile(
            "class Key {} function main(): int { let map = new Map(); map.set(1.0, 2.0); let set = new Set(); set.add(1.0); let key = new Key(); let value = new Key(); let weak = new WeakMap(); weak.set(key, value); if (map.has(1.0)) { if (set.has(1.0)) { if (weak.has(key)) { if (map.size() == 1) { if (set.size() == 1) { map.clear(); set.clear(); return 1; } } } } } return 0; }",
        )
        .expect("compile managed collection methods");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run managed collection methods");
    assert!(matches!(result, Value::Int(1, _)));
}

#[test]
fn removes_and_clears_runtime_backed_collections() {
    let module = zun_compiler::compile(
            "import { mapNew, mapSet, mapHas, mapDelete, mapClear, setNew, setAdd, setHas, setDelete, setClear } from \"zun:core\"; function main(): int { let map = mapNew(); mapSet(map, 1.0, 2.0); mapDelete(map, 1.0); if (mapHas(map, 1.0)) { return 1; } mapSet(map, 1.0, 2.0); mapClear(map); let set = setNew(); setAdd(set, 1.0); setDelete(set, 1.0); if (setHas(set, 1.0)) { return 2; } setAdd(set, 1.0); setClear(set); if (setHas(set, 1.0)) { return 3; } return 0; }",
        )
        .expect("compile collection removal intrinsics");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run collection removal intrinsics");
    assert!(matches!(result, Value::Int(0, _)));
}

#[test]
fn uses_gc_backed_weak_map_handles() {
    let module = zun_compiler::compile(
            "import { weakMapNew, weakMapSet, weakMapHas } from \"zun:core\"; class Key {} function main(): int { let key = new Key(); let value = new Key(); let key2 = new Key(); let value2 = new Key(); let map = weakMapNew(); weakMapSet(map, key, value); weakMapSet(map, key2, value2); if (weakMapHas(map, key)) { if (weakMapHas(map, key2)) { return 1; } } return 0; }",
        )
        .expect("compile weak map intrinsics");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run weak map intrinsics");
    assert!(matches!(result, Value::Int(1, _)));
}

#[test]
fn executes_dynamic_property_il() {
    let mut module = zun_module::Module::new();
    let property = module.strings.intern("answer");
    let mut code = Vec::new();
    for instruction in [
        Instruction::simple(Opcode::NewDynamic),
        Instruction::simple(Opcode::Dup),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(41)),
        Instruction::simple(Opcode::BoxAny),
        Instruction::new(Opcode::StoreDynamic, Operand::StringRef(property.0)),
        Instruction::simple(Opcode::Pop),
        Instruction::new(Opcode::LoadDynamic, Operand::StringRef(property.0)),
        Instruction::simple(Opcode::UnboxAny),
        Instruction::simple(Opcode::Ret),
    ] {
        encode(&instruction, &mut code);
    }
    let offset = write_body(
        &mut module.code,
        0,
        0,
        true,
        zun_il::JitKind::Num(zun_il::NumKind::I32).to_byte(),
        &[],
        &code,
    );
    module.tables.add_method_def(MethodDef {
        name: module.strings.intern("main"),
        signature: zun_metadata::BlobIndex(0),
        body_offset: offset,
        is_static: false,
    });
    module.entry_point = Some(Token::new(TableKind::MethodDef, 1));

    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("dynamic property IL");
    assert!(matches!(result, Value::Int(41, zun_il::NumKind::I64)));
}

#[test]
fn executes_dynamic_property_source() {
    let module = zun_compiler::compile(
            "function main(): any { let object = dynamic_new(); dynamic_set(object, \"answer\", box_any(41)); return dynamic_get(object, \"answer\"); }",
        )
        .expect("compile dynamic source");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run dynamic source");
    assert!(matches!(result, Value::Any(any) if any.as_float() == Some(41.0)));
}

#[test]
fn executes_extern_method_with_receiver() {
    let module = zun_compiler::compile(
            "import { receiverValue } from \"zun:core\";\n\
             class Service { extern receiverValue(value: number): number; }\n\
             function main(): number { let service = new Service(); return service.receiverValue(7.5); }",
        )
        .expect("compile extern method");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run extern method");
    assert!(matches!(result, Value::Float(value, _) if value == 7.5));
}

#[test]
fn executes_array_il_with_bounds_checked_storage() {
    let mut module = zun_module::Module::new();
    let mut code = Vec::new();
    for instruction in [
        Instruction::new(Opcode::ConstI32, Operand::Imm32(1)),
        Instruction::simple(Opcode::NewArray),
        Instruction::simple(Opcode::Dup),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(41)),
        Instruction::simple(Opcode::StoreArray),
        Instruction::simple(Opcode::Pop),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
        Instruction::simple(Opcode::LoadArray),
        Instruction::simple(Opcode::Ret),
    ] {
        encode(&instruction, &mut code);
    }
    let offset = write_body(
        &mut module.code,
        0,
        0,
        true,
        zun_il::JitKind::Num(zun_il::NumKind::I32).to_byte(),
        &[],
        &code,
    );
    module.tables.add_method_def(MethodDef {
        name: module.strings.intern("main"),
        signature: zun_metadata::BlobIndex(0),
        body_offset: offset,
        is_static: false,
    });
    module.entry_point = Some(Token::new(TableKind::MethodDef, 1));
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run array IL");
    assert!(matches!(result, Value::Int(41, zun_il::NumKind::I64)));
}

#[test]
fn executes_source_array_constructor() {
    let module = zun_compiler::compile("function main(): Array { return new Array(2); }")
        .expect("compile array constructor");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run array constructor");
    assert!(matches!(result, Value::Object(_)));
}

#[test]
fn executes_source_array_index_read() {
    let module = zun_compiler::compile(
        "function main(): any { let values = new Array(1); return values[0]; }",
    )
    .expect("compile array index");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run array index");
    // A fresh `new Array(1)` slot is a hole, which reads as `undefined`.
    assert!(
        matches!(result, Value::Void)
            || matches!(result, Value::Any(any) if any.raw_parts() == zun_core::Any::void().raw_parts() || any.is_hole())
    );
}

#[test]
fn executes_source_array_index_assignment() {
    let module = zun_compiler::compile(
        "function main(): any { let values = new Array(1); values[0] = 41; return values[0]; }",
    )
    .expect("compile array assignment");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run array assignment");
    assert!(
        matches!(result, Value::Float(41.0, zun_il::NumKind::F64)),
        "{result:?}"
    );
}

#[test]
fn executes_array_count_source_method() {
    let module = zun_compiler::compile(
            "class Array { count(value: any): u64 { let total: u64 = 0; let index: u64 = 0; while (index < this.length) { if (this[index] == value) { total = total + 1; } index = index + 1; } return total; } }\n\
             function main(): u64 { let values = new Array(4); values[0] = 7; values[1] = 3; values[2] = 7; values[3] = 7; let needle: any = values[0]; return values.count(needle); }",
        )
        .expect("compile Array count");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array count");
    assert!(matches!(result, Value::Int(value, _) if value == 3));
}

#[test]
fn executes_array_clear_source_method() {
    let module = zun_compiler::compile(
            "class Array { clear(): void { while (this.length > 0) { this.pop(); } } }\n\
             function main(): u64 { let values = new Array(2); values[0] = 7; values[1] = 3; values.clear(); return values.length; }",
        )
        .expect("compile Array clear");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array clear");
    assert!(matches!(result, Value::Int(value, _) if value == 0));
}

#[test]
fn executes_array_average_source_method() {
    let module = zun_compiler::compile(
            "class Array { sum(): number { let total: number = 0.0; let index: u64 = 0; while (index < this.length) { total = total + (this[index] as number); index = index + 1; } return total; } average(): number { if (this.length == 0) { return 0.0; } return this.sum() / (this.length as number); } }\n\
             function main(): number { let values = new Array(2); values[0] = 2.0; values[1] = 6.0; return values.average(); }",
        )
        .expect("compile Array average");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array average");
    assert!(matches!(result, Value::Float(value, _) if value == 4.0));
}

#[test]
fn executes_array_min_and_max_source_methods() {
    let module = zun_compiler::compile(
            "class Array { min(): number { if (this.length == 0) { return 0.0; } let result: number = this[0] as number; let index: u64 = 1; while (index < this.length) { let value: number = this[index] as number; if (value < result) { result = value; } index = index + 1; } return result; } max(): number { if (this.length == 0) { return 0.0; } let result: number = this[0] as number; let index: u64 = 1; while (index < this.length) { let value: number = this[index] as number; if (value > result) { result = value; } index = index + 1; } return result; } }\n\
             function main(): number { let values = new Array(3); values[0] = 9.0; values[1] = 2.0; values[2] = 7.0; return values.min() + values.max(); }",
        )
        .expect("compile Array min and max");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array min and max");
    assert!(matches!(result, Value::Float(value, _) if value == 11.0));
}

#[test]
fn executes_array_sort_source_method() {
    let module = zun_compiler::compile(
            "class Array { sort(): Array { let index: u64 = 0; while (index < this.length) { let smallest: u64 = index; let scan: u64 = index + 1; while (scan < this.length) { let candidate: number = this[scan] as number; let currentSmallest: number = this[smallest] as number; if (candidate < currentSmallest) { smallest = scan; } scan = scan + 1; } let temporary: any = this[index]; this[index] = this[smallest]; this[smallest] = temporary; index = index + 1; } return this; } }\n\
             function main(): number { let values = new Array(3); values[0] = 9.0; values[1] = 2.0; values[2] = 7.0; values.sort(); return values[0] as number; }",
        )
        .expect("compile Array sort");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array sort");
    assert!(matches!(result, Value::Float(value, _) if value == 2.0));
}

#[test]
fn executes_array_find_index_source_method() {
    let module = zun_compiler::compile(
            // `findIndex` takes an ECMAScript predicate `(value, index, array)`
            // now (previously a value); the native invokes the callback, so the
            // entry stays on the Tier 0 interpreter.
            "function is_eight(value: any, index: any, array: any): boolean { return (value as number) == 8.0; }\n\
             function main(): int { let values = new Array(3); values[0] = 4.0; values[1] = 8.0; values[2] = 15.0; return values.findIndex(is_eight); }",
        )
        .expect("compile Array findIndex");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array findIndex");
    assert!(matches!(result, Value::Int(value, _) if value == 1));
}

#[test]
fn executes_array_last_index_of_source_method() {
    let module = zun_compiler::compile(
            "class Array { lastIndexOf(value: any): int { let index: int = this.length as int; while (index > 0) { index = index - 1; if (this[index as u64] == value) { return index; } } return -1; } }\n\
             function main(): int { let values = new Array(3); values[0] = 4; values[1] = 8; values[2] = 8; let needle: any = values[1]; return values.lastIndexOf(needle); }",
        )
        .expect("compile Array lastIndexOf");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array lastIndexOf");
    assert!(matches!(result, Value::Int(value, _) if value == 2));
}

#[test]
fn executes_array_all_source_method() {
    let module = zun_compiler::compile(
            "class Array { all(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] != value) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 4.0; values[1] = 4.0; values[2] = 4.0; let needle: any = values[0]; return values.all(needle); }",
        )
        .expect("compile Array all");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array all");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_array_none_source_method() {
    let module = zun_compiler::compile(
            "class Array { none(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; let needle: any = values[0]; return values.none(needle); }",
        )
        .expect("compile Array none");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array none");
    assert!(matches!(result, Value::Bool(false)));
}

#[test]
fn executes_array_any_source_method() {
    let module = zun_compiler::compile(
            "class Array { any(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; let needle: any = values[1]; return values.any(needle); }",
        )
        .expect("compile Array any");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array any");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_array_is_constant_source_method() {
    let module = zun_compiler::compile(
            "class Array { isConstant(): boolean { if (this.length < 2) { return true; } let first: any = this[0]; let index: u64 = 1; while (index < this.length) { if (this[index] != first) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 9.0; values[1] = 9.0; values[2] = 9.0; return values.isConstant(); }",
        )
        .expect("compile Array isConstant");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array isConstant");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_array_is_palindrome_source_method() {
    let module = zun_compiler::compile(
            "class Array { isPalindrome(): boolean { if (this.length < 2) { return true; } let left: u64 = 0; let right: u64 = this.length - 1; while (left < right) { if (this[left] != this[right]) { return false; } left = left + 1; right = right - 1; } return true; } } function main(): boolean { let values = new Array(5); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values[3] = 2.0; values[4] = 1.0; return values.isPalindrome(); }",
        )
        .expect("compile Array isPalindrome");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array isPalindrome");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_array_is_disjoint_source_method() {
    let module = zun_compiler::compile(
            "class Array { isDisjoint(other: Array): boolean { let left: u64 = 0; while (left < this.length) { let right: u64 = 0; while (right < other.length) { if (this[left] == other[right]) { return false; } right = right + 1; } left = left + 1; } return true; } } function main(): boolean { let left = new Array(2); left[0] = 1.0; left[1] = 2.0; let right = new Array(2); right[0] = 3.0; right[1] = 4.0; return left.isDisjoint(right); }",
        )
        .expect("compile Array isDisjoint");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array isDisjoint");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_array_intersection_source_method() {
    let module = zun_compiler::compile(
            "class Array { intersection(other: Array): Array { let matches: u64 = 0; let left: u64 = 0; while (left < this.length) { let right: u64 = 0; while (right < other.length) { if (this[left] == other[right]) { matches = matches + 1; break; } right = right + 1; } left = left + 1; } let result = new Array(matches); let output: u64 = 0; left = 0; while (left < this.length) { let right: u64 = 0; while (right < other.length) { if (this[left] == other[right]) { result[output] = this[left]; output = output + 1; break; } right = right + 1; } left = left + 1; } return result; } } function main(): number { let left = new Array(3); left[0] = 1.0; left[1] = 2.0; left[2] = 3.0; let right = new Array(2); right[0] = 2.0; right[1] = 4.0; let common = left.intersection(right); return common[0] as number; }",
        )
        .expect("compile Array intersection");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array intersection");
    assert!(matches!(result, Value::Float(2.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_array_difference_source_method() {
    let module = zun_compiler::compile(
            "class Array { difference(other: Array): Array { let remaining: u64 = 0; let left: u64 = 0; while (left < this.length) { let right: u64 = 0; let found: boolean = false; while (right < other.length) { if (this[left] == other[right]) { found = true; break; } right = right + 1; } if (!found) { remaining = remaining + 1; } left = left + 1; } let result = new Array(remaining); let output: u64 = 0; left = 0; while (left < this.length) { let right: u64 = 0; let found: boolean = false; while (right < other.length) { if (this[left] == other[right]) { found = true; break; } right = right + 1; } if (!found) { result[output] = this[left]; output = output + 1; } left = left + 1; } return result; } } function main(): number { let left = new Array(3); left[0] = 1.0; left[1] = 2.0; left[2] = 3.0; let right = new Array(2); right[0] = 2.0; right[1] = 4.0; let remaining = left.difference(right); return remaining[1] as number; }",
        )
        .expect("compile Array difference");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array difference");
    assert!(matches!(result, Value::Float(3.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_array_union_source_method() {
    let module = zun_compiler::compile(
            "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } union(other: Array): Array { let total: u64 = this.length; let right: u64 = 0; while (right < other.length) { if (!this.contains(other[right])) { total = total + 1; } right = right + 1; } let result = new Array(total); let index: u64 = 0; while (index < this.length) { result[index] = this[index]; index = index + 1; } right = 0; while (right < other.length) { if (!this.contains(other[right])) { result[index] = other[right]; index = index + 1; } right = right + 1; } return result; } } function main(): number { let left = new Array(2); left[0] = 1.0; left[1] = 2.0; let right = new Array(2); right[0] = 2.0; right[1] = 3.0; let combined = left.union(right); return combined[2] as number; }",
        )
        .expect("compile Array union");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array union");
    assert!(matches!(result, Value::Float(3.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_array_symmetric_difference_source_method() {
    let module = zun_compiler::compile(
            "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } difference(other: Array): Array { let remaining: u64 = 0; let left: u64 = 0; while (left < this.length) { if (!other.contains(this[left])) { remaining = remaining + 1; } left = left + 1; } let result = new Array(remaining); let output: u64 = 0; left = 0; while (left < this.length) { if (!other.contains(this[left])) { result[output] = this[left]; output = output + 1; } left = left + 1; } return result; } union(other: Array): Array { let total: u64 = this.length; let right: u64 = 0; while (right < other.length) { if (!this.contains(other[right])) { total = total + 1; } right = right + 1; } let result = new Array(total); let index: u64 = 0; while (index < this.length) { result[index] = this[index]; index = index + 1; } right = 0; while (right < other.length) { if (!this.contains(other[right])) { result[index] = other[right]; index = index + 1; } right = right + 1; } return result; } symmetricDifference(other: Array): Array { let leftOnly = this.difference(other); let rightOnly = other.difference(this); return leftOnly.union(rightOnly); } } function main(): number { let left = new Array(2); left[0] = 1.0; left[1] = 2.0; let right = new Array(2); right[0] = 2.0; right[1] = 3.0; let result = left.symmetricDifference(right); return result[1] as number; }",
        )
        .expect("compile Array symmetricDifference");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array symmetricDifference");
    assert!(matches!(result, Value::Float(3.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_array_is_subset_of_source_method() {
    let module = zun_compiler::compile(
            "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } isSubsetOf(other: Array): boolean { let index: u64 = 0; while (index < this.length) { if (!other.contains(this[index])) { return false; } index = index + 1; } return true; } } function main(): boolean { let subset = new Array(2); subset[0] = 1.0; subset[1] = 2.0; let full = new Array(3); full[0] = 1.0; full[1] = 2.0; full[2] = 3.0; return subset.isSubsetOf(full); }",
        )
        .expect("compile Array isSubsetOf");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array isSubsetOf");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_array_is_superset_of_source_method() {
    let module = zun_compiler::compile(
            "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } isSubsetOf(other: Array): boolean { let index: u64 = 0; while (index < this.length) { if (!other.contains(this[index])) { return false; } index = index + 1; } return true; } isSupersetOf(other: Array): boolean { return other.isSubsetOf(this); } } function main(): boolean { let subset = new Array(2); subset[0] = 1.0; subset[1] = 2.0; let full = new Array(3); full[0] = 1.0; full[1] = 2.0; full[2] = 3.0; return full.isSupersetOf(subset); }",
        )
        .expect("compile Array isSupersetOf");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array isSupersetOf");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_array_swap_source_method() {
    let module = zun_compiler::compile(
            "class Array { swap(first: u64, second: u64): Array { let temporary: any = this[first]; this[first] = this[second]; this[second] = temporary; return this; } }\n\
             function main(): number { let values = new Array(2); values[0] = 4.0; values[1] = 8.0; values.swap(0, 1); return values[0] as number; }",
        )
        .expect("compile Array swap");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array swap");
    assert!(matches!(result, Value::Float(value, _) if value == 8.0));
}

#[test]
fn executes_array_rotate_left_source_method() {
    let module = zun_compiler::compile(
            "class Array { rotateLeft(shift: u64): Array { if (this.length == 0) { return this; } let amount: u64 = shift % this.length; let rotation: u64 = 0; while (rotation < amount) { let first: any = this[0]; let index: u64 = 1; while (index < this.length) { this[index - 1] = this[index]; index = index + 1; } this[this.length - 1] = first; rotation = rotation + 1; } return this; } }\n\
             function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values.rotateLeft(1); return values[0] as number; }",
        )
        .expect("compile Array rotateLeft");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array rotateLeft");
    assert!(matches!(result, Value::Float(value, _) if value == 2.0));
}

#[test]
fn executes_array_rotate_right_source_method() {
    let module = zun_compiler::compile(
            "class Array { rotateRight(shift: u64): Array { if (this.length == 0) { return this; } let amount: u64 = shift % this.length; let rotation: u64 = 0; while (rotation < amount) { let last: any = this[this.length - 1]; let index: u64 = this.length - 1; while (index > 0) { this[index] = this[index - 1]; index = index - 1; } this[0] = last; rotation = rotation + 1; } return this; } }\n\
             function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values.rotateRight(1); return values[0] as number; }",
        )
        .expect("compile Array rotateRight");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array rotateRight");
    assert!(matches!(result, Value::Float(value, _) if value == 3.0));
}

#[test]
fn executes_array_take_and_drop_source_methods() {
    let module = zun_compiler::compile(
            "class Array { slice(start: u64, end: u64): Array { let count: u64 = end - start; let result = new Array(count); let index: u64 = 0; while (index < count) { result[index] = this[start + index]; index = index + 1; } return result; } take(count: u64): Array { let limit: u64 = count; if (limit > this.length) { limit = this.length; } return this.slice(0, limit); } drop(count: u64): Array { let start: u64 = count; if (start > this.length) { start = this.length; } return this.slice(start, this.length); } } function main(): number { let values = new Array(3); values[0] = 10.0; values[1] = 20.0; values[2] = 30.0; let prefix = values.take(2); let suffix = values.drop(1); return prefix[1] as number + suffix[0] as number; }",
        )
        .expect("compile Array take/drop");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array take/drop");
    assert!(matches!(result, Value::Float(value, _) if value == 40.0));
}

#[test]
fn executes_array_remove_at_source_method() {
    let module = zun_compiler::compile(
            "class Array { removeAt(index: u64): any { let removed: any = this[index]; let cursor: u64 = index; while (cursor + 1 < this.length) { this[cursor] = this[cursor + 1]; cursor = cursor + 1; } this.pop(); return removed; } } function main(): number { let values = new Array(3); values[0] = 10.0; values[1] = 20.0; values[2] = 30.0; let removed: any = values.removeAt(1); return (removed as number) + (values[1] as number); }",
        )
        .expect("compile Array removeAt");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array removeAt");
    assert!(matches!(result, Value::Float(value, _) if value == 50.0));
}

#[test]
fn executes_array_insert_at_source_method() {
    let module = zun_compiler::compile(
            "class Array { insertAt(index: u64, value: any): Array { let cursor: u64 = this.length; this.push(value); while (cursor > index) { this[cursor] = this[cursor - 1]; cursor = cursor - 1; } this[index] = value; return this; } } function main(): number { let values = new Array(2); values[0] = 10.0; values[1] = 30.0; values.insertAt(1, values[0]); return (values[1] as number) + (values[2] as number); }",
        )
        .expect("compile Array insertAt");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array insertAt");
    assert!(matches!(result, Value::Float(value, _) if value == 40.0));
}

#[test]
fn executes_array_remove_first_source_method() {
    let module = zun_compiler::compile(
            "class Array { removeAt(index: u64): any { let removed: any = this[index]; let cursor: u64 = index; while (cursor + 1 < this.length) { this[cursor] = this[cursor + 1]; cursor = cursor + 1; } this.pop(); return removed; } removeFirst(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { this.removeAt(index); return true; } index = index + 1; } return false; } } function main(): number { let values = new Array(3); values[0] = 10.0; values[1] = 20.0; values[2] = 30.0; let needle: any = values[1]; if (!values.removeFirst(needle)) { return -1.0; } return values[1] as number; }",
        )
        .expect("compile Array removeFirst");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array removeFirst");
    assert!(matches!(result, Value::Float(value, _) if value == 30.0));
}

#[test]
fn executes_array_remove_all_source_method() {
    let module = zun_compiler::compile(
            "class Array { removeAt(index: u64): any { let removed: any = this[index]; let cursor: u64 = index; while (cursor + 1 < this.length) { this[cursor] = this[cursor + 1]; cursor = cursor + 1; } this.pop(); return removed; } removeAll(value: any): u64 { let removed: u64 = 0; let index: u64 = 0; while (index < this.length) { if (this[index] == value) { this.removeAt(index); removed = removed + 1; } else { index = index + 1; } } return removed; } } function main(): number { let values = new Array(4); values[0] = 7.0; values[1] = 3.0; values[2] = 7.0; values[3] = 5.0; let needle: any = values[0]; return values.removeAll(needle) as number; }",
        )
        .expect("compile Array removeAll");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array removeAll");
    assert!(matches!(result, Value::Float(value, _) if value == 2.0));
}

#[test]
fn executes_array_equals_source_method() {
    let module = zun_compiler::compile(
            "class Array { equals(other: Array): boolean { if (this.length != other.length) { return false; } let index: u64 = 0; while (index < this.length) { if (this[index] != other[index]) { return false; } index = index + 1; } return true; } } function main(): boolean { let left = new Array(2); left[0] = 4.0; left[1] = 8.0; let right = new Array(2); right[0] = 4.0; right[1] = 8.0; return left.equals(right); }",
        )
        .expect("compile Array equals");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array equals");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_array_is_sorted_source_method() {
    let module = zun_compiler::compile(
            "class Array { isSorted(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current < previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 2.0; return values.isSorted(); }",
        )
        .expect("compile Array isSorted");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array isSorted");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_array_is_strictly_increasing_source_method() {
    let module = zun_compiler::compile(
            "class Array { isStrictlyIncreasing(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current <= previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; return values.isStrictlyIncreasing(); }",
        )
        .expect("compile Array isStrictlyIncreasing");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array isStrictlyIncreasing");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_array_is_strictly_decreasing_source_method() {
    let module = zun_compiler::compile(
            "class Array { isStrictlyDecreasing(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current >= previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 3.0; values[1] = 2.0; values[2] = 1.0; return values.isStrictlyDecreasing(); }",
        )
        .expect("compile Array isStrictlyDecreasing");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array isStrictlyDecreasing");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_array_is_descending_source_method() {
    let module = zun_compiler::compile(
            "class Array { isDescending(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current > previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 3.0; values[1] = 2.0; values[2] = 2.0; return values.isDescending(); }",
        )
        .expect("compile Array isDescending");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array isDescending");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_array_is_unique_source_method() {
    let module = zun_compiler::compile(
            "class Array { isUnique(): boolean { let outer: u64 = 0; while (outer < this.length) { let inner: u64 = outer + 1; while (inner < this.length) { if (this[outer] == this[inner]) { return false; } inner = inner + 1; } outer = outer + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; return values.isUnique(); }",
        )
        .expect("compile Array isUnique");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array isUnique");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_array_transpose_source_method() {
    let module = zun_compiler::compile(
            "class Array { transpose(): Array { if (this.length == 0) { return new Array(0); } let first: Array = this[0] as Array; let rows: u64 = this.length; let columns: u64 = first.length; let result = new Array(columns); let column: u64 = 0; while (column < columns) { let rowResult = new Array(rows); let row: u64 = 0; while (row < rows) { let source: Array = this[row] as Array; rowResult[row] = source[column]; row = row + 1; } result[column] = rowResult; column = column + 1; } return result; } } function main(): number { let matrix = new Array(2); let first = new Array(2); first[0] = 1.0; first[1] = 2.0; let second = new Array(2); second[0] = 3.0; second[1] = 4.0; matrix[0] = first; matrix[1] = second; let result = matrix.transpose(); let column = result[1] as Array; return column[0] as number; }",
        )
        .expect("compile Array transpose");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array transpose");
    assert!(matches!(result, Value::Float(value, _) if value == 2.0));
}

#[test]
fn executes_array_chunk_source_method() {
    let module = zun_compiler::compile(
            "class Array { chunk(size: u64): Array { if (size == 0) { return new Array(0); } let groups: u64 = this.length / size; if (this.length % size != 0) { groups = groups + 1; } let result = new Array(groups); let group: u64 = 0; let start: u64 = 0; while (group < groups) { let remaining: u64 = this.length - start; let count: u64 = size; if (remaining < count) { count = remaining; } let part = new Array(count); let index: u64 = 0; while (index < count) { part[index] = this[start + index]; index = index + 1; } result[group] = part; start = start + count; group = group + 1; } return result; } } function main(): number { let values = new Array(5); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values[3] = 4.0; values[4] = 5.0; let groups = values.chunk(2); let last = groups[2] as Array; return last[0] as number; }",
        )
        .expect("compile Array chunk");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array chunk");
    assert!(matches!(result, Value::Float(value, _) if value == 5.0));
}

#[test]
fn executes_array_windowed_source_method() {
    let module = zun_compiler::compile(
            "class Array { windowed(size: u64): Array { if (size == 0) { return new Array(0); } if (size > this.length) { return new Array(0); } let count: u64 = this.length - size + 1; let result = new Array(count); let start: u64 = 0; while (start < count) { let window = new Array(size); let index: u64 = 0; while (index < size) { window[index] = this[start + index]; index = index + 1; } result[start] = window; start = start + 1; } return result; } } function main(): number { let values = new Array(4); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values[3] = 4.0; let windows = values.windowed(3); let first = windows[0] as Array; return first[0] as number; }",
        )
        .expect("compile Array windowed");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array windowed");
    assert!(matches!(result, Value::Float(value, _) if value == 1.0));
}

#[test]
fn executes_array_product_source_method() {
    let module = zun_compiler::compile(
            "class Array { product(): number { let total: number = 1.0; let index: u64 = 0; while (index < this.length) { total = total * (this[index] as number); index = index + 1; } return total; } }\n\
             function main(): number { let values = new Array(3); values[0] = 2.0; values[1] = 3.0; values[2] = 4.0; return values.product(); }",
        )
        .expect("compile Array product");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array product");
    assert!(matches!(result, Value::Float(value, _) if value == 24.0));
}

#[test]
fn executes_break_inside_loop() {
    let module = zun_compiler::compile(
            "function main(): u64 { let index: u64 = 0; while (index < 10) { if (index == 3) { break; } index = index + 1; } return index; }",
        )
        .expect("compile break");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run break");
    assert!(matches!(result, Value::Int(value, _) if value == 3));
}

#[test]
fn executes_continue_inside_loop() {
    let module = zun_compiler::compile(
            "function main(): u64 { let index: u64 = 0; let total: u64 = 0; while (index < 5) { index = index + 1; if (index == 3) { continue; } total = total + index; } return total; }",
        )
        .expect("compile continue");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run continue");
    assert!(matches!(result, Value::Int(value, _) if value == 12));
}

#[test]
fn executes_source_array_length() {
    let module = zun_compiler::compile(
        "function main(): u64 { let values = new Array(2); return values.length; }",
    )
    .expect("compile array length");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run array length");
    assert!(matches!(result, Value::Int(2, zun_il::NumKind::U64)));
}

#[test]
fn executes_source_array_pop() {
    let module = zun_compiler::compile(
            "function main(): any { let values = new Array(1); values[0] = 41.0; return values.pop(); }",
        )
        .expect("compile array pop");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run array pop");
    assert!(matches!(result, Value::Float(41.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_source_array_push() {
    let module = zun_compiler::compile(
        "function main(): u64 { let values = new Array(0); return values.push(41.0); }",
    )
    .expect("compile array push");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run array push");
    assert!(matches!(result, Value::Int(1, zun_il::NumKind::U64)));
}

#[test]
fn executes_explicit_numeric_unbox_from_array_value() {
    let module = zun_compiler::compile(
            "function main(): number { let values = new Array(1); values[0] = 41.0; return values[0] as number; }",
        )
        .expect("compile array numeric cast");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run array numeric cast");
    assert!(matches!(result, Value::Float(41.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_string_primitive_methods_through_static_class_binding() {
    let module = zun_compiler::compile(
            "import { length, contains } from \"zun:core\";\n\
             class String { extern length(): u64; extern contains(needle: string): boolean; }\n\
             function main(): boolean { let value: string = \"zun\"; if (value.length() == 3) { return value.contains(\"u\"); } return false; }",
        )
        .expect("compile String primitive methods");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String primitive methods");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_includes_method() {
    let module = zun_compiler::compile(
            "import { includes } from \"zun:core\";\n\
             class String { extern includes(needle: string): boolean; }\n\
             function main(): boolean { let value: string = \"zun runtime\"; return value.includes(\"runtime\"); }",
        )
        .expect("compile String includes");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String includes");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_index_of_with_utf16_indices() {
    let module = zun_compiler::compile(
            "import { indexOf } from \"zun:core\";\n\
             class String { extern indexOf(needle: string): int; }\n\
             function main(): int { let value: string = \"😀zun\"; return value.indexOf(\"zun\"); }",
        )
        .expect("compile String indexOf");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String indexOf");
    assert!(matches!(result, Value::Int(2, zun_il::NumKind::I32)));
}

#[test]
fn executes_string_last_index_of() {
    let module = zun_compiler::compile(
            "import { lastIndexOf } from \"zun:core\";\n\
             class String { extern lastIndexOf(needle: string): int; }\n\
             function main(): int { let value: string = \"zunzun\"; return value.lastIndexOf(\"zun\"); }",
        )
        .expect("compile String lastIndexOf");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String lastIndexOf");
    assert!(matches!(result, Value::Int(3, zun_il::NumKind::I32)));
}

#[test]
fn executes_string_replace_primitive_method() {
    let module = zun_compiler::compile(
            "import { replace } from \"zun:core\";\n\
             class String { extern replace(needle: string, replacement: string): string; }\n\
             function main(): string { let value: string = \"zun vm\"; return value.replace(\"vm\", \"runtime\"); }",
        )
        .expect("compile String replace");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String replace");
    assert!(matches!(result, Value::Str(value) if value.as_ref() == "zun runtime"));
}

#[test]
fn executes_string_substring_with_utf16_indices() {
    let module = zun_compiler::compile(
            "import { substring } from \"zun:core\";\n\
             class String { extern substring(start: u64, end: u64): string; }\n\
             function main(): string { let value: string = \"😀zun\"; return value.substring(2, 5); }",
        )
        .expect("compile String substring");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String substring");
    assert!(matches!(result, Value::Str(value) if value.as_ref() == "zun"));
}

#[test]
fn executes_string_pad_start_primitive_method() {
    let module = zun_compiler::compile(
            "import { padStart } from \"zun:core\";\n\
             class String { extern padStart(targetLength: u64, fill: string): string; }\n\
             function main(): string { let value: string = \"zun\"; return value.padStart(5, \"0\"); }",
        )
        .expect("compile String padStart");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String padStart");
    assert!(matches!(result, Value::Str(value) if value.as_ref() == "00zun"));
}

#[test]
fn executes_string_pad_end_primitive_method() {
    let module = zun_compiler::compile(
            "import { padEnd } from \"zun:core\";\n\
             class String { extern padEnd(targetLength: u64, fill: string): string; }\n\
             function main(): string { let value: string = \"zun\"; return value.padEnd(5, \"0\"); }",
        )
        .expect("compile String padEnd");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String padEnd");
    assert!(matches!(result, Value::Str(value) if value.as_ref() == "zun00"));
}

#[test]
fn executes_string_case_conversion_methods() {
    let module = zun_compiler::compile(
            "import { toLowerCase, toUpperCase } from \"zun:core\";\n\
             class String { extern toLowerCase(): string; extern toUpperCase(): string; }\n\
             function main(): string { let value: string = \"ZuN\"; return value.toLowerCase() + value.toUpperCase(); }",
        )
        .expect("compile String case methods");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String case methods");
    assert!(matches!(result, Value::Str(value) if value.as_ref() == "zunZUN"));
}

#[test]
fn executes_string_trim_boundary_methods() {
    let module = zun_compiler::compile(
            "import { trimStart, trimEnd } from \"zun:core\";\n\
             class String { extern trimStart(): string; extern trimEnd(): string; }\n\
             function main(): string { let value: string = \"  zun  \"; return value.trimStart().trimEnd(); }",
        )
        .expect("compile String trim boundary methods");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String trim boundary methods");
    assert!(matches!(result, Value::Str(value) if value.as_ref() == "zun"));
}

#[test]
fn executes_string_is_blank_source_method() {
    let module = zun_compiler::compile(
            "import { trim, length } from \"zun:core\";\n\
             class String { extern trim(): string; extern length(): u64; isEmpty(): boolean { return this.length() == 0; } isBlank(): boolean { return this.trim().isEmpty(); } }\n\
             function main(): boolean { let value: string = \"   \"; return value.isBlank(); }",
        )
        .expect("compile String isBlank");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isBlank");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_not_empty_source_method() {
    let module = zun_compiler::compile(
            "import { length } from \"zun:core\";\n\
             class String { extern length(): u64; isNotEmpty(): boolean { return this.length() != 0; } }\n\
             function main(): boolean { let value: string = \"zun\"; return value.isNotEmpty(); }",
        )
        .expect("compile String isNotEmpty");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isNotEmpty");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_reverse_native_method() {
    let module = zun_compiler::compile(
        "import { reverse } from \"zun:core\";\n\
             class String { extern reverse(): string; }\n\
             function main(): string { let value: string = \"zun\"; return value.reverse(); }",
    )
    .expect("compile String reverse");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String reverse");
    assert!(matches!(result, Value::Str(value) if value.as_ref() == "nuz"));
}

#[test]
fn executes_string_is_ascii_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\";\n\
             class String { extern length(): u64; extern charCodeAt(index: u64): u32; isAscii(): boolean { let index: u64 = 0; while (index < this.length()) { if (this.charCodeAt(index) > 127) { return false; } index = index + 1; } return true; } }\n\
             function main(): boolean { let value: string = \"zun\"; return value.isAscii(); }",
        )
        .expect("compile String isAscii");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isAscii");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_count_native_method() {
    let module = zun_compiler::compile(
        "import { count } from \"zun:core\";\n\
             class String { extern count(needle: string): int; }\n\
             function main(): int { let value: string = \"aaaa\"; return value.count(\"aa\"); }",
    )
    .expect("compile String count");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String count");
    assert!(matches!(result, Value::Int(value, _) if value == 2));
}

#[test]
fn executes_string_is_numeric_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\";\n\
             class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isNumeric(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { return false; } index = index + 1; } return true; } }\n\
             function main(): boolean { let value: string = \"12345\"; return value.isNumeric(); }",
        )
        .expect("compile String isNumeric");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isNumeric");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_digit_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isDigit(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"2048\"; return value.isDigit(); }",
        )
        .expect("compile String isDigit");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isDigit");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_vowel_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isVowel(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let vowel: boolean = false; if (code == 65) { vowel = true; } if (code == 69) { vowel = true; } if (code == 73) { vowel = true; } if (code == 79) { vowel = true; } if (code == 85) { vowel = true; } if (code == 97) { vowel = true; } if (code == 101) { vowel = true; } if (code == 105) { vowel = true; } if (code == 111) { vowel = true; } if (code == 117) { vowel = true; } if (!vowel) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"AeIOu\"; return value.isVowel(); }",
        )
        .expect("compile String isVowel");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isVowel");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_consonant_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isAlpha(): boolean { if (this.length() == 0) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 65) { return false; } if (code > 90) { if (code < 97) { return false; } if (code > 122) { return false; } } index = index + 1; } return true; } isConsonant(): boolean { if (!this.isAlpha()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 65) { return false; } if (code == 69) { return false; } if (code == 73) { return false; } if (code == 79) { return false; } if (code == 85) { return false; } if (code == 97) { return false; } if (code == 101) { return false; } if (code == 105) { return false; } if (code == 111) { return false; } if (code == 117) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"Rhythm\"; return value.isConsonant(); }",
        )
        .expect("compile String isConsonant");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isConsonant");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_quoted_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isQuoted(): boolean { if (this.length() < 2) { return false; } let zeroIndex: u64 = 0; let first: u32 = this.charCodeAt(zeroIndex); let last: u32 = this.charCodeAt(this.length() - 1); if (first == 34) { return last == 34; } if (first == 39) { return last == 39; } return false; } } function main(): boolean { let value: string = \"\\\"zun\\\"\"; return value.isQuoted(); }",
        )
        .expect("compile String isQuoted");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isQuoted");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_unquote_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt, substring } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; extern substring(start: u64, end: u64): string; isQuoted(): boolean { if (this.length() < 2) { return false; } let zeroIndex: u64 = 0; let first: u32 = this.charCodeAt(zeroIndex); let last: u32 = this.charCodeAt(this.length() - 1); if (first == 34) { return last == 34; } if (first == 39) { return last == 39; } return false; } unquote(): string { if (!this.isQuoted()) { return this; } return this.substring(1, this.length() - 1); } } function main(): string { let value: string = \"\\\"zun\\\"\"; return value.unquote(); }",
        )
        .expect("compile String unquote");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String unquote");
    assert!(matches!(result, Value::Str(value) if value == "zun"));
}

#[test]
fn executes_string_is_wrapped_by_source_method() {
    let module = zun_compiler::compile(
            "import { startsWith, endsWith } from \"zun:core\"; class String { extern startsWith(prefix: string): boolean; extern endsWith(suffix: string): boolean; isWrappedBy(prefix: string, suffix: string): boolean { if (!this.startsWith(prefix)) { return false; } return this.endsWith(suffix); } } function main(): boolean { let value: string = \"<zun>\"; return value.isWrappedBy(\"<\", \">\"); }",
        )
        .expect("compile String isWrappedBy");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isWrappedBy");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_quote_source_method() {
    let module = zun_compiler::compile(
            "class String { quote(): string { return \"\\\"\" + this + \"\\\"\"; } } function main(): string { let value: string = \"zun\"; return value.quote(); }",
        )
        .expect("compile String quote");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String quote");
    assert!(matches!(result, Value::Str(value) if value == "\"zun\""));
}

#[test]
fn executes_string_compare_native_method() {
    let module = zun_compiler::compile(
        "import { compare } from \"zun:core\";\n\
             class String { extern compare(other: string): int; }\n\
             function main(): int { let value: string = \"abc\"; return value.compare(\"abd\"); }",
    )
    .expect("compile String compare");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String compare");
    assert!(matches!(result, Value::Int(value, _) if value == -1));
}

#[test]
fn executes_string_is_whitespace_source_method() {
    let module = zun_compiler::compile(
            "import { length, trim } from \"zun:core\";\n\
             class String { extern length(): u64; extern trim(): string; isEmpty(): boolean { return this.length() == 0; } isWhitespace(): boolean { if (this.isEmpty()) { return false; } return this.trim().isEmpty(); } }\n\
             function main(): boolean { let value: string = \"   \"; return value.isWhitespace(); }",
        )
        .expect("compile String isWhitespace");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isWhitespace");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_alpha_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\";\n\
             class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isAlpha(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 65) { return false; } if (code > 90) { if (code < 97) { return false; } if (code > 122) { return false; } } index = index + 1; } return true; } }\n\
             function main(): boolean { let value: string = \"ZunRuntime\"; return value.isAlpha(); }",
        )
        .expect("compile String isAlpha");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isAlpha");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_alphanumeric_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\";\n\
             class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isAlphanumeric(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { if (code < 65) { return false; } if (code > 90) { if (code < 97) { return false; } if (code > 122) { return false; } } } index = index + 1; } return true; } }\n\
             function main(): boolean { let value: string = \"Zun2026\"; return value.isAlphanumeric(); }",
        )
        .expect("compile String isAlphanumeric");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isAlphanumeric");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_case_predicates_source_methods() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\";\n\
             class String { extern length(): u64; extern charCodeAt(index: u64): u32; isLowerCase(): boolean { if (this.length() == 0) { return false; } let index: u64 = 0; let hasLetter: boolean = false; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code >= 65) { if (code <= 90) { return false; } if (code >= 97) { if (code <= 122) { hasLetter = true; } } } index = index + 1; } return hasLetter; } isUpperCase(): boolean { if (this.length() == 0) { return false; } let index: u64 = 0; let hasLetter: boolean = false; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code >= 97) { if (code <= 122) { return false; } } if (code >= 65) { if (code <= 90) { hasLetter = true; } } index = index + 1; } return hasLetter; } }\n\
             function main(): boolean { let lower: string = \"hello\"; let upper: string = \"WORLD\"; if (!lower.isLowerCase()) { return false; } return upper.isUpperCase(); }",
        )
        .expect("compile String case predicates");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String case predicates");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_to_int_native_method() {
    let module = zun_compiler::compile(
        "import { toInt } from \"zun:core\";\n\
             class String { extern toInt(): int; }\n\
             function main(): int { let value: string = \" -42 \"; return value.toInt(); }",
    )
    .expect("compile String toInt");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String toInt");
    assert!(matches!(result, Value::Int(value, _) if value == -42));
}

#[test]
fn executes_string_to_float_native_method() {
    let module = zun_compiler::compile(
        "import { toFloat } from \"zun:core\";\n\
             class String { extern toFloat(): number; }\n\
             function main(): number { let value: string = \" 3.5 \"; return value.toFloat(); }",
    )
    .expect("compile String toFloat");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String toFloat");
    assert!(matches!(result, Value::Float(value, _) if value == 3.5));
}

#[test]
fn executes_string_to_boolean_native_method() {
    let module = zun_compiler::compile(
            "import { toBoolean } from \"zun:core\";\n\
             class String { extern toBoolean(): boolean; }\n\
             function main(): boolean { let value: string = \" true \"; return value.toBoolean(); }",
        )
        .expect("compile String toBoolean");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String toBoolean");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_number_primitive_methods_through_static_class_binding() {
    let module = zun_compiler::compile(
        "import { toFixed } from \"zun:core\";\n\
             class Number { extern toFixed(digits: int): string; }\n\
             function main(): string { let value: number = 12.5; return value.toFixed(1); }",
    )
    .expect("compile Number primitive methods");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number primitive methods");
    assert!(matches!(result, Value::Str(value) if value.as_ref() == "12.5"));
}

#[test]
fn executes_number_sign_source_method() {
    let module = zun_compiler::compile(
            "class Number { sign(): number { if (this < 0.0) { return -1.0; } if (this > 0.0) { return 1.0; } return 0.0; } }\n\
             function main(): number { let value: number = -42.0; return value.sign(); }",
        )
        .expect("compile Number sign");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number sign");
    assert!(matches!(result, Value::Float(value, _) if value == -1.0));
}

#[test]
fn executes_number_sign_predicates_source_methods() {
    let module = zun_compiler::compile(
            "class Number { isPositive(): boolean { return this > 0.0; } isNonZero(): boolean { return this != 0.0; } } function main(): boolean { let value: number = 3.0; if (!value.isPositive()) { return false; } return value.isNonZero(); }",
        )
        .expect("compile Number sign predicates");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number sign predicates");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_number_is_between_source_method() {
    let module = zun_compiler::compile(
            "class Number { isBetween(low: number, high: number): boolean { if (this < low) { return false; } return this <= high; } } function main(): boolean { let value: number = 5.0; return value.isBetween(1.0, 10.0); }",
        )
        .expect("compile Number isBetween");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number isBetween");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_number_is_in_unit_interval_source_method() {
    let module = zun_compiler::compile(
            "class Number { isBetween(low: number, high: number): boolean { if (this < low) { return false; } return this <= high; } isInUnitInterval(): boolean { return this.isBetween(0.0, 1.0); } } function main(): boolean { let value: number = 0.75; return value.isInUnitInterval(); }",
        )
        .expect("compile Number isInUnitInterval");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number isInUnitInterval");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_number_is_between_exclusive_source_method() {
    let module = zun_compiler::compile(
            "class Number { isBetweenExclusive(low: number, high: number): boolean { if (this <= low) { return false; } return this < high; } } function main(): boolean { let value: number = 5.0; return value.isBetweenExclusive(0.0, 10.0); }",
        )
        .expect("compile Number isBetweenExclusive");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number isBetweenExclusive");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_number_distance_to_source_method() {
    let module = zun_compiler::compile(
            "class Number { abs(): number { if (this < 0.0) { return -this; } return this; } distanceTo(other: number): number { return (this - other).abs(); } } function main(): number { let value: number = 3.0; return value.distanceTo(10.5); }",
        )
        .expect("compile Number distanceTo");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number distanceTo");
    assert!(matches!(result, Value::Float(7.5, zun_il::NumKind::F64)));
}

#[test]
fn executes_number_inverse_lerp_source_method() {
    let module = zun_compiler::compile(
            "class Number { inverseLerp(low: number, high: number): number { if (low == high) { return 0.0; } return (this - low) / (high - low); } } function main(): number { let value: number = 25.0; return value.inverseLerp(0.0, 100.0); }",
        )
        .expect("compile Number inverseLerp");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number inverseLerp");
    assert!(matches!(result, Value::Float(0.25, zun_il::NumKind::F64)));
}

#[test]
fn executes_number_smooth_step_source_method() {
    let module = zun_compiler::compile(
            "class Number { clamp(low: number, high: number): number { if (this < low) { return low; } if (this > high) { return high; } return this; } inverseLerp(low: number, high: number): number { if (low == high) { return 0.0; } return (this - low) / (high - low); } smoothStep(edge0: number, edge1: number): number { let amount: number = this.inverseLerp(edge0, edge1).clamp(0.0, 1.0); return amount * amount * (3.0 - 2.0 * amount); } } function main(): number { let value: number = 50.0; return value.smoothStep(0.0, 100.0); }",
        )
        .expect("compile Number smoothStep");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number smoothStep");
    assert!(matches!(result, Value::Float(0.5, zun_il::NumKind::F64)));
}

#[test]
fn executes_number_is_approximately_source_method() {
    let module = zun_compiler::compile(
            "class Number { isApproximately(other: number, epsilon: number): boolean { let difference: number = this - other; if (difference < 0.0) { difference = -difference; } return difference <= epsilon; } } function main(): boolean { let value: number = 1.001; return value.isApproximately(1.0, 0.01); }",
        )
        .expect("compile Number isApproximately");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number isApproximately");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_math_power_intrinsic() {
    let module = zun_compiler::compile(
            "import { powNumber } from \"zun:core\"; function main(): number { return powNumber(2.0, 3.0); }",
        )
        .expect("compile math power");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run math power");
    assert!(matches!(result, Value::Float(value, _) if value == 8.0));
}

#[test]
fn executes_math_trigonometry_intrinsics() {
    let module = zun_compiler::compile(
            "import { sinNumber, cosNumber } from \"zun:core\"; function main(): number { return sinNumber(0.0) + cosNumber(0.0); }",
        )
        .expect("compile math trigonometry");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run math trigonometry");
    assert!(matches!(result, Value::Float(value, _) if value == 1.0));
}

#[test]
fn executes_math_exponential_and_logarithm_intrinsics() {
    let module = zun_compiler::compile(
            "import { expNumber, logNumber } from \"zun:core\"; function main(): number { return expNumber(0.0) + logNumber(1.0); }",
        )
        .expect("compile math exponential and logarithm");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run math exponential and logarithm");
    assert!(matches!(result, Value::Float(value, _) if value == 1.0));
}

#[test]
fn executes_math_tangent_and_atan2_intrinsics() {
    let module = zun_compiler::compile(
            "import { tanNumber, atan2Number } from \"zun:core\"; function main(): number { return tanNumber(0.0) + atan2Number(0.0, 1.0); }",
        )
        .expect("compile math tangent and atan2");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run math tangent and atan2");
    assert!(matches!(result, Value::Float(value, _) if value == 0.0));
}

#[test]
fn executes_math_hypot_intrinsic() {
    let module = zun_compiler::compile(
            "import { hypotNumber } from \"zun:core\"; function main(): number { return hypotNumber(3.0, 4.0); }",
        )
        .expect("compile math hypot");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run math hypot");
    assert!(matches!(result, Value::Float(value, _) if value == 5.0));
}

#[test]
fn executes_number_finiteness_source_methods() {
    let module = zun_compiler::compile(
            "class Number { isNaN(): boolean { return this != this; } isFinite(): boolean { return this == this; } }\n\
             function main(): boolean { let value: number = 42.0; if (!value.isFinite()) { return false; } return !value.isNaN(); }",
        )
        .expect("compile Number finiteness methods");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number finiteness methods");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_number_is_integer_source_method() {
    let module = zun_compiler::compile(
        "class Number { isInteger(): boolean { return (this as int) as number == this; } }\n\
             function main(): boolean { let value: number = 42.0; return value.isInteger(); }",
    )
    .expect("compile Number isInteger");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number isInteger");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_number_parity_source_methods() {
    let module = zun_compiler::compile(
            "class Number { isInteger(): boolean { return (this as int) as number == this; } isEven(): boolean { if (!this.isInteger()) { return false; } return (this as int) % 2 == 0; } isOdd(): boolean { if (!this.isInteger()) { return false; } return (this as int) % 2 != 0; } }\n\
             function main(): boolean { let value: number = 42.0; if (!value.isEven()) { return false; } return !value.isOdd(); }",
        )
        .expect("compile Number parity methods");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number parity methods");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_number_to_int_source_method() {
    let module = zun_compiler::compile(
        "class Number { toInt(): int { return this as int; } }\n\
             function main(): int { let value: number = 12.75; return value.toInt(); }",
    )
    .expect("compile Number toInt");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number toInt");
    assert!(matches!(result, Value::Int(value, _) if value == 12));
}

#[test]
fn executes_number_to_boolean_source_method() {
    let module = zun_compiler::compile(
        "class Number { toBoolean(): boolean { return this != 0.0; } }\n\
             function main(): boolean { let value: number = 1.0; return value.toBoolean(); }",
    )
    .expect("compile Number toBoolean");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number toBoolean");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_number_to_string_primitive_method() {
    let module = zun_compiler::compile(
        "import { toString } from \"zun:core\";\n\
             class Number { extern toString(): string; }\n\
             function main(): string { let value: number = 12.5; return value.toString(); }",
    )
    .expect("compile Number toString");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number toString");
    assert!(matches!(result, Value::Str(value) if value.as_ref() == "12.5"));
}

#[test]
fn executes_number_source_methods() {
    let module = zun_compiler::compile(
            "class Number { abs(): number { if (this < 0.0) { return -this; } return this; } isZero(): boolean { return this == 0.0; } isNegative(): boolean { return this < 0.0; } }\n\
             function main(): number { let value: number = -7.5; if (value.isNegative()) { return value.abs(); } return 0.0; }",
        )
        .expect("compile Number source methods");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number source methods");
    assert!(matches!(result, Value::Float(7.5, zun_il::NumKind::F64)));
}

#[test]
fn executes_number_clamp_source_method() {
    let module = zun_compiler::compile(
            "class Number { clamp(low: number, high: number): number { if (this < low) { return low; } if (this > high) { return high; } return this; } }\n\
             function main(): number { let value: number = 12.0; return value.clamp(0.0, 10.0); }",
        )
        .expect("compile Number clamp");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number clamp");
    assert!(matches!(result, Value::Float(10.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_number_min_and_max_source_methods() {
    let module = zun_compiler::compile(
            "class Number { min(other: number): number { if (this < other) { return this; } return other; } max(other: number): number { if (this > other) { return this; } return other; } }\n\
             function main(): number { let value: number = 7.0; return value.min(10.0) + value.max(3.0); }",
        )
        .expect("compile Number min max");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number min max");
    assert!(matches!(result, Value::Float(14.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_number_lerp_source_method() {
    let module = zun_compiler::compile(
            "class Number { lerp(target: number, amount: number): number { return this + (target - this) * amount; } } function main(): number { let value: number = 10.0; return value.lerp(20.0, 0.25); }",
        )
        .expect("compile Number lerp");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number lerp");
    assert!(matches!(result, Value::Float(12.5, zun_il::NumKind::F64)));
}

#[test]
fn executes_number_remap_source_method() {
    let module = zun_compiler::compile(
            "class Number { remap(fromLow: number, fromHigh: number, toLow: number, toHigh: number): number { if (fromLow == fromHigh) { return toLow; } let amount: number = (this - fromLow) / (fromHigh - fromLow); return toLow + (toHigh - toLow) * amount; } } function main(): number { let value: number = 5.0; return value.remap(0.0, 10.0, 100.0, 200.0); }",
        )
        .expect("compile Number remap");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Number remap");
    assert!(matches!(result, Value::Float(150.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_string_source_methods_with_primitive_receiver() {
    let module = zun_compiler::compile(
            "class String { concat(other: string): string { return this + other; } withSeparator(separator: string, other: string): string { return this + separator + other; } }\n\
             function main(): string { let value: string = \"zun\"; return value.withSeparator(\":\", \"vm\"); }",
        )
        .expect("compile String source methods");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String source methods");
    assert!(matches!(result, Value::Str(value) if value.as_ref() == "zun:vm"));
}

#[test]
fn executes_string_is_empty_source_method() {
    let module = zun_compiler::compile(
            "import { length } from \"zun:core\";\n\
             class String { extern length(): u64; isEmpty(): boolean { return this.length() == 0; } }\n\
             function main(): boolean { let value: string = \"\"; return value.isEmpty(); }",
        )
        .expect("compile String isEmpty");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isEmpty");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_palindrome_source_method() {
    let module = zun_compiler::compile(
            "import { reverse } from \"zun:core\"; class String { extern reverse(): string; isPalindrome(): boolean { return this == this.reverse(); } } function main(): boolean { let value: string = \"level\"; return value.isPalindrome(); }",
        )
        .expect("compile String isPalindrome");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isPalindrome");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_hexadecimal_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isHexadecimal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { if (code < 65) { return false; } if (code > 70) { if (code < 97) { return false; } if (code > 102) { return false; } } } index = index + 1; } return true; } } function main(): boolean { let value: string = \"a1F0\"; return value.isHexadecimal(); }",
        )
        .expect("compile String isHexadecimal");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isHexadecimal");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_hex_digit_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isHexDigit(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 70) { valid = true; } } if (code >= 97) { if (code <= 102) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"aF09\"; return value.isHexDigit(); }",
        )
        .expect("compile String isHexDigit");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isHexDigit");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_binary_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isBinary(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 49) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"101001\"; return value.isBinary(); }",
        )
        .expect("compile String isBinary");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isBinary");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_octal_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isOctal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 55) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"701234\"; return value.isOctal(); }",
        )
        .expect("compile String isOctal");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isOctal");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_printable_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPrintable(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 32) { return false; } if (code > 126) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"Zun 2026!\"; return value.isPrintable(); }",
        )
        .expect("compile String isPrintable");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isPrintable");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_punctuation_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPunctuation(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let punctuation: boolean = false; if (code >= 33) { if (code <= 47) { punctuation = true; } } if (code >= 58) { if (code <= 64) { punctuation = true; } } if (code >= 91) { if (code <= 96) { punctuation = true; } } if (code >= 123) { if (code <= 126) { punctuation = true; } } if (!punctuation) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"!?.,\"; return value.isPunctuation(); }",
        )
        .expect("compile String isPunctuation");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isPunctuation");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_control_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isControl(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code > 31) { if (code != 127) { return false; } } index = index + 1; } return true; } } function main(): boolean { let value: string = \"\\n\\t\"; return value.isControl(); }",
        )
        .expect("compile String isControl");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isControl");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_signed_integer_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSignedInteger(): boolean { if (this.isEmpty()) { return false; } let start: u64 = 0; let zeroIndex: u64 = 0; let first: u32 = this.charCodeAt(zeroIndex); if (first == 43) { start = 1; } if (first == 45) { start = 1; } if (start == this.length()) { return false; } let index: u64 = start; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"-2048\"; return value.isSignedInteger(); }",
        )
        .expect("compile String isSignedInteger");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isSignedInteger");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_signed_decimal_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSignedDecimal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let digits: u64 = 0; let dots: u64 = 0; let zeroIndex: u64 = 0; let first: u32 = this.charCodeAt(zeroIndex); if (first == 43) { index = 1; } if (first == 45) { index = 1; } while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 46) { dots = dots + 1; if (dots > 1) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } digits = digits + 1; } index = index + 1; } return digits > 0; } } function main(): boolean { let value: string = \"-12.50\"; return value.isSignedDecimal(); }",
        )
        .expect("compile String isSignedDecimal");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isSignedDecimal");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_identifier_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isIdentifier(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code == 95) { valid = true; } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (index > 0) { if (code >= 48) { if (code <= 57) { valid = true; } } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"zun_core2\"; return value.isIdentifier(); }",
        )
        .expect("compile String isIdentifier");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isIdentifier");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_title_case_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isTitleCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let expectUpper: boolean = true; let hasLetter: boolean = false; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 32) { expectUpper = true; } else { if (expectUpper) { if (code < 65) { return false; } if (code > 90) { return false; } expectUpper = false; hasLetter = true; } else { if (code >= 65) { if (code <= 90) { return false; } } if (code < 97) { return false; } if (code > 122) { return false; } } } index = index + 1; } if (!hasLetter) { return false; } return !expectUpper; } } function main(): boolean { let value: string = \"Hello World\"; return value.isTitleCase(); }",
        )
        .expect("compile String isTitleCase");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isTitleCase");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_snake_case_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSnakeCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let previousSeparator: boolean = true; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 95) { if (previousSeparator) { return false; } previousSeparator = true; } else { if (code >= 97) { if (code > 122) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } } previousSeparator = false; } index = index + 1; } return !previousSeparator; } } function main(): boolean { let value: string = \"zun_runtime2\"; return value.isSnakeCase(); }",
        )
        .expect("compile String isSnakeCase");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isSnakeCase");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_kebab_case_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isKebabCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let previousSeparator: boolean = true; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 45) { if (previousSeparator) { return false; } previousSeparator = true; } else { if (code >= 97) { if (code > 122) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } } previousSeparator = false; } index = index + 1; } return !previousSeparator; } } function main(): boolean { let value: string = \"zun-runtime2\"; return value.isKebabCase(); }",
        )
        .expect("compile String isKebabCase");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isKebabCase");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_camel_case_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isCamelCase(): boolean { if (this.isEmpty()) { return false; } let zeroIndex: u64 = 0; let first: u32 = this.charCodeAt(zeroIndex); if (first < 97) { return false; } if (first > 122) { return false; } let index: u64 = 1; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"zunRuntime2\"; return value.isCamelCase(); }",
        )
        .expect("compile String isCamelCase");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isCamelCase");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_is_pascal_case_source_method() {
    let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPascalCase(): boolean { if (this.isEmpty()) { return false; } let zeroIndex: u64 = 0; let first: u32 = this.charCodeAt(zeroIndex); if (first < 65) { return false; } if (first > 90) { return false; } let index: u64 = 1; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"ZunRuntime2\"; return value.isPascalCase(); }",
        )
        .expect("compile String isPascalCase");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String isPascalCase");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_string_to_string_source_method() {
    let module = zun_compiler::compile(
        "class String { toString(): string { return this; } }\n\
             function main(): string { let value: string = \"zun\"; return value.toString(); }",
    )
    .expect("compile String toString");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run String toString");
    assert!(matches!(result, Value::Str(value) if value.as_ref() == "zun"));
}

#[test]
fn executes_array_source_methods_with_array_receiver_binding() {
    let module = zun_compiler::compile(
            "class Array {\n\
                 contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; }\n\
             }\n\
             function main(): boolean { let values = new Array(1); values[0] = 41; return values.contains(values[0]); }",
        )
        .expect("compile Array source method");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array source method");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_array_map_with_generic_function_callback() {
    let module = zun_compiler::compile(
            "class Array { map(callback: Function): Array { let result = new Array(this.length); let index: u64 = 0; while (index < this.length) { result[index] = callback(this[index]); index = index + 1; } return result; } }\n\
             function add_one(value: any): any { return (value as number) + 1.0; }\n\
             function main(): any { let values = new Array(1); values[0] = 41.0; let mapped = values.map(add_one); return mapped[0]; }",
        )
        .expect("compile Array.map callback");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array.map callback");
    assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_array_filter_with_generic_function_callback() {
    let module = zun_compiler::compile(
            "class Array { filter(callback: Predicate): Array { let result = new Array(0); let index: u64 = 0; while (index < this.length) { let value: any = this[index]; if (callback(value)) { result.push(value); } index = index + 1; } return result; } }\n\
             function is_positive(value: any): boolean { return (value as number) > 0.0; }\n\
             function main(): any { let values = new Array(2); values[0] = -1.0; values[1] = 41.0; let filtered = values.filter(is_positive); return filtered[0]; }",
        )
        .expect("compile Array.filter callback");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array.filter callback");
    assert!(matches!(result, Value::Float(41.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_array_reduce_with_generic_reducer_callback() {
    let module = zun_compiler::compile(
            "class Array { reduce(callback: Reducer, initial: any): any { let accumulator: any = initial; let index: u64 = 0; while (index < this.length) { accumulator = callback(accumulator, this[index]); index = index + 1; } return accumulator; } }\n\
             function add(accumulator: any, value: any): any { return (accumulator as number) + (value as number); }\n\
             function main(): any { let values = new Array(2); values[0] = 20.0; values[1] = 22.0; return values.reduce(add, 0.0); }",
        )
        .expect("compile Array.reduce callback");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array.reduce callback");
    assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_array_index_of_and_is_empty_methods() {
    let module = zun_compiler::compile(
            "class Array {\n\
                 indexOf(value: any): int { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return index as int; } index = index + 1; } return -1; }\n\
                 isEmpty(): boolean { return this.length == 0; }\n\
             }\n\
             function main(): int { let values = new Array(1); values[0] = 41; if (values.isEmpty()) { return -2; } return values.indexOf(values[0]); }",
        )
        .expect("compile Array search methods");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array search methods");
    assert!(matches!(result, Value::Int(0, zun_il::NumKind::I32)));
}

#[test]
fn executes_array_access_methods() {
    let module = zun_compiler::compile(
            "class Array { at(index: u64): any { return this[index]; } first(): any { return this[0]; } last(): any { return this[this.length - 1]; } }\n\
             function main(): number { let values = new Array(2); values[0] = 7.0; values[1] = 9.0; return values.at(1) as number; }",
        )
        .expect("compile Array access methods");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array access methods");
    assert!(matches!(result, Value::Float(9.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_array_reverse_source_method() {
    let module = zun_compiler::compile(
            "class Array { reverse(): Array { let left: u64 = 0; if (this.length == 0) { return this; } let right: u64 = this.length - 1; while (left < right) { let temporary: any = this[left]; this[left] = this[right]; this[right] = temporary; left = left + 1; right = right - 1; } return this; } first(): any { return this[0]; } }\n\
             function main(): number { let values = new Array(2); values[0] = 7.0; values[1] = 9.0; values.reverse(); return values.first() as number; }",
        )
        .expect("compile Array reverse");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array reverse");
    assert!(matches!(result, Value::Float(9.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_array_fill_source_method() {
    let module = zun_compiler::compile(
            "class Array { fill(value: any): Array { let index: u64 = 0; while (index < this.length) { this[index] = value; index = index + 1; } return this; } first(): any { return this[0]; } }\n\
             function main(): number { let values = new Array(2); values[0] = 9.0; values.fill(values[0]); return values.first() as number; }",
        )
        .expect("compile Array fill");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array fill");
    assert!(matches!(result, Value::Float(9.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_array_slice_source_method() {
    let module = zun_compiler::compile(
            "class Array { slice(start: u64, end: u64): Array { let count: u64 = end - start; let result = new Array(count); let index: u64 = 0; while (index < count) { result[index] = this[start + index]; index = index + 1; } return result; } first(): any { return this[0]; } }\n\
             function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 7.0; values[2] = 9.0; let selected = values.slice(1, 3); return selected.first() as number; }",
        )
        .expect("compile Array slice");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array slice");
    assert!(matches!(result, Value::Float(7.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_array_concat_source_method() {
    let module = zun_compiler::compile(
            "class Array { concat(other: Array): Array { let result = new Array(this.length + other.length); let index: u64 = 0; while (index < this.length) { result[index] = this[index]; index = index + 1; } let otherIndex: u64 = 0; while (otherIndex < other.length) { result[index + otherIndex] = other[otherIndex]; otherIndex = otherIndex + 1; } return result; } first(): any { return this[0]; } }\n\
             function main(): number { let left = new Array(1); left[0] = 7.0; let right = new Array(1); right[0] = 9.0; let joined = left.concat(right); return joined[1] as number; }",
        )
        .expect("compile Array concat");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array concat");
    assert!(matches!(result, Value::Float(9.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_array_copy_within_source_method() {
    let module = zun_compiler::compile(
            "class Array { slice(start: u64, end: u64): Array { let count: u64 = end - start; let result = new Array(count); let index: u64 = 0; while (index < count) { result[index] = this[start + index]; index = index + 1; } return result; } copyWithin(target: u64, start: u64, end: u64): Array { let copied = this.slice(start, end); let index: u64 = 0; while (index < copied.length) { this[target + index] = copied[index]; index = index + 1; } return this; } }\n\
             function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 7.0; values[2] = 9.0; values.copyWithin(0, 1, 3); return values[0] as number; }",
        )
        .expect("compile Array copyWithin");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array copyWithin");
    assert!(matches!(result, Value::Float(7.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_array_flat_source_method() {
    let module = zun_compiler::compile(
            "class Array { flat(): Array { let total: u64 = 0; let outer: u64 = 0; while (outer < this.length) { let nested: Array = this[outer] as Array; total = total + nested.length; outer = outer + 1; } let result = new Array(total); let output: u64 = 0; outer = 0; while (outer < this.length) { let nested: Array = this[outer] as Array; let inner: u64 = 0; while (inner < nested.length) { result[output] = nested[inner]; output = output + 1; inner = inner + 1; } outer = outer + 1; } return result; } }\n\
             function main(): number { let left = new Array(1); left[0] = 7.0; let right = new Array(1); right[0] = 9.0; let outer = new Array(2); outer[0] = left; outer[1] = right; let flattened = outer.flat(); return flattened[1] as number; }",
        )
        .expect("compile Array flat");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array flat");
    assert!(matches!(result, Value::Float(9.0, zun_il::NumKind::F64)));
}

#[test]
fn executes_array_includes_source_method() {
    let module = zun_compiler::compile(
            "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } includes(value: any): boolean { return this.contains(value); } }\n\
             function main(): boolean { let values = new Array(1); values[0] = 41; return values.includes(values[0]); }",
        )
        .expect("compile Array includes");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Array includes");
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn executes_managed_regexp_methods() {
    let module = zun_compiler::compile(
            "function main(): boolean { let pattern = new RegExp(\"zun-[0-9]+\"); if (pattern.source() != \"zun-[0-9]+\") { return false; } return pattern.isMatch(\"zun-42\"); }",
        )
        .expect("compile RegExp");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run RegExp");
    assert!(matches!(result, Value::Bool(true)));
}

fn run_regexp_bool(source: &str) -> bool {
    let module = zun_compiler::compile(source).expect("compile RegExp program");
    let mut output = CaptureOutput::default();
    match run(&module, &mut output).expect("run RegExp program") {
        Value::Bool(value) => value,
        other => panic!("expected bool, got {other:?}"),
    }
}

#[test]
fn regexp_flags_change_matching() {
    assert!(run_regexp_bool(
            "function main(): boolean { return !new RegExp(\"abc\").test(\"ABC\") && new RegExp(\"abc\", \"i\").test(\"ABC\"); }"
        ));
    assert!(run_regexp_bool(
            "function main(): boolean { return !new RegExp(\"^b$\").test(\"a\\nb\") && new RegExp(\"^b$\", \"m\").test(\"a\\nb\"); }"
        ));
    assert!(run_regexp_bool(
            "function main(): boolean { return !new RegExp(\"a.b\").test(\"a\\nb\") && new RegExp(\"a.b\", \"s\").test(\"a\\nb\"); }"
        ));
    assert!(run_regexp_bool(
        "function main(): boolean { return new RegExp(\"a\", \"g\").test(\"a\"); }"
    ));
}

fn run_regexp_value(source: &str) -> Value {
    let module = zun_compiler::compile(source).expect("compile RegExp program");
    let mut output = CaptureOutput::default();
    run(&module, &mut output).expect("run RegExp program")
}

#[test]
fn string_match_search_split_replace_use_the_regex_engine() {
    // match: same shape as exec, null on miss.
    assert!(run_regexp_bool(
            "function main(): boolean { let m = \"id-42!\".match(new RegExp(\"([a-z]+)-([0-9]+)\")); let a: string = m[0] as string; let b: string = m[2] as string; if (a != \"id-42\") { return false; } return b == \"42\"; }"
        ));
    assert!(run_regexp_bool(
            "function main(): boolean { let m: any = \"zzz\".match(new RegExp(\"[0-9]+\")); return m == null; }"
        ));
    // search: hit and miss.
    assert!(matches!(
        run_regexp_value(
            "function main(): int { return \"ab12\".search(new RegExp(\"[0-9]+\")); }"
        ),
        Value::Int(2, _)
    ));
    assert!(matches!(
        run_regexp_value("function main(): int { return \"abc\".search(new RegExp(\"[0-9]+\")); }"),
        Value::Int(-1, _)
    ));
    // split on a pattern.
    assert!(run_regexp_bool(
            "function main(): boolean { let p = \"a1b22c\".split(new RegExp(\"[0-9]+\")); if (p.length != 3) { return false; } let x: string = p[1] as string; let y: string = p[2] as string; if (x != \"b\") { return false; } return y == \"c\"; }"
        ));
    // replace: $1 / $& substitutions, first match only.
    assert!(run_regexp_bool(
            "function main(): boolean { let r: string = \"x a-b c-d\".replace(new RegExp(\"(\\\\w)-(\\\\w)\"), \"$2+$1 [$&]\"); return r == \"x b+a [a-b] c-d\"; }"
        ));
    // string/string replace is unchanged.
    assert!(run_regexp_bool(
            "import { replace } from \"zun:core\";\n\
             class String { extern replace(needle: string, replacement: string): string; }\n\
             function main(): boolean { let r: string = \"aXa\".replace(\"X\", \"-\"); return r == \"a-a\"; }"
        ));
}

#[test]
fn regexp_backrefs_and_lookaround_work_through_the_api() {
    // Backreference: RE2 cannot do this; Engine B is selected.
    assert!(run_regexp_bool(
            "function main(): boolean { return new RegExp(\"(\\\\w)\\\\1\").test(\"hello\") && !new RegExp(\"(\\\\w)\\\\1\").test(\"helo\"); }"
        ));
    assert!(run_regexp_bool(
            "function main(): boolean { let m = new RegExp(\"(\\\\w)\\\\1\").exec(\"xabbc\"); let a: string = m[0] as string; let b: string = m[1] as string; if (a != \"bb\") { return false; } return b == \"b\"; }"
        ));
    // Lookahead and lookbehind.
    assert!(run_regexp_bool(
            "function main(): boolean { return new RegExp(\"foo(?=bar)\").test(\"foobar\") && !new RegExp(\"foo(?=bar)\").test(\"foobaz\"); }"
        ));
    assert!(run_regexp_bool(
            "function main(): boolean { let m = new RegExp(\"(?<=\\\\$)[0-9]+\").exec(\"cost $42\"); let a: string = m[0] as string; return a == \"42\"; }"
        ));
    // search reports UTF-16 units for Engine B exactly like Engine A.
    assert!(matches!(
            run_regexp_value("function main(): int { return \"\u{1F600}x12\".search(new RegExp(\"(?<=x)[0-9]+\")); }"),
            Value::Int(3, _)
        ));
    assert!(matches!(
        run_regexp_value(
            "function main(): int { return \"\u{1F600}x12\".search(new RegExp(\"[0-9]+\")); }"
        ),
        Value::Int(3, _)
    ));
}

#[test]
fn regexp_catastrophic_pattern_fails_gracefully() {
    let module = zun_compiler::compile(
            "function main(): boolean { return new RegExp(\"(a+)+\\\\1b\").test(\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!\"); }",
        )
        .expect("compile RegExp");
    let mut output = CaptureOutput::default();
    let error = run(&module, &mut output).expect_err("step limit must surface as an error");
    assert!(format!("{error:?}").contains("step limit"), "got {error:?}");
}

#[test]
fn regexp_rejects_unsupported_flags() {
    let module = zun_compiler::compile(
        "function main(): boolean { return new RegExp(\"a\", \"z\").test(\"a\"); }",
    )
    .expect("compile RegExp");
    let mut output = CaptureOutput::default();
    assert!(run(&module, &mut output).is_err());
}

#[test]
fn regexp_exec_returns_groups_or_null() {
    assert!(run_regexp_bool(
            "function main(): boolean { let m = new RegExp(\"(a)(x)?(b)\").exec(\"zab\"); let a: string = m[0] as string; let b: string = m[1] as string; let c: string = m[3] as string; if (a != \"ab\") { return false; } if (b != \"a\") { return false; } return c == \"b\"; }"
        ));
    // A non-participating optional group is `undefined` (Any::void).
    let module = zun_compiler::compile(
        "function main(): any { let m = new RegExp(\"(a)(x)?(b)\").exec(\"zab\"); return m[2]; }",
    )
    .expect("compile optional group");
    let mut output = CaptureOutput::default();
    let value = run(&module, &mut output).expect("run optional group");
    assert!(matches!(value, Value::Void), "got {value:?}");
    assert!(run_regexp_bool(
            "function main(): boolean { let m: any = new RegExp(\"q\").exec(\"zab\"); return m == null; }"
        ));
}

#[test]
fn casts_any_to_a_class_with_runtime_type_check() {
    let module = zun_compiler::compile(
            "class Item {} function main(): boolean { let value: any = box_any(new Item()); let item: Item = value as Item; return true; }",
        )
        .expect("compile Any class cast");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run Any class cast");
    assert!(matches!(result, Value::Bool(true)));
}

struct ExternalHook;

impl CallHook for ExternalHook {
    fn try_call(&self, _method_rid: u32, _args: &[Value]) -> Option<Value> {
        None
    }

    fn try_call_external(
        &self,
        module: &str,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, RunError>> {
        if module == "native" && name == "answer" && args.len() == 1 {
            return Some(Ok(Value::Int(42, zun_il::NumKind::I32)));
        }
        None
    }
}

#[test]
fn external_member_ref_can_be_served_by_a_host_call_hook() {
    let module = zun_compiler::compile_with_externals(
        "import { answer } from \"native\"; function main(): int { return answer(41); }",
        &[zun_sema::check::ExternalFunction {
            module: "native".into(),
            name: "answer".into(),
            params: vec![zun_sema::Ty::INT],
            return_ty: zun_sema::Ty::INT,
        }],
    )
    .expect("compile external hook call");
    let mut output = CaptureOutput::default();
    let hook = ExternalHook;
    let result = run_with(&module, &mut output, Some(&hook)).expect("run external hook call");
    assert!(matches!(result, Value::Int(42, _)));
}

#[test]
fn generator_preserves_external_call_hook_across_polls() {
    let module = zun_compiler::compile_with_externals(
            "extern function answer(value: int): int from \"native\"; function* main(): int { yield answer(1); return 9; }",
            &[zun_sema::check::ExternalFunction {
                module: "native".into(),
                name: "answer".into(),
                params: vec![zun_sema::Ty::INT],
                return_ty: zun_sema::Ty::INT,
            }],
        )
        .expect("compile generator external hook call");
    let entry = module.entry_point.expect("generator entry point");
    let mut output = CaptureOutput::default();
    let hook = ExternalHook;
    let mut generator = GeneratorSession::new_with_context(
        &module,
        entry.rid(),
        Vec::new(),
        &mut output,
        Some(&hook),
        None,
    )
    .expect("create generator with host context");

    let yielded = generator
        .next_result_with_context(&mut output, Some(&hook), None)
        .expect("poll generator");
    assert!(matches!(
        yielded.value,
        Value::Int(42, zun_il::NumKind::I32)
    ));
    assert!(!yielded.done);
}

#[test]
fn tier0_loads_and_stores_static_fields() {
    let mut module = zun_module::Module::new();
    let field = module.tables.add_field_def(FieldDef {
        name: module.strings.intern("counter"),
        signature: zun_metadata::BlobIndex(0),
        kind: zun_il::JitKind::Num(zun_il::NumKind::I32).to_byte(),
        is_static: true,
        is_private: false,
    });
    module.tables.add_type_def(TypeDef {
        name: module.strings.intern("Globals"),
        namespace: zun_metadata::StringIndex(0),
        base: Token::null(TableKind::TypeDef),
        field_start: field.rid(),
        field_end: field.rid() + 1,
        method_start: 1,
        method_end: 2,
    });
    let body = [
        Instruction::new(Opcode::ConstI32, Operand::Imm32(41)),
        Instruction::new(Opcode::StoreStatic, Operand::Token(field)),
        Instruction::new(Opcode::LoadStatic, Operand::Token(field)),
        Instruction::simple(Opcode::Ret),
    ];
    let mut code = Vec::new();
    for instruction in body {
        encode(&instruction, &mut code);
    }
    let offset = write_body(
        &mut module.code,
        0,
        0,
        true,
        zun_il::JitKind::Num(zun_il::NumKind::I32).to_byte(),
        &[],
        &code,
    );
    let method = module.tables.add_method_def(MethodDef {
        name: module.strings.intern("main"),
        signature: zun_metadata::BlobIndex(0),
        body_offset: offset,
        is_static: true,
    });
    module.entry_point = Some(method);
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run static field roundtrip");
    assert!(matches!(result, Value::Int(41, zun_il::NumKind::I32)));
}

#[test]
fn source_reads_a_static_field() {
    let module = zun_compiler::compile(
        "class Math { static answer: number; } function main(): number { return Math.answer; }",
    )
    .expect("compile static field read");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run static field read");
    assert!(matches!(result, Value::Void));
}

#[test]
fn source_writes_and_reads_a_static_field() {
    let module = zun_compiler::compile(
            "class Math { static answer: number; } function main(): number { Math.answer = 42.0; return Math.answer; }",
        ).expect("compile static field assignment");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run static field assignment");
    assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
}

#[test]
fn private_fields_lower_inside_the_declaring_class() {
    let module = zun_compiler::compile(
            "class Box { #value: number; getValue(): number { return this.#value; } setValue(v: number): number { this.#value = v; return v; } } function main(): number { let b = new Box(41.0); b.setValue(42.0); return b.getValue(); }",
        ).expect("compile private field access");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run private field access");
    assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
}

#[test]
fn property_syntax_lowers_to_getter_and_setter_methods() {
    let module = zun_compiler::compile(
            "class Box { get value(): number { return 41.0; } set value(v: number) { } } function main(): number { let b = new Box(); b.value = 42.0; return b.value; }",
        ).expect("compile property accessors");
    let mut output = CaptureOutput::default();
    let result = run(&module, &mut output).expect("run property accessors");
    assert!(matches!(result, Value::Float(41.0, zun_il::NumKind::F64)));
}

/// Compile `source` and return the lines its `print` calls produced.
fn view_dyn_lines(source: &str) -> Vec<String> {
    let module = zun_compiler::compile(source).expect("compile any-typed view program");
    let mut output = CaptureOutput::default();
    run(&module, &mut output).expect("run any-typed view program");
    output.lines
}

#[test]
fn buffer_members_are_reachable_through_any() {
    let lines = view_dyn_lines(
        r#"function probe(v: any): void {
            print(String(v.length));
            print(String(v[0]));
            print(String(v.toString("utf8")));
            print(String(v.toString()));
            print(String(v.slice(1, 3).toString("utf8")));
            print(String(v.indexOf("n")));
            print(String(v.toString("hex", 1)));
        }
        function main() { probe(Buffer.from("ping", "utf8")); }"#,
    );
    assert_eq!(lines, vec!["4", "112", "ping", "ping", "in", "2", "696e67"]);
}

#[test]
fn buffer_accessors_through_any_apply_defaults_and_argument_order() {
    let lines = view_dyn_lines(
        r#"function probe(v: any): void {
            v.writeUInt16LE(258, 0);
            print(String(v.readUInt16LE(0)));
            print(String(v.readUInt8(1)));
            print(String(v.readUInt8()));
            v.writeUInt8(7);
            print(String(v[0]));
            print(String(v.equals(Buffer.from([7, 1, 3]))));
        }
        function main() { probe(Buffer.from([1, 2, 3])); }"#,
    );
    assert_eq!(lines, vec!["258", "1", "2", "7", "true"]);
}

#[test]
fn typed_array_members_are_reachable_through_any_for_every_kind() {
    for kind in ["Uint8Array", "Int16Array", "Float64Array"] {
        let source = format!(
            r#"function probe(v: any): void {{
                print(String(v.length));
                v[1] = 7;
                print(String(v[1]));
                print(String(v[99]));
                v.fill(3, 2);
                print(String(v.join("-")));
                print(String(v.indexOf(7)));
                print(String(v.includes(3)));
                print(String(v.subarray(1).length));
                print(String(v.byteLength));
                print(String(v.byteOffset));
                v.set([5, 5], 0);
                print(String(v.map((x: number) => x + 1).join(",")));
            }}
            function main() {{ probe(new {kind}(4)); }}"#
        );
        let lines = view_dyn_lines(&source);
        let width = match kind {
            "Uint8Array" => "4",
            "Int16Array" => "8",
            _ => "32",
        };
        assert_eq!(
            lines,
            vec!["4", "7", "undefined", "0-7-3-3", "1", "true", "3", width, "0", "6,6,4,4"],
            "{kind}"
        );
    }
}

#[test]
fn array_buffer_byte_length_is_reachable_through_any() {
    let lines = view_dyn_lines(
        r#"function probe(v: any): void { print(String(v.byteLength)); }
        function main() { probe(new ArrayBuffer(8)); }"#,
    );
    assert_eq!(lines, vec!["8"]);
}

#[test]
fn typed_array_element_store_through_any_writes_the_view() {
    let lines = view_dyn_lines(
        r#"function store(v: any): void { v[0] = 300; v[1] = 5; v[9] = 1; }
        function main() {
            const u = new Uint8Array(2);
            store(u);
            print(String(u[0]));
            print(String(u[1]));
            print(String(u.length));
        }"#,
    );
    assert_eq!(lines, vec!["44", "5", "2"]);
}

#[test]
fn any_receivers_of_other_builtins_still_dispatch() {
    let lines = view_dyn_lines(
        r#"function probe(s: any, a: any, m: any, t: any): void {
            print(String(s.toUpperCase()));
            print(String(a.length) + String(a[1]));
            print(String(m.get("k")));
            print(String(t.has(2)));
        }
        function main() {
            probe("abc", [1, 2, 3], new Map<string, number>([["k", 4]]), new Set<number>([2]));
        }"#,
    );
    assert_eq!(lines, vec!["ABC", "32", "4", "true"]);
}

#[test]
fn rest_parameter_function_called_through_an_any_member_packs_its_arguments() {
    let module = zun_compiler::compile_with_oxc(
        r#"
        function rest(...args: any[]): void {
          let out = "";
          for (const a of args) out = out + " " + String(a);
          console.log("n=" + String(args.length) + " ->" + out);
        }
        const viaAny: any = { rest: rest };
        viaAny.rest("a", "b", 5, true, "lit");
        "#,
        true,
    )
    .expect("compile");
    let mut output = CaptureOutput::default();
    run(&module, &mut output).expect("rest call through any member");
    assert_eq!(output.lines, vec!["n=5 -> a b 5 true lit"]);
}

// ---- ToNumber / regex stringification parity (checked against Node) ----

#[test]
fn to_number_runs_value_of_and_maps_null_to_zero() {
    // Node: Number({valueOf(){return null}}) === 0.
    let source = "function main(): string {\n\
               let o: any = { valueOf() { return null; } };\n\
               return String(Number(o));\n\
             }";
    assert_eq!(typeof_of(source), "0");
}

#[test]
fn to_number_of_arrays_uses_joined_text() {
    // Node: Number([5]) === 5, Number([]) === 0, Number([1,2]) is NaN.
    let number_of = |literal: &str| {
        typeof_of(&format!(
            "function main(): string {{ let a: any = {literal}; return String(Number(a)); }}"
        ))
    };
    assert_eq!(number_of("[5]"), "5");
    assert_eq!(number_of("[]"), "0");
    assert_eq!(number_of("[1, 2]"), "NaN");
}

#[test]
fn array_at_coerces_its_index_with_to_number() {
    // Node: [1,2,3].at("1") === 2 and [1,2,3].at(null) === 1.
    let source = "function main(): string {\n\
               let a: any = [1, 2, 3];\n\
               let one: any = \"1\";\n\
               let none: any = null;\n\
               return String(a.at(one)) + \",\" + String(a.at(none));\n\
             }";
    assert_eq!(typeof_of(source), "2,1");
}

#[test]
fn array_flat_coerces_its_depth_with_to_number() {
    // Node: [1,[2,[3]]].flat("1") is [1,2,[3]] (depth 1, not 0).
    let source = "function main(): string {\n\
               let a: any = [1, [2, [3]]];\n\
               let depth: any = \"1\";\n\
               let flat: any = a.flat(depth);\n\
               return String(flat.length) + \",\" + String(Array.isArray(flat[2]));\n\
             }";
    assert_eq!(typeof_of(source), "3,true");
}

#[test]
fn array_sort_comparator_result_goes_through_to_number() {
    // Node: [3,1,2].sort((a, b) => String(a - b)) is [1,2,3]; a string
    // comparator result is ToNumber'd rather than treated as NaN.
    let source = "function main(): string {\n\
               let a: any = [3, 1, 2];\n\
               a.sort((x: any, y: any): any => String(x - y));\n\
               return a.join(\",\");\n\
             }";
    assert_eq!(typeof_of(source), "1,2,3");
    let source = "function main(): string {\n\
               let a: any = [3, 1, 2];\n\
               let sorted: any = a.toSorted((x: any, y: any): any => String(x - y));\n\
               return sorted.join(\",\");\n\
             }";
    assert_eq!(typeof_of(source), "1,2,3");
}

#[test]
fn regex_string_form_orders_flags_like_the_flags_getter() {
    // Node: String(new RegExp("a", "ysg")) === "/a/gsy" === "/a/" + re.flags.
    let source = "function main(): string {\n\
               let re: any = new RegExp(\"a\", \"ysg\");\n\
               let flags: any = re.flags;\n\
               return String(re) + \"|\" + flags + \"|\" + `${re}`;\n\
             }";
    assert_eq!(typeof_of(source), "/a/gsy|gsy|/a/gsy");
}

}
