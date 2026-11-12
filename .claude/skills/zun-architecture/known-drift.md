# Known drift from the declared architecture

Audit snapshot: 2026-11-12, with a fix pass the same day. Line numbers move;
re-locate with a search before acting. Mark an entry FIXED (with the commit)
instead of deleting it.

Conformance at the end of the fix pass: 120/168 fixtures exit 0 (118 at commit
`ef117cff`), so the pass regressed nothing.

## Concept-level (decides whether the premise is exercised)

- **OPEN - Tiering reach.** Eligibility is all-or-nothing per method
  (`crates/zun_codegen/src/eligible.rs`), any dynamic op taints the method, and the
  module top level never JITs (about 297x slower than the same loop in a function,
  see `bench/README.md`). Most real programs therefore run in Tier 0. The fix is
  the deferred foundation (JIT the module entry / OSR out of any frame), not a
  local heuristic: the "drop the realm-store mirror for unread bindings" heuristic
  was tried and reverted because the analysis is whole-program.
- **OPEN - Type erasure.** Generics, unions, interfaces and structural types erase
  to `Any`. The typed fixed-layout world covers simple classes and numeric kinds.
- **OPEN - Tier 0 does metadata lookups per access.** Field access and virtual
  calls resolve through `self.types.get_field` / `resolve_virtual` in
  `crates/zun_interp/src/lib.rs` instead of a pre-resolved slot.

## Invariant-level

- **OPEN - Verifier checks stack height only.** `zun_verify::ErrorKind` has no
  type-mismatch or bad-token error; `StackValueKind` tracking is not implemented.
  A wrongly typed `ldfld` verifies, so the JIT trusts the compiler.
- **OPEN - No redirectable JIT entry.** No atomic entry cell exists in
  `zun_codegen` or `zun_vm/src/jit_hook.rs`; tiering uses `RefCell<HashMap>`
  counters, and compiled-to-compiled calls bind `func_ids` directly, so recompiling
  a callee does not redirect existing callers.
- **OPEN - No codegen boundary trait.** `zun_codegen` uses `zun_gc::Heap`,
  `HeapScope`, `NativeRootScope`, `StackMapRegistry`, `RelocationMap` concretely.
  `CallHook` (`zun_interp/src/hooks.rs`) is the only real boundary trait.
- **OPEN - Second body of opcode knowledge.** The interpreter dispatch and
  `zun_codegen/src/translate/mod.rs` hand-match opcodes (expected: each tier
  implements the op), but `eligible.rs` additionally keeps hand-copied allowlists
  (`opcode_supported`, `try_region_op_allowed`, nine `scalar_*_array_*` /
  `object_array_*` matchers, hard-coded string method names). GOAL.md says
  eligibility is centralized, not an allowlist.
- **OPEN - `call.builtin` carries a raw table index.** `CallBuiltin` takes an
  `Imm32` that indexes `zun_natives::MODULES` by position, so a `.zm` depends on
  the order of a Rust table. Mitigation in force: the table is append-only.

## Cranelift rules (CLAUDE.md "Cranelift standards")

- **OPEN - `CallConv::Tail` is not used** for managed functions (a comment in
  `zun_codegen/src/lib.rs` defers it).
- **OPEN - `use_egraphs` and `enable_alias_analysis` are not set**; only
  `opt_level=speed` and `preserve_frame_pointers`.
- **OPEN - No cold blocks.** There are no `set_cold_block` calls.
- **OPEN - No `FuncRef` cache.** About 50 `*_fref` helpers in
  `translate/arrays.rs`, `translate/strings.rs` and neighbours each build a
  signature and call `declare_func_in_func` on every use.
- HOLDS - saturating `fcvt_to_sint_sat`, `declare_value_needs_stack_map` for GC
  references.

## Correctness bugs found by the audit

- **FIXED - JIT `replace` skipped `$`-substitution.** The semantic now lives once
  in `zun_core::string_ops::replace_first`, called by both the thunk and the
  native. Tests in `zun_codegen/tests/equivalence_string_methods.rs` were checked
  against the old code and fail on it.
- **FIXED - JIT `padStart`/`padEnd` with an empty fill** returned a null handle;
  the thunk now goes through `zun_core::string_ops::pad_units` and returns the
  receiver, as Tier 0 does.
- **FIXED (by exclusion) - JIT `repeat` had no `REPEAT_MAX` check / RangeError.**
  A thunk cannot raise one (the exceptional-return channel only exists after
  method calls) and the limit depends on runtime values, so `repeat` was removed
  from JIT eligibility and its dead lowering deleted. It now runs in Tier 0.
  Re-admitting it needs a pending-exception path for string thunks.
- **FIXED - Array arguments were under-rooted.** Four of the five packed-call
  entry points rooted `JitKind::Object` only. There is now one predicate
  (`zun_codegen::any_abi::is_gc_reference`, exhaustive match) and one collector
  (`collect_native_roots`) used by all five. Ground truth: `Object`/`Array` are
  managed addresses; `String` is an index into the append-only `Heap::strings`;
  `Function` is a one-word rid and eligibility refuses function-typed params.
- **FIXED - Regex flag order differed.** `zun_core::ordered_flags` is the only
  place the `dgimsuvy` order is written; `zun_interp::collections::regex_to_string`
  assembles the string form, and all four sites use them.
