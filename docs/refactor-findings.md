# Refactor findings

Pre-existing issues surfaced during the pure-refactor reorganization (files →
submodules, giant-function splits). The refactor itself changes no behavior;
this file records defects found along the way so they are not lost. Each entry
is marked OPEN or FIXED (with the commit that fixed it).

Discovered 2026-11-06 during the `oxc.rs` and `zun_ilgen` reorganizations.

## crates/zun_syntax/src/oxc/ (the OXC → HIR adapter)

- **FIXED** — Duplicate, unreachable `Expression::Super` match arm in
  `convert_expression` (caused 3 "unreachable pattern" warnings).
- **FIXED** — Unreachable `_ =>` arms in the binary-operator and
  compound-assignment `AssignmentOperator` matches (all operators are already
  covered, so the catch-all is dead).
- **FIXED** — `convert_block_statement` is dead code (never called).
- **FIXED** — `prev_start = 0usize` is assigned and never read.
- **FIXED** — A doc comment ("Convert a single OXC class declaration…") sits on
  the `PRIVATE_SCOPES` `thread_local!` instead of on `convert_class_declaration`
  (caused an "unused doc comment" warning).
- **FIXED** (commit `8e259c2a`) — `static COMPUTED_FIELD_COUNTER`
  (class.rs), `static DESTRUCTURE_TEMP` (destructure.rs) AND `YIELD_STAR_COUNTER`
  (thread-local, mod.rs, never reset) generated synthetic temp/field names
  (`__cf`/`__ck`, `__zd`, `__fa_`/`__ys_`) that were **not deterministic** across
  multiple compiles in one process / across threads. Harmless for the
  conformance oracle (one program per process) but a latent bug for any
  multi-compile host (eval, a watch server). Fix: all three are now thread-local
  `Cell<u32>` (mirroring the existing `YIELD_STAR_COUNTER` style) and reset to 0
  at the top of `parse_subset` (next to the existing per-compile resets).
  Behavior-preserving on the single-compile path. A regression test
  (`synthetic_names_are_deterministic_across_repeated_compiles` in
  `tests/class_field_defaults.rs`) compiles source with a destructuring binding
  plus a computed class field twice and asserts the programs are identical.

## crates/zun_interp (interpreter)

- **FIXED (commit `09144837`)** — latent correctness bug (NOT a refactor
  artifact). In
  `call_inner`, the `Opcode::Call` MemberRef/external path rebinds
  `let body = read_body(&callee.code, …)` (~lib.rs:2012), shadowing the current
  frame's `body`. When the callee returns `Err(RunError::Thrown(exception))`,
  the handler (~lib.rs:2031) calls
  `route_exception(&body, &mut stack, &mut pc, next - size, &exception)` using
  that **shadowed callee-module `body`**, so a thrown exception from a
  cross-module member call is routed against the *callee's* handler table and
  pc arithmetic instead of the current method's try/catch/finally. Every other
  call arm routes against the correct outer `body`. A real bug for cross-module
  calls that throw. FIX: the shadowed binding is renamed `callee_body` (used
  only for its `param_count`); the `route_exception` call now routes against the
  current frame's outer `body`. Regression test
  `cross_module_member_call_throw_routes_to_caller_catch`
  (`zun_interp/src/method_tests.rs`) links two modules (`app` imports `core`),
  has `app.main` wrap a throwing `core.boom()` in try/catch, and asserts the
  catch runs (returns 7, prints 42). Fails before the fix (the throw escapes).
- **FIXED (commit `6f73b48b`)** — `fn value_object` (collections/runtime_iter.rs)
  was a byte-for-byte duplicate of the `object` free fn in `collections/mod.rs`.
  Converged on the single `pub(super) fn object` in `mod.rs`: the duplicate
  `value_object` is deleted and its callers (`runtime_iter.rs`, `generator.rs`,
  `array_construct.rs`) now call `object(...)`. Pure dedup, behavior-identical;
  covered by the existing green `zun_interp` suite.

## Goal phase status — verified against the tree at HEAD (2026-11-07)

- **Phase 1 (3 latent bugs): DONE.** Two have fail-before-fix regression tests
  (see the section below); the third (`value_object` dedup, `6f73b48b`) is a
  behavior-identical de-duplication with no dedicated test by design.
