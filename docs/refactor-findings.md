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

## Phase 5 feedback census over the conformance corpus (2026-11-07)

Item 2 of the user's decision: wire the Tier0 feedback into the corpus and
report how polymorphic the residual dynamic `any` is, to decide phase 6. Method:
ran every `tests/**/*.ts` fixture through `zun prove --runtime` (feedback on),
aggregated each recorded dynamic `any` call-site as monomorphic (every
receiver/arg histogram has one distinct tag) vs polymorphic (any histogram has
>1 tag or overflowed). Measured on the default compile path (partial spec off;
emission over these small/library programs is ~0 either way, so the flag does
not move the dynamic picture).

- fixtures: 85 ran (82 error/timeout — the same ones that fail conformance).
- **dynamic any call-sites: 1388 total — 1377 monomorphic (99.2%), 11
  polymorphic (0.8%).**
- **dynamic any calls observed: 311,980** (the residual `any` dispatch volume).
- The Rust hot-path microbenches are fully-typed loops with no dynamic `any`,
  so they record nothing — the typed floor already carries them.

Phase-6 reading: residual `any` dispatch is overwhelmingly **monomorphic at
runtime** (99.2% of sites, and the 311,980 calls are dominated by them). A
speculative Tier1 monomorphic specialization with a cheap guard + deopt would
therefore hit its fast path almost always (guards rarely fail), which is a
strong signal that phase 6 is worth doing. Decision is the user's.

## Typed floor is slow — two root causes, measured (2026-11-07)

Triggered by the phase-5/6 time study. A "typed" `const s: string;
s.charCodeAt(i % 26)` loop ran at ~11 µs/iter in debug; release 5-run medians
(startup subtracted via a 0-iteration twin) isolate two independent defects.

Release medians (2M iterations, `target/release/zun.exe`, startup ≈ 63 ms):
- arith loop (`acc += i % 26`) **in `main`**: ~304 ns/iter.
- same arith loop **in a called function `hot()`**: **~6.4 ns/iter** (JIT speed).
- `s.charCodeAt(i % 26)` in `main`: ~987 ns/iter; in `hot()`: ~1003 ns/iter.
- `any` `s.charCodeAt`: ~1151 ns/iter.

**Root cause 1 — module entry (`main`) is not JIT-compiled.** The identical hot
loop runs ~48× faster in a called function than in `main` (6.4 vs 304 ns/iter),
so top-level/module-entry code runs on the Tier0 interpreter. Hot work written
directly in `main` never tiers up.

**Root cause 2 — `String.charCodeAt` has no typed JIT path for a realistic
index.** Even in a JIT-able function and with a statically-typed `string`
receiver, `charCodeAt` lowers to the dynamic `call.builtin 354`
(`collections::stringMethod`, member name pushed as a string, result boxed then
unboxed) — ~1000 ns/call regardless of container. Reason
(`check/expr/mcall_string.rs:135`): the typed native (`managed_methods.rs:152`,
index `U64`) is used only when the index `is_int()`, but loop counters and
arithmetic are `f64` (number), and even a bitwise `i & 15` is `conv`-ed back to
`f64` before the call. It is intentional (comment lines 133-134: the integer
native cannot express JS out-of-range NaN/undefined — the same open typed-OOB
finding). Net effect: typed char-access is only ~19% faster than `any` because
both take the same dynamic path; the typed string floor is effectively absent
for char-access.

**Decision impact.** Per the user's rule, the typed floor must be fast before
phase 6 is worthwhile: phase 6 would speculate a guard onto a "typed" path that
today is itself dynamic. Priorities surfaced (not implemented): (1) make
module-entry/`main` JIT-eligible or move its hot body onto a tierable method;
(2) give `String.charCodeAt`/`charAt`/`codePointAt` a typed JIT path for `f64`
indices (coerce + bounds-check producing JS NaN/undefined), so a typed string
receiver leaves the dynamic `stringMethod` builtin. Measurement programs live in
the session scratchpad; nothing here was committed as a fix.

