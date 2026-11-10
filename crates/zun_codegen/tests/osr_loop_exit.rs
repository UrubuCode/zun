//! Slice 3, milestone B of the module-JIT/OSR epic (docs/design/module-jit-osr.md):
//! a return-to-interpreter OSR entry compiles ONLY the loop region, stores every
//! local into an out buffer at the loop's single exit edge and returns the IL PC
//! to resume at.

mod common;

use zun_codegen::Jit;
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

/// Decode `rid`'s body into `(offset, end, branch target if any)` triples.
fn decode_branches(module: &Module, rid: u32) -> Vec<(usize, usize, Option<usize>)> {
    let method = &module.tables.method_defs[rid as usize - 1];
    let body = zun_module::read_body(&module.code, method.body_offset).expect("body");
    let instrs = zun_il::decode_all(body.code).expect("decode");
    let mut offset = 0usize;
    let mut out = Vec::new();
    for instr in &instrs {
        let next = offset + instr.encoded_len();
        let target = match instr.operand {
            zun_il::Operand::Branch(d) => Some((next as i64 + i64::from(d)) as usize),
            _ => None,
        };
        out.push((offset, next, target));
        offset = next;
    }
    out
}

/// The loop header (first backedge target) and the half-open loop region.
fn loop_header_and_region(module: &Module, rid: u32) -> (usize, usize, usize) {
    let branches = decode_branches(module, rid);
    let (header, _) = branches
        .iter()
        .find_map(|&(off, _, t)| t.filter(|&t| t < off).map(|t| (t, off)))
        .expect("backedge");
    let end = branches
        .iter()
        .filter(|&&(_, next, t)| t == Some(header) && next > header)
        .map(|&(_, next, _)| next)
        .max()
        .expect("region end");
    (header, header, end)
}

/// The single branch target outside `[start, end)`.
fn single_exit_target(module: &Module, rid: u32, start: usize, end: usize) -> usize {
    let mut exits: Vec<usize> = decode_branches(module, rid)
        .iter()
        .filter(|&&(off, _, _)| off >= start && off < end)
        .filter_map(|&(_, _, t)| t)
        .filter(|&t| t < start || t >= end)
        .collect();
    exits.sort_unstable();
    exits.dedup();
    assert_eq!(exits.len(), 1, "expected exactly one exit edge: {exits:?}");
    exits[0]
}

#[test]
fn osr_loop_exit_returns_resume_pc_and_post_loop_locals() {
    for n in [0u32, 1, 5] {
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
        let (header, start, end) = loop_header_and_region(&module, rid);
        let exit_pc = single_exit_target(&module, rid, start, end);

        let mut jit = Jit::compile(&module).expect("jit compiles");
        let id = jit
            .ensure_osr_loop_compiled(&module, rid, header)
            .expect("loop-exit osr entry compiles");
        let kinds = jit.osr_param_kinds(id).expect("kinds").to_vec();
        // n, s, i (and nothing else for this body).
        assert_eq!(kinds.len(), 3, "locals: {kinds:?}");

        // Initial locals: n = arg, s = 0, i = 0.
        let mut locals = vec![0u64; kinds.len()];
        locals[0] = f64::from(n).to_bits();
        let mut out = vec![0xDEAD_BEEF_u64; kinds.len()];
        let mut heap = zun_gc::Heap::new();
        let resume = jit
            .call_osr_loop_packed_with_heap(id, &locals, &mut out, &mut heap)
            .expect("osr loop call");

        assert_eq!(resume as usize, exit_pc, "resume PC for n={n}");
        let expected_s: f64 = (0..n).map(|i| f64::from(i) * 2.0).sum();
        assert_eq!(f64::from_bits(out[0]), f64::from(n), "n unchanged, n={n}");
        assert_eq!(f64::from_bits(out[1]), expected_s, "s for n={n}");
        assert_eq!(f64::from_bits(out[2]), f64::from(n), "i == n for n={n}");

        // Cached per (method, pc, mode), and distinct from a to-method-end entry.
        assert_eq!(
            jit.ensure_osr_loop_compiled(&module, rid, header)
                .expect("cached"),
            id
        );
        // The loop-exit entry cannot be called through the milestone A path.
        assert!(jit.call_osr_packed(id, &locals).is_err());
    }
}

