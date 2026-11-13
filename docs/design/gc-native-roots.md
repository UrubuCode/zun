# GC roots across the native boundary — lifting the pinning policy (plan)

> Status: **phase 1 in progress.** Written 2026-11-13, after the nine-cause hunt
> that took conformance from 107/168 back to 120/168 (`e77901ec` …
> `020e678b`). The last of those nine landed a policy that works but is a
> hammer, and this is the plan to replace it.

## The problem

A collection that **moves** objects is unsafe while native code is on the
stack. Zun's natives are Rust functions inside the runtime, and they hold
managed references in ordinary locals:

```rust
let callback = unwrap_function_box(&args[1])?;   // hoisted once
for element in array {                            // a collection can land here
    self.invoke_function_value(callback, …)?;     // stale after the first one
}
```

The collector can neither find that local nor rewrite it. The first evacuating
collection inside the loop leaves `callback` pointing at the old address, which
by then belongs to a different object, and every remaining element reads
garbage. Measured: `forEach` over eight elements × 400 calls failed; one
element × 3200 calls was fine, because no collection ever landed *between* two
uses of the hoisted copy.

`020e678b` made a minor collection **non-moving whenever a native call is on
the stack** (`roots::collect_young_non_moving`, selected by
`Interp::native_link.is_some()`). That is correct and it is what got the last
fixtures back, but it is the wrong long-term shape: the heaviest allocation in
any real program happens *inside* natives — array, string and iterator builtins
— so it disables promotion and compaction exactly where they are worth most.

## What the CLR does, and which level applies here

From `.docs-dotnet/09-memory-runtime/12-gc-handles.md`,
`10-pinning-fragmentacao-heap.md` and `07-actions-operations/04-pinvoke-marshalling.md`:

1. **Handle table** — the outside world holds an opaque `OBJECTHANDLE`, and
   `Ref_UpdatePointers` rewrites the table's contents during relocation. The
   doc's opening sentence is this exact problem: *objects move, so whoever is
   outside cannot keep the address.*
2. **`GCPROTECT_BEGIN`/`GCFrame`** — for VM-internal C++ holding an
   `OBJECTREF` local. It registers the **address of the local**, so the GC both
   reports and relocates through it. (Not covered by the ten area docs, which
   describe the language-facing surface, not the VM interior.)
3. **Cooperative/preemptive transition** — only for real P/Invoke. Preemptive
   means the thread declares it touches no managed object; RyuJIT enforces *"no
   GC pointers may be live in registers"* with `RefTypeKillGCRefs`.

Zun's natives are case 2, not case 3: they are VM-internal code that genuinely
holds managed values. So the mechanism to copy is `GCPROTECT`, not a mode
transition.

And pinning is the last resort there, not the first. The canonical comment the
pinning doc quotes: *"PINNING AN OBJECT IS EXPENSIVE AS IT PREVENTS THE GC FROM
ACHIEVING OPTIMAL PACKING OF OBJECTS DURING EPHEMERAL COLLECTIONS. THIS TYPE OF
HANDLE SHOULD BE USED SPARINGLY!"*

## Where Zun already stands

The mechanism for case 2 **exists and is almost unused**:
`zun_gc::NativeRootScope::enter(&[Obj])` registers a slice,
`append_native_roots` reports it, and `rewrite_native_roots` **rewrites the
slice in place**. That is `GCPROTECT` with another name.

The one native on the path that uses it protects the wrong thing
(`collections/runtime_iter.rs`, `iter_next`):

```rust
let roots = [it];                                     // a copy of `it`
let _scope = unsafe { NativeRootScope::enter(&roots) };
let next_args = self.with_this_arg(next, it, …);      // uses `it`, not roots[0]
```

The registered slot gets rewritten; the local the native then uses does not.
This is the classic `GCPROTECT`-on-the-wrong-variable bug, except that CoreCLR
has debug builds that hunt it and we have nothing.

Missing against the doc's own "como aplicar na sua linguagem" list:

| Doc item | Zun today |
|---|---|
| strong, weak **and pinned** handles | strong exists but scope-shaped (`NativeRootScope`); weak exists ad hoc (`weak_refs`, `weak_maps`, finalizers); **no pinned handle at all**, which is why the policy pins the whole heap instead of one object |
| a handle table with durable identity | none — and so `__fnbag` keys, `pid_by_obj`, `generator_states`, `typed_views` are each a hand-rolled handle table keyed by address. Three of the nine causes were one of those forgetting to re-key |
| generational handle table | n/a |
| mandatory, traceable `Free` | n/a |

One nuance in our favour: our pin does **not** fragment. CoreCLR pins selected
objects, which creates pinned plugs and gaps and needs heuristics
(`decide_on_gen1_pin_promotion`). We just skip evacuation for that collection,
so the cost is lost promotion and compaction, not holes. That is why it is
survivable as an interim policy.

## Phases

### Phase 1 — GC stress mode

The doc's item 10 is *"test with a stressed GC: wrong handles cause sporadic
failures (object collected while a native uses it; or moved while its address is
in use)"*. Nine causes were found by bisecting nine times. One stress run over
the fixture suite would have surfaced all of them at once, and every later
phase needs this to prove it did not regress anything.

- `ZUN_GC_STRESS=<n>` pins the interpreter's GC budget to `n` allocations
  (`ZUN_GC_STRESS=1` collects at every safepoint). Unset keeps today's adaptive
  policy exactly.
- The budget is read once (`OnceLock`), and `next_gc_interval` must return the
  pinned value instead of growing, or the adaptive doubling defeats the mode
  after the first unproductive collection.
- `n = 1` is for a single reduced reproducer; a suite run wants something like
  `n = 4..16` and a much larger per-fixture timeout.

Deliverable: the env var, the pinned budget, a unit test that
`next_gc_interval` does not grow under stress, and a recorded suite result at
one stress level so the baseline exists.

**Done.** `ZUN_GC_STRESS=<n>` pins the budget; the parse and the policy are pure
functions tested directly, with the `OnceLock` only supplying the argument, so
no test touches the process environment. Cost on an allocation-heavy program
(800k allocations inside native callbacks): `n=1024` and `n=64` are within noise
of the adaptive policy, `n=4` is 1.6x, `n=1` is 3x. Measure after a warm-up run
— the first invocation of the release binary pays enough page-in to swamp the
difference and make stress look *faster* than the adaptive policy.

**It paid for itself on the first run: 114/168 under `ZUN_GC_STRESS=1`, against
120 under the shipped policy.** Six more latent bugs of this same class, in one
suite run instead of six bisections:

| Fixture | Symptom under `ZUN_GC_STRESS=1` | |
|---|---|---|
| `classes-errors/runtime-error-sites` | `call did not resolve to a method` | was in the original 13 |
| `classes-errors/throw-catch-finally-control` | `call did not resolve to a method`, in a closure | was in the original 13 |
| `typed-buffers/typedarray-element-writes-and-set` | `call did not resolve to a method` | new |
| `collections/constructor-protocol-and-subclassing` | panic, `zun_gc/src/strings.rs:11` index out of bounds | new |
| `collections/weak-collections-and-brand-metadata` | panic, same line | new |
| `iteration/iterable-protocol-contract` | panic, same line | new |

**Three of those six turned out to be one bug, and it was mine** — see phase 1b.
`Heap::string` is `&self.strings[handle as usize]`, and the interned-string
table only ever grows, so a handle can only be out of range if it was never a
handle: it is a `FIELD_STRING` slot read out of an object whose memory was
handed out again. That pointed at the nursery, not at a native frame.

Phase 1b fixed it and those three now pass under stress. **Three remain**, and
they are the genuine native-stale-reference class:

| Fixture | Note |
|---|---|
| `classes-errors/runtime-error-sites` | passes under the shipped policy |
| `classes-errors/throw-catch-finally-control` | passes under the shipped policy |
| `typed-buffers/typedarray-element-writes-and-set` | passes under the shipped policy |

All three pass without stress, which is the point: `dfb7a142`/`020e678b` moved
collections away from the bad place rather than closing the hazard. These three
are phase 2's acceptance test.

(`Heap::string` panicking rather than returning an error is its own small
diagnostic debt — `try_string` exists right beside it — but the panic is what
made this one easy to place, so it earned its keep once.)

The two that were in the original thirteen matter most: they pass under the
shipped policy, so `dfb7a142`/`020e678b` moved collections away from the bad
place rather than closing the hazard. This is the work list for phase 2, and
these six are its acceptance test.