- **Phase 2 (native parser retirement): DONE & verified.** `crates/zun_syntax/src/`
  contains only `ast.rs`, `lib.rs`, and `oxc/`. A workspace-wide grep for
  `parse_native` / `native_parser` / `parser_helpers` finds nothing; the only
  `class_field_defaults` hits are an unrelated runtime state `HashMap` in
  `zun_interp` (class field default *values*), not the retired parser file.
- **Phase 3 (static monomorphization): PARTIAL.** `prove.rs` analysis +
  `specialize.rs` emission with a soundness gate are in; the metric is now
  reproducible (two committed harnesses). The remaining lever, partial
  specialization (relax `BailReason::AnyArgument`), is proposed and **awaiting
  user approval** — it is the only un-gated path left that moves the metric.
- **Phase 4 (typed Array/Map/String floor): SLICE A done.** Shared string core
  `crates/zun_core/src/string_ops.rs` (12 pub fns) landed (commit `9c975def`).
  Slices D (GC kind-tag) and E (string representation) are **deferred by the
  user** (touch the GC object model / frozen string layout).
- **Phase 5 (Tier0 type feedback): foundation done.** `crates/zun_interp/src/
  feedback.rs` (12 pub fns) hooked on the slow/miss path in
  `collections/dynamic_call.rs`, env-gated (commit `d665ab1e`). Further hardening
  is downstream of the phase-6 decision.
- **Phase 6 (speculative Tier1 + deopt): surveyed read-only only.** Needs user
  decisions on the feedback→JIT flow and the bridge Any-carrier ABI (JIT ABI is
  a conditioned category).
- **Option B (metadata marking + verifier pairing for phase 3): proposed,
  awaiting approval** (touches the `.zm` contract).

Everything marked awaiting/deferred is gated on an explicit user decision under
the goal's propose-before-change conditions for the `.zm`/JIT/GC/string-layout
categories; none can be done unilaterally.

## Phase 3 partial specialization — IMPLEMENTED, flag-gated off (2026-11-07)

Approved by the user with conditions. Landed in commits `6bbc2bf0` (sema + gates)
and `476d43b3` (`zun prove --partial`).

- **What it does.** Relaxes the all-or-nothing `BailReason::AnyArgument`: a
  parameter that is one concrete type at some intra-module call sites and `any`
  at others becomes a candidate. The concrete sites route to the typed twin; the
  `any` sites keep calling the generic. Sound without a guard — `route_call`
  already requires an exact per-site type match, so an `any` argument never
  selects a concrete twin, and the generic always remains.
- **Flag.** `ZUN_PARTIAL_SPEC` env (mirrors `ZUN_FEEDBACK`), **off by default**.
  `check_specialized_partial` / `prove_program_with(.., true)` /
  `zun prove --partial` force it on for tests and measurement. No `.zm` change:
  the twin is an extra MethodDef, verifier pairing via the existing mechanism.
- **Before/after (prelude scale, `prove_prelude_scale.rs`).** partial OFF:
  candidates=5, any-argument bails=8. partial ON: candidates=12 (+7), all 8
  any-argument bails cleared, 1 became no-concrete-param; the 26 any call sites
  stay on the generic (that is `any` remaining the expensive exception).
- **Emitted vs candidates.** Over the raw prelude corpus, emission is 0: the 5
  non-partial candidates are already fully typed (skip), and the +7 partial
  candidates are rejected by the conservative emission gate on library bodies.
  Emission + routing is validated end-to-end by the sema gate test (one twin
  emitted, concrete site routed, `any` site kept on the generic).
- **Gates (all green).** 8 sema gate tests (positive route-split + negative
  soundness: read-as-value, closure capture, reassignment rejected, default
  param, eval/`new Function`, dynamic import); interp==JIT equivalence with the
  flag ON 221/221; round-trip + verifier load of a specialized module
  (byte-stable); hot-path bench unaffected (flag off → hot path byte-identical).
- **Pending to flip the default:** the user wants the compile-path emitted count
  and conformance re-checked once there is a corpus that actually emits twins
  (library bodies do not); phase 6 is explicitly deferred until phase-5 feedback
  numbers are gathered.

## Conformance baseline correction: 83/167, not 84 (2026-11-07)

Measured with the current `target/debug/zun.exe` via `scripts/tests_check.sh`
(`JOBS=2`): **83 pass, 1 Zun-divergent**, identical with `ZUN_PARTIAL_SPEC` on
and off. The divergent fixture is `tests/streams/web-streams-encoding.ts`
(`UnhandledPromiseRejection` instead of the stream output); it fails the same way
with the flag OFF and when run standalone, so it is a **pre-existing,
flag-independent** regression, not caused by partial specialization. The earlier
"84/167" figure predates this binary. OPEN (out of scope here): triage
`web-streams-encoding` separately.

