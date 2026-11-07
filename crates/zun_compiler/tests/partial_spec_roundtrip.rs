//! Round-trip + verifier gate for phase-3 partial specialization.
//!
//! A partial specialization is an extra `MethodDef` in the same module (no `.zm`
//! format change). This test compiles a program whose only route to a typed twin
//! is partial specialization, serializes the module, reads it back, and confirms
//! the bytes round-trip exactly and the module loads through the verifier. It
//! runs under both flag states: the round-trip fidelity must hold regardless,
//! and when `ZUN_PARTIAL_SPEC` is on a twin must actually be present (so the
//! round-trip genuinely covers a module carrying a specialization).

use zun_module::Module;

const SRC: &str = "function g(x: any): number { return x + 1; } \
     function main(): void { const a = 2; print(g(a)); const d: any = a; print(g(d)); }";

fn partial_enabled() -> bool {
    std::env::var_os("ZUN_PARTIAL_SPEC")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

#[test]
fn partial_specialized_module_round_trips_and_loads() {
    let module = zun_compiler::compile(SRC).expect("compiles");
    let method_count = module.tables.method_defs.len();

    // Round-trip the raw bytes through the format layer.
    let bytes = module.write();
    let decoded = Module::read(&bytes).expect("specialized module must read back");
    assert_eq!(
        decoded.tables.method_defs, module.tables.method_defs,
        "method defs (specializations included) must round-trip byte-identically",
    );
    assert_eq!(
        decoded.write(),
        bytes,
        "re-serialization must be byte-stable",
    );

    // Loading through the module scope runs the verifier; a specialized module
    // must pass it (the twin pairs with its generic via the existing MethodDef
    // mechanism, no new table).
    zun_loader::ModuleScope::from_bytes("partial_rt", &bytes)
        .expect("specialized module must load through the verifier");

    if partial_enabled() {
        // g, main, module-init = 3; a partial twin of `g` makes 4.
        assert!(
            method_count >= 4,
            "with the flag on a partial twin must be present (method_defs={method_count})",
        );
    }
}
