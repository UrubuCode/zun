# Known drift from the declared architecture

Audit snapshot: 2026-11-12, with a fix pass the same day. Line numbers move;
re-locate with a search before acting. Mark an entry FIXED (with the commit)
instead of deleting it.

Conformance: **135/168 (80.4%)** fixtures exit 0 (at `2d417873`), and the same
135 pass under `ZUN_GC_STRESS=1` with a byte-identical pass set. The 120 -> 135
run of 2026-11-13..15 is summarised in the memory file `zun-conformance-push`;
every commit in it carries a pass-set diff, a stdout diff against a binary
built from its parent, and a stress run.

**Measure proximity, not error counts.** A failing fixture's visible error
count is a *floor*, not a total: these are 80-160 line probe programs with
chains of independent failures, so clearing one root cause advances a fixture
and parks it on the next, unrelated one. Error *messages* mislead too - five
identical "private fields require an instance receiver" lines in
`class-private-brands-and-statics` were three different bugs (static-private
`this`, nested-class receiver, computed key). The useful metric is output
lines produced versus `node`'s:
`z=$(zun run F | wc -l); n=$(node F | wc -l)`. As of `551837f2` the fixtures
closest to passing are `structured-clone` 66%, `error-types-and-subclassing`
62%, `weak-collections-and-refs` 56%, `scoping-functions-and-calls` 41%;
about 40 fixtures are still at 0% because they fail to compile.

It survived lifting the heap-pinning
policy, and the same 120 then passed under `ZUN_GC_STRESS=1` and `=4` as well
(a collection at every allocation, with released memory poisoned), which was
117 *with* the heap pinned. Getting there: the audit fix pass
took 118 (at `ef117cff`) to 120; making the garbage collector actually run cost
13 fixtures and recovered 5 (`e77901ec`); rooting the calling frame across a
native call brought 7 back (`dfb7a142`); pinning the heap during a native call,
re-keying a moved generator's side tables and treating an array field as a
reference slot brought the last 6 back (`020e678b`). Measure with `zun run` per
fixture and a per-fixture timeout: three fixtures used to hang forever, so a
harness without one never finishes.

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
  `HeapScope`, `RootArray`, `StackMapRegistry`, `RelocationMap` concretely.
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
  these could fire while collections never happened; all nine are the same
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
  7. (`020e678b`, **lifted again**) A **native frame pinned the heap**, because
     native code keeps managed references in ordinary Rust locals - the
     callback a builtin pulled out of its arguments before looping, the
     receiver it resolved once - and nothing could find or rewrite those. The
     policy and `roots::collect_young_non_moving` are gone now that natives
     have a handle API (`zun_gc::Rooted` + `root!`); see the new entry below
     for what that cost and what is left.
  8. (`020e678b`) `generator_states` and `generator_frames` were the only
     address-keyed side tables the relocation pass did not **re-key**, so a
     moved generator stopped being a generator: `generator_state` answered
     `None` and the iterator protocol could not find `next`.
  9. (`020e678b`) `TypeSystem::reference_slots` filtered on `JitKind::Object`
     alone, so an **array-typed field** was never traced from its owner and
     never write-barriered. `JitKind::Array` holds a managed address in the
     slot exactly as `Object` does.

  Also: the finalizer and registry callbacks were rewritten with
  `FunctionHandle::new`, which drops the bound discriminant; they use
  `relocate_environment` now.
- **CLOSED - the GC regression is gone.** Thirteen fixtures regressed at
  `e77901ec` when the collector started running; `dfb7a142` and `020e678b`
  brought all thirteen back, to the same pass set as before. Kept here because
  the hunt is the reusable part: every cause was a managed value the mark phase
  or the relocation pass did not know about - an interpreter-owned table, a
  side table's *key*, a frame in transit, a native call's argument vector, the
  calling frame itself, a field kind missing from the reference-slot list, or a
  reference held in a native's Rust local that nothing can rewrite (hence the
  pinning policy, below).

  Two things make these bugs hard to see, and both are worth knowing before
  chasing the next one:

  - **A reproducer has to reach a safepoint.** The interpreter polls the GC
    budget at `newobj` and at loop backedges only, so a callee that builds
    dynamic objects never triggers a collection and the bug stays hidden. The
    callee has to allocate a *typed* instance. This is why the reproducers in
    issues #55 and #67 stopped failing on their own.
  - **A reproducer has to put the only reference where the collector cannot
    see it.** A module-level `const` is rooted by the realm store
    independently, so it never shows a frame-rooting bug; the value has to live
    in a *function's* locals.

  ```ts
  class Rec { a: number = 1; }
  function work(): string {
    const src: any[] = [1, 2, 3, 4, 5, 6, 7, 8];
    let hits = 0;
    for (let r = 0; r < 400; r++) {
      src.forEach((v: any) => { const o = new Rec(); hits += o.a; });
    }
    return "hits=" + String(hits);
  }
  console.log(work());
  ```

  The recipe that located most of them in a few minutes each: run the fixture,
  note the last line it printed, find that line in the fixture, wrap the
  *following* statement in `try { ... } catch (e: any) { console.log(e.name +
  ": " + e.message) }` (the thrown message names the broken operation), then
  cut the fixture down around it.
- **FIXED - `NativeRootScope` is deleted.** It registered `roots.as_ptr()` from
  a non-`mut` local and the collector wrote through `ptr as *mut Obj`:
  undefined behaviour the compiler acted on, so a re-read of `roots[i]` could
  answer the pre-collection address out of a register (`generator_return` did;
  `decorators.rs` and `json.rs` carried `read_volatile` workarounds). Replaced
  by two shapes, neither needing `unsafe`: `zun_gc::Rooted<T>` + `root!` for a
  single reference (carriers `Obj`, `FunctionHandle`, `Option<T>`), and
  `zun_gc::RootArray` for a run of them, owned by the registry and addressed by
  index, which is what `JSON.stringify` (two slots per nesting level, read back
  after every `toJSON`) and the JIT entry points (packed arguments, kept alive
  only) need. **Ground truth for the next author: allocation never collects in
  this engine** - the budget is polled only in the dispatch loop, at `newobj`
  and loop backedges (`Interp::poll_gc`), and from `zun_rt_alloc` on the JIT's
  allocation slow path. A native that only allocates cannot move anything; one
  that re-enters managed code can.
- **FIXED (`551837f2`) - the interpreter caller is now rewritten across a JIT
  bridge call.** `runner.rs` registered the caller's frame roots as a
  `RootArray`, which kept them alive, but what is registered is a *copy* of the
  addresses, so nothing wrote the relocated addresses back into the caller's
  `args` or its parent chain. `Interp::dispatch` now restores the frame from the
  registry after the call returns, through `RootLink::restore_roots` (the
  counterpart of `append_roots`), with `roots::managed_address` as the single
  definition of what counts as a frame root.
  Found from a four-line program that lost a *caught* exception under
  `ZUN_GC_STRESS=1`: `console.log` is JIT-compiled and reaches `emit` through
  `jit_bridge::bridge_call`, which builds a nested `Interp` with no caller link;
  the collection taken there left `main`'s `for...of` iterator stale, so the
  iterator-done check at the loop head - which sits *outside* the `try` - raised
  a TypeError nothing could catch. The thrown error was never the problem.
  Still open, one level down: a collection from `zun_rt_alloc` directly in JIT
  code evacuates without rewriting the interpreter caller chain (a different
  path, untested), a nested bridge `Interp` that evacuates leaves the outer
  `Interp`'s address-keyed side tables stale, and a JIT frame's own references
  depend on `declare_value_needs_stack_map` coverage.
- **FIXED (`551837f2`) - a full compacting collection reported no guards at
  all.** `collect_and_compact_current` (`zun_interp/src/roots.rs`) never called
  `zun_gc::append_native_roots`, unlike every sibling collector. An object held
  only by a `Rooted` or `RootArray` guard was therefore swept in that path, and
  the compaction then rewrote registry slots that no longer named a live object
  - so every guard added by the preceding rooting series was silently ignored
  there. A nested bridge interpreter reaches this path on every eighth
  collection, which is what made the first regression test flaky rather than
  failing outright. Worth remembering as a shape: a root registry is only as
  good as the collectors that remember to ask it.
- **OPEN - the collector's root-slice API has the same unsound cast.**
  `Heap::collect` and friends take `&[Obj]` and rewrite it through
  `roots.as_ptr() as *mut Obj` (`zun_gc/src/collection.rs`). Less dangerous in
  practice than the native scopes were, because callers pass a `Vec`'s heap
  buffer rather than a stack local, but it is the same shape: it should take
  `&mut [Obj]`.
- **OPEN - a `Value`/`Any` local held across a safepoint is still unprotected.**
  `Rooted` holds one managed address word, so it covers `Obj` and
  `FunctionHandle` but not an `Any` that happens to box an object (a bigint, a
  boxed instance). Known instances: `generator_return` passes `value` to
  `make_iter_result` after `resume_suspended`; `typed_dyn.rs` holds `kept:
  Vec<Value>` and `acc` across callbacks; `generatorIteratorNext` leaves
  `state.pending_return` in a state that is out of the table, and therefore
  untraced, while the body runs; `iter_step` returns a `Value` read after calls
  that can collect. Where a fixture forced the issue the workaround is a rooted
  one-slot array as a holder (`arrayFilter`, `arrayFind`, `arrayGroup`,
  `objectGroupBy`) - correct, but a side table standing in for a type that
  should carry itself. The real fix is the `Trace` trait sketched in
  `docs/design/gc-native-roots.md`, with one impl for `Value`.
  Note: the four-line lost-exception reproducer that led to `551837f2` was
  *not* an instance of this, despite looking like one. Don't assume a
  stress-only failure is the `Value`/`Any` gap until the collecting frame is
  identified.