## Phase-1 regression tests — fail-before-fix demonstrated (2026-11-07)

The two phase-1 bugs with dedicated regression tests were verified to fail
against their pre-fix code and pass once restored, non-destructively (revert the
code hunk on the working tree, keep the HEAD test, run, restore — no commit, no
worktree):
- `typed_array_single_any_arg_is_checked_once` (zun_sema): against the pre-fix
  `new_realm.rs` (`4d88c756^`) the arg-count diagnostic is emitted TWICE
  (`left: 2, right: 1`); passes at HEAD.
- `cross_module_member_call_throw_routes_to_caller_catch` (zun_interp): with the
  `body`/`callee_body` shadow reintroduced on HEAD `lib.rs`, the cross-module
  throw escapes the caller catch (`Thrown(..)` propagates); passes at HEAD.
The third phase-1 item (`value_object` dedup, `6f73b48b`) is a behavior-identical
de-duplication with no dedicated test by design (covered by the existing suite),
so it has no fail-before-fix to show.

## crates/zun_sema (checker)

- **FIXED (commit `4d88c756`)** — in `check_new`'s realm BufferView path
  (`check/expr/new_realm.rs::new_ctor_buffer_view` / `new_typed_single_arg`),
  `new <TypedArray>(x)` with a single `any`-typed argument checked that argument
  with `self.check_expr` TWICE: once in the numeric/array single-arg probe (which
  falls through when the arg is neither), then again in the tracking-view path.
  FIX: `new_typed_single_arg` now returns `Result<Expr, Expr>`, handing back the
  already-checked argument as `Err(checked)` on fall-through; the caller reuses
  it instead of re-checking `args[0]`. Regression test
  `typed_array_single_any_arg_is_checked_once` (`check/tests.rs`) checks
  `new Uint8Array(take())` where `take()` is `any`-typed and emits one arg-count
  diagnostic per check, asserting the diagnostic is emitted exactly once. Fails
  before the fix (emitted twice).

## Pre-existing clippy/rustc baseline warnings (not introduced by the refactor)

These are the warning baseline; the refactor must not add to it. Listed so a
later cleanup pass can clear them deliberately.

- `zun_core`: a type "does not implement `Debug`" (missing_debug_implementations).
- `zun_sema`: `unused variable: init`.
- `zun_ilgen`: `too_many_arguments` on `push_protected`; `byte_char_slices` in
  `program.rs`.
- `zun_syntax`: `parser_helpers.rs` warnings and a `class_field_defaults` test
  warning (native-parser path, transitional).

## Repo-wide hygiene

- **OPEN** — The repository is **not `cargo fmt`-clean** at baseline. Running
  `cargo fmt --all` reformats many unrelated files, so `cargo fmt --all -- --check`
  cannot be used as a per-PR gate until a dedicated, isolated "format the whole
  repo" commit lands. Until then, refactor commits preserve each file's existing
  style.
- **Note** — This machine FREEZES on concurrent cargo builds and on
  `cargo {test,build,clippy} --workspace`; validation is done per crate
  (`-p <crate>`), one build at a time. See CLAUDE.md / the build-limits memory.

## Workspace dependency boundary (CLAUDE.md rule)

- **FIXED** — `zun_codegen` declared `zun_compiler` as a normal `[dependencies]`
  entry but only used it in `tests/equivalence.rs`; moved to `[dev-dependencies]`
  (commit `7799abf0`), removing a runtime→compile-time edge from the build graph.
- **RESOLVED (documented exception)** — `zun_compiler → zun_loader`: the audit
  found this is NOT a strict CLAUDE.md breach. `zun_loader` holds the module
  *container contract* (`ModuleScope`/`ModuleManifest`/`ModuleLoader`/`Export`,
  pure data/registry over the format crates) AND both the compiler and the VM
  (`zun_interp`/`zun_vm`) depend on it — so it is a shared lower layer, not a
  compile↔runtime cross-edge. The only cost is cohesion: the compiler
  transitively links `zun_loader`'s runtime IO/crypto (ed25519/sha2/ureq).
  DECISION: document the exception (chosen) rather than create a new crate —
  priority 5 gates a new crate on a measured `cargo build --timings` win, which
  this machine cannot run safely (workspace/concurrent builds freeze it). The
  OPTION, if that win is ever measured: extract a format-layer `zun_scope` crate
  (ModuleScope/Manifest/Loader-container/Export + byte helpers; deps
  zun_metadata+zun_module+zun_types only), leaving fetch/trust/index/Zmlib in
  `zun_loader`, so the compile-time graph stops pulling crypto/network. For now
  `zun_loader/src/lib.rs` is split into cohesive submodules in-place (no new
  crate), which also realizes the scope/fetch boundary at the file level.
