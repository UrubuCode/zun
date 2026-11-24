//! `any` values crossing the VM boundaries of a compiled frame: a hot interpreted
//! caller handing `any` arguments to a compiled method and taking an `any` back
//! (the carrier entry points), a compiled caller bridging an `any` to and from an
//! interpreted callee, and an OSR loop whose locals include an `any`. Each
//! program must print the same lines with and without the JIT, under moving
//! collections, and the tests check the paths were actually taken.

use zun_interp::{run_with, CaptureOutput};

/// Output with no JIT, and with the bridged JIT at the default tiering
/// threshold.
fn both_tiers(src: &str) -> (Vec<String>, Vec<String>) {
    let module = zun_compiler::compile_with_oxc(src, true).expect("compiles");
    let bytes = module.write();
    let loaded = zun_vm::load(&bytes).unwrap();
    let mut plain = CaptureOutput::default();
    run_with(&loaded, &mut plain, None).expect("interpreter runs");
    let mut jitted = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut jitted).expect("jit run");
    (plain.lines, jitted.lines)
}

const B: &str = "class B { v: number = 0; w: number = 0; }\n\
    function churn(n: number): number { let t = 0; for (let i = 0; i < n; i++) { const p = new B(); p.v = i; t += p.v; } return t; }\n";

/// A compiled method taking and returning an `any`, reached from a hot
/// interpreted loop: the arguments cross as (tag, payload) words, the result as a
/// carrier, and the object it holds across an allocating callee is still the
/// same object afterwards.
#[test]
fn a_hot_interpreted_caller_passes_any_arguments_to_a_compiled_method() {
    let src = format!(
        "{B}function keep(a: any, n: number): any {{ const b: any = a; const r = churn(n); if (r < 0) {{ return null; }} return b; }}
        let sum = 0;
        for (let k = 0; k < 40; k++) {{
            const o = new B(); o.v = k; o.w = k * 2;
            const back = keep(o, 2000) as B;
            sum += back.v + back.w;
        }}
        const n: any = keep(null, 10);
        const u: any = keep(undefined, 10);
        console.log(sum, n === null, u === undefined, keep(2.5, 10) as number, keep(true, 10) as boolean);"
    );
    let module = zun_compiler::compile_with_oxc(&src, true).expect("compiles");
    let loaded = zun_vm::load(&module.write()).unwrap();
    let mut plain = CaptureOutput::default();
    run_with(&loaded, &mut plain, None).expect("interpreter runs");
    let rid = loaded
        .tables
        .method_defs
        .iter()
        .position(|m| loaded.strings.get(m.name) == "keep")
        .map(|i| i as u32 + 1)
        .expect("keep");
    let jit = zun_codegen::Jit::new(&loaded).unwrap();
    assert!(jit.is_compiled(rid), "`keep` takes and returns an any and must be eligible");
    let hook = zun_vm::JitHook::with_threshold(jit, &loaded, 2);
    let mut hooked = CaptureOutput::default();
    run_with(&loaded, &mut hooked, Some(&hook)).expect("hooked run");
    assert_eq!(plain.lines, hooked.lines);
    assert_eq!(plain.lines, ["2340 true true 2.5 true"]);
    assert!(hook.jit_is_defined(rid), "`keep` must have run natively");
}

/// A compiled caller passes an `any` to an interpreted callee and gets one back
/// through the bridge's result carrier; the callee collects while the compiled
/// frame is suspended.
#[test]
fn a_compiled_caller_bridges_any_arguments_and_results() {
    // `describe` is declined by the JIT (it uses a class cast), so the call is
    // bridged to the interpreter.
    let src = format!(
        "{B}function describe(a: any): any {{ churn(300); if (a === null) {{ return 1.5; }} return (a as B).v; }}
        function caller(a: any, n: number): number {{
            const held: any = a;
            const r: any = describe(a);
            if (held === null) {{ return (r as number) + n; }}
            return (r as number) + (held as B).v + n;
        }}
        let total = 0;
        for (let k = 0; k < 200; k++) {{
            const o = new B(); o.v = k;
            total += caller(o, 1) + caller(null, 2);
        }}
        console.log(total);"
    );
    let (plain, jitted) = both_tiers(&src);
    assert_eq!(plain, jitted);
}

