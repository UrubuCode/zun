//! A class reference may be `null` (`C | null`). Dereferencing it must raise the
//! same JS `TypeError` in the interpreter and in JIT-compiled code, where the
//! check is a cold branch that leaves the exception pending for the frame's
//! caller (or a catch in the frame) to pick up.

use zun_interp::{run_with, CaptureOutput};

/// Output of the program with no JIT, and with the bridged JIT at its default
/// tiering threshold (the methods are called enough times to compile).
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

const NODE: &str = "class N { v: number = 1; next: N | null = null; m(): number { return this.v; } }\n";

#[test]
fn a_null_dereference_raises_the_same_type_error_in_both_tiers() {
    let src = format!(
        "{NODE}
        function readV(p: N | null): number {{ return p!.v; }}
        function writeV(p: N | null): number {{ p!.v = 2; return 1; }}
        function callM(p: N | null): number {{ return p!.m(); }}
        function attempt(which: number, p: N | null): string {{
            try {{
                if (which === 0) return 'r' + readV(p);
                if (which === 1) return 'w' + writeV(p);
                return 'c' + callM(p);
            }} catch (e) {{ return (e as Error).message; }}
        }}
        const out: string[] = [];
        for (let i = 0; i < 12; i++) {{
            for (let which = 0; which < 3; which++) {{
                out.push(attempt(which, i % 2 === 0 ? null : new N()));
            }}
        }}
        console.log(out.join('|'));"
    );
    let (plain, jitted) = both_tiers(&src);
    assert_eq!(plain, jitted);
    assert!(plain[0].contains("Cannot read properties of null (reading 'v')"), "{plain:?}");
    assert!(plain[0].contains("Cannot set properties of null (setting 'v')"), "{plain:?}");
    assert!(plain[0].contains("Cannot read properties of null (reading 'm')"), "{plain:?}");
}

#[test]
fn a_null_dereference_inside_a_hot_loop_raises_in_both_tiers() {
    let src = format!(
        "{NODE}
        function drain(h: N | null, n: number): number {{
            let z = 0;
            for (let i = 0; i < n; i++) {{ z += (h as N).v; if (i === n - 2) h = null; }}
            return z;
        }}
        const out: string[] = [];
        for (let k = 0; k < 4; k++) {{
            try {{ out.push('ok' + drain(new N(), 5000)); }}
            catch (e) {{ out.push((e as Error).message); }}
        }}
        console.log(out.join('|'));"
    );
    let (plain, jitted) = both_tiers(&src);
    assert_eq!(plain, jitted);
    assert!(plain[0].contains("Cannot read properties of null (reading 'v')"), "{plain:?}");
}

#[test]
fn a_null_terminated_list_survives_collections_in_both_tiers() {
    let src = "class L { v: number = 0; next: L | null = null; }
        function build(n: number): L | null {
            let h: L | null = null;
            for (let i = 0; i < n; i++) { const x = new L(); x.v = i; x.next = h; h = x; }
            return h;
        }
        function len(h: L | null): number { let c = 0; let p: L | null = h; while (p !== null) { c++; p = p.next; } return c; }
        const out: number[] = [];
        for (let k = 0; k < 6; k++) out.push(len(build(3000)));
        console.log(out.join(','));";
    let (plain, jitted) = both_tiers(src);
    assert_eq!(plain, jitted);
    assert_eq!(plain, ["3000,3000,3000,3000,3000,3000"]);
}

/// Compiled code that is suspended in a bridged call into the interpreter keeps its
/// references: the interpreter allocates enough to run moving collections, and the
/// compiled caller must still find its objects (at their new addresses) afterwards.
#[test]
fn a_compiled_caller_survives_collections_run_by_a_bridged_callee() {
    let src = "class N { v: number = 0; next: N | null = null; }
        function churn(k: number): number {
            let s = 0;
            try { for (let i = 0; i < k; i++) { const x = new N(); x.v = i; s += x.v; } }
            catch (e) { s = -1; }
            return s;
        }
        function outer(i: number): number {
            const keep = new N(); keep.v = i;
            const other = new N(); other.v = i * 2; keep.next = other;
            const r = churn(400);
            return keep.v + (keep.next as N).v + r;
        }
        let total = 0;
        for (let k = 0; k < 4000; k++) total += outer(k);
        console.log(total);";
    let (plain, jitted) = both_tiers(src);
    assert_eq!(plain, jitted);
    assert_eq!(plain, ["343194000"]);
}

/// Compiled, bridged interpreter, compiled again: the innermost compiled code
/// allocates and collects while two activations of compiled frames are on the
/// stack. Each activation holds references of its own across the call below it.
#[test]
fn nested_activations_of_compiled_code_are_all_scanned() {
    let src = "class N { v: number = 0; next: N | null = null; }
        function leaf(i: number): number {
            const a = new N(); a.v = i;
            const b = new N(); b.v = a.v + 1; b.next = a;
            return (b.next as N).v + b.v;
        }
        function churn(k: number): number {
            let s = 0;
            try { for (let i = 0; i < k; i++) { s += leaf(i); } } catch (e) { s = -1; }
            return s;
        }
        function middle(i: number): number {
            const held = new N(); held.v = i * 3;
            const r = churn(300);
            return held.v + r;
        }
        function outer(i: number): number {
            const keep = new N(); keep.v = i; keep.next = new N();
            (keep.next as N).v = i + 5;
            const r = middle(i);
            return keep.v + (keep.next as N).v + r;
        }
        let total = 0;
        for (let k = 0; k < 3000; k++) total += outer(k);
        console.log(total);";
    let (plain, jitted) = both_tiers(src);
    assert_eq!(plain, jitted);
}

/// An OSR loop keeps a reference in its locals across a call that allocates, under
/// many moving collections. The loop must have transferred to native code.
#[test]
fn an_osr_loop_holds_references_across_a_collecting_call() {
    let src = "class N { v: number = 0; next: N | null = null; }
        function mk(i: number): number {
            const a = new N(); a.v = i;
            const b = new N(); b.v = a.v + 1; b.next = a;
            return b.v;
        }
        function main() {
            const keep = new N(); keep.v = 3;
            keep.next = new N(); (keep.next as N).v = 4;
            let s = 0;
            let i = 0;
            while (i < 60000) { s = s + mk(i) + keep.v + (keep.next as N).v; i = i + 1; }
            console.log(s);
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
