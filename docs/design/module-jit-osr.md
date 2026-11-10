# Module-entry JIT & OSR (design / epic)

Status: epic in progress. Slice 1 lands with this document.

## Problem

The JIT compiles functions/methods, not the synthetic module entry (`<main>`),
and eligibility is a per-method, whole-call-graph fixpoint
(`crates/zun_codegen/src/eligible.rs`). The module body almost always contains a
dynamic op, so the **entire top-level — including any hot loop in it — runs in
the Tier-0 interpreter**. Measured (`bench/`): an identical numeric loop runs
~297× slower at module top level than wrapped in a function.

In .NET even `Main` is JITed, and OSR tiers a hot loop up out of a frame that
started interpreted. We want the same, in slices.

## Current scaffolding (observation-only)

`crates/zun_vm/src/jit_hook.rs` already:

- counts per-loop-header backedges (`on_backedge`/`on_backedge_state`);
- when a loop is hot **and its method is already JIT-eligible**, captures an
  `OsrSnapshot` (pc, target_pc, stack, locals) and an `OsrRequest`, validated by
  `is_valid_osr_snapshot` (stack-empty backedge, patchpoint outside try/handler
  intervals, locals arity matches the body);
- keeps those snapshots GC-safe (`append_roots`, `rewrite_osr_snapshot_roots`).

Nothing consumes the snapshots: "the active frame continues in Tier 0 until
frame transfer is available." And it only engages for methods that are already
eligible — never `<main>`.

So two independent gaps: (A) a hot loop in an *ineligible* method cannot be
compiled; (B) there is no OSR transfer that enters native code at a loop header
with live state and returns.

## Why the top-level case does NOT need the hard OSR transfer

For a top-level numeric loop the only things keeping `<main>` ineligible are
dynamic ops that sit **outside** the loop body, plus one that the compiler puts
**inside** it:

- the per-write `globalSet` **mirror** of a top-level `let`/`const` to the realm
  store (emitted so a function can read the binding via `globalGet`) — this lands
  *inside* the loop when the loop writes the binding; and
- `console.log` / array+box for its arguments, *after* the loop.

If the mirror is elided when no function reads the binding (Slice 1), and the
residual entry builtins are made bridgeable/eligible (Slice 2), then `<main>`
becomes an ordinary eligible method and the existing pre-compile path
(`JitHook::with_threshold`) runs it native on its single invocation — **no OSR
transfer required**. OSR (Slices 3-5) is only needed for the harder case where an
ineligible op is genuinely *inside* a hot loop and cannot be elided.

## Slices (value front-loaded)

1. **Sound module-binding mirror elision (sema).** Mirror a top-level binding to
   the realm store only when some function/arrow/method body references its name.
   Analysis is whole-program and must be sound: an earlier ad-hoc span scan
   missed function *expressions* in top-level `let`s (reassigned top-level
   functions lower to a top-level `let f = function…`), nested functions, exports
   and `eval`, and silently broke 2 conformance fixtures. The sound rule: collect
   identifiers from (a) every declared function / method / symbol-method body
   span, and (b) every top-level statement whose source contains `function` or
   `=>` (so any function/arrow expression there is covered); if the module uses
   `eval(`/`new Function(`, keep every mirror. Over-inclusion only keeps an
   unnecessary mirror — never drops a needed one. **Gate: full `tests/**/*.ts`
   stdout diff against a pre-change baseline (normalise nondeterministic
   `Obj(0x..)` / panic TIDs) + `cargo test -p zun_sema`.**

2. **Make the module entry eligible.** Route the residual top-level builtins
   (`console.log`, the array/box for its args) through the JIT↔interp bridge
   (see `jit-interp-bridge.md`) or the eligible-builtin allowlist, so a top-level
   program with pure loops + I/O compiles end-to-end and the pre-compile path
   runs it native. Target: close the ~297× on `bench/compute.ts` / `methods.ts`
   with no OSR.

3. **OSR entry codegen (loop region).** Compile an OSR variant of a method that
   enters at a patchpoint, taking live locals as parameters. In-process
   equivalence tests only (`cargo test -p zun_codegen`), no transfer wiring.

4. **OSR transfer + return-to-interp.** Consume `take_valid_osr_snapshots`: call
   the OSR entry with the live frame state, and on loop exit hand updated locals
   back to the interpreter to finish the method. Reuses the existing snapshot /
   validity / root-rewrite scaffolding.

5. **Tiering policy & de-opt safety.** Thresholds, patchpoint placement, guards,
   and the interaction with the GC safepoint contract.

Slices 1-2 deliver the measured top-level win. Slices 3-5 are the true OSR
foundation for in-loop-dynamic cases, and are the part the PLAN defers next to
the call bridge.

## Invariants to preserve

- Interp (Tier 0) stays the always-correct fallback; interp and JIT must agree on
  the equivalence suite.
- `zun_codegen` must not depend on `zun_interp`; the OSR transfer seam lives in
  the VM, like the call bridge.
- No patchpoint inside a protected/handler interval (already enforced by
  `is_valid_osr_snapshot` / `is_osr_patchpoint_allowed`).
- Mirror elision must never drop a mirror a function (or `eval`) could read.