/// A loop that is OSR-compiled with an `any` local holding an object: the local
/// crosses into native code as two words, survives the moving collections of the
/// allocating callee the loop calls, and comes back out as the same object.
#[test]
fn an_osr_loop_keeps_an_any_local_across_a_collecting_call() {
    let src = "class B { v: number = 0; }
        function mk(i: number): number {
            const a = new B(); a.v = i;
            const b = new B(); b.v = a.v + 1;
            return b.v;
        }
        function main() {
            const keep = new B(); keep.v = 3;
            let held: any = keep;
            let nothing: any = null;
            let s = 0;
            let i = 0;
            while (i < 60000) {
                s = s + mk(i);
                if (held) { s = s + 1; }
                if (nothing === null) { s = s + 2; }
                i = i + 1;
            }
            console.log(s);
            console.log((held as B).v);
        }";
    let module = zun_compiler::compile_with_oxc(src, true).expect("compiles");
    let loaded = zun_vm::load(&module.write()).unwrap();
    let mut plain = CaptureOutput::default();
    run_with(&loaded, &mut plain, None).expect("interpreter runs");
    let jit = zun_codegen::Jit::new(&loaded).unwrap();
    let hook = zun_vm::JitHook::with_threshold(jit, &loaded, 2);
    let mut hooked = CaptureOutput::default();
    run_with(&loaded, &mut hooked, Some(&hook)).expect("hooked run");
    assert_eq!(plain.lines, hooked.lines);
    assert!(hook.osr_transfers() > 0, "the loop must have run natively");
}

/// `x as number` is unchecked in TypeScript, so a compiled `unbox.any; conv` can meet a
/// `boolean`. Tier 0 fails the run with the uncatchable `cannot convert Bool(true)`;
/// compiled code must fail the same way through the host, not fault the process
/// (`Illegal instruction`). Node answers `bad 4000` here: that divergence belongs to
/// Tier 0 and is recorded as a follow-up, not changed in this stage.
#[test]
fn a_mismatching_unbox_in_compiled_code_fails_the_run_like_tier0_and_does_not_fault() {
    let src = "function pick(x: any, i: number): number {
          let s = 0;
          for (let k = 0; k < 2000; k++) { s = s + (x as number) + i; }
          return s;
        }
        let t = 0;
        for (let i = 0; i < 300; i++) { t = t + pick(2, i); }
        console.log('warm', t);
        const bad: any = true;
        try { console.log('bad', pick(bad, 1)); } catch (e) { console.log('caught', String(e)); }
        console.log('after');";
    let module = zun_compiler::compile_with_oxc(src, true).expect("compiles");
    let bytes = module.write();
    let loaded = zun_vm::load(&bytes).unwrap();
    let mut plain = CaptureOutput::default();
    let plain_error = run_with(&loaded, &mut plain, None).expect_err("Tier 0 fails the run");
    let mut jitted = CaptureOutput::default();
    let jit_error = zun_vm::execute_with(&bytes, &mut jitted).expect_err("the compiled run fails the same way");
    assert_eq!(plain.lines, ["warm 90900000"]);
    assert_eq!(jitted.lines, plain.lines, "no `caught` (uncatchable) and no `after`");
    assert!(plain_error.to_string().contains("cannot convert Bool(true)"), "{plain_error}");
    // `VmError` prefixes the run error with its own label; the text after it is Tier 0's.
    assert!(jit_error.to_string().ends_with(&plain_error.to_string()), "{jit_error} vs {plain_error}");
}