- **INTENDED (to document)** — `zun_vm → zun_compiler` is `optional = true`
  (feature-gated): the eval-capable runtime (PLAN §4.1/§5, "one eval-capable
  runtime"; `new Function`/eval compile at runtime). Keep it; document the
  exception in `zun_vm/Cargo.toml`.

## cargo fmt convergence (diagnosed)

`cargo fmt --all -- --check` does not pass. CORRECTION: this is NOT a rustfmt
limitation — the content is ordinary formattable code (e.g.
`zun_interp/src/collections/array_iterate.rs` just needs normal line-wrapping,
no string-literal/idempotency problem). The real blocker is SCOPE: the gate
requires the ENTIRE repo (~347 files) to be fmt-clean, and the repo was never
fmt-clean to begin with (hundreds of pre-existing diffs in files this refactor
never touched). Making it pass means a repo-wide `cargo fmt --all` reformat
commit — behavior-neutral and reversible, but a large change unrelated to the
"reorganize in small steps" mandate and not requested. It is available on
request; the few files that needed a second `cargo fmt` pass (the 5 new/extracted
ones) format cleanly when run through rustfmt directly.

## Benchmark baseline (post-refactor, recorded)

`cargo run --release --example bench -p zun_codegen` (the interpreter/JIT bench):
- interpreter Tier 0 arithmetic loop: 264.9 ns/iter; JIT Tier 1: 2.34 ns/iter
- i32 Map mutation loop: interp 659.7 ns/iter; JIT 36.4 ns/iter
- field load/store loop (Tier 0): 266.6 ns/iter
Matches the pre-refactor call_inner baseline (~256-265 ns/iter) — no hot-path
regression from the interpreter/collections/translate splits.

## Workspace validation baseline (recorded)

The machine freezes on concurrent/workspace cargo builds, so validation was done
the only safe equivalent way — per crate, serially, one build at a time:
- `cargo test -p <crate>` for ALL 23 workspace crates: 23/23 GREEN (serial sweep).
- `cargo clippy -p <crate> --lib/--all-targets` per touched crate: no NEW warnings
  vs each crate's measured baseline (the repo has hundreds of PRE-EXISTING
  warnings, so a literal `--workspace -D warnings` cannot pass regardless).
- `scripts/tests_check.sh` (cross-runtime conformance oracle): 84/167 pass,
  re-run and unchanged after every refactor step.
- Interpreter/JIT equivalence suite (zun_codegen): 373 pass, unchanged.
- Hot-path benchmark: see the baseline section above (no regression).
`cargo {test,clippy,build} --workspace` and `--timings` are intentionally NOT run:
they build many crates concurrently and freeze this machine (see the build-limits
note). The per-crate serial results above are the faithful, safe equivalent.

## Workspace clippy + full test run (machine free, recorded)

Run with the machine idle (it only freezes under concurrent load):
- `cargo test --workspace`: GREEN, except the 5 `zun_cli/tests/aot.rs`
  interpreter-fallback tests, which link `zun_vm.lib` and have a guard that fails
  if that artifact is older than the runtime sources. `--workspace` does not
  rebuild the standalone `.lib` in the right order. Fix/flow: run
  `cargo build -p zun_vm` before `cargo test -p zun_cli` (then aot = 20/20). Not
  a regression — a build-artifact ordering quirk.
- `cargo clippy --workspace --all-targets`: 0 ERRORS, ~442 warnings, ALL
  pre-existing (zun_interp 231, zun_sema 36, zun_vm 32, zun_codegen 22, zun_gc 19,
  …: missing `# Safety` docs, too_many_arguments, etc.). This refactor introduced
  no new warnings (verified per crate vs baseline). `-- -D warnings` therefore
  cannot pass without a SEPARATE lint-cleanup pass over the whole codebase
  (~442 fixes, many via `clippy --fix`), which is out of scope for a
  behavior-preserving reorganization.
- `cargo fmt --all -- --check`: CLEAN (commit 124232b1).

