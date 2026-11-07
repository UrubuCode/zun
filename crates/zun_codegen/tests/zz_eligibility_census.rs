//! Measurement-only (ignored): JIT eligibility census over the conformance
//! corpus. For each fixture that compiles, classify every method with a body as
//! JIT-eligible (bridged, the runtime config) or rejected, and bucket the
//! rejected methods by the disqualifier opcode family present. Answers "how much
//! does defect 2 (a non-eligible CallBuiltin disqualifies the whole method)
//! block".
//!
//! Run: `cargo test -p zun_codegen --test zz_eligibility_census -- --ignored --nocapture`

use std::collections::BTreeMap;

use zun_codegen::eligible_methods_bridged;
use zun_il::{decode_all, Opcode, Operand};
use zun_module::Module;

#[test]
#[ignore]
fn eligibility_census() {
    let fixtures: Vec<_> = glob_ts("../../tests");
    let mut total_methods = 0usize;
    let mut eligible = 0usize;
    let mut rejected = 0usize;
    // Rejected-method buckets (a method can be in several).
    let mut has_nonlisted_builtin = 0usize;
    let mut has_throw = 0usize;
    let mut has_cast = 0usize;
    let mut has_indirect = 0usize;
    // Rejected methods whose ONLY disqualifier family is a builtin call
    // (no throw/rethrow/cast/indirect) — the strongest D2-unblockable signal.
    let mut builtin_only = 0usize;
    let mut other_only = 0usize;
    // Which builtin names show up in rejected methods.
    let mut builtin_names: BTreeMap<String, usize> = BTreeMap::new();
    let mut compiled = 0usize;

    for path in &fixtures {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(module) = zun_compiler::compile(&src) else {
            continue;
        };
        compiled += 1;
        let elig = eligible_methods_bridged(&module, false);
        let bytes = module.write();
        let module = Module::read(&bytes).expect("round-trip");

        for (i, method) in module.tables.method_defs.iter().enumerate() {
            if method.body_offset == u32::MAX {
                continue; // abstract / no body
            }
            let rid = i as u32 + 1;
            total_methods += 1;
            if elig.contains(&rid) {
                eligible += 1;
                continue;
            }
            rejected += 1;

            let Some(body) = read_body(&module, method.body_offset) else {
                continue;
            };
            let Ok(instrs) = decode_all(body) else {
                continue;
            };
            let mut builtin = false;
            let mut throw = false;
            let mut cast = false;
            let mut indirect = false;
            for instr in &instrs {
                match instr.opcode {
                    Opcode::CallBuiltin => {
                        if let Operand::Imm32(id) = instr.operand {
                            builtin = true;
                            let name = zun_natives::by_id(id as u32)
                                .map(|f| f.name.to_string())
                                .unwrap_or_else(|| format!("#{id}"));
                            *builtin_names.entry(name).or_default() += 1;
                        }
                    }
                    Opcode::Throw | Opcode::Rethrow => throw = true,
                    Opcode::CastClass => cast = true,
                    Opcode::CallIndirect => indirect = true,
                    _ => {}
                }
            }
            if builtin {
                has_nonlisted_builtin += 1;
            }
            if throw {
                has_throw += 1;
            }
            if cast {
                has_cast += 1;
            }
            if indirect {
                has_indirect += 1;
            }
            let other_families = throw || cast || indirect;
            if builtin && !other_families {
                builtin_only += 1;
            } else if !builtin && !other_families {
                other_only += 1;
            }
        }
    }

    eprintln!("=== JIT eligibility census (bridged) ===");
    eprintln!("fixtures compiled: {compiled}/{}", fixtures.len());
    eprintln!("methods with body: {total_methods}");
    eprintln!("  eligible:  {eligible}");
    eprintln!("  rejected:  {rejected}");
    eprintln!("rejected buckets (overlapping):");
    eprintln!("  any CallBuiltin present:   {has_nonlisted_builtin}");
    eprintln!("  throw/rethrow present:     {has_throw}");
    eprintln!("  castclass present:         {has_cast}");
    eprintln!("  callindirect present:      {has_indirect}");
    eprintln!(
        "rejected, CallBuiltin present & no throw/cast/indirect (D2 upper bound): {builtin_only}"
    );
    eprintln!("rejected, none of the tracked families (other/unsupported-opcode): {other_only}");
    eprintln!("CallBuiltin names in rejected methods (top 15):");
    let mut names: Vec<_> = builtin_names.into_iter().collect();
    names.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for (name, count) in names.into_iter().take(15) {
        eprintln!("  {count:>5}  {name}");
    }
}

fn read_body(module: &Module, offset: u32) -> Option<&[u8]> {
    // MethodBody layout: the code slice starting at `offset`; decode_all stops at
    // the body's end marker. We hand decode_all the tail; it reads the prologue.
    zun_module::read_body(&module.code, offset)
        .ok()
        .map(|b| b.code)
}

fn glob_ts(root: &str) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("ts") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}
