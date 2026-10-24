//! Stage 2d slice 1: a literal-only regex compiled to a synthesized ZIL
//! function runs on Tier 0 and Tier 1 and agrees with the `zun_regex`
//! interpreter.

use std::rc::Rc;
use zun_il::NumKind;
use zun_interp::value::Value;
use zun_interp::{CallHook, CaptureOutput};
use zun_regex::zil::{compile_literal, StringNatives};
use zun_regex::Regex;

fn natives() -> StringNatives {
    StringNatives {
        length: zun_natives::resolve("string", "length").unwrap().0,
        char_code_at: zun_natives::resolve("string", "charCodeAt").unwrap().0,
    }
}

fn args(input: &str, pos: usize) -> Vec<Value> {
    vec![
        Value::Str(Rc::from(input)),
        Value::Int(pos as i64, NumKind::U64),
    ]
}

const PATTERNS: &[&str] = &["a", "abc", "hello", "zun", "aa", "ab-c 1", "é"];
const INPUTS: &[&str] = &["", "a", "abc", "xabcx", "hello world", "aaa", "zzun zun", "ab-c 1!", "caf\u{e9}", "ab", "abd"];

#[test]
fn literal_matcher_runs_on_tier0_and_tier1_and_matches_the_interpreter() {
    for pattern in PATTERNS {
        let compiled = compile_literal(pattern, natives()).expect("literal-only");
        let bytes = compiled.module.write();
        let loaded = zun_vm::load(&bytes).expect("module verifies and loads");
        let rid = compiled.method.rid();

        // Tier 1: threshold 0 compiles on first call. Native calls made from
        // outside an interpreter run need a current GC heap (strings are
        // interned there), so enter one for the duration.
        let mut heap = zun_gc::Heap::new();
        let _heap_scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        let jit = zun_codegen::Jit::new(&loaded).unwrap();
        assert!(jit.is_compiled(rid), "{pattern:?} must be JIT-eligible");
        let hook = zun_vm::JitHook::with_threshold(jit, &loaded, 0);

        let regex = Regex::new(pattern, "").unwrap();
        for input in INPUTS {
            let chars: Vec<char> = input.chars().collect();
            for pos in 0..=chars.len() {
                // BMP-only inputs: char index == UTF-16 index.
                let expected = regex.match_at_chars(&chars, pos).unwrap().is_some();

                let mut out = CaptureOutput::default();
                let tier0 = zun_interp::run_method(&loaded, rid, args(input, pos), &mut out)
                    .expect("tier0 runs");
                assert!(
                    matches!(tier0, Value::Bool(b) if b == expected),
                    "tier0 {pattern:?} on {input:?}@{pos}: {tier0:?}, want {expected}"
                );

                let tier1 = hook
                    .try_call(rid, &args(input, pos))
                    .expect("synthesized matcher is JIT-eligible");
                assert!(
                    matches!(tier1, Value::Bool(b) if b == expected),
                    "tier1 {pattern:?} on {input:?}@{pos}: {tier1:?}, want {expected}"
                );
            }
        }
        assert!(hook.jit_is_defined(rid), "{pattern:?} was not JIT-compiled");
    }
}

#[test]
fn find_via_compiled_matcher_agrees_with_regex_find() {
    let compiled = compile_literal("abc", natives()).unwrap();
    let loaded = zun_vm::load(&compiled.module.write()).unwrap();
    let rid = compiled.method.rid();
    let regex = Regex::new("abc", "").unwrap();
    for input in ["", "abc", "xxabcxx", "ababc", "ab", "abcabc"] {
        let mut found = None;
        for pos in 0..=input.len() {
            let mut out = CaptureOutput::default();
            let v = zun_interp::run_method(&loaded, rid, args(input, pos), &mut out).unwrap();
            if matches!(v, Value::Bool(true)) {
                found = Some(pos);
                break;
            }
        }
        assert_eq!(found, regex.find(input, 0).unwrap().map(|m| m.start()), "{input:?}");
    }
}

#[test]
fn non_literal_patterns_are_not_compiled() {
    for p in ["a+", "a|b", "(a)", "[ab]", ".", r"a\d", "a*", "^a"] {
        assert!(compile_literal(p, natives()).is_none(), "{p}");
    }
}

// --- Slice 2: backtracking matcher ----------------------------------------

use zun_regex::zil::{compile_backtracking, unpack_result, CompiledMatcher, MatcherNatives};

fn matcher_natives() -> MatcherNatives {
    MatcherNatives {
        length: zun_natives::resolve("string", "length").unwrap().0,
        char_code_at: zun_natives::resolve("string", "charCodeAt").unwrap().0,
        buffer_read_u32: zun_natives::resolve("buffer", "bufferReadU32").unwrap().0,
        buffer_write_u32: zun_natives::resolve("buffer", "bufferWriteU32").unwrap().0,
        buffer_new: zun_natives::resolve("buffer", "bufferNew").unwrap().0,
    }
}

