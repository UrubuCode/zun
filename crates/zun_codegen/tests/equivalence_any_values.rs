//! `any` as a first-class value in compiled frames: a local, a parameter, a
//! return and an argument of a compiled call, `box.any`, `const.null` /
//! `const.undef`, the lowered `unbox.any` forms, strict equality against
//! `null` / `undefined` and numbers / booleans, and truthiness. Every program
//! must give the same answer in the interpreter and in compiled code, and every
//! program that holds an object in an `any` across an allocating callee must
//! find it again, moved, after moving collections.

mod common;
use common::*;
use zun_codegen::{JitAnyAbi, JitKind, RetVal};
use zun_core::{Any, AnyTag};
use zun_interp::value::Value;

/// Compile `src`, require `names` (and `main`) to be JIT-compiled, and compare
/// the entry point's result in both tiers. A heap is installed because some
/// programs allocate.
fn assert_equivalent_compiling(src: &str, names: &[&str]) {
    let module = compile(src);
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    for name in names.iter().chain(&["main"]) {
        assert!(
            jit.is_compiled(method_rid(&module, name)),
            "{name} must be eligible:\n{src}"
        );
        assert!(
            jit.is_defined(method_rid(&module, name)) || *name == "main",
            "{name} must be defined:\n{src}"
        );
    }
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        jit.call_entry_with_heap(entry, &mut heap).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    let same = match (&jit_result, &interp_result) {
        (RetVal::Float(a), Value::Float(b, _)) => a == b || (a.is_nan() && b.is_nan()),
        (RetVal::Int(a), Value::Int(b, _)) => a == b,
        (RetVal::Bool(a), Value::Bool(b)) => a == b,
        _ => false,
    };
    assert!(
        same,
        "JIT {jit_result:?} vs interpreter {interp_result:?} for:\n{src}"
    );
}

/// Method rid by name (rids are 1-based method-table positions).
fn method_rid(module: &zun_module::Module, name: &str) -> u32 {
    module
        .tables
        .method_defs
        .iter()
        .position(|m| module.strings.get(m.name) == name)
        .map(|i| i as u32 + 1)
        .unwrap_or_else(|| panic!("no method {name}"))
}

// ---------------------------------------------------------------------------
// Each tag through a local, a parameter, a return and a compiled call.
// ---------------------------------------------------------------------------

#[test]
fn a_number_through_a_local_a_parameter_a_return_and_a_call() {
    assert_equivalent_compiling(
        "function id(x: any): any { return x; }\n\
         function twice(x: any): number { let y: any = id(x); let z: any = y; return (z as number) + (y as number); }\n\
         function main(): number { let a: any = 20.5; return twice(a) + (id(a) as number); }",
        &["id", "twice"],
    );
}

#[test]
fn a_bool_through_a_local_a_parameter_and_a_return() {
    assert_equivalent_compiling(
        "function id(x: any): any { return x; }\n\
         function pick(x: any): number { let y: any = id(x); if (y) { return 1.0; } return 2.0; }\n\
         function main(): number { let t: any = true; let f: any = false; return pick(t) * 10.0 + pick(f); }",
        &["id", "pick"],
    );
}

#[test]
fn null_and_undefined_through_a_local_a_parameter_and_a_return() {
    assert_equivalent_compiling(
        "function id(x: any): any { return x; }\n\
         function kind(x: any): number { \
             let y: any = id(x); \
             if (y === null) { return 1.0; } \
             if (y === undefined) { return 2.0; } \
             return 3.0; }\n\
         function main(): number { let n: any = null; let u: any = undefined; let v: any = 7.0; \
             return kind(n) * 100.0 + kind(u) * 10.0 + kind(v); }",
        &["id", "kind"],
    );
}

