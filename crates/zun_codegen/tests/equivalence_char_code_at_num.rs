//! `charCodeAtNum` (`s.charCodeAt(i)` with a JS `number` index) lowers to a host
//! thunk that shares `zun_core::string_ops::unit_at_number` with the native.
//! Interpreter and JIT must agree for every index class: fractional, negative,
//! `-0`, `NaN`, infinities, past the end, and strings with non-BMP characters or
//! a lone surrogate.

mod common;
use common::*;

use zun_interp::value::Value;

const SRC: &str = "function at(s: string, i: number): number { return s.charCodeAt(i); }\n\
                   function main(): number { return 0; }";

fn rid_of(module: &zun_module::Module, name: &str) -> u32 {
    module
        .tables
        .method_defs
        .iter()
        .position(|m| module.strings.get(m.name) == name)
        .unwrap() as u32
        + 1
}

#[test]
fn char_code_at_num_matches_the_interpreter() {
    let module = compile(SRC);
    let rid = rid_of(&module, "at");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid), "`at` must be JIT-eligible");

    let strings = ["", "abc", "a\u{1F600}z\u{e9}", "\u{FFFF}x", "line\nbreak"];
    let indices = [
        0.0,
        1.0,
        2.9,
        -0.0,
        -1.0,
        -0.5,
        0.9999,
        3.0,
        4.0,
        100.0,
        1e300,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut heap = zun_gc::Heap::new();
    heap.intern_string(std::rc::Rc::from("<handle-0-sentinel>"));
    for s in strings {
        let handle = heap.intern_string(std::rc::Rc::from(s));
        for &i in &indices {
            let raw = jit
                .call_packed_with_heap(rid, &[u64::from(handle), i.to_bits()], &mut heap)
                .expect("jit call");
            let jit_value = f64::from_bits(raw);
            let mut out = zun_interp::CaptureOutput::default();
            let interp = zun_interp::run_method(
                &module,
                rid,
                vec![
                    Value::Str(s.to_string().into()),
                    Value::Float(i, zun_il::NumKind::F64),
                ],
                &mut out,
            )
            .expect("interpreter call");
            let Value::Float(expected, _) = interp else {
                panic!("interpreter returned {interp:?}");
            };
            assert!(
                (jit_value.is_nan() && expected.is_nan()) || jit_value == expected,
                "charCodeAt({s:?}, {i}): jit {jit_value} vs interp {expected}"
            );
        }
    }
}

#[test]
fn char_code_at_num_loop_is_eligible_and_equivalent() {
    // A scanning loop over a parameter string: the realistic shape (hashing,
    // tokenizing). The whole method compiles; the sum matches Tier 0.
    let src = "function sum(s: string, n: number): number {\n\
               let t: number = 0; let i: number = 0;\n\
               while (i < n) { t = t + s.charCodeAt(i); i = i + 1; }\n\
               return t; }\n\
               function main(): number { return 0; }";
    let module = compile(src);
    let rid = rid_of(&module, "sum");
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid), "`sum` must be JIT-eligible");
    let text = "hello \u{1F600} world";
    let mut heap = zun_gc::Heap::new();
    heap.intern_string(std::rc::Rc::from("<handle-0-sentinel>"));
    let handle = heap.intern_string(std::rc::Rc::from(text));
    // 20 > length: the tail reads NaN, so the sum is NaN on both tiers.
    for n in [0.0_f64, 5.0, 8.0, 20.0] {
        let raw = jit
            .call_packed_with_heap(rid, &[u64::from(handle), n.to_bits()], &mut heap)
            .expect("jit call");
        let jit_value = f64::from_bits(raw);
        let mut out = zun_interp::CaptureOutput::default();
        let Value::Float(expected, _) = zun_interp::run_method(
            &module,
            rid,
            vec![
                Value::Str(text.to_string().into()),
                Value::Float(n, zun_il::NumKind::F64),
            ],
            &mut out,
        )
        .expect("interpreter call") else {
            panic!("interpreter did not return a float");
        };
        assert!(
            (jit_value.is_nan() && expected.is_nan()) || jit_value == expected,
            "sum n={n}: jit {jit_value} vs interp {expected}"
        );
    }
}