### Phase 1b — poison from-space under stress

After an evacuating collection, fill the released nursery with a trap pattern
instead of leaving the old bytes readable. A stale address then faults at first
use rather than reading a recycled object and returning a wrong answer much
later. Every one of the nine causes was silent; this is the cheapest lever that
makes the whole class loud, and it belongs with the stress mode because that is
where the cost is acceptable.

**Done, and it immediately found a bug in `020e678b` — the nursery release
itself.** `Heap::alloc`'s bump path writes only the two header words and relies
on the chunk being zero-initialised for the object's fields. Before
`020e678b`, every evacuating collection abandoned the chunk and the next
allocation came from a freshly zeroed one, so that held. Reusing the chunk
broke it: a new object started life holding a dead object's field values. That
is what three of the six stress failures were, and the `strings.rs` panic was a
`FIELD_STRING` slot full of someone else's bytes.

`reset_nursery` now restores the invariant, zeroing only the extent the epoch
actually handed out — the whole chunk once the epoch grew past one, the bump
cursor otherwise, because the cursor refers to a *later* chunk in that case and
reading it would leave most of the retained chunk dirty. A typical
256-allocation epoch dirties about 8 KiB of the 512 KiB chunk, so the extent
matters: 64x the work otherwise, and it keeps the per-allocation path free of a
memset. Under poisoning the whole chunk is filled instead, and the bump path
zeroes what it hands out.

Result: **117/168 under `ZUN_GC_STRESS=1`** (114 before), and the normal policy
is still 120/168 with a byte-identical pass set. The three remaining stress
failures are the real phase 2 work list.

Worth stating plainly, because it is the lesson of the phase: the stress mode's
first catch was not a pre-existing bug the hunt had missed. It was a bug the
hunt had *introduced*, three days old, invisible to 168 fixtures under the
normal collection budget.

### Phase 2 — a rooted slot natives can hold (done, see outcome below)

Replace hoisted copies with a slot the collector rewrites. The open question is
its shape (see below). Whatever it is, it must cover the three carriers a
native actually holds: `zun_gc::Obj`, `zun_core::FunctionHandle` (its
environment is a managed address) and `zun_interp::Value`.

Scope: 84 call sites of `invoke_function_value` / `invoke_callback` across 20
files under `crates/zun_interp/src/collections/`. Mechanical, but it is the
difference between protecting the variable and protecting a photograph of it.

Order within the phase: the mechanism and its tests first, then one native
converted and proven under `ZUN_GC_STRESS=1` with evacuation forced on, then
the rest in batches by file.

### Phase 3 — lift the pinning policy (done, see outcome below)

Delete the `pinned` branch in `Interp::collect` and
`roots::collect_young_non_moving`, so every minor collection evacuates again.
Phase 3 is done when the full suite passes at 120/168 under
`ZUN_GC_STRESS=4` with pinning removed.

### Phase 4 — a real handle table (not scheduled)

Retire the address-keyed side tables in favour of handles with stable
identity, which removes the whole "forgot to re-key" bug class rather than
fixing instances of it. Also where a `pinned` handle type would live, so a
native that needs a raw interior pointer (byte buffers, interop) can pin one
object instead of the heap.

## Open design question for phase 2

Three candidate shapes, in increasing generality:

- **(a) Re-read from the published argument vector.** `call_builtin` already
  publishes `args` as the innermost activation (`020e678b`/`dfb7a142`), and the
  collector rewrites that memory. A native that re-reads `args[i]` inside its
  loop instead of hoisting gets the fresh value for free — no new API. Covers
  the callback and the receiver, which is where every bug so far was. Does not
  cover an intermediate the native allocated itself.
- **(b) A `NativeRoots` scope owning a `Vec<Value>`,** with natives holding
  indices into it. This is the CLR's `GCPROTECT_ARRAY_BEGIN` shape and covers
  intermediates too.
- **(c) A generic `Rooted<T>` RAII guard** registering the address of the local
  itself, over a trait that enumerates the managed addresses inside `T`. The
  closest analogue to `GCPROTECT_BEGIN`, and the only one that reads naturally
  at the use site (`rooted.get()`).