#[test]
fn osr_loop_exit_resumes_from_mid_loop_state() {
    // Start the loop "mid-flight": s = 0+2+4 (i = 0..3 done), i = 3, n = 6.
    let module = common::compile(
        "function run(n: number): number { \
           let s: number = 0; \
           for (let i: number = 0; i < n; i = i + 1) { s = s + i * 2; } \
           return s; \
         } \
         function main(): number { return run(6); }",
    );
    let rid = rid_of(&module, "run");
    let (header, start, end) = loop_header_and_region(&module, rid);
    let exit_pc = single_exit_target(&module, rid, start, end);
    let mut jit = Jit::compile(&module).expect("jit compiles");
    let id = jit
        .ensure_osr_loop_compiled(&module, rid, header)
        .expect("compiles");
    let locals = [6f64.to_bits(), 6f64.to_bits(), 3f64.to_bits()];
    let mut out = [0u64; 3];
    let mut heap = zun_gc::Heap::new();
    let resume = jit
        .call_osr_loop_packed_with_heap(id, &locals, &mut out, &mut heap)
        .expect("call");
    assert_eq!(resume as usize, exit_pc);
    // s = 6 + 3*2 + 4*2 + 5*2 = 30, i = 6.
    assert_eq!(f64::from_bits(out[1]), 30.0);
    assert_eq!(f64::from_bits(out[2]), 6.0);
}

#[test]
fn osr_loop_exit_refuses_an_early_return_inside_the_loop() {
    // An early `return` is a `ret` inside the region and a second exit.
    let module = common::compile(
        "function run(n: number): number { \
           let s: number = 0; \
           for (let i: number = 0; i < n; i = i + 1) { \
             if (i == 3) { return 99; } \
             s = s + i; \
           } \
           return s; \
         } \
         function main(): number { return run(9); }",
    );
    let rid = rid_of(&module, "run");
    let (header, _, _) = loop_header_and_region(&module, rid);
    let mut jit = Jit::compile(&module).expect("jit compiles");
    let result = jit.ensure_osr_loop_compiled(&module, rid, header);
    assert!(
        result.is_err(),
        "a loop with a return exit must be refused, got {result:?}"
    );
}

#[test]
fn osr_loop_exit_accepts_break_that_shares_the_loop_exit_target() {
    // `break` jumps to the same post-loop target as the loop test, so there is
    // still exactly ONE distinct exit target and the entry is valid.
    let module = common::compile(
        "function run(n: number): number { \
           let s: number = 0; \
           for (let i: number = 0; i < n; i = i + 1) { \
             if (i == 3) { break; } \
             s = s + i; \
           } \
           return s; \
         } \
         function main(): number { return run(9); }",
    );
    let rid = rid_of(&module, "run");
    let (header, start, end) = loop_header_and_region(&module, rid);
    let exit_pc = single_exit_target(&module, rid, start, end);
    let mut jit = Jit::compile(&module).expect("jit compiles");
    let id = jit
        .ensure_osr_loop_compiled(&module, rid, header)
        .expect("single-target break compiles");
    let locals = [9f64.to_bits(), 0, 0];
    let mut out = [0u64; 3];
    let mut heap = zun_gc::Heap::new();
    let resume = jit
        .call_osr_loop_packed_with_heap(id, &locals, &mut out, &mut heap)
        .expect("call");
    assert_eq!(resume as usize, exit_pc);
    // Breaks at i == 3 with s = 0 + 1 + 2.
    assert_eq!(f64::from_bits(out[1]), 3.0);
    assert_eq!(f64::from_bits(out[2]), 3.0);
}