#[test]
fn an_object_reference_is_boxed_into_an_any_and_tested() {
    assert_equivalent_compiling(
        "class B { v: number = 0; }\n\
         function id(x: any): any { return x; }\n\
         function main(): number { let o = new B(); o.v = 3.0; let a: any = o; let b: any = id(a); \
             if (b === null) { return 0.0; } if (b) { return 1.0 + o.v; } return 2.0; }",
        &["id"],
    );
}

// ---------------------------------------------------------------------------
// A mixed-tag phi: a number on one path, `null` on another, merged after a
// branch and around a loop back-edge.
// ---------------------------------------------------------------------------

#[test]
fn a_mixed_tag_local_merges_after_a_branch() {
    assert_equivalent_compiling(
        "function pick(n: number): number { \
             let acc: any = 0.0; \
             if (n > 0.0) { acc = n; } else { acc = null; } \
             if (acc === null) { return -1.0; } \
             return acc as number; }\n\
         function main(): number { return pick(5.0) * 10.0 + pick(-3.0); }",
        &["pick"],
    );
}

#[test]
fn a_mixed_tag_local_goes_around_a_loop_back_edge() {
    assert_equivalent_compiling(
        "function run(n: number): number { \
             let acc: any = 0.0; let s = 0.0; \
             for (let i = 0.0; i < n; i = i + 1.0) { \
                 acc = i % 2.0 === 0.0 ? i : null; \
                 if (acc === null) { s = s + 1.0; } else { s = s + (acc as number); } \
             } return s; }\n\
         function main(): number { return run(1000.0); }",
        &["run"],
    );
}

#[test]
fn a_ternary_merging_a_number_and_null_on_the_stack() {
    assert_equivalent_compiling(
        "function f(c: boolean): number { \
             let a: any = c ? 4.0 : null; \
             if (a === null) { return 100.0; } return a as number; }\n\
         function main(): number { return f(true) + f(false); }",
        &["f"],
    );
}

// ---------------------------------------------------------------------------
// The lowered unbox, equality and truthiness forms with their edge values.
// ---------------------------------------------------------------------------

#[test]
fn truthiness_of_every_number_edge() {
    // 0, -0, NaN are falsy; a non-zero number and Infinity are truthy.
    assert_equivalent_compiling(
        "function t(x: any): number { if (x) { return 1.0; } return 0.0; }\n\
         function main(): number { \
             let zero: any = 0.0; let negzero: any = -0.0; let nan: any = 0.0 / 0.0; \
             let one: any = 1.0; let neg: any = -2.5; let inf: any = 1.0 / 0.0; \
             return t(zero) + t(negzero) * 2.0 + t(nan) * 4.0 + t(one) * 8.0 + t(neg) * 16.0 + t(inf) * 32.0; }",
        &["t"],
    );
}

#[test]
fn truthiness_of_null_undefined_and_booleans() {
    assert_equivalent_compiling(
        "function t(x: any): number { if (x) { return 1.0; } return 0.0; }\n\
         function main(): number { \
             let n: any = null; let u: any = undefined; let f: any = false; let tr: any = true; \
             return t(n) + t(u) * 2.0 + t(f) * 4.0 + t(tr) * 8.0; }",
        &["t"],
    );
}

#[test]
fn equality_against_a_number_and_a_boolean() {
    assert_equivalent_compiling(
        "function eq(x: any, y: number): number { if (x === y) { return 1.0; } return 0.0; }\n\
         function eqb(x: any, y: boolean): number { if (x === y) { return 1.0; } return 0.0; }\n\
         function main(): number { \
             let a: any = 5.0; let n: any = null; let t: any = true; let nan: any = 0.0 / 0.0; \
             let z: any = 0.0; \
             return eq(a, 5.0) + eq(a, 6.0) * 2.0 + eq(n, 0.0) * 4.0 + eq(nan, nan as number) * 8.0 \
                 + eq(z, -0.0) * 16.0 + eqb(t, true) * 32.0 + eqb(t, false) * 64.0 + eqb(a, true) * 128.0; }",
        &["eq", "eqb"],
    );
}