- **FIXED (`3ebbf3fb`) - a natively thrown error read uninitialised heap.**
  `make_error_value` (`zun_interp/src/native_error.rs`) allocated an Error with
  its full field count but wrote only three of the five slots, leaving
  `nameOverride` and the trailing flag as whatever the allocator left behind.
  So `.name` answered garbage - a JSON TypeError reported itself as
  `"Iterator"` - on **every** error raised through `throw_native!`. All five
  slots are initialised now, matching `new_error_object`. Worth a look for the
  same shape elsewhere: `heap.alloc(rid, field_count)` followed by a shorter
  value list is silent and compiles.
- **OPEN - opcode arms that reach user code without publishing the frame.** The
  four dynamic ops now publish the activation before running a getter, setter
  or `valueOf`, and `async_start` reached from `Call` publishes the caller. The
  other arms that can reach user code the same way (`ToPrimitive` from
  arithmetic, for one) have not been audited.
- **OPEN (guarded, not repaired) - the core runtime type ids alias the `TypeDef`
  rid space.** `CoreTypeId::Promise` is 6, `BigInt` is 7, and so on, and those
  are also valid `TypeDef` rids, so an object allocated with a core id answers
  `type_rid()` with a number that names an unrelated class. A promise is
  allocated with **no fields at all**, so every name-based lookup keyed on the
  object's own rid resolved the fields, accessors and methods of whatever class
  is `TypeDef` 6 - in the prelude, an Error-family class - and then read or
  wrote a field slot the promise does not own. `String(promise)` answered
  `name + ": " + message` out of a neighbouring object's bytes; under
  `ZUN_GC_STRESS`, where released memory is poisoned, the bytes were
  `0xDEADBEEF` and the string table panicked (this was the last stress-only
  fixture failure, `classes-errors/runtime-error-sites`).
  `Interp::type_rid_describes_object` now gates those lookups: a real instance
  is allocated with exactly its type's declared field count, so a core id whose
  count disagrees is an alias rather than an instance, and the field read and
  write are additionally bounded by `Heap::field_count`. That is a guard, not a
  repair - `promise instanceof Error` still answers `true`, and any new lookup
  keyed on `object.type_rid()` is wrong again by default. The repair is to stop
  overloading the rid space: a tag bit, or a runtime-kind field in the object
  header beside the type rid.
- **OPEN - a `JSON.stringify` replacer can fail to resolve its own call.** Found
  while probing the rooting work; **not** a rooting bug, because it fails
  identically with and without `ZUN_GC_STRESS`. This exact program dies with
  `runtime error: call did not resolve to a method at <main>`, pointing at the
  `JSON.stringify` call:

  ```ts
  class Rec { a: number = 1; }
  function churn(): number { let n = 0; for (let i = 0; i < 40; i++) { const o = new Rec(); n += o.a; } return n; }
  const deep: any = { l1: { l2: { l3: { l4: { l5: [1, 2, { l6: "x" }] } } } }, tail: [3, 4] };
  console.log("A=" + JSON.stringify(deep, function (k: any, v: any) { churn(); return v; }));
  ```

  It needs the whole combination: every smaller variant passes - four levels of
  nesting without an array, an array without the nesting, the allocating callee
  without either, and each pair of the three. A `toJSON` hook doing the same
  allocation at four nesting depths works fine. Likely the same family as the
  closure-conversion and top-level-dispatch entries above, since the failing
  thing is resolving a call from inside a top-level function expression.
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

## New findings from the conformance push (2026-11-13, `56ee7980`..`551837f2`)

Six subsystems landed; conformance moved 120 -> 121. The gap between those two
numbers is the lesson: see "Measure proximity, not error counts" in the header.

### Engine panics from ordinary user code (both FIXED)
A user program must never be able to panic the Rust process. Two paths could:

- **FIXED (`7003b4d4`) - a nullish dynamic store.** `function g(x: any) { x.v = 1 }`
  called with `undefined` panicked in `Value::as_object` (`value.rs`) instead of
  throwing. It now raises `TypeError: Cannot set properties of undefined`,
  matching the read side. The guard lives in an `#[inline(never)]` helper
  because `op_store_dynamic` is `#[inline(always)]` into the dispatch loop -
  keep it that way, or the hot frame grows and `zun_codegen`'s recursion
  equivalence tests overflow their 8 MB stack.
- **FIXED (`56ee7980`, `7003b4d4`) - `new` on a declared `function F(this: any)`.**
  The receiver was dropped, `this` lowered to `undefined`, and constructing it
  panicked. See the declared-receiver entry below.

### Silent wrong answers (all FIXED) - the failure mode to fear here
Three paths produced plausible but wrong output rather than an error, and the
168-fixture suite stayed green over all of them, because no fixture covers
them. They were caught only by probing the "known limits" an implementer
reported, against `node`, by hand.

- **FIXED (`7003b4d4`) - `f(...xs)` dropped an argument.** With a hidden
  receiver in the signature, `function f(this: any, ...r)` let the receiver eat
  `xs[0]`: `n=2 sum=5` for a three-element spread. `call_sugar.rs`.
- **FIXED (`7003b4d4`) - `g(...ys)` onto fixed parameters** answered
  `5,undefined` instead of `4,5`. `spread_optional.rs`.
- **FIXED (`3ebbf3fb`) - a natively thrown error's `.name`** - see the
  uninitialised-heap entry above.

**Process note.** When a subagent reports a "known limit", test it against
`node` before accepting it: two of the three above were reported as
pre-existing limits and were in fact regressions introduced by the same change.
A green suite is not evidence here.

