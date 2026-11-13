# Known drift from the declared architecture

Audit snapshot: 2026-11-12, with a fix pass the same day. Line numbers move;
re-locate with a search before acting. Mark an entry FIXED (with the commit)
instead of deleting it.

Conformance: **114/168** fixtures exit 0 at `dfb7a142`. The audit fix pass took
118 (at `ef117cff`) to 120; making the garbage collector actually run cost 13
fixtures and recovered 5 (`e77901ec`), and rooting the calling frame across a
native call brought 7 of those 13 back (`dfb7a142`). Six are still out, as the
first OPEN item below. Measure with `zun run` per fixture and a per-fixture
timeout: three fixtures used to hang forever, so a harness without one never
finishes.

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

- **FIXED - The interpreter only collected after a typed `newobj`.**
  `self.allocs` was incremented in exactly one opcode arm, so a program that
  allocated only dynamic objects, arrays, strings, maps or promises never
  reached the GC budget and **never collected at all**. The budget now comes
  from `Heap::allocations_since_collection`, counted at the single allocation
  point, so no allocation site can forget to report itself. Measured on a loop
  allocating 900k short-lived dynamic objects: peak RSS 872 MB -> 15.9 MB, and
  it also got faster (2.0s -> 1.7s).
- **FIXED - An evacuating minor collection abandoned the nursery.** It bumped
  `nursery_cursor` to `CHUNK_SLOTS` so the next allocation pushed a fresh
  512 KiB chunk, and nothing ever released or reused the old ones; a
  compaction did not reclaim the nursery either. The nursery is now released
  after each evacuation. The hazard that forced the old behaviour - a
  non-moving full collection promotes survivors *in place*, leaving old
  objects in nursery storage - is handled by evacuating any object whose
  storage is in the nursery, old or young (`address_in` in
  `zun_gc/src/collection.rs`). Behaviour change: a minor collection can now
  move an old object, so a caller must use the rewritten root, not a stale
  `Obj` copy.
- **FIXED - The record table grew with every object ever allocated.** Dead
  young records were dropped from the address-preserving free list and never
  came back. They now go to `free_records`, which hands the index out again
  with a freshly allocated address (`Heap::release_record_storage`).
