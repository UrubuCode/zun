//! Slice 2 of the module-JIT/OSR epic (docs/design/module-jit-osr.md):
//! loop-region eligibility. A method whose body is not eligible as a whole can
//! still have a hot loop region that is; `loop_region_eligible` is the gate the
//! OSR entry compiler (slice 3) will consume.

mod common;

use zun_module::Module;

/// RID of the first method named `name`.
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

#[test]
fn pure_numeric_loop_region_is_eligible() {
    let module = common::compile(
        "function run(n: number): number { \
           let s: number = 0; \
           for (let i: number = 0; i < n; i = i + 1) { s = s + i * 2; } \
           return s; \
         } \
         function main(): number { return run(10); }",
    );
    let rid = rid_of(&module, "run");
    let header = first_backedge_target(&module, rid);
    assert!(
        zun_codegen::loop_region(&module, rid, header).is_some(),
        "loop region should resolve"
    );
    assert!(
        zun_codegen::loop_region_eligible(&module, rid, header),
        "a pure f64 loop region must be JIT-eligible"
    );
}

#[test]
fn loop_region_with_dynamic_op_is_ineligible() {
    // The loop body calls console.log — a dynamic builtin outside the typed JIT
    // subset — so the region is not eligible even though the arithmetic is.
    let module = common::compile(
        "function run(n: number): void { \
           for (let i: number = 0; i < n; i = i + 1) { console.log(i); } \
         } \
         function main(): void { run(3); }",
    );
    let rid = rid_of(&module, "run");
    let header = first_backedge_target(&module, rid);
    assert!(
        !zun_codegen::loop_region_eligible(&module, rid, header),
        "a loop region containing a dynamic call must not be eligible"
    );
}

#[test]
fn eligible_loop_region_inside_ineligible_entry() {
    // `<main>` carries dynamic head/tail (statics init, console.log) and is not
    // eligible as a whole, yet its numeric loop region is — exactly the case
    // OSR targets.
    let module = common::compile(
        "let sum: number = 0; \
         for (let i: number = 0; i < 1000; i = i + 1) { sum = sum + i; } \
         console.log(\"sum=\" + sum);",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let whole_method_eligible = zun_codegen::eligible_methods(&module).contains(&entry);
    assert!(
        !whole_method_eligible,
        "the module entry is not eligible as a whole (dynamic head/tail)"
    );
    let header = first_backedge_target(&module, entry);
    assert!(
        zun_codegen::loop_region_eligible(&module, entry, header),
        "the entry's numeric loop region must be eligible"
    );
}