## Tiering/OSR + interpreter base cost — deep diagnosis (2026-11-07)

Follow-up time study (release, 5-run medians, startup subtracted). Per-iteration
over 2M, `acc += i % 26`:
- in a called function `hot()`: **~6.4 ns/iter** (Tier1 native).
- in `main`, eligible (no `print`): **~438 ns/iter** (interpreter).
- in `main` with a trailing `print`: ~304 ns/iter (interpreter).
- `hot()` with a `print` *inside* the loop: ~2160 ns/iter (interpreter).

**Finding 1 — OSR (interp→JIT) is not wired into the run path.** The machinery
exists: the interpreter calls `hook.on_backedge_state` on every back-edge
(`lib.rs:976-1015`), `JitHook` keeps per-loop back-edge counters with
`DEFAULT_THRESHOLD = 2` and builds `OsrRequest`/`OsrSnapshot`s, and codegen has
`zun_jit_osr_poll`. But the only callers of `take_osr_requests` /
`take_osr_snapshots` outside `#[cfg(test)]` are tests — nothing in
`run_with_eval_host` / `execute_*` drains or applies them. So a method that
starts on the interpreter never transfers to native mid-loop. The entry method
`main` is always interpreted, so a hot loop written at top level never tiers up
(~438 vs ~6.4 ns, ~68×). `zun_jit_osr_poll` only replenishes the counter of a
loop already running in Tier1. This matches the predicted "separate function
fast, main slow ⇒ missing OSR."

**Finding 2 — a non-eligible `CallBuiltin` disqualifies the whole method.**
`eligible.rs:115` only admits `CallBuiltin` for date/math/collection/allocation/
string builtins; `print` is none, so any method containing `print` (or other IO)
is JIT-ineligible and runs entirely interpreted (hot+print = ~2160 ns/iter).

**Finding 3 — interpreter base cost ≈ 20-23 ns/opcode.** The arith loop body is
pure f64 (ldloc/const.f64/cmp/br/rem.f64/add.f64/dup/stloc/pop), ~18-20
opcodes/iter, with **no** box/unbox, allocation, name lookup, or Value clone.
The cost is structural (`lib.rs:554-564`): every instruction re-runs
`decode_one(&code[pc..])` (byte re-parse, no decoded cache) plus two frame
bookkeeping writes (`active_frame`, `call_stack` `frame.pc`) before the match;
each back-edge also runs `poll_gc` + the hook. Re-decode + bookkeeping dominate.

**Finding 4 — charCodeAt under JIT (in a JIT-able `hot()`):** typed-declared
string ~1003 ns/iter vs `any` ~2766 ns/iter (~2.75×). Both are dominated by the
non-inlined dynamic `call.builtin 354` (`stringMethod`) with box/unbox; the typed
U64 intrinsic is never used for an f64 index, so even "typed under JIT" is ~1 µs.