#[test]
fn not_equal_to_null_and_undefined() {
    assert_equivalent_compiling(
        "function f(x: any): number { \
             if (x !== null) { if (x !== undefined) { return 1.0; } return 2.0; } return 3.0; }\n\
         function main(): number { let a: any = 1.0; let n: any = null; let u: any = undefined; \
             return f(a) * 100.0 + f(n) * 10.0 + f(u); }",
        &["f"],
    );
}

#[test]
fn unbox_to_a_number_after_a_phi_and_a_call() {
    assert_equivalent_compiling(
        "function num(x: any): number { return x as number; }\n\
         function main(): number { let a: any = 2.5; let b: any = 4.0; return num(a) * num(b); }",
        &["num"],
    );
}

#[test]
fn a_bare_unbox_into_a_typed_return_stays_interpreted() {
    // `x as boolean` is a bare `unbox.any; ret`: Tier 0 hands the unboxed value on
    // unchanged whatever its tag (a number comes back as a number), which a `bool`
    // return cannot represent, so the form is not lowered.
    let module = compile(
        "function b(x: any): boolean { return x as boolean; }
\n         function main(): number { let t: any = true; let r = 0.0; if (b(t)) { r = r + 1.0; } return r; }",
    );
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(!jit.is_compiled(method_rid(&module, "b")));
}

// ---------------------------------------------------------------------------
// Moving collections: an `any` holding an object stays the same object.
// ---------------------------------------------------------------------------

fn load_field_number(address: usize, slot: usize) -> f64 {
    // The first field slot of a `class B { v: number; w: number }` holds the
    // `f64` bits of `v`.
    let object = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
    f64::from_bits(object.field(slot))
}

/// Run `name(any, ..)` through the carrier entry with an `Object` argument and
/// return the carrier it comes back with.
fn call_returning_any(
    jit: &zun_codegen::Jit,
    heap: &mut zun_gc::Heap,
    rid: u32,
    args: &[u64],
) -> JitAnyAbi {
    jit.call_packed_any_with_heap(rid, args, heap)
        .expect("compiled call returns a carrier")
}

const CHURN: &str = "class B { v: number = 0; w: number = 0; }\n\
     function churn(n: number): number { let t = 0.0; let i = 0.0; \
         while (i < n) { let p = new B(); p.v = i; t = t + p.v; i = i + 1.0; } return t; }\n";

#[test]
fn an_any_object_survives_an_allocating_compiled_callee() {
    let src = format!(
        "{CHURN}function keep(a: any, n: number): any {{ let b: any = a; let r = churn(n); \
             if (r < 0.0) {{ return null; }} return b; }}\n\
         function main(): number {{ return 0.0; }}"
    );
    let module = compile(&src);
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let keep = method_rid(&module, "keep");
    assert!(jit.is_compiled(keep) && jit.is_compiled(method_rid(&module, "churn")));
    let mut heap = zun_gc::Heap::new();
    let type_rid = module
        .tables
        .type_defs
        .iter()
        .position(|t| module.strings.get(t.name) == "B")
        .map(|i| i as u32 + 1)
        .expect("class B");
    let object = heap.alloc(type_rid, 2);
    object.set_field(0, 12345.0f64.to_bits());
    let before = object.addr() as usize;
    let carrier = JitAnyAbi::from_any(Any::object(before));
    let (tag, payload) = carrier.raw_words();
    let result = call_returning_any(&jit, &mut heap, keep, &[tag, payload, 400000.0f64.to_bits()]);
    assert_eq!(result.tag, AnyTag::Object as u64);
    assert_ne!(
        result.payload as usize, before,
        "a moving collection must have relocated the object"
    );
    assert_eq!(load_field_number(result.payload as usize, 0), 12345.0);
}