**Decided: (c), with the safepoint entry points taking `&Rooted<…>`, and (b)
folded in as `impl Trace for [T; N]`.**

(a) is rejected on evidence, not taste. Two findings sink it:

- **The argument vector is often not the published one.** Natives re-dispatch
  through `call_managed_collection` with fresh vectors nobody publishes:
  `collections/mod.rs` (`let mut stripped = args.to_vec()` in the
  `dynamicCall` optional probe), `collections/array_iterate.rs`
  (`&[Value::Object(target), list]`), `collections/array_construct.rs`
  (`&forwarded`, three sites). A native reached that way would be re-reading
  memory nothing rewrites — so (a) is not merely incomplete, it is a hazard in
  its own right that the pinning policy is currently hiding.
- **The existing mechanism cannot express the one carrier that matters.**
  `NativeRootScope::enter` takes `&[Obj]`, and a `FunctionHandle`'s environment
  is not an `Obj`. `array_iterate.rs`'s iterate arm is instructive: it roots
  `[source, result]` and re-reads `roots[0]` every iteration — the author got
  that part right, with a comment saying why — but `callback` is hoisted and
  unrooted, because there is nowhere to put it. Every callback-holding native
  has the same hole for the same reason.

(b) is rejected as the *primary* shape because the copy-out mistake
(`let it = roots[0]`) stays silent, and `iter_next` is standing proof that it
gets made: it registers a copy of `it` and then goes on using the original.

What makes (c) worth the extra work is not fidelity to CoreCLR, it is that
changing the dispatch entry points turns every call site into a compile error
until it is converted, and makes the mistake unavailable to the next author.
Nothing short of that is loud.

**Narrowed once the call sites were counted (78 of them across
`invoke_function_value` and `invoke_callback`).** The plan was for the entry
points to take `&mut FunctionHandle` and root it *internally*, so the
conversion would be a compile error at all 78 sites.

**That forcing function was not built.** What landed is `root!` at the call
site, applied to the natives on the failing fixtures' paths. The reason is
that the handle turned out to be the smaller half of the problem: the same
natives hold `Obj`s they resolved themselves, and the object and the handle
have to be rooted together or the site is still wrong, so a signature change
that only catches handles would have declared the sweep finished while the
objects stayed stale. The cost of skipping it is that the compiler says
nothing about the sites still to convert; they are listed below instead,
which is strictly weaker and should not be mistaken for equivalent.

`Rooted` is therefore the tool for both things a native holds across
re-entry — the `Obj` it resolved or allocated, and the handle it pulled out of
its arguments. `NativeRootScope`, which was supposed to cover the first case,
turns out not to work at all (see below).

Mechanics settled with it:

- `unsafe trait Trace { fn trace(&mut self, visit: &mut dyn FnMut(usize) -> usize); }`
  in `zun_gc`, the visitor taking the old address and returning the new one.
  Reporting is the same walk with an identity visitor. A registry entry is
  `(*mut (), unsafe fn(*mut (), &mut dyn FnMut(usize) -> usize))`, where the fn
  pointer is a monomorphised `trace_erased::<T>` — an ordinary pointer cast, no
  transmute and no shadow copy, so **`Value` being an enum is not a problem**:
  the collector never interprets its layout, it calls code that knows the type.
- `impl Trace for Value` is the match that already exists twice, in
  `Value::rewrite_managed_reference` and (for reporting) in
  `roots::collect_frame_roots`. One impl replaces both, which is a dedup worth
  having on its own: those two matches disagreeing is precisely how a carrier
  gets rooted but not rewritten.
- Orphan rule is fine: `Obj`, `FunctionHandle`, `Any`, `[T; N]`, `Option<T>` in
  `zun_gc`; `Value` in `zun_interp`.
- A `root!(let cb = handle);` macro in the style of `std::pin::pin!`, shadowing
  the owner so the slot cannot be moved. Drops are then strictly LIFO, so the
  registry is a push/pop stack instead of the id search
  `NativeRootScope::drop` does today.
- The slot holds an `UnsafeCell`. This also fixes a real defect in the current
  mechanism: `NativeRootScope::enter(&roots)` takes a shared reference to a
  non-`mut` local and `rewrite_native_roots` writes through
  `ptr as *mut Obj`. It works in practice because the pointer escapes to a
  thread-local, but writing through a pointer derived from a `&` is formally
  undefined behaviour.
