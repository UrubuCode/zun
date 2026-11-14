# Known drift from the declared architecture

Audit snapshot: 2026-11-12, with a fix pass the same day. Line numbers move;
re-locate with a search before acting. Mark an entry FIXED (with the commit)
instead of deleting it.

Conformance: **121/168** fixtures exit 0 (at `551837f2`), and the same 121
pass under `ZUN_GC_STRESS=1` with a byte-identical pass set. The one gain over
the long-standing 120 is `json-stringify-replacer-and-tojson` (`3ebbf3fb`).

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