- **FIXED - Three ToNumber variants disagreed.** `Interp::dyn_to_number` is the
  single ToNumber (now `pub(crate)`, with the array shortcut); `to_number_value`
  forwards to it; the lossy free `collections::to_number` is deleted. Four call
  sites were diverging from JS and are fixed: the `sort` and `toSorted` comparator
  results, `Array.prototype.at`'s index, and `Array.prototype.flat`'s depth.
- **FIXED (incidental) - rid 0 could panic** in the exception-catch base-chain
  walk (`current as usize - 1` on a debug build); the shared function uses
  `checked_sub(1)`.

## New findings from the fix pass

- **OPEN - The interned-string table is never reclaimed.** `Heap::intern_string`
  pushes onto a `Vec<ZStr>` and `zun_gc/src/collection.rs` never touches
  `strings`, so a program that builds strings in a loop grows without bound. This
  is also why `JitKind::String` needs no rooting. Likely more serious than
  anything in the original audit list.
- **OPEN - `compare` orders by UTF-8 bytes, not UTF-16 code units**, in BOTH
  tiers (`zun_codegen/src/thunks/string.rs`, `zun_natives/src/string.rs`). Not a
  tier divergence; a probable conformance bug. Unchecked against Node.
- **OPEN - `substring` and the pad thunks return handle 0 for a negative
  argument** where the Tier 0 native returns an error. The typed surface uses
  `u64`, so negatives should not reach either path; latent.
- **OPEN - `toInt`/`toFloat` thunks return 0/NaN on a parse error** where the
  native returns an error.
- **OPEN - `toLowerCase`/`toUpperCase`/`contains`/`startsWith`/`endsWith`**
  duplicate rather than share between tiers. They agree today.
- **OPEN - `flat(undefined)` uses depth 0**, Node uses 1. Sema defaults a missing
  argument to `1.0`, so only the dynamic path is affected.
- **OPEN - `re.toString()` on an `any`-typed regex throws** in the
  `zun_interp` unit-test harness. Unknown whether this is a harness limit or a
  real gap.
- **OPEN - `int_recursion` in `zun_codegen/tests/equivalence_string_methods.rs`
  overflows its stack**, aborting the whole test binary. Verified pre-existing by
  stashing every change and re-running. The test pins `.stack_size(8 MiB)`
  explicitly, overriding the project's 32 MiB `RUST_MIN_STACK`, and `sumTo(100)`
  no longer fits. Raise the pinned size or shrink the debug-build frame.
- **OPEN - `JitKind::Function`'s doc comment in `zun_il/src/num.rs` says "two
  words"**; the implementation uses one (a rid). Stale comment.
- **OPEN - `call_entry`'s return match in `zun_codegen/src/lib.rs` has no `Array`
  arm** and returns `Ineligible` for it.

## Behaviour changes to be aware of

- `Jit::call_packed_with_any_args` used to skip an invalid `Any` tag silently and
  run the native code with the malformed carrier; it now returns
  `JitError::InvalidAnyTag`, matching `call_packed_any`.
- `string.repeat` is no longer JIT-compiled (see above).

## Dead code

- **KEEP - `zun_codegen/src/fn_abi.rs`** is unreferenced but deliberately staged;
  `docs/design/jit-closure-carrier.md` is the plan and the closure-carrier change
  must consume it. Its header already says so. Do not delete it.
- **FIXED - `eligible.rs::methods_that_may_throw`** is deleted; it was superseded
  when the bridge's uniform exceptional-return channel landed and eligibility
  stopped gating on `!may_throw`.

## Duplication closed in the fix pass

These now have exactly one implementation; reuse them rather than re-deriving:

- Exception-catch base-chain walk: `zun_module::type_is_caught` (new
  `crates/zun_module/src/catch.rs`), used by both `zun_vm/src/jit_bridge.rs` and
  `zun_codegen/src/eligible.rs`. It has the cyclic-hierarchy test neither copy
  had.
- String search / pad / substring / count / replace: `zun_core::string_ops` and
  `zun_core::replace_first`.
- Regex flag ordering: `zun_core::ordered_flags`.
- ToNumber: `Interp::dyn_to_number`.
- `Value` to `zun_gc::Obj`: `zun_interp::collections::object` (`pub(crate)`). Six
  copies deleted, each removing an `unsafe` cast. About 27 call sites still
  open-code `.map(|address| unsafe { Obj::from_addr(..) })` on an `Any` rather
  than a `Value`, so they need a different helper; converting them is open work.
- The "requires an interpreter heap" native stubs: the `unavailable_stub!` macro
  in `zun_natives/src/lib.rs` (a macro because `Imp` is a bare `fn` pointer, so a
  capturing closure cannot coerce to it).

## Organization

- **OPEN - No lint/test CI.** `.github/workflows/` runs only the conformance
  comparison; nothing runs `cargo fmt --check`, `cargo clippy` or `cargo test`.
- **OPEN - External dependency versions are partly per-crate** (oxc, `zstd`,
  `libloading`, `icu`, `jiff`, `getrandom`, `memchr`).
- **OPEN - Crate roots carry logic.** `zun_codegen/src/lib.rs` (one ~1500-line
  `impl Jit`), `zun_interp/src/lib.rs` (dispatch loop), `zun_compiler/src/lib.rs`,
  `zun_natives/src/lib.rs`, `zun_gc/src/lib.rs`, plus
  `zun_interp/src/property_ops.rs` (~2400 lines).
