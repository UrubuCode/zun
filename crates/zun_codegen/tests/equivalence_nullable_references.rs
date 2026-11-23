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

/// A moving collection scans only the frame that allocates. A method that holds a
/// reference across a call that allocates must stay interpreted, or it would resume
/// on a stale address.
#[test]
fn a_reference_held_across_an_allocating_call_stays_interpreted() {
    let module = compile(&format!(
        "{NODE}\
         function build(d: number): Node1 | null {{ if (d === 0.0) {{ return null; }} \
             let t = new Node1(); t.v = d; t.next = build(d - 1.0); return t; }}\n\
         function main(): number {{ let r = build(3.0); if (r !== null) {{ return r.v; }} return 0.0; }}"
    ));
    let rid = |name: &str| {
        module
            .tables
            .method_defs
            .iter()
            .position(|m| module.strings.get(m.name) == name)
            .map(|i| i as u32 + 1)
            .unwrap()
    };
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(rid("build")), "build holds `t` across build()");
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