- **FIXED - Latent rooting and relocation bugs the live GC exposed.** None of
  these could fire while collections never happened; all five are the same
  mistake, a managed value the collector cannot see:
  1. `DynamicObject::rewrite_object_references` rewrote properties and the
     prototype but not the `FunctionHandle`s in `string_functions`,
     `accessor_get/set`, `symbol_functions` and `symbol_getters`. Their closure
     environments were *traced* and so survived, at an address the handle no
     longer pointed at - which is every closure stored in a container
     (`arr.push(() => i)`, `o.f = () => i`). The trace and the rewrite now go
     through one `function_handles()` / `function_handles_mut()` pair so a new
     function-valued map cannot update only one of them.
  2. `function_expando_bag` keyed its bag on the environment's raw address, so
     a recycled address aliased a dead closure's properties onto a new one and
     a moved environment lost its bag. It keys on the identity hash now.
  3. The mark phase never traced a typed array's or DataView's ArrayBuffer
     (the edge is a raw header address, like an iterator's source), so a
     buffer referenced only by its view was swept while the view was alive.
  4. `class_symbol_methods` and `class_field_defaults` were neither rooted nor
     rewritten; they now go through the decorator side-table pair.
  5. A frame in transit (`suspended`, `resume_frame`) and a `yielded` value
     were not rooted - nothing else references them between a generator's
     yield and the boundary that takes it.
  6. (`dfb7a142`, issue #67) The **calling** frame was dropped from the root
     chain whenever a native re-entered a managed callee: a native has no
     `RootLink` to thread and passed `parent: None`. The interpreter publishes
     the activation in `Interp::native_link` for the duration of the call and
     `Interp::call` / `Interp::dispatch` resolve `None` against it, so the fix
     is two points rather than the dozen native call sites. The same commit
     publishes the native's argument vector as the innermost activation -
     `pop_n` takes the arguments off the interpreter stack, so until then
     nothing rooted a callback's captured environment while the native ran.
  Also: the finalizer and registry callbacks were rewritten with
  `FunctionHandle::new`, which drops the bound discriminant; they use
  `relocate_environment` now.
- **OPEN - 6 fixtures still regressed since the GC started running, and they
  share one cause.** Thirteen regressed at `e77901ec` (accepted deliberately);
  rooting the calling frame across a native call (`dfb7a142`, issue #67) brought
  seven back. The six left are `array-iterators-and-protocols`,
  `throw-catch-finally-control`, `generator-expressions-and-capture`,
  `yield-star-delegation`, `iterator-helper-pipeline` and
  `data-structures-and-algorithms`, and the shared cause has a minimal
  reproducer: **a closure with captures, re-entered from a native, loses its
  environment**. The read of a captured slot comes back `Void`, or the cell
  behind it reads `undefined`, which means the environment object was swept or
  its address recycled even though the function value that points at it is
  rooted through the native's argument vector. The next fix belongs here.

  ```ts
  class Rec { a: number = 1; }
  function work(): string {
    const src: any[] = [1, 2, 3, 4, 5, 6, 7, 8];
    let hits = 0;                       // captured by cell
    for (let r = 0; r < 400; r++) {
      src.forEach((v: any) => { const o = new Rec(); hits += o.a; });
    }
    return "hits=" + String(hits);
  }
  console.log(work());
  ```

  The callback has to allocate a *typed* instance: the interpreter polls the GC
  budget at `newobj` and at loop backedges only, so a callback that builds
  dynamic objects never reaches a safepoint and the bug stays hidden. That is
  also why the reproducers in issues #55 and #67 stopped failing on their own.
  The recipe that locates one of these in a few minutes: run the fixture,
  note the last line it printed, find that line in the fixture, wrap the
  *following* statement in `try { ... } catch (e: any) { console.log(e.name +
  ": " + e.message) }` (the thrown message is what names the broken operation),
  then cut the fixture down around it. Every one found so far was a managed
  value the mark phase or the relocation pass did not know about: an
  interpreter-owned table, a side table, a frame in transit, a native call's
  argument vector, or the calling frame itself.
- **OPEN - The interned-string table is never reclaimed.** `Heap::intern_string`
  pushes onto a `Vec<ZStr>` and `zun_gc/src/collection.rs` never touches
  `strings`. Measured: ~108 bytes per interned string, linear and unbounded
  (900k strings boxed into `any` = 111 MB peak, flat in everything else after
  the fixes above). This is also why `JitKind::String` needs no rooting.
  Reclaiming it is a design decision, not a patch, because a stale handle
  silently reads a *different* string rather than crashing. Every handle
  source would have to be traced: `Any::string` payloads in the ten heap side
  tables, `FIELD_STRING` slots of frozen layouts (which needs a
  `string_slots(type_rid)` query beside `reference_slots`), string-tagged
  `Any`s in interpreter frames, and handles held in JIT registers and native
  locals - which stack maps do not describe, since they track addresses. The
  two candidate designs are (a) a mark/free-list pass over the table, gated on
  a safepoint with no active native frame, plus the metadata above, or (b)
  strings become ordinary GC objects, which `zun_core/src/zstr.rs` already
  anticipates ("a GC-handle string") and which gets all the existing tracing
  and rooting for free.
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
- The interpreter now actually collects in programs that allocate only
  dynamic objects, so code paths whose rooting was never exercised are
  exercised for the first time. A missing root shows up as a use-after-free,
  not as a leak.
- An evacuating minor collection can move an *old* object (one a full
  collection promoted in place). Read the rewritten root after
  `collect_young_evacuating`; a stale `Obj` copy taken before the call points
  at released storage.

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