### Still open, with reproducers
- **OPEN - a namespace object assigned to a dynamic local loses its members.**
  ```ts
  const M: any = Math;
  Math.max(1, 2)   // 2
  M.max(1, 2)      // throws "max is not a function"
  M.PI             // undefined (node: 3.14159...)
  ```
  `JSON` in the same position works, so it is not all namespaces. This is what
  parks `misc-platform/globalthis-namespaces-and-tags` at 8% - it opens with
  `const g: any = globalThis`. Note for whoever picks this up: it was first
  reported to this session as a *GC* bug ("an object held in a module-level
  `const` dies after heavy allocation"), and it is not - it fails on the first
  iteration with no collection at all.
- **OPEN - `===` between two BigInts reached through `any` compares addresses.**
  `const c: any = 255n, d: any = 255n; c === d` is `false`, and so are
  `Object.is` and `[c].includes(d)`. `same_value` in
  `zun_interp/src/collections/mod.rs` compares `(Object, Object)` by `addr()`
  and needs a bigint value comparison.
- **OPEN - a computed-key *static* method is never installed on the class.**
  `Object.getOwnPropertyNames(C)` lists only `length,name,prototype`. The
  instance case is fixed (`00406f3d`); the static `defineProperty` target is the
  bare class identifier and apparently does not reach the class object.
- **OPEN - primitive wrappers have no prototype-chain linkage.** A BigInt
  instance's dynamic prototype does not reach `BigInt.prototype`, so a hook
  installed there is invisible to a chain walk. `3ebbf3fb` worked around this
  for `toJSON` by naming `BigInt` in the serializer's prototype-name lookup
  (beside `Map` and `Set`) and by resolving dynamic member calls through
  `builtin_prototype_by_name`. The general fix is real linkage.
- **OPEN - `Object.*` statics are not readable as values.** `typeof
  Object.defineProperties` is `undefined`, which parks
  `object-meta/object-statics-tostring-hasown` at 1% right after it starts
  compiling.
- **OPEN - accessor properties on built-in prototypes.** `6f201f27` installs
  method *values* but no getters: `RegExp.prototype.flags`/`source` and
  `Symbol.prototype.description` are still missing, as are the
  `toLocaleString` family, the String HTML methods and FinalizationRegistry
  `register`/`unregister`.
- **OPEN - `WeakMap` and `WeakSet` share one store**, so
  `WeakMap.prototype.get.call(aWeakSet, k)` is not rejected as a brand error.
- **OPEN - strict arity on declared functions.** `e(1, 2, 3, 4)` and `e(1)` on
  a two-parameter declaration are compile errors where `node` pads with
  `undefined`; a spread through `.call` reports "spread arguments are not
  lowered yet". Both pre-date the receiver work and apply equally to functions
  with no `this`.

## The 120 -> 135 conformance run (2026-11-13..15)

Eighteen subsystems, `56ee7980`..`2d417873`. Full per-commit detail is in the
git log; what belongs here is what the run *taught*, and what it left open.

### Verification that actually catches damage
Pass/fail hides silent output changes, and a dozen changes in this run passed
all 168 fixtures while producing plausible-but-wrong output. Three checks per
commit became the standard, and every one of them caught something:
1. a fixture **pass-set diff**, never just the count;
2. stdout of all 168 fixtures diffed against a binary built from the parent
   commit in a temporary worktree, with mismatches against node counted
   **symmetrically** (`diff | grep -c '^[<>]'`);
3. the suite under `ZUN_GC_STRESS=1`.

Two traps in that process, both hit for real:
- A one-directional mismatch count makes an **empty** output score best, so a
  fixture that prints nothing looks perfect. It produced a report of "43
  fixtures regressing" when four had improved.
- Removing the comparison worktree before the comparison finishes makes every
  baseline run read empty, with the same effect. Keep it until the last
  measurement.

### The shape to fear: lists but does not behave
Not a crash — a plausible wrong answer no fixture covers. Instances found:
`new Error(0 as any).message` answered `"Math"` (a misread slot);
`new Object(5)` answered the `Object` constructor itself; `fn.prototype`
returned a *fresh empty object on every read*, so `F.prototype.x = …` was
silently lost; `f(...xs)` dropped an argument when a hidden receiver was
added; a natively thrown error read uninitialised heap for its `name`;
installing names on `Object.prototype` made them list while calling them
threw. **Prove both directions — reflection and behaviour — against node, and
treat a subagent's "pre-existing limit" as a claim to test, not a fact.**

### Engine panics closed (user code must never panic the process)
- a nullish dynamic store (`x.v = 1` with `x` undefined);
- `new` on a declared `function F(this: any)`;
- a typed class instance used as an iterator in native `for...of`/spread;
- `Object.setPrototypeOf(7, Number.prototype)`.

### Still open, with reproducers
- **CLOSED in `6b08da32` - output that varies with the GC budget**, and with
  it the `async function*` that silently stopped under stress. Two separate
  bugs, both worth remembering as shapes. (a) The `method_contains_yield`
  branch of the CallIndirect arm built no `RootLink` and published no native
  link, so the caller's frame was neither rooted nor rewritten across
  `create_generator` -> the prelude's `__zunMakeAsyncGen`, which can collect.
  A caller local went stale and the next `arraypush` raised `BadToken`, which
  surfaced as *missing output* rather than as an error. (b) `pid_by_obj`, the
  promise-to-scheduler bridge, is keyed by heap address with nothing rooting
  its keys: a dead entry let a later promise at the reused address inherit an
  already-settled scheduler id, and the moving-collection rewrite re-inserted
  in hash order, so a dead unmoved key could overwrite a live promise that had
  just moved onto that address. The value now carries an identity hash that
  every lookup validates. **The collector was discarding correct output, not
  producing wrong answers:** the three affected fixtures went from 28, 9 and 2
  symmetric mismatches against node to **zero**. 167 of 168 fixtures are now
  byte-identical at budgets 0, 1, 4 and 16. Caveat recorded: the two fixes
  were not attributed individually to the `rt_ag*` reproducers.
- **OPEN - `WeakMap` entries keyed by unregistered Symbols are lost while the
  keys are still live.** The one fixture still varying with the collection
  budget: `collections/weak-collections-and-brand-metadata` line 34 reads
  `dup_symbols=first:second` normally, `undefined:undefined` at
  `ZUN_GC_STRESS=1` and `undefined:second` at `=16`, from
  `new WeakMap([[dupA,"first"],[dupB,"second"]])`. `crates/zun_gc/src/weak.rs`
  keys weak tables by object address, which a Symbol key does not fit. Exits 0
  and still passes, so only the budget sweep catches it. Highest-value GC lead
  remaining.
- **OPEN - `.call`/`.apply` had shifted a free function's arguments since
  `5b75749a` (2026-11-11), fixed in `4ae8a539`.** Worth remembering as a
  method lesson rather than a live bug: it was reported as a regression from
  this run's prototype-method work, and a `git bisect run` with a real build
  per step proved it four days older. The culprit clause tested
  `types.virtual_slot(method_rid).is_some()`, which is true for free
  functions too; the right question is "owned by a class and non-static",
  which `method_rid_is_instance_method` now answers. No fixture covered it —
  a regression test was added with the fix.
- **OPEN - not globals at all:** `Blob`, `Request`, `File`, `MessageChannel`,
  `MessagePort` (block `platform-classes` at 59/151), and
  `queueMicrotask`/`setTimeout`/`Intl`, which live in `zun-node` and need an
  event loop (block `globalthis` at 24/160).
- **OPEN - `Function` is a global but not callable through it**, and lacks
  `name`/`length`/`prototype` identity.
- **OPEN - a static class symbol method** (`class D { static
  [Symbol.hasInstance]() {} }`) is not an own property of the class value;
  they live in `class_symbol_methods` and need a class-constructor
  own-property model.
- **OPEN - an arrow function is still constructible via `new`**: the runtime
  function metadata carries no constructibility flag.
- **OPEN - `structuredClone` keeps a class instance typed** where node gives a
  plain object (`serialize`/`deserialize` keep types deliberately, so this
  needs a flatten mode), and array expandos are not cloned.
- **OPEN - an own method installed on a dynamic object or array is ignored,
  and `delete` does not remove it.** The widest-reaching gap found in the
  135 -> 138 step, and a textbook "lists but does not behave": for
  `const b: any = [1,2]; b.join = () => "OWNJOIN"`, `hasOwnProperty("join")`
  answers `true` while `b.join()` still runs the built-in and answers
  `"1,2"`; after `delete b.join`, `hasOwnProperty` *stays* `true`. The same
  holds for `toString` on an array and, more surprisingly, for a plain
  object over a prototype: `p = Object.create(o); p.m = () => "own"` calls
  `o.m` instead. Only members whose dispatch explicitly consults
  `dynamic_get_function` first (today `toLocaleString`) honour the override,
  which is why `toLocaleString` behaves and then resists `delete`. The three
  lines above are the whole reproducer. Identical on `2e976e59`, so it is not a regression
  from that step; it blocks own-override lines in `array-join-tostring-and-
  generic` and most of `obfuscated/*`.
- **OPEN - an uncaught thrown object prints a heap address, not its message.**
  `throw new TypeError("boom")` reports
  `runtime error: unhandled exception: Object(Obj(0x28be372fcf0))`; only a
  thrown *string* renders. This blinds every diagnosis of a fixture that dies
  at run time, which is most of the remaining thirty, so it is worth fixing
  before more conformance work. `errors::thrown_text` already tries and its
  comment blames the heap scope being gone at print time, but a time-boxed
  attempt (reverted, not committed) ruled that out and narrowed it:
  - Hooking `ExecutionOutcome::Thrown` in `runner.rs` does **not** help: the
    CLI escapes through `interp.call(...)` -> `interp.annotate_error(error)`
    in `run_method_with`, not through `resume`.
  - Hooking `annotate_error` in `state.rs` and rendering there -- including
    walking the `SourceFrame` chain for a wrapped `Thrown`, and reading
    `name`/`message` as **typed fields** via `instance_field_map` +
    `load_field` rather than through the GC error table or a dynamic property
    -- still printed the address. So either `annotate_error` is not on the
    escaping path either, or the field read fails for a prelude `Error`
    subclass. The next attempt should instrument first and find the real
    escape boundary before writing any rendering code.
  - Rendering must not call a `get_name` accessor: it runs while an exception
    is already unwinding and managed code could throw again.

### After the 138-mark cluster work (`c07a0cea`)
- **OPEN - an unbound class method used as a callback shifts its arguments.**
  `class R { describe(x) { return "d:" + x } }` then
  `["-3"].map(inst.describe)` answers `d:0` where node answers `d:-3`: the
  element is consumed as the receiver and the *index* arrives as `x`. This is
  the receiver-insertion shape of `4ae8a539` and `c07a0cea` a third time, now
  on the array-callback path rather than `call`/`apply`/`bind`. A subagent
  reported this as a panic at `value.rs:226`; the panic did not reproduce, the
  wrong answer did, which is the worse of the two. **Whenever a receiver gate
  is touched, probe the callback paths too** -- `map`, `filter`, `forEach`,
  `sort` -- not only the explicit `call`/`apply` forms.
- **OPEN - own function properties are dropped by the object statics.**
  `Object.values`, `entries`, `assign` and object spread lose a function-valued
  own property on a plain object; on an array, `assign` and spread also lose
  the indices. `c07a0cea` fixed the *dispatch* side of own methods
  (`arr.join = f` is now called, listed by `Object.keys`/`for...in`/`in`, and
  deletable), so this is the remaining half of that family.
- **OPEN - a few own-method sites still bypass the override:** `hasOwnProperty`
  assigned on an object (sema lowers the call to an intrinsic), and a `Map` or
  `Set` with an own method override.
- **OPEN - descriptor and integrity-level gaps:**
  `Object.defineProperty(arr, "x", { enumerable: false })` throws "Cannot
  redefine property", and a frozen or sealed array or object still accepts new
  own properties.
- **OPEN - `.name` of an object-literal method and of a function expression**
  answers `""` where node derives it from the binding (`cA.bump.name` should be
  `bump`). `fn-name-length-metadata` is at 86 symmetric mismatches, down from
  136, and most of what is left is this.
- **OPEN - `Array.prototype.slice.call(5)`** on a primitive throws; it is the
  next death point of `array-join-tostring-and-generic` at 43 of 53 lines.
  Also there: `class L extends Array { join() {...} }` with `sub.toString()`
  answers the element count, and `Symbol.toPrimitive` on an array is ignored by
  `Number()`.

### Found while making native subclassing work (`efbce851`)
- **OPEN, memory safety - `Reflect.construct(<native>, args, K)` where `K` has
  typed fields does raw field access on the native's slots.** Not a wrong
  answer but an out-of-contract memory access:
  ```ts
  class K { tag: string = "k"; n: number = 1; describe() { return this.tag + ":" + this.n } }
  const a: any = Reflect.construct(Array, [1, 2, 3], K as any);
  a.describe();        // zun: "Math:9.4e-323"   node: "undefined:undefined"
  a.tag = "z";         // writes through a slot the native array does not have
  ```
  `LoadField`/`StoreField` run against the native array's words, so the read
  surfaces a leaked interned string (`"Math"` -- the same signature as the old
  `new Error(0 as any).message` bug) and a denormal float. Native-backed
  classes (`efbce851`) escape this because their `this` is `any`; the
  `Reflect.construct` path does not. **A typed field access must be refused
  when the receiver is a native value rather than an instance of the TypeDef.**
- **OPEN, engine panic - a static field initializer never runs in the project's
  own canonical program shape.** A `function main()` entry point with **no
  top-level statements** -- the form every `zun_vm` test uses -- leaves a
  `static` field as `Void`, and the engine panics instead of reporting
  anything:
  ```ts
  class T { static seen: string[] = []; static add(s: string) { T.seen.push(s) } }
  function main() { T.add("a"); console.log(T.seen.join(",")); }
  // panics: value.rs:228 "expected object, found Void"
  ```
  Adding a single top-level statement (`main();`) makes it work, which is why
  the fixtures never caught it: they all have top-level code.
- **OPEN - a declared field on a typed-array subclass lists but is skipped by
  the enumerating operations.** `class U extends Uint8Array { tag = "u" }`:
  the field reads, deletes, and answers `hasOwnProperty`,
  `getOwnPropertyNames` and its descriptor, but `Object.keys`, `for...in`,
  `JSON.stringify` and `in` omit it. Same bug as an expando on a plain typed
  array. Array and `ArrayBuffer` subclasses are correct, so this is specific
  to the typed-array carriers.
- **OPEN - smaller gaps in the new carrier:** a RegExp subclass ignores
  `Symbol.species` in `split`; typed-array `map` with a BigInt species throws
  `slice`'s message instead of node's "Cannot convert 1 to a BigInt";
  typed-array subclasses have no `from`/`of` statics; `new this(...parts)`
  (spread into a dynamic `new`) is still unlowered; and a field declared
  without an initializer is adopted as `0` rather than `undefined`.
- **OPEN - `SharedArrayBuffer` is indistinguishable from `ArrayBuffer` in the
  heap**, so `Object.prototype.toString.call` answers `[object ArrayBuffer]`
  where node answers `[object SharedArrayBuffer]`. It answered
  `[object Object]` before `efbce851`; both are wrong, and telling them apart
  needs a heap-level distinction.

### At the 140 mark (`3fa7a60b`)
- **OPEN, the root cause behind several symptoms - a property store of an
  unknown key on a typed class instance is silently dropped, and on a built-in
  class value it panics.** `Reflect.set(boxInstance, "length", 2)` reports
  success and changes nothing, which is why `Array.of.call(Box, "x", "y")`
  constructs the right `Box` but leaves `length` unset (node answers 2). On a
  *built-in* class value reached through `any` the same store is worse:
  `const O: any = Map; O.zzz = 5` **panics the engine** at `value.rs:226`.
  This is the remaining half of the own-property family whose dispatch side was
  fixed in `c07a0cea`, and it is the widest open item: it also explains
  `Object.values`/`entries`/`assign`/spread dropping function-valued own
  properties, an assigned `hasOwnProperty` being ignored, and a frozen or
  sealed object still accepting new properties.
- **OPEN - `Array.of.length` reads 1 where node says 0.** The arity in
  `STATIC_METHODS` is already 0, so the wrong number comes from the function
  *value's* own length, not the table.
- **OPEN - an array callback's third argument is the temporary, not the
  receiver**, so `r === u` is false inside
  `Array.prototype.forEach.call(u8, (x, i, r) => ...)`. A plain array-like
  behaved this way before `3fa7a60b` too, so the generic path inherited it
  rather than introduced it.
- **OPEN - `typedarray-methods-views-and-iteration` passes while 28 of its 100
  lines still differ from node.** Exit 0 is the suite's bar, not correctness:
  this fixture is a reminder that the pass set and node-agreement are different
  measurements. Remaining there: a number written into a `BigInt64Array` is
  accepted where node throws, and `toSpliced` through a dynamic call is "not a
  function" even on a plain array.

### After the property-store repair (`6e38aec3`)
- **CLOSED - the store half of the own-property family.** Both store opcodes
  called `as_object()` on whatever the receiver was, so a store on a class
  value, a function or a primitive through `any` **aborted the engine** at
  `value.rs:226`. `store_receiver` replaces it. `Reflect.set` now reaches
  typed instances, `Map`/`Set`, class values and functions; function-valued own
  properties survive `Object.values`/`entries`/`assign`/spread; an assigned
  `hasOwnProperty`/`propertyIsEnumerable`/`isPrototypeOf` is honoured; and
  `isFrozen`/`isSealed`/`isExtensible` answer node's false/false/true for an
  object that was never frozen instead of true/true/false.
- **Deliberate divergence from node, kept:** a store on a frozen object throws
  `TypeError` where *sloppy* node is silent. The interpreter was already strict
  for data stores, so this extends one rule instead of mixing two modes, and it
  fails loudly rather than losing a write. A store on a *primitive* is silent,
  where strict node throws.
- **OPEN - a typed-code `StoreField` to a frozen instance is not refused.**
  `box.tag = "z"` compiled as a direct field store bypasses the integrity
  check; only `any` stores, `Reflect.set` and `Object.assign` are guarded.
  Closing it needs an integrity bit that `StoreField` consults in **both** the
  interpreter and the JIT, which does not exist -- the interp/JIT equivalence
  invariant makes this a two-sided change, not a one-line check.
- **OPEN - a function or class value cannot be frozen or sealed**
  (`Object.freeze(fn)` is the identity), and `Reflect.set` with a function or an
  array as the borrowed receiver returns false.
- **OPEN - `F.m()` in typed code still calls the declared static** after
  `(F as any).m = fn`, because the call is bound at compile time. The dynamic
  read and call do see the override. Inherited statics are missing from
  `for...in` on a subclass.
- **OPEN - eight built-in static descriptors carry no `.value`**
  (`Map.groupBy` and the seven `Promise` statics), kept folded in sema through
  `FOLDED_STATIC_DESCRIPTORS` because the runtime has no function value for
  them. The same shape `6f201f27`, `4141e09e` and `3fa7a60b` each fixed for a
  different family.
- **OPEN - `Date` instances leak their internal `t` field** into `Object.keys`
  and `JSON.stringify`; `Error.stackTraceLimit` and an error's own
  `stack`/`message` names are missing; `Reflect.ownKeys(fn)` throws; and a
  store on the `JSON` namespace through `any` is dropped (no longer a panic).
- **OPEN - `array-construction-and-species` did not move** (59 of 92 lines, 45
  mismatches): its death point is `s.toSpliced(...)` through a dynamic call,
  **not** the `Box.length` store that was assumed. Worth recording as a method
  note: the blocker named in a report was one fixture earlier than the real one,
  and only running it proved which.

### At the 141 mark (`529da44a`)
- **CLOSED - the `value.rs:228` abort on a typed field access.** Both typed
  field opcodes called `as_object()` on the receiver, so `this.field` with no
  receiver aborted the process where node throws
  `TypeError: Cannot read properties of undefined (reading 'n')`. Also closes
  the static-initializer abort recorded earlier. **Note the sequence as a
  lesson:** `0f47da98` fixed a wrong answer (the element was being eaten as the
  receiver) and by doing so turned it into an abort. A correct fix upstream can
  surface a latent abort downstream, so after fixing a receiver bug, run the
  probe that previously produced the wrong answer and check it does not now
  crash.
- **Borrowed receivers**: a typed field access on a plain dynamic object falls
  back to the named property **only when the object carries it**. Being public
  is not enough -- a built-in's internal slot is an ordinary field too, and a
  public-name-only fallback made `Date.prototype.getTime.call({})` answer
  `undefined` where node throws. **The pass count stayed at 141 through both
  versions; only the 168-fixture stdout comparison caught it.** That is the
  whole argument for measuring stdout and not just the counter.
- **OPEN - a *direct* call of a detached instance method shifts its
  arguments.** `const d: any = C.prototype.m; d(5)` puts `5` in the receiver
  slot, leaving the real parameter `Void`, which now panics in
  `value.rs:157` (`as_number`) instead of answering wrongly. This is the fifth
  distinct site of the receiver-insertion family (after `5b75749a`/`4ae8a539`,
  `c07a0cea`, the array-callback path in `0f47da98`, and this commit's load
  side). `.call`, `.apply`, `.bind` and the array callbacks are all correct;
  only the bare call is not.
- **OPEN - `as_number` aborts on a non-number**, the same shape as the
  `as_object` abort just closed. Worth sweeping the remaining `as_*` accessors
  in `value.rs` for callers that can be reached with a user-controlled value.
- **OPEN - `f.apply(null, Object("abc"))` cannot match node today.**
  `Object(primitive)` does not build a wrapper object in Zun -- `typeof`
  answers `"string"` where node says `"object"` -- and node accepts a wrapper
  but *rejects* a bare primitive as an argument list, so accepting
  `Value::Str` there would make `f.apply(null, "abc")` wrongly succeed. Needs
  real primitive wrapper objects. It is the last mismatching line of
  `call-apply-bind-semantics`.
- **OPEN, test-harness divergence - a bare module compiled in a unit test
  takes a different array-callback path than the CLI.** The same program
  (`[5].map(detachedMethod)`) throws node's TypeError through
  `target/release/zun.exe` but panics in `as_number` under
  `zun_compiler::compile` + `run` in `zun_interp`'s own tests, so a regression
  test for the fix above could not be written there. Worth understanding: the
  project's invariant is that the interpreter is the always-correct fallback,
  and two different answers for one program in two harnesses undercuts every
  unit test written that way.

### At the 143 mark (`8f5c54c8`)
- **Known-red, NOT ours and NOT new:** `zun_compiler --lib`'s
  `dynamic_fallback_report_covers_reflective_property_access` and
  `reachability_excludes_unused_methods` both fail identically on `205c1b3d`,
  verified in a worktree. The monomorphization report now sees a dynamic site
  outside `main` and sets `dynamic_fallback` where the tests expect neither, so
  the `prove` analysis has drifted from its own tests. Add these to the
  known-red list beside `zun_codegen`'s stack-size pair and `wintercg`'s
  `headers_set_append_delete`.
- **`zun_cli --test aot` fails with a *stale artifact*, not a bug.** Five tests
  assert the AOT staticlib is newer than the runtime sources; after any change
  under `zun_vm`/`zun_interp` they fail until `cargo build -p zun_vm` is run.
  The assertion message says exactly that. Run that build before trusting
  `cargo test -p zun_cli`. (A good example of a test that explains itself --
  worth imitating.)
- **`cargo test -p zun_compiler` cannot link on this machine**: `link.exe`
  exits 1285 on the `prove_baseline` and `prove_prelude_scale` test binaries,
  with or without anything else running. Use `--lib` there.
- **FRAGILE - the checker decides whether to keep module-binding mirrors by
  scanning the source *text* for `eval(` or `Function(`.** It matched the
  prelude's own source and silently changed the output of every program; the
  prelude now writes `function eval (x: any)` with a space to dodge its own
  scan, pinned by a `zun_prelude` test. A textual trigger for a semantic
  decision is a trap: it fires on a comment or a string literal, and it is the
  reason a one-line prelude edit could move an unrelated fixture. Worth
  replacing with a real binding-use check.
- **`EvalHost` gained a required method, `compile_eval`.** `CompilerEvalHost`
  is the only implementor in this workspace; an out-of-tree host (the drift
  notes mention `zun-node`) will not compile until it adds one.
- **OPEN - module bindings still leak onto `globalThis`.**
  `const secret = "x"` at module level makes `globalThis.secret` readable,
  where node (CommonJS module scope) answers `undefined`. Verified on
  `205c1b3d`, so it predates the eval work; `8f5c54c8` only hid these names
  from *generated code's* identifier lookups, so `(0, eval)("this.secret")`
  still sees one.
- **OPEN - typed `.length` folds trailing `any` parameters to 0**, so
  `function one(x: any) {}; one.length` answers 0 where node says 1, and
  `eval.length` differs between the typed and dynamic paths.
- **OPEN - `Function("a", "return this.x + a")` and
  `Function("return this")(1)`** are refused or answer the argument instead of
  `globalThis`, both for the same open receiver bug; `Function.bind` is still
  missing; and a generator function used as an array callback or through
  `.bind` aborts with "malformed bytecode: parked a frame that is not awaiting
  a promise".

### At the 144 mark (`49dfa830`)
- **CLOSED - `new base.Member(args)` with `base` a local typed `any`.** Sema
  lowered it to `dynamicConstruct`, which only handled an `import()` namespace.
  Recorded as a **reduction lesson**: the symptom read as "fails at module top
  level", and that framing was wrong in both directions -- a top-level binding
  is a *local of the module initializer* (so it broke) while the same name read
  inside a function is a module binding (so it worked), and a function with its
  own `any` local broke too. When a defect looks positional, check what the
  binding *is*, not where it sits.
- **CLOSED - `new Response()` aborted the engine** at `value.rs:157`
  (`as_number` on `Void`), the same accessor-panic family as the `as_object`
  pair closed in `6e38aec3` and `529da44a`. The remaining `as_*` accessors in
  `value.rs` are still worth a sweep.
- **The prelude must not use typed array methods.** `request.zun` used
  `string[].indexOf` and broke 48 `zun_interp` tests at once, because those
  compile programs that supply their own `Array` methods. Use an index loop.
- **OPEN - dynamic construction does not apply parameter defaults** and pads
  omitted parameters with `null`: for
  `class K { constructor(a: any = undefined, b: any = "dflt") {} }`, a
  `new (K as any-local)()` or `Reflect.construct(K, [])` gives `null|null`
  where node gives `undefined|dflt`, and non-literal field initializers do not
  run on that path. This is why the new platform classes all take a rest
  parameter -- a workaround, not a fix.
- **OPEN - `Reflect.construct(URL, [...], Tagged)` throws `Invalid URL: null`**,
  and `Reflect.construct(Headers, [{a:"1"}])` throws "Cannot read properties of
  undefined (reading 'push')"; the plain two-argument forms and
  `const U: any = URL; new U(...)` fail identically. Verified on `acb561ce`.
  It is the next park of `platform-classes-and-helpers` (line 201) and sits in
  `construct_class_from_descriptor`, next to the receiver gate.
- **OPEN - a `static` method cannot read a `#private` field** ("only accessible
  inside its declaring class"), though a static calling a private *instance
  method* works.
- **OPEN - prelude class prototype members are non-enumerable** where node has
  them enumerable, `String(<platform class value>)` prints `[any]`, and
  `X instanceof Function` is false for a class value (also for `Headers` and
  `Map`), so the platform classes still differ from node in reflection even
  where behaviour matches.

### At `71d61617` (still 144) — a measurement correction and six blockers
- **MEASUREMENT: a symmetric mismatch count is not monotonic with progress.**
  `buffer-compare-rw-and-views` went from printing **nothing** to printing 52
  lines, 9 of which match node, and its symmetric count *rose* 143 -> 177.
  Empty output scores exactly one mismatch per node line and nothing worse, so
  partial-but-wrong output can score worse than silence. Symmetric counting
  remains the right guard against the one-directional trap (where empty output
  scores *perfect*), but **rank by matching lines**:
  `(zun_lines + node_lines - symmetric) / 2`. Report both.
- **Known-red, verified at the parent in a worktree:**
  `zun_vm --test function_apply_args` (both tests). Joins `zun_codegen`'s
  `equivalence_string_methods`/`int_recursion`, `wintercg`'s
  `headers_set_append_delete`, and `zun_compiler --lib`'s two report tests.
- **OPEN, engine panic:** `5n - (x as any)` -- a typed BigInt against an `any`
  operand, for `-`, `*`, `/`, `%`, `**` and the bitwise/shift operators --
  aborts at `value.rs:157` (`as_number`). The same accessor family as the three
  aborts already closed; needs a dynamic BigInt arithmetic native.
- **OPEN - BigInt identity through `any` compares by heap address**, so `===`,
  `switch`, `Map`/`Set` keys, `includes`/`indexOf` and `Object.is` all answer
  wrongly for equal BigInts. This is most of what
  `bigint-conversion-and-equality` still mismatches (36 of 204 lines).
- **OPEN - BigInt arithmetic through `any`:** `a - b` with two BigInts throws
  where node computes; a mixed `a - 1` throws with "Cannot convert a BigInt
  value to a number" instead of node's "Cannot mix..." (only `+` has the right
  message); and unary `-a` silently answers a *number*.
- **OPEN - more intrinsics missing as function values**, the same shape fixed
  for prototype methods in `6f201f27`, `Number` statics in `4141e09e`, `Array`
  statics in `3fa7a60b` and `eval`/`Function` in `8f5c54c8`: `Buffer` read as a
  value has no statics (`B.concat` is `undefined`),
  `ArrayBuffer.prototype.slice` is not a value, and `Symbol.for`,
  `Symbol.keyFor` and `Symbol.iterator` are missing through the `Symbol` value
  even though calling `Symbol` itself now works. `typeof Symbol.call` reads
  `undefined` while the call succeeds -- lists-vs-behaves, inverted.
- **OPEN - `new String("w")` has no string methods at all** (`.trim()` throws),
  and a method assigned on `String.prototype` is not honoured.
- **OPEN - `Reflect.set(255n as any, ...)` answers true where node throws**,
  because a BigInt primitive and `Object(255n)` are indistinguishable at run
  time -- the same missing primitive-wrapper subsystem that blocks
  `f.apply(null, Object("abc"))`.
- **Noted trade-off:** when `.call` takes the dynamic path for a primitive
  receiver, the `this` argument is evaluated before the callee expression.
  Observable only if both have side effects.

### At `e6183f7b` (145 of 168)
- **CLOSED - the fifth and last known site of the receiver-insertion family.**
  A class instance method *stored on* a plain object (`o.show =
  C.prototype.show; o.show()`) was called with **no receiver**, because the
  own-function dispatch decided "takes a receiver" from the `@this` closure
  suffix alone -- the marker an object-literal method carries. It now also asks
  `method_rid_is_instance_method`. The whole family, for the record:
  `5b75749a` -> `4ae8a539` (`.call`/`.apply` shifted a free function's
  arguments), `c07a0cea` (a rest function with >= 2 parameters), `0f47da98`
  (the array-callback `thisArg`), `529da44a` (the typed field receiver), and
  this one. Four inserted the receiver wrongly; this one omitted it. **The
  question is always "owned by a class and non-static, or explicit `@this`",
  and five different sites each asked it differently.** A sixth site is
  recorded below.
- **OPEN - the sixth site, and the only one still known: a *bare* call of a
  detached instance method.** `const d: any = C.prototype.m; d(5)` puts `5` in
  the receiver slot. The fix belongs in `invoke_function_value`
  (`collections/runtime_iter.rs`), but that has twelve callers, several of
  which legitimately supply a receiver as the first argument (the `kind >= 16`
  built-in branch does exactly that), so a change there risks the four fixes
  above. No fixture needs it and it fails loudly, so it is deliberately left.
- **OPEN - `Function.prototype`'s methods do not read off a function value.**
  `typeof m.call`, `m.apply`, `m.bind` and `m.toString` are all `undefined`
  for a function value `m`, while `m.call(...)` works (the call syntax is
  special-cased) and `Function.prototype.call.bind(m)` works. So the
  uncurried-this idiom `m.call.bind(m)` throws. It is the park of
  `fn-meta/this-binding-and-arrows` at 73 of 99 lines. **A time-boxed attempt
  failed and was reverted:** adding the four names to the `Value::Function`
  arm of the member read in `dyn_member.rs` (around the `function_meta` call)
  changed nothing, so that arm is not the path a `m.call` read takes. The next
  attempt should instrument to find the real read site first -- the same
  mistake, and the same remedy, as the uncaught-throw rendering recorded
  earlier.

### Target reached: 152 of 169 (`23ffa5e3`)
- **SECURITY, closed - `crypto` had a predictable fallback.**
  `crypto_random_u64` degraded to `nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15)`
  when the OS source failed, handing out a guessable value under the `crypto`
  name, with a comment calling it "essentially unreachable". It now returns an
  error. **The rule this run settled on: a `crypto` primitive either has the
  OS CSPRNG or it fails loudly.** Never substitute time or a PRNG behind that
  name, and never under a fixture's "values are never printed" excuse -- the
  probe would pass and the defect would ship.
- **Node's `getRandomValues` rejections are `DOMException`s, not TypeErrors:**
  `TypeMismatchError` (code 17) for a float array, plain array, string,
  object, `null`, number, `ArrayBuffer` or `DataView`, and
  `QuotaExceededError` (code 22) over 65536 **bytes** (so `Uint16Array(32769)`
  throws). A **detached** buffer does not throw -- it answers length 0. Only
  the no-argument call is a TypeError.
- **`crypto` is modelled as a namespace class of statics** (like `JSON` and
  `Atomics`) where node has an instance of `Crypto`. Deliberate, and the cost
  is reflection: members are own and non-enumerable, `getOwnPropertyNames`
  is not empty, `getRandomValues.length` is 0 where node says 1, and
  `Crypto`/`SubtleCrypto`/`CryptoKey` are not globals. `subtle` is a branded
  object whose algorithms fail loudly rather than returning plausible bytes.
- **OPEN, engine panic:** typed
  `Array.from(new Int8Array([1, 2]), (v: number) => v * 3)` aborts at
  `value.rs:157` ("expected float, found Int"). The fourth site of the `as_*`
  accessor-panic family; three others were closed in this run.
- **OPEN - `Array.from(<BigInt64Array>, mapper)` hands the mapper numbers**
  (the unmapped form was fixed), `Int8Array.from([300])` saturates to 127
  where node wraps to 44, and each typed-array constructor owns its own
  `from`/`of` so `Int8Array.from === Uint8Array.from` is false where node says
  true.
- **OPEN - a dynamic call pads an omitted argument with `null`, not
  `undefined`**, and `arguments.length` reads 1 for a call with none. This is
  what forced `getRandomValues` to be declared with a rest parameter, and it
  is the same gap recorded at the 140 mark for parameter defaults.

### Run-time classes (after `02ffaaaa`, still 153 of 169)
Design and rationale: `docs/design/runtime-classes.md`. What belongs here is
what the work taught and what it left open.

- **CLOSED - a class evaluated more than once had one identity.** A class is
  one static TypeDef, so `make("a") !== make("b")` answered false for
  `function make(tag) { return class { label = tag } }`, the two shared one
  prototype, and members could not see `tag`. A class that is a run-time value
  -- a class expression used as a value inside a function, or any class whose
  base is only known at run time -- is no longer hoisted: `zun_syntax`
  (`oxc/runtime_class.rs`) lowers it in place to a constructor function, a
  prototype object and member closures, with the class rules in the prelude's
  `builtin/runtime_class.zun`. No format change, no opcode, no type invented at
  run time. **The descriptor model could not carry this**: it could have held
  identity (an instance may already carry a prototype other than its
  TypeDef's), but a typed method has no environment slot, so captures and the
  per-application home object of `super` had nowhere to live.
- **CLOSED - the `super` placeholder of `efbce851`.** "`super` is not
  supported in a class whose base class is a run-time value" is now reached
  only by a class the closure form cannot express (`#private` members,
  decorators, parameter properties); every other such class has a real
  `super(...)` (`Reflect.construct(base, args, newTarget)`) and `super.m()`.
- **Five gaps in the dynamic object model were closed on the way**, none of
  them specific to classes -- an ordinary constructor function needed each:
  `F.prototype.constructor` was never defined (a function does not box into
  `Any`, so the definition silently did nothing); `instance.constructor`
  ignored the prototype chain; a store through an inherited setter created an
  own property; statics were not inherited through
  `Object.setPrototypeOf(F, Base)`; and ToPrimitive ignored an inherited
  `toString` / `valueOf` / `Symbol.toPrimitive`. Also: `new list[i]()` on a
  function read out of an array, and `new C()` inside a closure where `C` is
  an enclosing local (it threw `C is not defined` -- the capture collector did
  not look at the callee of `new`).
- **BEHAVIOUR CHANGE - a function-valued property of a function is called with
  the function as `this`**, not its property bag (`fn.m = function () { return
  this === fn }; fn.m()` now answers true, as node does). Only the receiver
  *value* changed; whether a receiver is passed is decided where it was.
- **FRAGILE, the same shape three times - sema decides from the closure's
  source text.** Whether a closure takes `this` (`mentions_this_word`),
  whether a self-referencing initializer binds its own name
  (`body.contains(name)`), and whether a captured local is boxed up front (the
  identifier tokens of the closure's text) are all textual. A compiler-made
  reference has no text, so each one silently misfired for the lowered class:
  a constructor received no `this`, `__zcC is not defined`, and an arrow in a
  derived constructor read a stale `this`. The lowering works around the last
  two (assign-after-declare; a one-element cell that is never reassigned) and
  sema gained an AST fallback for the first (`closure_reads_this`). **Any
  future AST-level desugaring will hit these again**; the real fix is to make
  the three decisions on the AST.
- **OPEN - a class declared, or bound by a declarator, inside a function
  captures nothing.** `function f(p) { class K { m() { return p } } return new
  K().m() }` throws `p is not defined`, exactly as before; only class
  *expressions used as values* are lowered. Widening the rule moves typed local
  classes onto the dynamic path, so it needs a capture test first (lower only
  the classes that actually capture).
- **OPEN - a `Proxy` over a class value is opaque**: `new Proxy(C, {}).prototype`
  (and every other member) reads `undefined`, and `Reflect.construct(proxy,
  args)` answers "value is not a constructor". So `class X extends proxy`
  throws at the class definition. Verified on `02ffaaaa`. It is the next park
  of `class-mixins` after the item below, and the theme of
  `proxy-constructor-callable-and-brand`.
- **OPEN, engine panic - a wrongly typed argument reaching a typed field
  store.** `class User { age: number; constructor(n: string, age: number) {
  this.age = age } }` then `new User("Bob", null as any)` aborts at
  `value.rs:157` ("expected float, found Null") -- in fully static code, with
  no run-time class involved, and identically on `02ffaaaa`. It is the fifth
  site of the `as_*` accessor family and the park of `class-mixins` (line 2 of
  59; 20 more lines print correctly behind it).
- **Where the three fixtures park now.** `class-expressions` 25 of 57 matching
  lines (was 6): `Dog.bind(null)` on a static class value. `class-mixins` 1 of
  59 (was 0, it did not compile): the panic above. `proxy-constructor-
  callable-and-brand` 0 of 285: four compile errors unrelated to classes
  (`new Proxy(..)()` called directly, `this.#shared` in a static method), and
  with those patched out it stops on line 3 inside the Proxy surface.

## An abort on the second call is a tiering symptom

A typed field that cannot hold `null` threw a clean TypeError the first time and
**aborted the process the second**. Three sites were repaired for it
(`c0659585`, `06b2b159`): `NewObj` field init and `StoreField` in
`zun_interp/src/lib.rs`, the dynamic store in `metadata.rs`, and the predicate
`value::unboxed_field_rejects`.

What matters is not the fix but how the site was found. Two hypotheses were
wrong, each with a rebuild: that a bare `return Err` skipped `route_exception`,
then that throwing after `pop_n` left the operand stack short. Both looked at
the interpreter. One temporary backtrace behind an env var named the real
caller in a single run: `zun_vm::jit_hook::JitHook::try_call`. The method had
warmed up, so the second call went to Tier 1, and the JIT's argument packing
reaches `as_number`/`as_int` directly -- the interpreter-side guard was never
consulted because the call never entered the interpreter.

**Rule: when a wrong answer or abort appears only on the second or later call of
the same code, suspect tiering before anything else, and check whether the Tier
1 path bypasses the check you just added.** A JIT path that aborts where Tier 0
throws violates the invariant that the interpreter is the always-correct
fallback; `try_call` now declines an argument it cannot represent so the call
stays on Tier 0. Any guard added to an interpreter arm needs the same question
asked of `jit_hook.rs`.

Two further notes from the same run. A string field's `_ => 0` fallback meant
intern id 0, so `class S { s: string }` with `new S(null as any)` answered the
unrelated live string `"Math"` -- a silent wrong answer that leaked another
value's bytes, and it was live inside a **passing** fixture
(`classes-errors/error-types-and-subclassing`, 97 of 102 matching lines, now
98). Only a *reference* field has a null encoding. And coercion was rejected on
purpose: ToNumber would turn `null` into 0, which flips
`Object.values(this).every(v => v !== null)` to the opposite of node's answer
with nothing to show for it. Matching node needs the slot widened to hold
`null`, which is a layout decision and still open.

## A brand is the type chain; `instanceof` is the prototype chain

Confusing the two was a memory-safety hole (`803ce8f3`). A private field access
checked neither, so:

    class Money { #amount: number = 1234.5; read() { return this.#amount } }
    class Label { #text: string = "hi";     read() { return this.#text } }
    const m: any = new Money();
    Object.setPrototypeOf(m, Label.prototype);
    m.read();   // node: TypeError.  zun: the string "Math".

Dispatch correctly followed the swapped prototype into `Label.read`, whose
`LoadField` read slot 0 of a *Money* object **as a string**, interpreting the
bits of the float `1234.5` as an intern id. A larger float indexes out of
range. `setPrototypeOf` is enough to reach it -- no `any` beyond the cast.
`private_field_brand_ok` now walks the receiver's base chain for the type that
declares the field, and `LoadField`/`StoreField` consult it. The pre-existing
dynamic-object guard at those sites does not cover this, because a typed
instance of the *wrong* class is not a dynamic object.

**The two predicates, and which is which.** `instanceof` must follow the
**prototype** chain, and `instanceofClass`
(`collections/object_descriptor.rs`) is right to do so for a typed instance. A
**brand** -- `#x in o`, and any private member access -- must follow the
**type** chain (`types.is_assignable`, or the base walk above), because the
brand is installed on the instance by the declaring constructor and a prototype
swap cannot move it. Anything that answers a brand question with a prototype
walk is wrong in both directions at once: it says false after
`Object.setPrototypeOf` where node says true, and true for
`Object.create(C.prototype)` where node says false.

**Method note.** The fixture that led here could not have found it: its two
classes both declare `#balance: number`, so reading the wrong object's slot
answers plausibly and the only symptom was `10` instead of `TypeError`. It took
a probe whose two classes differ in *type* to turn a wrong answer into a
visible leak. When a wrong answer involves a slot, write the probe so the two
layouts disagree -- otherwise type confusion looks like arithmetic. This was
the third `"Math"` of the 153 run, after the string field slot and
`newtarget-arrow`; that string is what intern id 0 resolves to, so **`"Math"`
appearing anywhere unexpected means raw bits were read as a string handle.**

## Two `instanceof` bugs, and one is a fold the run time can contradict

Both surfaced while probing the brand work (`3b09567f`) and both were confirmed
pre-existing by running a parent binary, not by reading the code. Neither is
fixed.

    class Base { x: number = 1 }
    const a: any = new Base();
    Object.setPrototypeOf(a, null);
    a instanceof Base          // zun: true.  node: false.

    const b: Base = new Base();
    Object.setPrototypeOf(b as any, Decoy.prototype);
    b instanceof Base          // zun: true.  node: false.
    (b as any) instanceof Base // zun: false -- correct.

The first looks like `instance_class_prototype` falling back to the class
prototype when the relink is `null`.

**The second is the serious one.** Sema folds `instanceof` to a constant when
the operand's static type is `Ty::Instance`, and `Object.setPrototypeOf` can
then make the run time disagree with the fold -- the same object reached
through `any` answers correctly, which is the signature. A fold is only sound
when nothing observable can change the answer, and a relinkable prototype can.
Any other `Ty::Instance`-based fold deserves the same question. It is also a
reminder that the typed and `any` paths must agree: two answers for one object
is the shape to look for.

## Rank fixtures by first diverging line, not by matching lines

The matching-line metric says how close a fixture's *output* is. It does not
say which blocker runs first, and that is what decides what to fix.

Acting on total matches, five `tests/classes/` fixtures looked like one shared
site: 308 of node's lines, matching 57, all dense with class expressions, all
plausibly blocked by a method of a class-in-a-function not seeing enclosing
locals. That subsystem was built (`cc7f0aab`) -- correct, by reference, with
six new tests -- and it **moved zero fixtures**, with stdout byte-identical on
all 169. Each of the five stops earlier on something else: `super` in a
closure, constructor arity, `Dog.bind`, a typed `number` field refusing `null`,
and private names in a lowered class.

Use `diff <zun> <node> | grep -m1 '^[0-9]'` per failing fixture -- the first
diverging line over node's total. That ranking put
`class-private-brands-and-statics` at line 32 of 34, the nearest flip in the
suite, which no matching-line ordering surfaced. Both metrics have a use:
first-divergence picks the target, matching lines measure whether the fix
helped.

## The eight opaque fixtures, and what they actually throw

Until `6ee17919` eight fixtures died printing `unhandled exception:
Object(Obj(0x...))`, so none of them could be diagnosed. The cause was that a
thrown `Error` **subclass** is a typed class instance -- fixed slots, so its
`name` and `message` are typed fields -- while `errors::thrown_text` is handed
only a `&Heap` and cannot reach the metadata that says which slot holds
`name`. `Interp::thrown_object_text` reads them through `instance_field_map`;
the text is rendered at the interpreter's terminal boundary, where the heap is
still registered, and remembered by object address for `Display`.

Their real first blockers, which is where work on them should start:

| fixture | throws |
|---|---|
| `class-expressions` | `TypeError: bind is not a function` |
| `class-mixins` | `TypeError: Cannot store null in the typed field` |
| `fn-kinds-prototype` | `TypeError: reportOwn is not a function` |
| `globalthis` | `ReferenceError: queueMicrotask is not defined` |
| `timers-microtasks` | `ReferenceError: setTimeout is not defined` |
| `proxy-trap-surface` | `TypeError: invalid argument to built-in function` |
| `unicode-sets-v` | `SyntaxError: Invalid regular expression: invalid escape` |
| `symbol-wellknown` | `TypeError: Cannot read properties of undefined` |

`class-expressions` is narrower than it looks: `bind`, `call` and `apply` are
**undefined on a class value** while they work on a plain function, and
`Dog.name`, `Dog.length` and `Dog.prototype.speak.bind(..)` all work. The gate
in `dynamic_call.rs` tests for `Value::Function`, which a class value is not.
Making it work needs bound-constructor semantics -- `new` on the bound result
must construct -- so it is not a one-liner, and the fixture needs roughly seven
further fixes past it.

**Two process lessons from this one bug, which took three attempts.**
Instrument, one env-gated trace per round: the first two attempts were reverted
for guessing, and each trace produced exactly one fact -- the failure is *when*
the value is formatted, not *where* the error travels; then that the boundary
hook is reached with the heap alive and still fails; then that the value is
typed, not dynamic. And **do not collapse an error variant to carry
information**: two versions turned `Thrown` into `Native` once the text was
known, and four tests match on `Thrown(..)`. "Nothing catches past this
boundary" was true and beside the point, because the variant itself is
observable. A side channel keyed by the object's address carries the text and
leaves the value alone.


## `new.target`: fixed for arrows, and one case left that exposes the lowering

`40136d9b` made an arrow inside a constructor inherit `new.target`, which the
language requires and which needed three separate gaps closed: `closures.rs`
zeroing `is_constructor` for every closure (only an arrow inherits it -- a
`function` expression has its own), the capture walk never recording a use of
`this` because `new.target`'s text does not spell it, and `check_new_target`
resolving `this` with `cx.lookup`, which answers for **locals only** while in
an arrow `this` is a capture.

**Method note, because the first three attempts were wasted.** Two of those
three pieces had been written and thrown away: one was verified by trace to
fire and reverted anyway when the whole did not work, and the other was
inserted at a line that earlier reverts had moved out of `add_name`, so it
never ran -- and that silence was written into this file as "the arrow never
reaches the capture walk", which was false. A trace printing the source text of
every closure the walk analyses disproved it in one run. In a codebase this
size a one-line placement slip can invalidate an otherwise correct diagnosis;
confirm the instrumented line is reached before drawing a conclusion from its
silence, and do not discard a piece that was verified to work just because the
composite still fails.

### The remaining case, and what it reveals

`Reflect.construct(P, [], S)` then calling such an arrow answers `undefined`
where node answers `S`; `new S()` is correct. The mechanism is an inconsistency
inside the engine rather than a `Reflect` semantics gap:

    const a: any = new S();
    const b: any = Reflect.construct(P, [], S);
    a.constructor === b.constructor        // true
    typeof a.constructor.__classDesc       // "function"
    typeof b.constructor.__classDesc       // "undefined"

The same value answers a property read two different ways depending on the
receiver expression it came through, so `this.constructor.__classDesc` is not
really two property reads: the class is being resolved from the **instance**
(its `type_rid`), not from the constructor value. An object built with an
explicit newTarget is allocated from `P`'s layout with `S.prototype`, so that
resolution has nothing to answer with.

`new.target` is a property of the *construction*, not of the instance, so the
real fix is to thread the newTarget through construction rather than recover it
from the receiver. Note also that node has no `__classDesc` at all, so zun
answering a function for `a.constructor.__classDesc` is itself a small
divergence on an internal name.


## `fn.length` subtracted an `any` parameter, and two wrong theories on the way

Fixed in `82dfb2ac`. `Signature::optional_count` treats a trailing parameter as
omittable when it has a default **or when its type is `Ty::Any`**, which is
right for arity and wrong for `Function.prototype.length`, which was computed
by subtracting it. `queueMicrotask(callback: any)` therefore answered `0`
against node's `1`. A new `default_count` counts only real defaults and the
length computation uses that.

**Two notes I wrote in this file while chasing it were wrong, and the way they
were wrong is the useful part.** The symptom looked impossible: `fn.length`
seemed to depend on surrounding source text --
`"structuredClone=" + structuredClone.length` answered 2 while
`"x=" + structuredClone.length` answered 0, a literal with a mismatched name
did nothing, and the same literal *broke* `fn.name` in the opposite direction.
From that I theorised first a source-text scan (there is precedent:
`closure_takes_this` really does scan text) and then an interned id used as an
index (there is precedent too: the `"Math"` leak was intern id 0 read as a
handle). Both theories were coherent, had precedent, and were false.

What ended it was one instrumented build printing the signature the fold
actually used: `func=9 name="structuredClone" params=2 opt=2` -- two parameters,
one default, two subtracted. The context-dependence was a red herring
throughout and disappeared with the fix.

So: when a symptom looks impossible, the cost of instrumenting the computation
is one build, and it beats any number of probes that only vary the input. Probes
narrow *what* changes the answer; they cannot show *which* code produced it, and
a plausible mechanism with precedent elsewhere in the codebase is exactly the
kind of theory that survives probing while being wrong.

The prelude's `structuredClone` lost its `= undefined` default in the same
change: a default lowers `length` by JS rules, and node's native declares two
parameters.


## `function_apply_args` was a stale test, not an engine gap

Both tests in `crates/zun_vm/tests/function_apply_args.rs` asserted a call
shape **node rejects**, and are fixed in place. They read

    g.apply(1.0, [cb, 2.0])      // expecting a=1.0, f=cb, x=2.0

which only works if `apply`'s `thisArg` fills the first ordinary parameter.
That is the convention the receiver gate deliberately removed: a free function
takes `this` only when it declares one. node answers
`TypeError: f is not a function` for that source, so zun throwing was correct
all along -- the expectation was wrong. One test now declares
`function g(this: any, f: Function, x: number)`, the other passes all three
arguments in the array; both forms were checked against node, and both pass.

**I wrote a note here claiming it was a real prelude-less gap and named two
`typed_dyn.rs` sites to look at.** That was wrong on both counts. The
prelude-less observation was true but irrelevant -- the test compiles without
the prelude, which is why no `.ts` probe reproduced it, and I took "cannot
reproduce through the CLI" as evidence of a configuration-specific gap instead
of asking whether the assertion itself was right. I then changed those two
sites to `box_any_cell` on the strength of that note; the tests stayed red, the
169-fixture stdout was byte-identical, and the probes already matched node, so
the edits were reverted rather than carried on faith.

The lesson is narrow and worth keeping: **before hunting for the code that
makes a test fail, check what node says about the test's own source.** A red
test is a claim about behaviour, and the claim can be the broken part --
especially one written before an intentional semantic change.

## Prelude top-level statements run AFTER the user's synchronous code

`zun_prelude::with_prelude` concatenates the prelude **after** the user source
("the user's text comes first so its byte offsets are unchanged"), and a
top-level statement in a prelude file therefore executes after the user's
top-level code. Measured:

    console.log("sync=" + DOMException.ABORT_ERR);                       // undefined
    queueMicrotask(() => console.log("micro=" + DOMException.ABORT_ERR)); // 20
    setTimeout(() => console.log("timer=" + DOMException.ABORT_ERR), 0);  // 20

with an installer at the end of `builtin/error.zun` calling
`Object.defineProperty` for each constant. `realm.zun:20`'s `global.set(..)`
does not notice because a global is consulted lazily, long after init.

**So the prelude cannot install anything the user's synchronous code reads.**
Declarations are fine -- they are collected at compile time -- but any
side-effecting setup is too late. A built-in that needs data properties
present from the first statement needs them as class members or engine-side,
not as a prelude `defineProperty` call.

That is why `DOMException`'s legacy numeric constants
(`INDEX_SIZE_ERR` = 1 ... `DATA_CLONE_ERR` = 25, own properties of both the
constructor and the prototype, `w:false e:true c:false`) are still missing:
the installer was written, verified to work from user code, found to run too
late, and removed rather than carried. Class `static` fields would be visible
in time but are non-enumerable and would not land on the prototype, so they do
not satisfy the shape either.

`get [Symbol.toStringTag]()` on the class *does* work and landed:
`Object.prototype.toString.call(new DOMException())` now answers
`[object DOMException]`.

## The typed-value family: one premise, six sites, found by chasing `"Math"`

A `string`/`number`/`boolean`-typed binding can hold anything once an `as any`
reaches it, and every site that trusted the static type failed differently.
Each was fixed where it stood; the premise is untouched and is a soundness
decision, not a defect at any one site.

| site | symptom | commit |
|---|---|---|
| `String(x)` elided for a `Ty::String` operand | a number travelled on wearing a `string` type | `83a99227` |
| `Number(x)`/`Boolean(x)` elided likewise | engine abort | `bf7fc33e` |
| JIT argument packing (`jit_hook::try_call`) | abort, but only after the method warmed up | `bf7fc33e` |
| `encode_field`'s string slot | handed back an unrelated interned string | `222d1d67` |
| string concatenation (`as_str`) | engine abort | `f2671ce6` |
| typed arithmetic (`bin_float`) | engine abort | `5985c2db` |

**Not one of these changed a single fixture's output.** The conformance suite
never exercises a value that violates its static type, which is why the family
survived, and why it only surfaced when an unexplained `"Math"` in a *passing*
fixture was chased rather than written off.

Three method notes worth keeping:

- **Fix the premise's sites by representation, not by a list of values.** The
  first JIT guard named `null` and `undefined`; a `Str` in a numeric parameter
  still aborted. Asking "does this value fit this kind" is one predicate, and
  writing it turned up that `try_loop_osr` already had the same question as a
  local closure -- so it became one shared `packs_into` rather than a third copy.
- **Coercion is not always the answer.** `+` with a string operand concatenates
  even in a slot the compiler typed `number` (`(ops as any)("7", 2)` is
  `"72,5"`, not `8`), while `-`, `*`, `/`, `%`, `min` and `max` do run ToNumber.
  Reaching for ToNumber everywhere would have been plausible and wrong, and no
  fixture would have caught it.
- **A symptom can be a leak, an abort, or a quiet wrong answer depending only
  on which site reads the value next.** When one is fixed, look for the others
  before concluding the class is closed -- `222d1d67` and `f2671ce6` were both
  believed to be the whole bug at the time.

## Pick targets by distinct compile diagnostics, not by line count

Counting *distinct* `[SEM` messages per non-compiling fixture found three
fixtures each held back by a single blocker, worth hundreds of node lines:

| fixture | distinct | total | blocker |
|---|---|---|---|
| `abortsignal-abort-ordering` | 2 | 10 | `onabort` missing; `addEventListener` not forwarding `options` |
| `regexp-object-protocols` | 1 | 1 | `+` rejecting a `RegExp` operand |
| `proxy-constructor-callable-and-brand` | 1 | 3 | a `Ty::Proxy` callee not lowering to an indirect call |

The first went **0 to 53 of 53, a true pass**. No line-count ordering
surfaced any of them: a fixture stuck at zero looks equally expensive whether
it has one blocker or six.

Filter with `grep "\[SEM"`. A plain `grep error:` also matches
`zun: runtime error:`, which inflated several fixtures and briefly made
compiling ones look like they had compile errors.

**Two of the three then needed a second fix, because clearing the diagnostic
exposed a wrong answer.** Routing `RegExp` to `addAny` made `"x" + /a/g`
compile and answer `"x[object Object]"` -- worse than the compile error -- until
`addAny` was pointed at `regex_to_string`, which the file already documents as
"the one assembly every stringification path goes through". Allowing a `Proxy`
callee exposed a non-callable target answering `BadToken`, which is not a JS
error and could not be caught; `RunError::Native` was not enough either, since
the message reached the driver instead of the enclosing `try` -- it has to go
through `throw_native!` at the arm. **Expect to clear a diagnostic and find a
divergence behind it, and treat "it compiles now" as the halfway point.**

## A built-in instance's method is not a first-class value

    const wm: any = new WeakMap();
    wm.get            // undefined -- node: a function
    wm.get.length     // TypeError: Cannot read properties of undefined

The same for `Map`, `WeakMap` and `FinalizationRegistry`, and it is a
**two-layer** gap, which is what makes it a subsystem rather than a patch:

- sema rejects `fr.register` outright with
  `type FinalizationRegistry has no fields` -- the dynamic-read escape in
  `check/expr/field.rs` (~line 668) that covers these built-in types is gated on
  `fname == "constructor"` and nothing else.
- the runtime does not produce the value either: casting through `any` gets
  past sema and still answers `undefined`, so routing sema alone would only
  move the failure. `classMethodBox` boxes a *user* class's method; the
  built-in prelude classes have no equivalent reachable from a member read.

This is what blocks `finalization-registry-surface` (81 node lines) at its
line 22, together with `Reflect.ownKeys(fr)` being refused at compile time with
`` `Reflect` target must be an object ``. Calling the methods works -- only
reading one as a value does not.

Worth knowing before starting: the fix has to land in both layers at once, and
a probe through `any` is the cheap way to tell which layers a gap spans. Here
it turned a one-line sema change into a correctly scoped "no".
