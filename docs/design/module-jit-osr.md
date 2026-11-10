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

## Why the top-level case needs OSR (revised after measuring)

Slice 1's hope was that eliding the `globalSet` mirror would make `<main>`
eligible once a residual `console.log` was bridged. A disassembly of
`bench/compute.ts`'s `<main>` *after* slice 1 disproves that: with the mirror
gone the hot loop body is pure `f64` (eligible), but the rest of the entry is
full of the **intentionally-costly dynamic path** the typed JIT does not model —
`ConstNull` (synthesised static inits at the top), and `newarray` + `box.any` +
the dynamic call for `console.log`'s argument at the bottom. Those are the same
`NewDynamic`/`Load/StoreDynamic`/`ConstNull`/`box`/`Any` opcodes listed as a
*deferred foundation* in the JIT-eligibility notes, not a single bridgeable call.

So making the **whole** `<main>` eligible means building the dynamic-path JIT —
a large, separate effort. The realistic path to the top-level win is therefore
**OSR of the eligible hot loop region alone**: compile just the loop, transfer
in at its header and back out at its exit, and leave the dynamic head/tail of the
entry (static inits, `console.log`) running in the interpreter. The loop region
of `bench/compute.ts` is pure `f64` and is eligible as-is.

Slice 1 still stands on its own: it removes a per-iteration dynamic `globalSet`
from top-level loops (measured ~2.4× on `compute.ts`, interpreter-only) and is a
precondition for the loop region being eligible at all.

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

2. **Loop-region eligibility analysis.** Given a method and a backedge target
   (loop header) PC, compute the loop region and decide whether every opcode in
   it is JIT-lowerable — independent of the ineligible dynamic ops in the
   method's head/tail. This is the gate the OSR compiler consumes. Pure
   analysis, unit-tested (`cargo test -p zun_codegen`), no Cranelift, no risk.
   **(this slice)**

3. **OSR entry codegen (loop region).** Compile a native function that enters at
   the patchpoint, takes the live locals as parameters, runs the eligible loop,
   and returns the updated locals at loop exit. In-process equivalence tests
   only; no transfer wiring yet.

   **Known blocker (found while scoping slice 3).** `translate::translate_body`
   builds the function with a single linear model: an entry block that defines
   the method *params* and jumps to IL block 0, then `Translator::run` lowers
   instructions 0..N in order, lazily threading the operand stack across each
   block edge by *fallthrough* and discovering each block's parameter signature
   on the first edge into it. An OSR entry has to (a) take *every* local as a
   parameter, not just the method params, and (b) jump from the entry straight
   to the loop-header block mid-body. That mid-body jump gives the header block
   an extra predecessor whose argument list is fixed *before* `run` has
   discovered the block's lazy parameter signature, so a naive
   `jump(header, &[])` risks an arg-count mismatch against params `run` appends
   later (it is only safe at a provably stack-empty header, which
   `is_valid_osr_snapshot` already requires). Slice 3 therefore needs `run` to
   support a distinct entry point (seed the all-locals Variables, pin the
   header's parameter signature to empty up front, and leave IL block 0
   unreachable) rather than a localized prologue tweak. This is a real
   translator change, not a quick edit — do it behind the equivalence harness
   on a method whose body *from the patchpoint* is fully eligible first (OSR to
   method end, the .NET model), and only then add the return-to-interp exits
   (slice 4) for the ineligible-tail case the module entry needs.

4. **OSR transfer + return-to-interp.** Consume `take_valid_osr_snapshots`: call
   the OSR entry with the live frame state, and on loop exit hand updated locals
   back to the interpreter to finish the method (the dynamic head/tail of
   `<main>` keeps running interpreted). Reuses the existing snapshot / validity /
   root-rewrite scaffolding.

5. **Tiering policy & de-opt safety.** Thresholds, patchpoint placement, guards,
   and the interaction with the GC safepoint contract.

Slice 1 delivers an interpreter-only speedup and unblocks the loop region.
Slices 2-5 are the true OSR foundation (the part the PLAN defers next to the
call bridge) and are what delivers the top-level win.

## Invariants to preserve

- Interp (Tier 0) stays the always-correct fallback; interp and JIT must agree on
  the equivalence suite.
- `zun_codegen` must not depend on `zun_interp`; the OSR transfer seam lives in
  the VM, like the call bridge.
- No patchpoint inside a protected/handler interval (already enforced by
  `is_valid_osr_snapshot` / `is_osr_patchpoint_allowed`).
- Mirror elision must never drop a mirror a function (or `eval`) could read.
