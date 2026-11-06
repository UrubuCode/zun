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

- **OPEN — latent correctness bug (NOT a refactor artifact).** In
  `call_inner`, the `Opcode::Call` MemberRef/external path rebinds
  `let body = read_body(&callee.code, …)` (~lib.rs:2012), shadowing the current
  frame's `body`. When the callee returns `Err(RunError::Thrown(exception))`,
  the handler (~lib.rs:2031) calls
  `route_exception(&body, &mut stack, &mut pc, next - size, &exception)` using
  that **shadowed callee-module `body`**, so a thrown exception from a
  cross-module member call is routed against the *callee's* handler table and
  pc arithmetic instead of the current method's try/catch/finally. Every other
  call arm routes against the correct outer `body`. A real bug for cross-module
  calls that throw; needs its own `fix` commit + a regression test. The
  interpreter split must not "normalize" this away silently.
- **OPEN (cleanup)** — `fn value_object` (collections.rs:~8633) is a
  byte-for-byte duplicate of the `object` closure in the preamble of
  `call_managed_collection_inner` (~140). The collections split is the natural
  moment to converge on one; flagged, not changed.

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
- **OPEN (to audit)** — `zun_compiler → zun_loader`: the compiler uses
  `ModuleScope` / `ModuleManifest` / `ModuleLoader` to assemble its output. If
  `zun_loader` is a runtime crate this is a compile-time→runtime violation; the
  likely fix is to move the module-container contract types into the format
  layer (`zun_module` or a thin contract crate). Needs a decision before acting.
- **INTENDED (to document)** — `zun_vm → zun_compiler` is `optional = true`
  (feature-gated): the eval-capable runtime (PLAN §4.1/§5, "one eval-capable
  runtime"; `new Function`/eval compile at runtime). Keep it; document the
  exception in `zun_vm/Cargo.toml`.