- `get()` returns a copy (`Obj`, `FunctionHandle`) or a clone (`Value`), and
  must never hand out a `&T` that outlives a safepoint.

## Outcome of phases 2 and 3

**Landed.** `Rooted<T>` plus `root!`, 40-odd natives converted, and the
pinning policy deleted. Measured on the full suite (168 fixtures, 15s each):

| policy | before phase 2 | after |
| --- | --- | --- |
| adaptive (shipping) | 120 (pinned) / 117 (unpinned) | **120** |
| `ZUN_GC_STRESS=1` | 117 (pinned) / 103 (unpinned) | **120** |
| `ZUN_GC_STRESS=4` | — | **120** |

The adaptive pass set is byte-identical to the one at `020e678b`, so nothing
was traded away for the lift: every minor collection evacuates again and
conformance is where it was. Under stress it is the *same* 120 — a collection
at every allocation, with released memory poisoned, now costs nothing, where
pinning the heap still lost three. That is the point: pinning was hiding the
bugs, not preventing them.

Two holes were outside the natives, in the dispatch loop itself:

- `async_start` reached from a `Call` opcode dropped the caller's activation,
  so a collection inside the coroutine body left the caller's eval stack stale.
- The four dynamic ops (`LoadDynamic`, `StoreDynamic`, `LoadDynamicValue`,
  `StoreDynamicValue`) run user getters, setters and `valueOf` without
  publishing the frame. They now publish it, at a small cost on the dynamic
  path. **Other arms that reach user code the same way (`ToPrimitive` from
  arithmetic, for one) have not been audited.**

### `NativeRootScope` is not just formally unsound, it loses writes

The UB predicted above (writing through a pointer derived from a `&` to a
non-`mut` local) is observable in the release build. `generator_return`
re-read `roots[0]` correctly after `resume_suspended` and still got the
pre-collection address; converting the site to `root!` fixed it.
`decorators.rs` already carried a `read_volatile(&roots[0])` to work around
the same thing. The compiler is entitled to keep `roots[0]` in a register
because nothing it can see ever writes to that local.

So `NativeRootScope` was not an equivalent older mechanism, it was a broken
one: anything re-reading `roots[i]` after a call that can collect was a live
bug, whether or not a fixture exposed it. That is why it was swept rather than
left as an alternative — the next section is what the sweep found and what
replaced the type.

### `NativeRootScope` is gone; what replaced it is two shapes

The 55 fixed-size sites under `crates/zun_interp/` became `root!` guards. Nine
sites could not: a guard holds one value, and they register a
**variable-length run** of them. Those split into two genuinely different
needs, which is why the replacement is two shapes and not one.

**`RootArray`** — a run of managed references the registry **owns**, addressed
by index. Reads and writes go through the registry, which is the same storage
the collector rewrites, so there is no copy in the caller's frame to go stale
and no pointer for the optimiser to assume nothing writes through. The UB is
gone rather than avoided, and nothing here needs to be `unsafe`.

It covers both remaining needs:

- `JSON.stringify` keeps two slots per nesting level and reads them back after
  every `toJSON` and replacer call. It had `write_volatile`/`read_volatile`
  accessors to defeat exactly the register-caching described above; those are
  deleted, because the storage is no longer a local the compiler can reason
  about.
