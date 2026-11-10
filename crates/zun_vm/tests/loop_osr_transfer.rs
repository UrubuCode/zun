//! End-to-end equivalence for live loop OSR (module-JIT slice 4): a hot
//! numeric loop must produce identical output whether it runs purely in the
//! interpreter or is transferred to native code mid-loop.

use zun_interp::{run_with, CaptureOutput};

fn load(src: &str) -> zun_module::Module {
    let mut diags = Vec::new();
    let ast = zun_syntax::oxc::parse_subset(src, true)
        .unwrap_or_else(|errors| panic!("parse errors: {errors:?}"));
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema errors: {diags:?}");
    let module = zun_ilgen::lower(&hir);
    zun_vm::load(&module.write()).unwrap()
}

/// Returns (plain output, hooked output, osr transfers).
fn run_both(src: &str, threshold: u32) -> (Vec<String>, Vec<String>, u64) {
    let module = load(src);
    let mut plain = CaptureOutput::default();
    run_with(&module, &mut plain, None).unwrap();

    let jit = zun_codegen::Jit::new(&module).unwrap();
    let hook = zun_vm::JitHook::with_threshold(jit, &module, threshold);
    let mut hooked = CaptureOutput::default();
    run_with(&module, &mut hooked, Some(&hook)).unwrap();
    (plain.lines, hooked.lines, hook.osr_transfers())
}

#[test]
fn top_level_hot_loop_transfers_and_matches() {
    let src = "function main() { let sum = 0; let i = 0; while (i < 1000) { sum = sum + i; i = i + 1; } print(sum); }";
    let (plain, hooked, transfers) = run_both(src, 2);
    assert_eq!(plain, ["499500"]);
    assert_eq!(plain, hooked);
    assert!(transfers > 0, "loop OSR must have fired");
}

#[test]
fn function_hot_loop_transfers_and_matches() {
    let src = "function run(n: number): number { let sum = 0; let i = 0; while (i < n) { sum = sum + i * 2; i = i + 1; } print(sum); return sum; }\n\
               function main() { print(run(500)); }";
    let (plain, hooked, transfers) = run_both(src, 2);
    assert_eq!(plain, hooked);
    assert!(transfers > 0, "loop OSR must have fired");
}

#[test]
fn loop_that_is_already_done_on_entry_matches() {
    let src = "function main() { let i = 0; let k = 0; while (i < 3) { i = i + 1; } while (k < 0) { k = k + 1; } print(i); print(k); }";
    let (plain, hooked, _) = run_both(src, 1);
    assert_eq!(plain, hooked);
}

#[test]
fn oxc_module_top_level_for_loop_matches() {
    let src = "let sum = 0; for (let i = 0; i < 2000; i++) { sum += i; } console.log(sum);";
    let module = zun_compiler::compile_with_oxc(src, true).expect("compiles");
    let module = zun_vm::load(&module.write()).unwrap();
    let mut plain = CaptureOutput::default();
    run_with(&module, &mut plain, None).unwrap();

    let jit = zun_codegen::Jit::new(&module).unwrap();
    let hook = zun_vm::JitHook::with_threshold(jit, &module, 2);
    let mut hooked = CaptureOutput::default();
    run_with(&module, &mut hooked, Some(&hook)).unwrap();
    assert_eq!(plain.lines, hooked.lines);
    assert_eq!(plain.lines, ["1999000"]);
    assert!(hook.osr_transfers() > 0, "loop OSR must have fired");
}
