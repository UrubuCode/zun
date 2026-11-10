//! Slice 3, milestone A of the module-JIT/OSR epic (docs/design/module-jit-osr.md):
//! an OSR entry variant takes every local, jumps to the loop header and runs to
//! the method's own `ret`. At the first arrival the locals equal the method's
//! initial state, so OSR-entry(initial locals) must equal a normal call.

mod common;

use zun_codegen::{Jit, JitKind, NumKind};
use zun_interp::value::Value;
use zun_module::Module;

fn rid_of(module: &Module, name: &str) -> u32 {
    module
        .tables
        .method_defs
        .iter()
        .position(|m| module.strings.get(m.name) == name)
        .map(|index| index as u32 + 1)
        .unwrap_or_else(|| panic!("no method named {name}"))
}

/// The target PC of the first backedge in a method body (the loop header).
fn first_backedge_target(module: &Module, rid: u32) -> usize {
    let method = &module.tables.method_defs[rid as usize - 1];
    let body = zun_module::read_body(&module.code, method.body_offset).expect("body");
    let instrs = zun_il::decode_all(body.code).expect("decode");
    let mut offset = 0usize;
    for instr in &instrs {
        let next = offset + instr.encoded_len();
        if let zun_il::Operand::Branch(displacement) = instr.operand {
            let target = next as i64 + i64::from(displacement);
            if target >= 0 && (target as usize) < offset {
                return target as usize;
            }
        }
        offset = next;
    }
    panic!("no backedge in method {rid}");
}

fn as_f64(value: &Value) -> f64 {
    match value {
        Value::Float(f, _) => *f,
        Value::Int(i, _) => *i as f64,
        other => panic!("unexpected interpreter value {other:?}"),
    }
}

#[test]
fn osr_entry_with_initial_locals_matches_normal_call_and_interpreter() {
    for n in [0u32, 1, 10, 137] {
        let src = format!(
            "function run(n: number): number {{ \
               let s: number = 0; \
               for (let i: number = 0; i < n; i = i + 1) {{ s = s + i * 2; }} \
               return s; \
             }} \
             function main(): number {{ return run({n}); }}"
        );
        let module = common::compile(&src);
        let rid = rid_of(&module, "run");
        let header = first_backedge_target(&module, rid);

        let mut jit = Jit::compile(&module).expect("jit compiles");
        assert!(jit.is_compiled(rid), "run must be JIT-eligible");
        let id = jit
            .ensure_osr_compiled(&module, rid, header)
            .expect("osr entry compiles");

        let kinds = jit.osr_param_kinds(id).expect("osr kinds").to_vec();
        assert!(kinds.len() > 1, "OSR entry takes every local: {kinds:?}");
        assert_eq!(&kinds[..1], jit.param_kinds(rid).expect("param kinds"));
        assert_eq!(jit.osr_return_kind(id), jit.return_kind(rid));
        assert_eq!(kinds[0], JitKind::Num(NumKind::F64));

        // Initial locals: params = the call args, every other local = zero.
        let mut locals = vec![0u64; kinds.len()];
        locals[0] = f64::from(n).to_bits();
        let mut heap = zun_gc::Heap::new();
        let osr = f64::from_bits(
            jit.call_osr_packed_with_heap(id, &locals, &mut heap)
                .expect("osr call"),
        );
        let normal = f64::from_bits(
            jit.call_packed_with_heap(rid, &[f64::from(n).to_bits()], &mut heap)
                .expect("normal call"),
        );
        let interp = as_f64(&common::interp_entry(&module));
        assert_eq!(osr, normal, "OSR vs normal JIT for n={n}");
        assert_eq!(osr, interp, "OSR vs interpreter for n={n}");

        // Compiling the same patchpoint again is cached.
        assert_eq!(
            jit.ensure_osr_compiled(&module, rid, header).expect("cached"),
            id
        );
    }
}

#[test]
fn osr_entry_is_refused_when_the_method_is_not_fully_eligible() {
    let module = common::compile(
        "function run(n: number): void { \
           for (let i: number = 0; i < n; i = i + 1) { console.log(i); } \
         } \
         function main(): void { run(3); }",
    );
    let rid = rid_of(&module, "run");
    let header = first_backedge_target(&module, rid);
    let mut jit = Jit::new(&module).expect("jit builds");
    assert!(
        jit.ensure_osr_compiled(&module, rid, header).is_err(),
        "an ineligible body must not produce an OSR entry"
    );
}

#[test]
fn osr_entry_rejects_a_pc_that_is_not_a_block_leader() {
    let module = common::compile(
        "function run(n: number): number { \
           let s: number = 0; \
           for (let i: number = 0; i < n; i = i + 1) { s = s + i; } \
           return s; \
         } \
         function main(): number { return run(4); }",
    );
    let rid = rid_of(&module, "run");
    let mut jit = Jit::compile(&module).expect("jit compiles");
    // Offset 1 is inside the first instruction's encoding or a non-leader.
    assert!(jit.ensure_osr_compiled(&module, rid, 1).is_err());
}