- The JIT entry points (`zun_codegen`, `jit_hook`, `runner`) only need their
  packed arguments **kept alive**: the arguments are packed into raw words
  *before* the call, so what a collection would have to rewrite afterwards is
  the `packed` vector and the JIT frame's own slots — neither of which is the
  registered copy. They build a `RootArray::from_objects(…)` and never read it,
  which is what the comment at the `jit_hook` site has always said ("keep
  object arguments visible").

Order mattered: while a site still re-read `roots[i]` and relied on the
collector rewriting its local array, moving the storage into the registry would
have handed that site its own un-rewritten copy. Hence two commits, conversion
first.

What this does **not** fix, and should not be read as fixing:

- **A JIT frame's own references.** Marking the arguments keeps the objects
  alive; it says nothing about the frame, which is the stack-map question
  (`declare_value_needs_stack_map`), not this one.
- **The interpreter caller across a JIT call.** `runner.rs` registers the
  caller's frame roots so a collection from `zun_rt_alloc` marks them — and
  `zun_rt_alloc` really can collect, and even evacuate — but nothing rewrites
  the caller's `args` or its parent chain, because what is registered is a copy
  of the addresses. That was true before this work and is still true.
- **The collector's own root-slice API.** `Heap::collect` and friends take
  `&[Obj]` and rewrite it through `roots.as_ptr() as *mut Obj`
  (`collection.rs`), which is the same cast that made `NativeRootScope`
  unsound. It is less dangerous in practice, because those callers pass a
  `Vec`'s heap buffer rather than a stack local, but it is the same shape and
  should take `&mut [Obj]`.

### What `Rooted` still does not cover

A guard holds one managed *address word*, so its carriers are `Obj`,
`FunctionHandle` and `Option<T>` of either. A `Value`/`Any` local that holds a
boxed object across a safepoint is still unprotected, and these are known:

- `generator_return` passes `value` to `make_iter_result` after
  `resume_suspended`.
- `typed_dyn.rs` holds `kept: Vec<Value>` and `acc` across callbacks (bigints
  are objects).
- `generatorIteratorNext` leaves `state.pending_return` in a state that is out
  of the table, and therefore untraced, while the body runs.
- `iter_step` returns a `Value` that is read after calls that can collect.

That is what the `Trace` trait sketched above was for, and it is the natural
phase 4 companion to a handle table: one impl for `Value` instead of four
hand-rooted holders. Where a fixture forced the issue, the workaround used
here is a rooted one-slot array as a holder (`arrayFilter`, `arrayFind`,
`arrayGroup`, `objectGroupBy`), which is correct but is a side table standing
in for a type that should carry itself.

### The last stress failure was not a GC bug at all

`classes-errors/runtime-error-sites.ts` failed under stress only, on a
`load_field` that read the poison pattern. The stale-object reading was the
symptom; the cause is that **the core runtime type ids alias the `TypeDef` rid
space**. A promise is allocated as `CoreTypeId::Promise` (6) *with no fields at
all*, and every name-based member lookup keyed on `object.type_rid()` then
resolved whatever class is `TypeDef` 6 — in the prelude, an Error-family class.
So `p.name` bound that class's `get_name()` to the promise and read field slot
0 of an object with no slots, and `String(p)` reached its `toString` and
answered `name + ": " + message` out of a neighbouring object's bytes
(`"Iterator: Iterator"`, a string handle that happened to be in range). Under
`ZUN_GC_STRESS` the neighbour was released and poisoned, so the handle became
`0xDEADBEEF` and the string table panicked. `p.name = x` wrote out of bounds
the same way.

The fix is one predicate, `Interp::type_rid_describes_object`: a real instance
is allocated with exactly its type's declared field count, so a *core* id whose
count disagrees is an alias, not an instance. It gates the four lookups keyed on
an object's own rid (`instance_method_rid`, `instance_member_get`/`_set`, the
`class_named_methods` getter in `op_load_dynamic`, the `toString`/`valueOf`
lookup in `runtime_coerce`, and `instance_class_prototype`), and the field read
and write are additionally bounded by `Heap::field_count`.

Both conditions are needed: a prelude class that genuinely lives at a low rid
still answers true. The real repair is to stop overloading the rid space —
a tag bit, or a separate runtime-kind field in the object header — and it is
worth doing, because the predicate is a guard against the confusion rather
than an end to it. `p instanceof Error` still answers `true` for a promise.

## Verification rules for every phase

- A fix for a rooting or relocation bug lands with a reproducer that **fails on
  the unfixed code** — a mutation check, not just a green test. Every one of the
  nine causes was confirmed that way.
- A reproducer must reach a safepoint: the interpreter polls the budget at
  `newobj` and at loop backedges only, so the callee has to allocate a *typed*
  instance. A callee that only builds dynamic objects never triggers a
  collection and the bug stays hidden.
- A reproducer must put the only reference where the collector cannot see it: a
  module-level `const` is rooted by the realm store independently, so it never
  exercises frame rooting. The value has to live in a function's locals.
- The fixture suite is measured with a per-fixture timeout. Three fixtures used
  to hang forever, so a harness without one never finishes.
