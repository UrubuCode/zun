//! Stage 2d slice 3: the static-regex `isMatch` entry (JS `RegExp.prototype.test`
//! / `isMatch` semantics) synthesized by `zun_regex::zil::compile_is_match` runs
//! on Tier 0 and Tier 1 and agrees, bit for bit, with the runtime regex engine
//! (`zun_regex::Regex`) on `is_match` over a corpus of linear-subset patterns.
//!
//! The generated code indexes UTF-16 code units; the interpreted engine indexes
//! `char`. They coincide on BMP-only input, so the corpus is kept BMP (the
//! astral-plane caveat is documented in `zun_regex::zil`).

use std::rc::Rc;

use zun_il::NumKind;
use zun_interp::value::Value;
use zun_interp::{CallHook, CaptureOutput};
use zun_regex::zil::{compile_is_match, MatcherNatives};
use zun_regex::Regex;

fn matcher_natives() -> MatcherNatives {
    MatcherNatives {
        length: zun_natives::resolve("string", "length").unwrap().0,
        char_code_at: zun_natives::resolve("string", "charCodeAt").unwrap().0,
        buffer_read_u32: zun_natives::resolve("buffer", "bufferReadU32").unwrap().0,
        buffer_write_u32: zun_natives::resolve("buffer", "bufferWriteU32").unwrap().0,
        buffer_new: zun_natives::resolve("buffer", "bufferNew").unwrap().0,
    }
}

/// Linear-subset patterns (no quantifiers / assertions / backref / lookaround):
/// literals, alternation, character classes, `.`, groups. `compile_is_match`
/// returns `Some` for exactly these; anything else is the runtime engine's job.
const PATTERNS: &[(&str, &str)] = &[
    ("abc", ""),
    ("a|b", ""),
    ("foo|bar|baz", ""),
    ("(a)(b)", ""),
    ("a|", ""),
    ("(a|b)c", ""),
    ("[ab]c", ""),
    ("a.c", ""),
    ("a.c", "s"), // dotAll: `.` also matches a line terminator
    ("(x)|(y)", ""),
    ("abc|abx", ""), // backtracking across a shared prefix
    ("(a|ab)c", ""), // capture restored across a backtrack
    ("[0-9]x", ""),
    ("[^ab]", ""),
    ("a[bc]d", ""),
    ("x|y|z", ""),
    (".", ""),
    ("é", ""), // BMP non-ASCII
    ("[a-c]|[x-z]", ""),
    ("hello", ""),
];

const INPUTS: &[&str] = &[
    "", "a", "b", "c", "ab", "abc", "abx", "abd", "xabc", "abcx", "foo", "bar",
    "baz", "x", "y", "z", "xy", "ac", "a.c", "aXc", "a\nc", "bc", "cc", "aabc",
    "ba", "0x", "9x", "hello", "xhellox", "café", "é", "zzz", "qqq",
];

fn args(input: &str) -> Vec<Value> {
    vec![Value::Str(Rc::from(input))]
}

#[test]
fn is_match_runs_on_tier0_and_tier1_and_matches_the_runtime_engine() {
    let nat = matcher_natives();
    // A persistent ambient heap for the Tier-1 calls, which allocate the work
    // buffer here and read the packed result back (see `regex_zil.rs`).
    let mut heap = zun_gc::Heap::new();
    let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };

    for (pattern, flags) in PATTERNS {
        let compiled =
            compile_is_match(pattern, flags, nat).expect("linear-subset pattern must compile");
        let buffer_bytes = compiled.buffer_bytes;
        let bytes = compiled.module.write();
        let loaded = zun_vm::load(&bytes).expect("module verifies and loads");
        // Methods: matchAt (rid 1), matchAtAlloc (rid 2), isMatch (rid 3).
        let match_rid = 1;
        let is_match_rid = compiled.is_match_method.rid();

        let jit = zun_codegen::Jit::new(&loaded).unwrap();
        assert!(
            jit.is_compiled(match_rid),
            "{pattern:?}/{flags:?}: matchAt must be JIT-eligible"
        );
        let hook = zun_vm::JitHook::with_threshold(jit, &loaded, 0);

        let regex = Regex::new(pattern, flags).unwrap();

        for input in INPUTS {
            let want = regex.is_match(input).unwrap();

            // Tier 0: run the synthesized unanchored search directly.
            let mut out = CaptureOutput::default();
            let r0 = zun_interp::run_method(&loaded, is_match_rid, args(input), &mut out)
                .unwrap_or_else(|e| panic!("tier0 {pattern:?} {input:?}: {e:?}"));
            assert!(
                matches!(r0, Value::Bool(b) if b == want),
                "tier0 {pattern:?}/{flags:?} on {input:?}: {r0:?}, want {want}"
            );

            // Tier 1: compose the JIT-compiled anchored `matchAt` into the same
            // unanchored search in Rust (positions 0..=len, inclusive so an
            // empty match at the end is found). This exercises the generated
            // matcher on Tier 1 exactly as `isMatch` drives it on Tier 0.
            let units = input.encode_utf16().count();
            let mut hit = false;
            for pos in 0..=units {
                let buf = zun_gc::with_current_heap_mut(|h| h.alloc_buffer(buffer_bytes));
                let packed = hook
                    .try_call(
                        match_rid,
                        &[
                            Value::Str(Rc::from(*input)),
                            Value::Int(pos as i64, NumKind::U64),
                            Value::Object(buf),
                        ],
                    )
                    .expect("matchAt is JIT-eligible");
                let packed = match packed {
                    Value::Int(n, _) => n,
                    other => panic!("tier1 {pattern:?} {input:?}@{pos}: {other:?}"),
                };
                if packed >= 0 {
                    hit = true;
                    break;
                }
            }
            assert_eq!(hit, want, "tier1 {pattern:?}/{flags:?} on {input:?}");
        }
        assert!(hook.jit_is_defined(match_rid), "{pattern:?}: matchAt not JIT-compiled");
    }
}

#[test]
fn out_of_subset_patterns_do_not_compile() {
    let nat = matcher_natives();
    // Quantifiers, anchors, backref, lookaround, and the `i` flag are later
    // slices: `compile_is_match` returns `None`, so the front-end keeps them on
    // the runtime engine.
    for (p, f) in [
        ("a+", ""),
        ("a*", ""),
        ("a?", ""),
        ("a{2,3}", ""),
        ("^a", ""),
        ("a$", ""),
        (r"\ba", ""),
        (r"(a)\1", ""),
        ("a(?=b)", ""),
        ("abc", "i"),
    ] {
        assert!(compile_is_match(p, f, nat).is_none(), "{p:?}/{f:?} should fall back");
    }
}