#[test]
fn a_number_that_looks_like_an_address_is_not_rewritten() {
    // A float whose bit pattern is a plausible nursery address must come back
    // bit-identical after moving collections: only an `Object` tag is a reference.
    let src = format!(
        "{CHURN}function keep(a: any, n: number): any {{ let b: any = a; let r = churn(n); \
             if (r < 0.0) {{ return null; }} return b; }}\n\
         function main(): number {{ return 0.0; }}"
    );
    let module = compile(&src);
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let keep = method_rid(&module, "keep");
    let mut heap = zun_gc::Heap::new();
    let type_rid = module
        .tables
        .type_defs
        .iter()
        .position(|t| module.strings.get(t.name) == "B")
        .map(|i| i as u32 + 1)
        .expect("class B");
    // A live object whose address is then passed as an *Int* and as a *Float*.
    let live = heap.alloc(type_rid, 2);
    let address = live.addr() as u64;
    let _keep_alive = zun_gc::RootArray::from_objects(vec![live]);
    for any in [Any::int(address as i64), Any::float(f64::from_bits(address))] {
        let (tag, payload) = JitAnyAbi::from_any(any).raw_words();
        let result =
            call_returning_any(&jit, &mut heap, keep, &[tag, payload, 400000.0f64.to_bits()]);
        assert_eq!(result.tag, tag, "the tag is unchanged");
        assert_eq!(
            result.payload, payload,
            "a payload that looks like an address is not rewritten"
        );
    }
}

#[test]
fn an_any_object_two_frames_up_survives() {
    let src = format!(
        "{CHURN}function inner(a: any, n: number): any {{ let r = churn(n); if (r < 0.0) {{ return null; }} return a; }}\n\
         function mid(a: any, n: number): any {{ let m: any = a; let r: any = inner(a, n); return m; }}\n\
         function top(a: any, n: number): any {{ let t: any = a; let r: any = mid(a, n); return t; }}\n\
         function main(): number {{ return 0.0; }}"
    );
    let module = compile(&src);
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    for name in ["inner", "mid", "top"] {
        assert!(jit.is_compiled(method_rid(&module, name)), "{name}");
    }
    let mut heap = zun_gc::Heap::new();
    let type_rid = module
        .tables
        .type_defs
        .iter()
        .position(|t| module.strings.get(t.name) == "B")
        .map(|i| i as u32 + 1)
        .expect("class B");
    let object = heap.alloc(type_rid, 2);
    object.set_field(0, 777.0f64.to_bits());
    let before = object.addr() as usize;
    let (tag, payload) = JitAnyAbi::from_any(Any::object(before)).raw_words();
    let result = call_returning_any(
        &jit,
        &mut heap,
        method_rid(&module, "top"),
        &[tag, payload, 300000.0f64.to_bits()],
    );
    assert_eq!(result.tag, AnyTag::Object as u64);
    assert_ne!(
        result.payload as usize, before,
        "a moving collection must have relocated the object"
    );
    assert_eq!(load_field_number(result.payload as usize, 0), 777.0);
}

#[test]
fn param_kinds_of_an_any_method_are_other() {
    let module = compile(
        "function f(x: any, n: number): any { return x; } function main(): number { return 0.0; }",
    );
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    let rid = method_rid(&module, "f");
    assert!(jit.is_compiled(rid));
    assert_eq!(
        jit.param_kinds(rid),
        Some(&[JitKind::Other, JitKind::Num(zun_il::NumKind::F64)][..])
    );
    assert_eq!(jit.return_kind(rid), Some(JitKind::Other));
}

// ---------------------------------------------------------------------------
// Forms that stay declined, with the interpreter behaviour that forces each.
// ---------------------------------------------------------------------------

#[test]
fn comparing_two_non_constant_anys_stays_interpreted() {
    // Equal objects can have different addresses (function boxes, constructor
    // records, BigInts) and equal strings can have different handles.
    let module = compile(
        "function same(a: any, b: any): boolean { return a === b; } function main(): number { return 0.0; }",
    );
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(!jit.is_defined(method_rid(&module, "same")));
}

