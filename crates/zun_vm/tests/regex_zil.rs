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
        length: zun_natives::resolve("zun:string", "length").unwrap().0,
        char_code_at: zun_natives::resolve("zun:string", "charCodeAt").unwrap().0,
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