/// Expected group spans from the interpreter (with group 0 filled).
fn expected(regex: &Regex, chars: &[char], pos: usize) -> Option<Vec<Option<(usize, usize)>>> {
    regex
        .match_at_chars(chars, pos)
        .unwrap()
        .map(|m| m.groups().to_vec())
}

const SLICE2_PATTERNS: &[&str] = &[
    "abc",
    "a|b",
    "foo|bar|baz",
    "(a)(b)",
    "a|",
    "(a|b)c",
    "[ab]c",
    "a.c",
    "(x)|(y)",
    "abc|abx",   // backtracking: first arm matches a prefix then fails
    "(a|ab)c",   // capture must be restored across a backtrack
];

const SLICE2_INPUTS: &[&str] = &[
    "", "a", "b", "c", "ab", "abc", "abx", "abd", "xabc", "foo", "bar", "baz", "baz",
    "x", "y", "xy", "ac", "a.c", "aXc", "bc", "cc", "aabc", "ba",
];

#[test]
fn backtracking_matcher_agrees_with_interpreter_on_tier0_and_tier1() {
    let nat = matcher_natives();
    // A persistent ambient heap for the Tier-1 (JIT) calls, which allocate the
    // work buffer here and read the packed result back. Tier 0 runs through the
    // wrapper, which allocates its own buffer in the interpreter's own heap.
    let mut heap = zun_gc::Heap::new();
    let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };

    for pattern in SLICE2_PATTERNS {
        let Some(compiled) = compile_backtracking(pattern, "", nat) else {
            continue;
        };
        let CompiledMatcher { buffer_bytes, cap_cells, .. } = compiled;
        let bytes = compiled.module.write();
        let loaded = zun_vm::load(&bytes).expect("module verifies and loads");
        let match_rid = compiled.method.rid();
        let alloc_rid = compiled.alloc_method.rid();

        let jit = zun_codegen::Jit::new(&loaded).unwrap();
        assert!(jit.is_compiled(match_rid), "{pattern:?} matchAt must be JIT-eligible");
        let hook = zun_vm::JitHook::with_threshold(jit, &loaded, 0);

        let regex = Regex::new(pattern, "").unwrap();

        for input in SLICE2_INPUTS {
            let chars: Vec<char> = input.chars().collect();
            for pos in 0..=chars.len() {
                let want = expected(&regex, &chars, pos);

                // Tier 0: run the wrapper, which allocates its own scratch.
                let mut out = CaptureOutput::default();
                let r0 = zun_interp::run_method(
                    &loaded,
                    alloc_rid,
                    vec![Value::Str(Rc::from(*input)), Value::Int(pos as i64, NumKind::U64)],
                    &mut out,
                )
                .unwrap_or_else(|e| panic!("tier0 {pattern:?} {input:?}@{pos}: {e:?}"));
                let packed0 = int_of(r0, pattern, input, pos, "tier0");
                assert_eq!(
                    unpack_result(packed0, cap_cells),
                    want,
                    "tier0 {pattern:?} on {input:?}@{pos}"
                );

                // Tier 1: call matchAt directly with an ambient-heap buffer.
                let buf = zun_gc::with_current_heap_mut(|h| h.alloc_buffer(buffer_bytes));
                let r1 = hook
                    .try_call(
                        match_rid,
                        &[
                            Value::Str(Rc::from(*input)),
                            Value::Int(pos as i64, NumKind::U64),
                            Value::Object(buf),
                        ],
                    )
                    .expect("synthesized matcher is JIT-eligible");
                let packed1 = int_of(r1, pattern, input, pos, "tier1");
                assert_eq!(
                    unpack_result(packed1, cap_cells),
                    want,
                    "tier1 {pattern:?} on {input:?}@{pos}"
                );
            }
        }
        assert!(hook.jit_is_defined(match_rid), "{pattern:?} matchAt was not JIT-compiled");
    }
}

fn int_of(v: Value, pattern: &str, input: &str, pos: usize, tier: &str) -> i64 {
    match v {
        Value::Int(n, _) => n,
        other => panic!("{tier} {pattern:?} {input:?}@{pos}: non-int result {other:?}"),
    }
}

#[test]
fn out_of_scope_patterns_fall_back() {
    let nat = matcher_natives();
    // Loops, backrefs, lookaround, assertions, and the `i` flag are later slices.
    for p in ["a+", "a*", "a?", "a{2,3}", r"(a)\1", "a(?=b)", "(?<=a)b", "^a", "a$", r"\ba"] {
        assert!(compile_backtracking(p, "", nat).is_none(), "{p} should fall back");
    }
    assert!(compile_backtracking("abc", "i", nat).is_none(), "ignore-case falls back");
}