#[test]
fn an_unbox_to_a_class_stays_declined() {
    // `castclass` of an `any` may throw a TypeError in Tier 0.
    let module = compile(
        "class B { v: number = 0; }\n\
         function read(a: any): number { return (a as B).v; } function main(): number { return 0.0; }",
    );
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(!jit.is_compiled(method_rid(&module, "read")));
}

// ---------------------------------------------------------------------------
// Tags that only reach a compiled method from the host: every tag through the
// carrier entry, against the interpreter given the same value.
// ---------------------------------------------------------------------------

/// Run `name(x: any): number` with `any` in compiled code (through the carrier entry
/// point, which interns strings in the heap it installs) and with `value` in the
/// interpreter; both results are the `f64` the method returns.
fn call_with_any(src: &str, name: &str, any: Any, string: Option<&str>, value: Value) -> (f64, f64) {
    let module = compile(src);
    let rid = method_rid(&module, name);
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(rid), "{name} must be eligible");
    let mut heap = zun_gc::Heap::new();
    let any = match string {
        Some(text) => Any::string(heap.intern_string(std::rc::Rc::from(text))),
        None => any,
    };
    let (tag, payload) = JitAnyAbi::from_any(any).raw_words();
    let bits = jit
        .call_packed_with_any_args_with_heap(rid, &[tag, payload], &mut heap)
        .expect("compiled call");
    let mut out = zun_interp::CaptureOutput::default();
    let interp = match zun_interp::run_method(&module, rid, vec![value], &mut out).expect("interpreter") {
        Value::Float(number, _) => number,
        other => panic!("unexpected interpreter result {other:?}"),
    };
    (f64::from_bits(bits), interp)
}

const TRUTHY: &str = "function t(x: any): number { if (x) { return 1.0; } return 0.0; }\n\
                      function main(): number { return 0.0; }";

#[test]
fn truthiness_matches_the_interpreter_for_every_tag_a_host_can_pass() {
    let cases: Vec<(&str, Any, Option<&str>, Value)> = vec![
        ("int 0", Any::int(0), None, Value::Any(Any::int(0))),
        ("int 5", Any::int(5), None, Value::Any(Any::int(5))),
        ("float 0", Any::float(0.0), None, Value::Any(Any::float(0.0))),
        ("float -0", Any::float(-0.0), None, Value::Any(Any::float(-0.0))),
        ("float NaN", Any::float(f64::NAN), None, Value::Any(Any::float(f64::NAN))),
        ("float 2", Any::float(2.0), None, Value::Any(Any::float(2.0))),
        ("false", Any::bool(false), None, Value::Any(Any::bool(false))),
        ("true", Any::bool(true), None, Value::Any(Any::bool(true))),
        ("null", Any::null(), None, Value::Any(Any::null())),
        ("undefined", Any::void(), None, Value::Any(Any::void())),
        ("empty string", Any::null(), Some(""), Value::Str("".into())),
        ("string", Any::null(), Some("x"), Value::Str("x".into())),
    ];
    for (label, any, string, value) in cases {
        let (jit, interp) = call_with_any(TRUTHY, "t", any, string, value);
        assert_eq!(jit, interp, "truthiness of {label}");
    }
}

#[test]
fn unboxing_a_number_matches_the_interpreter_for_every_numeric_tag() {
    let src = "function num(x: any): number { return x as number; } function main(): number { return 0.0; }";
    for (label, any, value) in [
        ("int", Any::int(-7), Value::Any(Any::int(-7))),
        ("float", Any::float(2.5), Value::Any(Any::float(2.5))),
        ("undefined", Any::void(), Value::Any(Any::void())),
    ] {
        let (jit, interp) = call_with_any(src, "num", any, None, value);
        assert!(
            jit == interp || (jit.is_nan() && interp.is_nan()),
            "unbox of {label}: jit {jit} vs interpreter {interp}"
        );
    }
}
