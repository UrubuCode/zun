//! A class reference may be `null` (`C | null`): `null` is the zero handle, a
//! compare against it is a pointer compare, and a null-terminated walk compiles.
//! Each program must give the same answer in both tiers.

mod common;
use common::*;
use zun_codegen::RetVal;
use zun_interp::value::Value;

/// `assert_equivalent` with a heap installed, since these programs allocate.
fn assert_equivalent(src: &str) {
    let module = compile(src);
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry), "entry should be JIT-eligible:
{src}");
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    let same = match (&jit_result, &interp_result) {
        (RetVal::Float(a), Value::Float(b, _)) => a == b,
        (RetVal::Int(a), Value::Int(b, _)) => a == b,
        (RetVal::Bool(a), Value::Bool(b)) => a == b,
        _ => false,
    };
    assert!(same, "JIT {jit_result:?} vs interpreter {interp_result:?} for:
{src}");
}

const NODE: &str = "class Node1 { v: number = 0; next: Node1 | null = null; }\n";

#[test]
fn null_terminated_walk_matches() {
    assert_equivalent(&format!(
        "{NODE}\
         function walk(head: Node1 | null): number {{ let s: number = 0; let p: Node1 | null = head; \
             while (p !== null) {{ s = s + p.v; p = p.next; }} return s; }}\n\
         function main(): number {{ let a = new Node1(); a.v = 1.0; let b = new Node1(); b.v = 20.0; \
             b.next = a; let c = new Node1(); c.v = 300.0; c.next = b; return walk(c); }}"
    ));
}

#[test]
fn walk_of_null_is_zero() {
    assert_equivalent(&format!(
        "{NODE}\
         function walk(head: Node1 | null): number {{ let s: number = 0; let p: Node1 | null = head; \
             while (p !== null) {{ s = s + p.v; p = p.next; }} return s; }}\n\
         function main(): number {{ return walk(null); }}"
    ));
}

#[test]
fn null_compares_in_both_senses() {
    assert_equivalent(&format!(
        "{NODE}\
         function isNull(p: Node1 | null): number {{ if (p === null) {{ return 1.0; }} return 0.0; }}\n\
         function notNull(p: Node1 | null): number {{ if (p !== null) {{ return 1.0; }} return 0.0; }}\n\
         function main(): number {{ let a = new Node1(); \
             return isNull(null) * 1000.0 + isNull(a) * 100.0 + notNull(null) * 10.0 + notNull(a); }}"
    ));
}

#[test]
fn storing_and_returning_null() {
    assert_equivalent(&format!(
        "{NODE}\
         function none(): Node1 | null {{ return null; }}\n\
         function main(): number {{ let a = new Node1(); a.v = 5.0; a.next = a; a.next = null; \
             let q: Node1 | null = none(); if (q === null && a.next === null) {{ return a.v; }} return 0.0; }}"
    ));
}

#[test]
fn a_subclass_is_stored_in_a_nullable_field() {
    assert_equivalent(
        "class Animal { legs: number = 4.0; next: Animal | null = null; }\n\
         class Bird extends Animal { }\n\
         function main(): number { let a = new Animal(); let b = new Bird(); b.legs = 2.0; a.next = b; \
             let p: Animal | null = a.next; if (p !== null) { return p.legs; } return 0.0; }",
    );
}