**Evidence-backed priority (user's provisional order, not implemented):**
1. Get the JIT into loops: wire interp→JIT OSR (machinery exists, unused) and/or
   compile + natively enter the entry; revisit the `print`/CallBuiltin cliff.
2. Interpreter base cost: pre-decode/cache instructions, trim per-op bookkeeping.
3. Typed string intrinsics for f64 indices (proposal A).
4. Int narrowing (proposal B).
5. Phase 6 only after the above.

## Typed-floor / JIT defects — enumerated, with two measurements (2026-11-07)

### The four defects (file:line + numbers)
- **D1 — OSR interp→JIT is not applied in the run path (only in tests).** The
  interpreter raises back-edges (`zun_interp/src/lib.rs:976-1015`
  `on_backedge_state`), `JitHook` counts them (`zun_vm/src/jit_hook.rs:23`
  `DEFAULT_THRESHOLD=2`, `:50-52` request/snapshot buffers) and GC can rewrite
  the snapshot roots (`:148` `take_valid_osr_snapshots`), but the only callers of
  `take_osr_*` outside `#[cfg(test)]` are tests. Result: the entry `main` and any
  method that starts interpreted never tier up mid-loop. Same arith loop: ~6.4
  ns/iter in a called function vs ~438 ns/iter (eligible) / ~304 ns/iter in
  `main` (~48-68×).
- **D2 — a non-eligible `CallBuiltin` disqualifies the whole method.**
  `zun_codegen/src/eligible.rs:115` admits `CallBuiltin` only for string/
  collection/math/date/allocation builtins; everything else (print, the
  dynamic-`any` core ops, class setup) makes the method JIT-ineligible.
- **D3 — `charCodeAt` (and the char-access family) with an f64 index falls into
  the dynamic `stringMethod` builtin (354).** `zun_sema/src/check/expr/
  mcall_string.rs:135` gates the typed native on `is_int()`; loop counters and
  arithmetic are f64, and even `i & 15` is `conv`-ed to f64, so char-access on a
  statically-typed string takes the same dynamic path as `any` (~1 µs/call).
- **D4 — the interpreter re-runs `decode_one` per instruction, no pre-decode.**
  `zun_interp/src/lib.rs:554-562`: each executed instruction re-parses its bytes
  and writes two frame-bookkeeping fields before the match. Base cost ≈ 20-23
  ns/opcode on a box/alloc-free arith loop.

### Measurement A — why builtin 354 (`stringMethod`) costs ~1 µs even under JIT
Path: JIT does not inline it — it is an out-of-line native call. Inside
(`zun_interp/src/collections/string_num.rs:140-155` →
`zun_interp/src/string_dyn.rs:124`): per call it unboxes the receiver `Any`,
**clones the member-name string** (`name.clone()`), **dispatches by string-name
`match`** over all String methods, clones the args, builds a fresh `Value::Str`,
and **boxes the result into the GC heap** (`box_any`, an allocation); the caller
then `unbox.any` + `conv`s. A typed intrinsic (direct charCodeAt on a known
string, f64 index, raw f64 result) would be a few ns inline with none of this.

### Measurement B — JIT eligibility census (bridged) over the corpus
105 fixtures compiled; 45,924 methods-with-body (inflated: the prelude links into
every module — read the ratios, not the absolute counts):
- eligible 4,825 (**10.5%**); rejected 41,099 (89.5%).
- rejected carrying a `CallBuiltin`: 27,141 (66%); throw/rethrow 3,231; castclass
  654; callindirect 419.
- rejected with a `CallBuiltin` and **no** throw/cast/indirect (D2 upper bound):
  **23,390 (~57% of rejected)**.
- top blocking builtins in rejected methods: `addAny`, `globalSet`,
  `dynamicCall0/1`, `classSetFieldDefault`, `realmDescriptor`, `typeofAny`,
  `looseEquals`, `numberCoerce`, `stringCoerce`, `classSetMethod` — the
  dynamic-`any` core ops and class setup. So D2 is the single largest eligibility
  lever. (Measurement: `zun_codegen/tests/zz_eligibility_census.rs`, ignored.)

### Design proposals (NOT implemented — one PR each, flag-gated for JIT/GC)
- **D2 (priority 1) — bridge unsupported builtins instead of disqualifying.**
  Lower a nonlisted `CallBuiltin` to a slow call into a `zun_jit_call_builtin`
  thunk that runs the interpreter's native dispatch, with a stack map at the call
  (precise GC roots) and the result marshaled back — the exact analogue of the
  existing direct-`Call` bridge (`eligible_methods_bridged`). Files:
  `zun_codegen/src/eligible.rs` (stop hard-disqualifying when a builtin-bridge is
  available), `translate/mod.rs` (lowering), `thunks/` (new thunk),
  `zun_codegen/src/lib.rs` (symbol), `zun_vm/src/jit_bridge.rs` (wire to interp
  dispatch). Risk: medium-high — GC safepoint/roots across the call and
  exception propagation back to Tier0 (non-throwing builtins first; throwing
  ones need the exceptional-return channel). Tests: interp==JIT equivalence on
  methods using print/typeofAny/looseEquals/coerce; GC stress with a safepoint at
  the bridged call; a builtin that throws caught in Tier0. Gain: up to ~57% of
  rejected methods become JIT-candidates; unblocks the dynamic-`any` hot methods.
- **D1 — get the JIT into loops; two options.**
  (a) **Enter eligible methods (incl. the entry `main`) natively** instead of
  always interpreting the entry. Files: `zun_vm/src/lib.rs` entry invocation,
  `jit_hook.rs`. Risk: low-medium (entry ABI). Gain: fixes top-level hot loops
  ~48-68× — but only once the method is eligible, so it is paired with D2 (main
  is usually ineligible via `print`).
  (b) **Wire the existing OSR into the run path**: drain `take_osr_requests`/
  snapshots at threshold, rewrite roots, transfer the live frame into a native
  OSR entry. Files: `zun_vm/src/lib.rs`+`jit_hook.rs`, `zun_codegen` OSR entries,
  GC root rewrite (`rewrite_osr_snapshot_roots` exists). Risk: high (mid-method
  frame/root transfer, stack-map correctness). Gain: rescues any loop that heats
  up mid-method regardless of entry.
  **Recommendation: do (a) first** (low risk; with D2 it fixes the measured
  main-loop case with no mid-frame transfer), and keep (b) OSR for a later PR to
  cover loops that only get hot deep inside an already-running method.
- **D3 (proposal A) — typed char-access for an f64 index.** Relax
  `mcall_string.rs:135` to accept `Ty::F64` and route `charCodeAt`/`charAt`/
  `codePointAt`/`at` to a typed native that does ToInteger + bounds-check and
  returns the JS out-of-range result (NaN / undefined / "") as a typed value,
  JIT-lowerable (ideally inline). Files: `zun_sema/src/check/expr/
  mcall_string.rs`, `zun_natives/src/string.rs`+`managed_methods.rs`,
  `zun_codegen` (inline lowering), `zun_interp`. Risk: medium (exact JS semantics
  for fractional/negative/NaN/OOB indices and wtf16/lone-surrogate), no JIT-ABI/
  GC. Tests: conformance char-access fixtures byte-identical to Node;
  interp==JIT; closes the open typed-OOB finding. Gain: the ~99% residual-`any`
  char-access (311,980 calls) leaves the ~1 µs dynamic path.
- **D4 — interpreter body pre-decode cache.** Decode each method body once into
  an indexed `Vec<Instruction>` with precomputed branch targets
  (`decode_all_with_offsets` already gives the offset↔index map), cached per
  method; trim per-instruction frame bookkeeping to call/throw/safepoint sites.
  Files: `zun_interp/src/lib.rs` (run loop + per-method cache), maybe `zun_il`
  (decoded form). Risk: medium (exact semantics, branch-target mapping, cache
  memory). Tests: interp==JIT + conformance unchanged; **mandatory bench** — the
  arith-loop ns/opcode before/after. Gain: removes the per-instruction re-decode
  (the dominant Tier0 base cost); helps everything that stays on Tier0.

Rules (user): one PR per item; flag-gated when touching JIT or GC; interp==JIT
and conformance no regression; bench before/after; phase 6 and Option B remain
suspended; approval item by item.

## D3 — typed f64-index char-access — DONE (option A, interp floor) (2026-11-07)

Scope (authorized): `charCodeAt` / `charAt` / `s[i]` accept a JS `number` (f64)
index with exact JS semantics; audit (not change) the other `is_int`-gated
methods. No JIT-ABI (option A): the typed natives are interp-only this PR; JIT
inlining is the C work, bundled with D2/D5.

- New natives (`crates/zun_natives/src/string.rs`): `charCodeAtNum (Str,F64)->F64`
  (NaN out of range), `charAtNum (Str,F64)->Str` ("" out of range),
  `charIndex (Str,F64)->Str|undefined` (ECMAScript canonical index: non-negative
  integer in range only; fractional/negative/OOB -> undefined). ToIntegerOrInfinity
  via `string_ops::integer_or_infinity` (NaN->0); code unit read lazily with
  `wtf16::encode(..).nth(i)` — no per-call allocation.
- Sema: `check/expr/mcall_string.rs` routes `charCodeAt`/`charAt` with an `f64`
  index to the typed natives (result F64/String, no boxing); an explicit integer
  index keeps the existing U64->U32 native (so a `String` class method declared
  `charCodeAt(u64): u32` still type-checks — the near-regression that proved this
  gate matters). `check/expr/member.rs` lowers `str[i]` to `charIndex` (fixing the
  prior `.at` desugaring, which wrongly counted negative-from-end and truncated
  fractionals), reusing the already-checked index (no double `check_expr`).
- **Semantics: byte-identical to Node** (committed fixture
  `tests/string/char-access-f64-index.ts`): charCodeAt/charAt/`[]` across
  fractional, negative, NaN, ±Infinity, OOB, and a surrogate pair.
- **Perf (release, 5-run median, startup subtracted):** typed `charCodeAt` loop
  **987 -> 497 ns/iter (-50%)** — the dynamic `stringMethod` path (member-name
  clone + name-match dispatch + GC box) is gone, and removing a per-call
  `Vec<u16>` collect (first cut 735 -> 497) took the rest at the interpreter
  level. `hotcc2m` 1.07 s. The remaining cost is interpreter dispatch + the
  out-of-line native call; a further ~90% needs machine-level JIT (C: inline
  bounds-check + `movzx` + `cvtsi2sd`), which needs D1 for `main` and D5 first.
- **Gates:** fmt clean; clippy `-D warnings` clean (touched crates); interp==JIT
  equivalence all green; conformance **83 -> 85 / 168** (one new fixture + one
  previously-divergent fixture now byte-identical; the lone remaining divergence
  is the pre-existing flag-independent `web-streams-encoding`). `cargo test
  --workspace` green except the two `zun_runtime/tests/shared.rs` tests, which
  load the standalone `zun_runtime` cdylib and fail (`-2`) until it is rebuilt
  (`cargo build -p zun_runtime`) — the same stale-artifact ordering quirk as
  `zun_vm.lib`/aot, not a regression. No `.zm` change.

## S6 — heap.strings is append-only and never frees strings (2026-11-07)

**OPEN (do not fix now, recorded per decision).** `Heap::intern_string`
(`crates/zun_gc/src/strings.rs:6-10`) pushes into a `Vec` indexed by a stable
handle and never removes entries; GC compaction does not touch the strings table
(`compact_impl` leaves it alone). So every distinct string interned during a run
is retained for the life of the heap — an unbounded leak that grows with program
runtime (dynamic string creation, `box_any` of strings, etc.). Handles never
reindex, which is what makes the ZStr handle path GC-safe, but the flip side is
no reclamation. Deferred: a future string-table GC (or the WTF-16/handle string
model, option 3) is where this gets addressed.

## C / ZStr string header — plan (3 PRs, approved) (2026-11-07)

Approved layout change behind an opaque `ZStr` so future moves (GC-handle string,
WTF-16) stay internal to it. Sequence: (a) mechanical `Value::Str(ZStr)` +
`heap.strings: Vec<ZStr>`, ZStr wrapping `Rc<str>` with no header, no behavior
change; (b) header `{ascii, len16}` computed once at creation + O(1) interp fast
paths for `length`/`charCodeAt`/`charAt`/`s[i]`; (c) Cranelift inline reading the
header, behind a flag, target `hotcc2m` « 0.1 s. `ZStr` is opaque (private
fields; access only via `From<&str>`/`From<String>`/`ZStr::from` and
`Deref<Target=str>`). JIT (PR c) never offsets into the `RcBox`: it takes
`Rc::as_ptr` and reads a `#[repr(C)]` `StrHeader` at fixed offsets (DST built with
documented unsafe; `triomphe::ThinArc` considered as the thin-pointer
alternative — chosen and documented in PR b/c). `len16` counts each lone
surrogate as one UTF-16 unit (matching `wtf16::len16`); a size past `u32` fails
or takes the slow path, never a silent truncation. `ascii = true` only when
`is_ascii()` proved. Gates per PR: equivalence, conformance, OOB/NaN, the
eligible⟺lowering invariant, and a 100k-linear bench (interp and, for c, JIT).

## D5 — eligible-but-untranslatable methods recompile on every backedge (2026-11-07)

**Latent, pre-existing JIT tiering defect** (found while wiring D3; independent
of it). When a method is JIT-eligible but its body fails to translate, the
failure is not memoized, so the lazy-tiering path retries a full Cranelift
compilation on every interpreted loop backedge.

Mechanism:
- `is_compiled(rid)` is defined as merely "in the eligible set"
  (`crates/zun_codegen/src/lib.rs:953-955` — `self.eligible.contains(&rid)`),
  not "has a finalized body".
- `Jit::define_method` returns `Err` on a translation failure and, by design,
  resets the builder and leaves `rid` **out** of `self.defined`
  (`crates/zun_codegen/src/lib.rs:830-856`); `ensure_compiled` propagates the
  `Err` without recording it (`:787-817`). `rid` stays in `self.eligible`.
- The interpreter raises `on_backedge` on every back-edge
  (`crates/zun_interp/src/lib.rs:976-1015`); `JitHook::on_backedge`
  (`crates/zun_vm/src/jit_hook.rs:390-398`) then sees
  `is_compiled(rid) && !is_defined(rid)` true and calls `ensure_compiled`
  **again** — one failing full compile per iteration.

Measured: a JIT-eligible `hot()` whose body used a builtin with no lowering
entry ran at **10.53 s / 2M iters**, vs ~2.07 s for the clean interpreter
fallback and ~1.55 s once the builtin was removed from eligibility — a ~5× and
~7× slowdown from pure recompilation churn, not wrong output (the result stayed
byte-correct). It bites any method the eligibility predicate admits but the
lowering table cannot translate (an easy `eligible.rs` vs `builtins_table.rs`
mismatch).

**FIXED** (D5 PR): negative-cache the failure. `Jit` gains a `failed: HashSet`
and a per-method `attempts` counter (`crates/zun_codegen/src/lib.rs`).
`ensure_compiled` short-circuits to `Ok(false)` when the rid is cached-failed and,
on a `define_method` error, records both the failing member and the requested
entry as failed. `is_compiled` is now `eligible && !failed` (no longer "merely in
the eligible list"), so the `on_backedge` gate (`is_compiled && !is_defined`)
stops retrying a failed method — it falls back to the interpreter exactly once.
Tests: `failed_translation_is_attempted_once_then_cached` (force an eligible
method whose body has `print` — an unlisted CallBuiltin — to be translated;
assert `compile_attempts == 1` and that 1000 further `ensure_compiled` calls do
not re-attempt) and `every_eligible_builtin_has_a_lowering` (iterate all native
ids: any builtin an eligibility predicate admits must have a `builtin_desc` entry
or be one of the hand-lowered `op_call_builtin` names — fails if `eligible.rs`
and `builtins_table.rs` drift apart). Before/after: the storm measured 10.53 s /
2M iters (~2M recompiles); after, one attempt (unit test) and the interpreter
fallback (~1 s class). The C work (JIT thunks for the D3 char-access natives, +
the shared ASCII fast-path bit) lands with D2 phase 1, the matching JIT-ABI
effort.

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
