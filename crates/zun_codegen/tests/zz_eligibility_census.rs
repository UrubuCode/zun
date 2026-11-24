//! Measurement-only (ignored): JIT eligibility census over the corpus. Every
//! method with a body is classified eligible or rejected using the runtime
//! configuration (bridged, literals allowed), and the first rejection reason
//! comes from `method_rejections` -- the same analysis that decides eligibility.
//! Methods are deduplicated by (name, body bytes) so the std prelude compiled
//! into every fixture counts once; loop-containing methods (a backward branch)
//! are tallied separately because those are the ones that run hot.
//!
//! Run: `cargo test --release -p zun_codegen --test zz_eligibility_census -- --ignored --nocapture`

use std::collections::{BTreeMap, HashSet};

use zun_codegen::{method_blockers, method_rejections};
use zun_il::{decode_all, Opcode, Operand};
use zun_module::Module;

/// Collapse a reason to a bucket: callee reasons keep only the root cause.
fn bucket(reason: &str) -> String {
    let mut r = reason;
    while let Some(rest) = r.strip_prefix("callee ") {
        match rest.split_once(": ") {
            Some((_, root)) => r = root,
            None => break,
        }
    }
    r.to_string()
}

#[test]
#[ignore]
fn eligibility_census() {
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    files.extend(glob_ts("../../tests", true));
    files.extend(glob_ts("../../bench", false));
    files.extend(glob_ts("../../bench/dotnet", false));
    files.extend(glob_ts("../../../rts/bench", false));
    if let Ok(filter) = std::env::var("CENSUS_FILTER") {
        files.retain(|p| p.to_string_lossy().contains(&filter));
    }
    let mut seen: HashSet<(String, Vec<u8>)> = HashSet::new();
    // Methods of an empty program are the std prelude; "user" below excludes them.
    let mut prelude: HashSet<(String, Vec<u8>)> = HashSet::new();
    if let Ok(m) = zun_compiler::compile("export {};") {
        let m = Module::read(&m.write()).unwrap();
        for me in &m.tables.method_defs {
            if me.body_offset != u32::MAX {
                if let Ok(b) = zun_module::read_body(&m.code, me.body_offset) {
                    prelude.insert((m.strings.get(me.name).to_string(), Vec::new()));
                }
            }
        }
    }
    eprintln!("prelude methods: {}", prelude.len());
    let mut sole: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut pair: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut conststr_mix: BTreeMap<String, usize> = BTreeMap::new();
    let mut user: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut blockers: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut user_blockers: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let (mut utotal, mut uelig, mut uloop, mut uloop_elig) = (0usize, 0usize, 0usize, 0usize);
    let mut compiled = 0usize;
    let (mut attempted, mut failed_translation) = (0usize, 0usize);
    let mut translate_failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut translate_examples: BTreeMap<String, String> = BTreeMap::new();
    let (mut total, mut elig, mut loop_total, mut loop_elig) = (0usize, 0usize, 0usize, 0usize);
    let mut all: BTreeMap<String, (usize, usize)> = BTreeMap::new(); // bucket -> (methods, loop methods)
    let mut direct: BTreeMap<String, (usize, usize)> = BTreeMap::new(); // local reason only
    let mut examples: BTreeMap<String, String> = BTreeMap::new();
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(module) = zun_compiler::compile(&src) else {
            continue;
        };
        compiled += 1;
        let bytes = module.write();
        let module = Module::read(&bytes).expect("round-trip");
        let rej = method_rejections(&module, true, true);
        // Eligible methods must also translate: attempt every one and tally the
        // failures by their detail (eligibility and translation share no code, so
        // this is how a disagreement is found).
        {
            let config = zun_codegen::BridgeConfig {
                call_interp: dummy_host as *const u8,
                call_indirect: dummy_host as *const u8,
                route_pending: dummy_host as *const u8,
                null_deref: dummy_host as *const u8,
                unbox_fault: dummy_host as *const u8,
                pending_slot_addr: 0,
            };
            if let Ok(mut jit) = unsafe { zun_codegen::Jit::new_bridged(&module, config) } {
                for rid in 1..=module.tables.method_defs.len() as u32 {
                    if !jit.is_compiled(rid) || rej.contains_key(&rid) {
                        continue;
                    }
                    attempted += 1;
                    if let Err(error) = jit.ensure_compiled(&module, rid) {
                        failed_translation += 1;
                        let detail = match error {
                            zun_codegen::JitError::Compile { detail, .. } => detail,
                            other => format!("{other:?}"),
                        };
                        translate_examples.entry(detail.clone()).or_insert_with(|| {
                            let name = module
                                .tables
                                .method_defs
                                .get(rid as usize - 1)
                                .map(|m| module.strings.get(m.name).to_string())
                                .unwrap_or_default();
                            format!("{name} ({})", path.display())
                        });
                        *translate_failures.entry(detail).or_default() += 1;
                    }
                }
            }
        }
        for (i, method) in module.tables.method_defs.iter().enumerate() {
            if method.body_offset == u32::MAX {
                continue;
            }
            let Ok(body) = zun_module::read_body(&module.code, method.body_offset) else {
                continue;
            };
            let name = module.strings.get(method.name).to_string();
            if !seen.insert((name.clone(), body.code.len().to_le_bytes().to_vec())) {
                continue;
            }
            let has_loop = decode_all(body.code).is_ok_and(|instrs| {
                instrs.iter().any(|i| {
                    matches!(i.operand, Operand::Branch(d) if d < 0)
                        && matches!(
                            i.opcode,
                            Opcode::Branch | Opcode::BranchTrue | Opcode::BranchFalse
                        )
                })
            });
            let is_user = !prelude.contains(&(name.clone(), Vec::new()));
            total += 1;
            if is_user {
                utotal += 1;
                uloop += has_loop as usize;
            }
            loop_total += has_loop as usize;
            match rej.get(&(i as u32 + 1)) {
                None => {
                    elig += 1;
                    loop_elig += has_loop as usize;
                    if is_user {
                        uelig += 1;
                        uloop_elig += has_loop as usize;
                    }
                }
                Some(reason) => {
                    if reason.starts_with("ConstStr literal mixed") {
                        let mut set: std::collections::BTreeSet<String> = Default::default();
                        for i2 in decode_all(body.code).unwrap_or_default() {
                            if !matches!(i2.opcode, Opcode::Nop | Opcode::ConstStr | Opcode::NewObj | Opcode::LoadLocal | Opcode::StoreLocal | Opcode::LoadField | Opcode::StoreField | Opcode::Concat | Opcode::LoadStatic | Opcode::StoreStatic | Opcode::Pop | Opcode::Ret) {
                                set.insert(format!("{:?}", i2.opcode));
                            }
                        }
                        for o in set { *conststr_mix.entry(o).or_default() += 1; }
                    }
                    let b = bucket(reason);
                    {
                        let bl = method_blockers(&module, i as u32 + 1);
                        if bl.len() == 1 {
                            let e = sole.entry(bl[0].clone()).or_default();
                            e.0 += 1;
                            e.1 += has_loop as usize;
                        }
                        if bl.len() == 2 {
                            let mut b2 = bl.clone();
                            b2.sort();
                            let e = pair.entry(b2.join(" + ")).or_default();
                            e.0 += 1;
                            e.1 += has_loop as usize;
                        }
                    }
                    for k in method_blockers(&module, i as u32 + 1) {
                        let e = blockers.entry(k.clone()).or_default();
                        e.0 += 1;
                        e.1 += has_loop as usize;
                        if is_user {
                            let e = user_blockers.entry(k).or_default();
                            e.0 += 1;
                            e.1 += has_loop as usize;
                        }
                    }
                    if is_user {
                        let e = user.entry(b.clone()).or_default();
                        e.0 += 1;
                        e.1 += has_loop as usize;
                    }
                    let e = all.entry(b.clone()).or_default();
                    e.0 += 1;
                    e.1 += has_loop as usize;
                    if !reason.starts_with("callee ") {
                        let d = direct.entry(b.clone()).or_default();
                        d.0 += 1;
                        d.1 += has_loop as usize;
                    }
                    examples
                        .entry(b)
                        .or_insert_with(|| format!("{} ({})", name, path.display()));
                }
            }
        }
    }
    eprintln!("=== JIT eligibility census (bridged, literals) ===");
    eprintln!("files compiled: {compiled}/{}", files.len());
    eprintln!("unique methods: {total}  eligible: {elig}  rejected: {}", total - elig);
    eprintln!(
        "loop methods:   {loop_total}  eligible: {loop_elig}  rejected: {}",
        loop_total - loop_elig
    );
    eprintln!(
        "translation attempts of eligible methods (per file, not deduplicated): {attempted}, failed: {failed_translation}"
    );
    {
        let mut v: Vec<_> = translate_failures.iter().collect();
        v.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
        for (detail, count) in v.into_iter().take(15) {
            eprintln!(
                "  {count:>5}  translation failed: {detail}   e.g. {}",
                translate_examples.get(detail).map(String::as_str).unwrap_or("")
            );
        }
    }
    { let mut v: Vec<_> = conststr_mix.iter().collect(); v.sort_by_key(|(_, c)| std::cmp::Reverse(**c)); eprintln!("ConstStr-mix: opcodes present (methods): {:?}", &v[..v.len().min(25)]); }
    eprintln!("USER methods (non-prelude): {utotal} eligible {uelig}; loop methods {uloop} eligible {uloop_elig}");
    for (title, map) in [
        ("USER first reason (root)", &user),
        ("ALL first reason (root)", &all),
        ("ALL local first reason", &direct),
        ("SOLE blocker (all other checks pass)", &sole),
        ("PAIR of blockers", &pair),
        ("ALL any-blocker presence", &blockers),
        ("USER any-blocker presence", &user_blockers),
    ] {
        eprintln!("--- {title}: methods / loop-methods ---");
        let mut v: Vec<_> = map.iter().collect();
        v.sort_by_key(|(_, c)| std::cmp::Reverse((c.1, c.0)));
        for (b, c) in v.into_iter().take(30) {
            eprintln!("{:>5} {:>5}  {}   e.g. {}", c.0, c.1, b, examples.get(b).map(String::as_str).unwrap_or(""));
        }
    }
}

fn glob_ts(root: &str, recurse: bool) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if recurse {
                    stack.push(path);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("ts") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

extern "C" fn dummy_host() {}
