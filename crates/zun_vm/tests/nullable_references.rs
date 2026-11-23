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