/// `null` is the zero handle only for a class reference: an `any` or a defaulted
/// numeric parameter treats it differently, so such a method must not be compiled.
#[test]
fn a_null_that_is_not_a_class_reference_stays_interpreted() {
    let module = compile(
        "function add(value: number = 2.0): number { return value; }\n\
         function main(): number { return add(); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
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

/// `assert_equivalent` for a program whose listed methods must be compiled and
/// that allocates enough, under the smallest nursery, to force many moving
/// collections while compiled frames are suspended in calls.
fn assert_collecting_equivalent(src: &str, compiled: &[&str]) {
    let module = compile(src);
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    for name in compiled.iter().chain(&["main"]) {
        assert!(jit.is_compiled(method_rid(&module, name)), "{name} must be compiled:\n{src}");
    }
    let mut heap = zun_gc::Heap::new();
    let jit_result = jit.call_entry_with_heap(entry, &mut heap).expect("jit call");
    let interp_result = interp_entry(&module);
    let same = match (&jit_result, &interp_result) {
        (RetVal::Float(a), Value::Float(b, _)) => a == b,
        (RetVal::Int(a), Value::Int(b, _)) => a == b,
        _ => false,
    };
    assert!(same, "JIT {jit_result:?} vs interpreter {interp_result:?} for:\n{src}");
}

/// A recursive build holds `t` across two allocating calls, and a recursive sum
/// reads the result back: every collection must rewrite the references that the
/// outer frames hold, not only the innermost one.
#[test]
fn a_recursive_tree_build_and_sum_survive_moving_collections() {
    assert_collecting_equivalent(
        "class T { v: number = 0; left: T | null = null; right: T | null = null; }\n\
         function build(d: number): T | null { if (d === 0.0) { return null; } \
             let t = new T(); t.v = d; t.left = build(d - 1.0); t.right = build(d - 1.0); return t; }\n\
         function sum(t: T | null): number { if (t === null) { return 0.0; } \
             return t.v + sum(t.left) + sum(t.right); }\n\
         function main(): number { let total = 0.0; let i = 0.0; \
             while (i < 40.0) { total = total + sum(build(10.0)); i = i + 1.0; } return total; }",
        &["build", "sum"],
    );
}

/// A -> B -> C where only C allocates, and A and B each hold a different live
/// reference across the call: both outer frames must be scanned.
#[test]
fn a_three_deep_chain_keeps_each_frames_references_alive() {
    assert_collecting_equivalent(
        "class P { a: number = 0; b: number = 0; }\n\
         function c(x: number): number { let p = new P(); p.a = x; let q = new P(); q.b = x + 1.0; return p.a + q.b; }\n\
         function b(x: number): number { let kb = new P(); kb.a = x * 3.0; kb.b = 7.0; \
             let r = c(x); let j = new P(); j.a = r; return kb.a + kb.b + j.a; }\n\
         function a(x: number): number { let ka = new P(); ka.a = x; ka.b = x * 2.0; \
             let r = b(x); return ka.a + ka.b + r; }\n\
         function main(): number { let t = 0.0; let i = 0.0; \
             while (i < 30000.0) { t = t + a(i); i = i + 1.0; } return t; }",
        &["a", "b", "c"],
    );
}

/// The same shape without the allocation compiles: a recursive sum only reads.
#[test]
fn a_recursive_read_over_a_tree_compiles() {
    let module = compile(
        "class T { v: number = 0; left: T | null = null; right: T | null = null; }\n\
         function sum(t: T | null): number { if (t === null) { return 0.0; } return t.v + sum(t.left) + sum(t.right); }\n\
         function main(): number { let a = new T(); a.v = 1.0; let b = new T(); b.v = 2.0; a.left = b; return sum(a); }",
    );
    let rid = module
        .tables
        .method_defs
        .iter()
        .position(|m| module.strings.get(m.name) == "sum")
        .map(|i| i as u32 + 1)
        .unwrap();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(jit.is_compiled(rid));
}

/// Callers with more parameters than fit in registers pass the rest on the stack,
/// which moves the callee's frame relative to the caller's stack pointer at the
/// call. The outer frame's references must still be found.
#[test]
fn a_caller_with_stack_arguments_keeps_its_references_alive() {
    assert_collecting_equivalent(
        "class P { a: number = 0; b: number = 0; }\n\
         function leaf(x: number, p: P, a1: number, a2: number, a3: number, a4: number, a5: number, \
             a6: number, a7: number, a8: number, a9: number): number { \
             let q = new P(); q.a = x; return q.a + p.a + a1 + a2 + a3 + a4 + a5 + a6 + a7 + a8 + a9; }\n\
         function mid(x: number, p: P, q: P): number { \
             let r = leaf(x, p, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0); \
             let s = new P(); s.a = r; return s.a + p.a + q.b; }\n\
         function top(x: number): number { let p = new P(); p.a = x; let q = new P(); q.b = x * 2.0; \
             let r = mid(x, p, q); return r + p.a + q.b; }\n\
         function main(): number { let t = 0.0; let i = 0.0; \
             while (i < 20000.0) { t = t + top(i); i = i + 1.0; } return t; }",
        &["leaf", "mid", "top"],
    );
}