## Phase 3 primary metric (`zun prove`) is not reproducible — OPEN

- **OPEN (process/evidence gap, not a code defect)** — The goal's primary
  metric ("`any` becomes the expensive exception") was tracked in-session as
  `zun prove` reporting ~653 any-argument call sites, ~231 monomorphization
  candidates, and 66 emitted static specializations. Those numbers were produced
  by ad-hoc `zun prove` runs over a prelude-prepended corpus during the session;
  there is **no committed harness** that reproduces them, so the baseline is lost
  on memory/context clear and cannot be regression-gated per phase.
- The clean, un-gated fix is a small committed measurement test that runs
  `prove_program` over a fixed corpus and asserts the counts, so each phase can
  show the number moving. **DONE** — two committed harnesses:
  `crates/zun_compiler/tests/prove_baseline.rs` (a small hand-derivable corpus,
  exact verdicts) and `crates/zun_compiler/tests/prove_prelude_scale.rs` (the
  whole prelude, structural invariants + printed snapshot).
- **Correction to the in-session metric.** The "653 any-argument sites / 231
  candidates / 66 specializations" figure quoted during the session could NOT be
  reproduced: no committed corpus or path produces it, and its exact input is
  unknown. The honest, reproducible prelude-scale snapshot (`with_prelude("")`,
  all top-level functions analyzed) is:
  `total=40, candidates=5, generic=35, any_call_sites=26`, with bail histogram
  `no concrete parameter=24, any-typed argument=8, async=2, spread/default=1`.
  So at prelude scale the partial-specialization lever (the 8 `any-typed
  argument` bails) could convert up to 8 more functions; the 24 `no concrete
  parameter` bails are functions with no intra-module callers and are not
  addressable by partial specialization. The 653/231/66 numbers should be
  treated as retired/unverifiable, not as a live baseline.
- Known lever to move the metric, within the approved phase-3 envelope but
  beyond its literal "all call sites agree" rule (so it needs explicit approval,
  per the goal's propose-before-change conditions): **partial specialization** —
  relax the all-or-nothing `BailReason::AnyArgument` in `prove.rs` so a parameter
  that sees one concrete type at some static sites and `any` at others still gets
  `f$T(T)`, routing only the statically-`T` sites to the twin and leaving the
  `any` sites on the generic. Sound without a guard (static per-site dispatch;
  generic remains the fallback); stays entirely in `zun_sema` (prove.rs +
  specialize.rs routing); does not touch `.zm`, JIT, GC, or the string layout.

## Workspace validation — current at HEAD 35338991 (recorded)

The per-crate serial baseline in the sections above was re-confirmed at the
current HEAD (35338991): 23/23 crates `cargo test -p <crate>` green,
`cargo test -p zun_cli` 20/20 after `cargo build -p zun_vm` (the stale-`.lib`
guard, not a regression), conformance 84/167 unchanged, interp==JIT equivalence
373 unchanged, `cargo fmt --all -- --check` clean, hot-path bench ≈ baseline
(JIT arith ~2.3 ns). Remaining goal phases (4 typed floor completion, 5 feedback
hardening, 6 speculative Tier1, Option B metadata marking, slices D/E) are gated
on explicit user design decisions on the conditioned categories and are not
started.

## crates/zun_natives string typed floor (found during phase 4 slice A)

- **OPEN (latent, likely non-spec)** — the TYPED `zun:string` floor errors
  ("index N is out of bounds") for `charCodeAt`/`codePointAt` with an
  out-of-range index, while JS/Node returns `NaN` / `undefined` (the dynamic
  `any` surface already returns NaN/undefined correctly). Only reachable on a
  statically-typed string receiver. Preserved as-is by the surface convergence
  (Slice A); needs a spec-correctness decision + fix (make the typed floor
  return NaN/undefined) with a regression test.

## crates/zun_sema number interceptor overlap (found during table-driven eval)

- **OPEN (latent, order-dependent)** — `mcall_number_a` and `mcall_number_b`
  (crates/zun_sema/src/check/expr/mcall_number.rs ~:37 vs ~:133/:175) both match
  the same `(F64, name)` keys for `toExponential`/`toPrecision`/`toFixed`; only
  the first-match ORDER makes `_a` win. A reorder would silently change behavior.
  When the ordered method_call chain is later table-driven, add a duplicate-key
  `debug_assert` and a test pinning this precedence (and the array-before-managed
  exclusion in array_method_descriptor).
