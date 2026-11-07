# Promise + event loop + timers: optimization audit

## Summary

- Everything (promise natives, reactions, combinators, timers, the loop) runs Tier 0 only, in `zun_interp`. `Await`, `promise*` and timer builtins are absent from the JIT whitelist in `crates/zun_codegen/src/eligible.rs` (the whitelist at ~L88-120 does not list `Opcode::Await`; `CallBuiltin` is allowed only for date/math/collection/alloc/string names). Verified by whitelist reading, not by a runtime census.
- The dominant verified cost is that async state is never reclaimed and is rescanned on every step. `pid_by_obj` and `PromiseTable.cells` grow monotonically and `reconcile_settlements()` walks all of `pid_by_obj` before every single microtask (O(P) per step, O(n*P) per drain). Every GC also rebuilds that whole map and re-clones every pending reaction to collect roots.
- Per-call allocation is high: `then` makes 1 promise, 1 reaction, 2 bridge cells and 2 hash inserts. `queueMicrotask` makes 2 promises. `new Promise` makes 2 dynamic-object settlers. Returning a promise from a callback adds a dummy promise and an extra tick. Timers are a linear `Vec` (O(T) per fire, O(T^2) total) plus linear cancel lists.

## Per-method table

Paths are in `crates/`. "allocs" counts heap or Rust allocations that were verified by reading; the HashMap/Vec insertions that may reallocate are counted as one.

| method | current path file:line | current complexity | possible complexity | allocs/call | JIT eligible/lowered | severity |
|---|---|---|---|---|---|---|
| dispatch of every promise/timer native | `zun_interp/src/collections/mod.rs:587-606` (arm per name) then `promise.rs:10` (second `match name`) | string compare chain in a very large `match`, twice | O(1) by native id | 0 | no (Tier 0 only) | Medium (cross-module) |
| `new Promise(executor)` | `collections/promise.rs:41-83` | O(1) | O(1) | 1 promise + 2 dynamic records (each 2 string-keyed `dynamic_set`) + `vec![resolve,reject]` | no | High |
| `Promise.withResolvers` | `promise.rs:19-39`, `runtime_class_proxy.rs:150-170` | O(1) | O(1) | promise + result obj + 2 settler records (4 string-keyed sets) | no | Medium |
| settler call `resolve(v)`/`reject(e)` | `runtime_class_proxy.rs:174-200` (`settler_of`), `runtime_typed_array.rs:500-506` | 2 string-keyed `dynamic_get` per call, plus again in `invoke_callback` | O(1) typed field read | `args.remove(0)` on a `Vec`; reject boxes a `String` | no | Medium |
| `promiseResolve`/`promiseReject`/`State`/`Result`/`IsReady` | `promise.rs:84-125`; heap `zun_gc/src/allocation.rs:128-166` | 1 side-`HashMap<usize,ManagedPromise>` lookup each | O(1) inline field | reject copies a `String` | no | Medium |
| `then` / `catch` | `runtime_reaction.rs:~286-317`, `attach_reaction` L14-39 | O(1) plus O(P) amortised GC rebuild | O(1) | 1 promise, 1 `Reaction`, 2 `bridge_pid` (scheduler cell + hash insert each), 1 `HashMap` insert, 1 waiter push | no | High |
| reaction run (microtask) | `runtime_reaction.rs:44-290`, `state.rs:~455-500` (`next_microstep`) | O(P) per step (`reconcile_settlements`) | O(1) per step | callback `Vec<Value>` arg; rejected-no-reason path interns an error string per reaction; `invoke_callback` redoes `read_body` per call | no | Critical |
| `finally` | `runtime_reaction.rs:~320-340`, `run_reaction` L113-190 | as `then` | as `then` | as `then`; a thenable cleanup result adds a promise + `FinallyPass` reaction | no | Medium |
| return a promise from a callback / async fn | `runtime_async.rs:10-60` (`settle_target_with`) | O(1) | O(1) | dummy promise + `ThenableJob` reaction + second reaction (extra ticks) | no | High |
| `Promise.resolve` (static) | `promise.rs:127-165` | O(1) | O(1) | promise; `heap.is_dynamic_object` + `instance_method_rid(obj,"then")` name lookup for any object arg | no | Low |
| `Promise.reject` (static) | `promise.rs:167-190` | O(1) | O(1) | promise; boxes reason, `value_to_error_string` | no | Low |
| `Promise.all` | `runtime_reaction.rs:~375-420`, `combinator_input_settled` | O(n) attach + O(P) per step | O(n) | `read_array_values` Vec (n), holder array (n), per non-promise element a boxed promise, n reactions, n HashMap inserts, `aggregates` entry (never freed) | no | High |
| `Promise.allSettled` | same, `runtime_reaction.rs:~535-580` | as `all` | as `all` | plus 1 dynamic object per element with 2 string-keyed sets and 1 boxed status string | no | High |
| `Promise.race` / `Promise.any` | same | as `all` | as `all`; `race` can detach losers' reactions | `race` leaves n-1 reactions parked until their sources settle; `any` builds an AggregateError dynamic obj (3 sets, 2 boxed strings) | no | Medium |
| `await` | `zun_interp/src/lib.rs:1566-1595`, `runtime_async.rs:~330-400` | O(locals+stack) frame copy per suspend + the reaction costs | O(1) amortised | `locals.to_vec()`, `stack.to_vec()`, `ExecutionFrame`, `parked` HashMap insert, then `attach_reaction` | no (no `Await` in whitelist) | High |
| `setTimeout`/`setInterval`/`setImmediate` | `promise.rs:166-205`, `runtime_coerce.rs:28-45` | O(1) push | O(log T) heap push | 1 `TimerEntry` (Vec grow) | no | Low |
| timer fire | `runtime_coerce.rs:~83-145` (`earliest_timer_index`, `fire_one_timer`) | O(T) scan + O(T) `Vec::remove` per fire, O(T^2) total | O(log T) with `BinaryHeap` | `entry.args.clone()`, `callback.clone()` per fire | no | High (many timers) |
| `clearTimeout`/`clearInterval` | `runtime_coerce.rs:47-55`; `zun_sched/src/queue.rs:89-100` | O(T) `retain` + O(C) `contains` on an append-only cancelled list | O(1) or O(log T) | cancelled list grows forever | no | Medium |
| `setTimeoutArgs` | `promise.rs:207-240` | O(T) `iter_mut().find` after arming | O(1) (pass args into `timer_arm`) | `read_array_values` Vec | no | Low |
| `queueMicrotask` | `runtime_coerce.rs:60-79` | as `then` | O(1) with a plain job queue | 2 promises + Reaction + 2 bridge cells + hash inserts | no | High |
| event loop drain | `runner.rs:833-910` | O(n*P) | O(n) | per-step `reconcile_settlements` allocs a `Vec` when settlements are found | no | Critical |

## Findings

### F1. Per-microtask `reconcile_settlements` scans every promise ever bridged (Critical)

Evidence: `zun_interp/src/state.rs` `next_microstep` loop calls `self.reconcile_settlements()` before every `scheduler.next()`. That function does:

```rust
for (&addr, &pid) in &self.async_state.pid_by_obj {
    if self.async_state.promises.state(pid) != zun_sched::PromiseState::Pending { continue; }
    ...
    match zun_gc::with_current_heap(|heap| heap.promise_state(promise)) {
```

`pid_by_obj` (`state.rs:82`) is only inserted into (`bridge_pid`, `state.rs:413-425`); the only other write is the full drain-and-reinsert in `rewrite_after_move` (`state.rs:315-321`). No removal path was found (grep for `pid_by_obj` shows only these sites). `PromiseTable::cells` (`zun_sched/src/promise.rs:103,120`) is a `Vec` that is only pushed to. Pending entries each cost a hash lookup into the GC heap `promises` side table. A drain of n reactions over P live-or-dead bridged promises is O(n*P).

Why it matters: a long-running program that creates many promises (every `then`, `await`, `queueMicrotask`) pays more per microtask the longer it runs.

Proposal: (a) drop the per-step scan; keep it as a once-per-turn safety net before the drain (its own comment says it covers only host-driven `heap_mut()` settlement), or have the host path call `wake_scheduler`. (b) Remove `pid_by_obj`/cell entries when a promise settles and has no waiters, and on GC when the promise is dead. (c) Longer term, store the `PromiseId` or settle state on the promise object so no side table is needed.

Est. gain: turns O(n*P) into O(n) for async-heavy loops; likely the largest win in this module (not measured).
Risk: medium. Host-driven settlement must still wake reactions; needs a test where the host resolves a promise between polls. Spec microtask order is unaffected if the scan only moves earlier.
Frozen layout/.zm/ABI/GC: GC side only (promise side-table keys); no .zm or frozen layout. (b)/(c) touch GC bookkeeping, requires approval if the `Promise` object layout changes.

### F2. Per-promise allocation and bridging cost in `then`, `catch`, `finally`, `queueMicrotask` (High)

Evidence: `attach_reaction` (`runtime_reaction.rs:14-39`) calls `bridge_pid(source)` and `bridge_pid(target)` (a scheduler cell plus a `HashMap<usize,PromiseId>` insert each), inserts a `Reaction` into `reactions: HashMap<ContinuationId, Reaction>` (SipHash default hasher), then `scheduler.park` pushes a waiter `Vec`. `promise_then` additionally calls `alloc_promise`, which inserts a `ManagedPromise` into the heap side `HashMap` (`zun_gc/src/allocation.rs:96-100`) with `error: Option<String>`. `queue_microtask` (`runtime_coerce.rs:60-79`) allocates a resolved promise and a target promise just to run one callback.

Why: three separate keyed tables (heap `promises`, `pid_by_obj`, `reactions`) are maintained per promise, all hashed with the default hasher, and `Reaction` carries `source`/`target` as `Value` (two enum clones plus `Option<Value>` callbacks).

Proposal: (a) `queueMicrotask` should push a `Microtask` job onto the continuation queue directly, with no promise pair. (b) Use an `FxHash`/identity hasher or a `Vec` slab indexed by `ContinuationId` for `reactions`/`parked`. (c) Do not bridge `target` eagerly; bridge it lazily when something attaches to it. (d) Keep the reaction inline in the promise cell (the standard fulfill/reject reaction list) instead of side tables.

Est. gain: roughly halves allocations and hashing per `then`/`queueMicrotask` (estimate, not measured).
Risk: (a) low; (b) low; (c) must preserve the reconcile-on-first-bridge behaviour (`bridge_pid` doc comment) and unhandled-rejection tracking; (d) high.
Frozen layout/.zm/ABI/GC: (a)-(c) none; (d) touches the Promise object layout and GC tracing, requires approval.

### F3. Extra promise and tick when a callback or async function returns a promise (High)

Evidence: `settle_target_with` (`runtime_async.rs:10-60`): for a native Promise value it allocates a `dummy` promise, resolves it, and attaches a `ThenableJob { then: Void }` reaction to it, which in turn (`runtime_reaction.rs` ThenableJob arm) attaches a second `Then` reaction to the inner promise. The code after the early `return;` (the direct adopt/attach match) is dead (`#[allow(unreachable_code)]`).

Why: this matches the spec's NewPromiseResolveThenableJob tick count (kept intentionally per the comment), but it pays 1 promise + 2 reactions + 2 bridge cells for each such return, and every `async` function that returns a promise goes through it.

Proposal: keep the observable tick count but implement the job as a direct job-queue entry (no dummy promise, no bridge). Delete the dead branch. This is the same direct job queue as F2(a).
Est. gain: removes 1 promise + 1 reaction + 2 hash inserts per promise-returning callback.
Risk: medium; microtask ordering tests (`crates/zun_vm/tests/promise_surface.rs`) must pass unchanged.
Frozen layout/.zm/ABI/GC: none.

### F4. Timers: linear selection, linear cancel, append-only cancel list (High with many timers)

Evidence: `earliest_timer_index` (`runtime_coerce.rs:~83-100`) scans all entries; `fire_one_timer` then does `entries.remove(index)` (O(T) shift), so n timers cost O(T^2). The `TimerHeap` doc (`state.rs:~105`) says it is a `Vec` on purpose so callbacks can be rewritten after a GC move. `timer_clear` calls `scheduler.cancel_task` (`zun_sched/src/queue.rs:89-93`: `cancelled_tasks.contains` then `push`, append-only) and `entries.retain`. `is_task_cancelled` is a linear `contains` and is called for every task pop and every interval re-arm. `setTimeoutArgs` arms then does `entries.iter_mut().find` to attach args. Each fire clones `entry.args` and `callback`.

Why: fine for tens of timers; a program that arms thousands (debounce, polling loops) goes quadratic, and the cancelled list never shrinks.

Proposal: `BinaryHeap<(due, seq)>` plus a side `Vec`/slab of entries so rewrite-after-move still iterates the slab; cancel by tombstone flag on the slab entry (O(1)) and drop the `cancelled_tasks` `Vec` (or make it a `HashSet` and remove the id when the cancelled task is popped). Pass `args` into `timer_arm` directly. Move args instead of cloning for one-shot timers.
Est. gain: O(T^2) to O(T log T); removes unbounded growth of the cancelled list.
Risk: low; the fire order is `(due, seq)` and must stay stable (FIFO among equal due).
Frozen layout/.zm/ABI/GC: GC rooting of timer callbacks must keep working (they stay in the slab walked by `append_parked_roots`); no layout or .zm change.

### F5. GC root collection and relocation touch all async state on every collection (High)

Evidence: `append_parked_roots` (`state.rs:~345-385`) is called from `collect()` (`lib.rs:1635-1640`) for every collection including young ones. For each reaction it builds a fresh `vec![reaction.source.clone(), reaction.target.clone()]` plus kind clones and calls `collect_frame_roots`. `rewrite_after_move` (`state.rs:~290-322`) drains and reinserts the whole `pid_by_obj` map after any evacuating or compacting move. Both are O(R + P + parked + timers), independent of how much of that state is young.

Why: with many pending reactions or a large dead-promise `pid_by_obj`, each minor GC pays a cost proportional to total async state.

Proposal: iterate roots without the temporary `Vec` (push straight into `roots`); keep `pid_by_obj` small via F1(b); for young collections consider a remembered set for reactions that point to young objects. Combined with F1/F2(d) the map can disappear.
Est. gain: removes per-GC allocation per reaction; the rest depends on F1.
Risk: medium; GC-root completeness is correctness-critical.
Frozen layout/.zm/ABI/GC: GC integration yes (root enumeration); requires approval for any change to the root-set contract.

### F6. Aggregates and combinator allocation (Medium)

Evidence: `promise_combinator` pushes into `async_state.aggregates: Vec<Aggregate>` (`state.rs:98`). It is iterated by GC root/rewrite code (`state.rs:305`, `364`), and a settled aggregate is only flagged `settled = true`, never removed. So every `Promise.all` ever called stays in the Vec and is rooted (its `target` and `holder` array stay reachable). `read_array_values` copies the input array into a `Vec<Value>` and unboxes every slot. `allSettled` builds one dynamic object per input (`runtime_reaction.rs:~535-580`, `alloc_dynamic` + 2 `dynamic_set` by string key + a boxed status string created each time via `Rc::from`). `race` leaves the other n-1 reactions parked.

Proposal: free the `aggregates` slot when `settled` (slab with free list, since the index is baked into `ReactionKind::Combinator`); intern the `"fulfilled"`/`"rejected"` status strings once; give descriptors a fixed shape instead of string-keyed dynamic sets; avoid the intermediate `Vec` by reading elements as needed.
Est. gain: bounds memory for combinator-heavy loops; less per-element allocation in `allSettled`.
Risk: low to medium (index reuse must not alias live reactions; a settled aggregate with still-parked reactions must be tombstoned, not reused until its remaining count reaches zero).
Frozen layout/.zm/ABI/GC: none unless descriptors get a dedicated layout (then dynamic-object shape, requires approval).

### F7. Settler records are dynamic objects with string-keyed fields (Medium)

Evidence: `make_promise_settler` (`runtime_class_proxy.rs:150-170`) allocates a dynamic object and sets `__settleMode` and `__settlePromise` by string key. `settler_of` (L174-190) does two string-keyed `dynamic_get`s, and `invoke_callback` (`runtime_typed_array.rs:500-506`) calls `settler_of` on every callback invocation (guarded only by `handle.is_bound()`).

Proposal: a dedicated settler handle kind (a typed record with fixed `promise` and `mode` slots), or a `FunctionHandle` flag for native settlers, so detection is a bit test and the payload is two fixed fields. Also share one record for the resolve/reject pair (mode derived from the handle tag).
Est. gain: removes 2 allocations and 4 string-keyed accesses per `new Promise`.
Risk: medium; changes `FunctionHandle` semantics.
Frozen layout/.zm/ABI/GC: touches the frozen function handle encoding if a new tag is used (requires approval); a typed record alone does not.

### F8. Callback invocation overhead (`invoke_callback`) (Medium)

Evidence: `runtime_typed_array.rs:500-523`: each call does `generator_method()`, `settler_of`, `resolve_call_target`, `method_defs.get`, `read_body` (decodes a body header), `strings.get(target.name).ends_with("@rest>")` (a string compare per call), `final_args.truncate`, then `self.call`. `run_then_callback` allocates `vec![arg]` per reaction.

Proposal: cache per `FunctionHandle`/method rid the resolved `(param_count, is_rest)`; a precomputed per-method flag array replaces `read_body` + name `ends_with`.
Est. gain: constant-factor per callback (reactions, timers, microtasks, array callbacks share this).
Risk: low.
Frozen layout/.zm/ABI/GC: none (cache is interpreter-side). Cross-module, see final report.

### F9. Promise state lives in a heap-side `HashMap`, and doubles as the "is this a Promise" test (Medium)

Evidence: `zun_gc/src/lib.rs:223` `promises: HashMap<usize, zun_core::ManagedPromise>`; `promise_state(obj)` is `self.promises.get(&addr)`. Used as a type test in `as_input_promise`, `settle_target_with`, `promiseResolveStatic`, `Await` (`settled_await_value`) and as the read path for every state/result read. `ManagedPromise` (`zun_core/src/promise_value.rs`) carries `error: Option<String>` (a heap `String` per rejection) and a second `reason: Option<Any>`.

Proposal: store `state`, `result`, `reason` in the object payload (needs a layout slot) and keep the message lazily derived from `reason` only when a string surface needs it. Type test becomes a header type-id check.
Est. gain: removes a hash lookup per promise access and a `String` allocation per rejection.
Risk: high (moving GC rewrites the side map keys today; layout change).
Frozen layout/.zm/ABI/GC: yes, frozen Promise object layout and GC tracing; requires approval.

### F10. Unhandled-rejection report is a reverse linear search (Low)

Evidence: `runner.rs:~863-868` finds the heap address by scanning `pid_by_obj` per unhandled rejection: O(U*P) at turn end.

Proposal: store the `Obj` alongside the id in the `unnoticed` set, or a `Vec` slab indexed by `PromiseId`. Low priority; fixed by F1.
Risk: low. Frozen layout/.zm/ABI/GC: none.

### Not confirmed (hypotheses)

- `reactions`/`parked` HashMap default hasher cost vs an identity hasher: not measured.
- `sema` mcall_managed.rs: grep found no Promise or timer references there, so no sema-side dispatch cost was identified; the Promise surface appears to route via `call_builtins.rs`. Not audited further.
- `zun_vm/src/promise.rs` (1074 lines) exists and may be a second Promise implementation; not read in this audit, so nothing is claimed about it.
- Whether a JIT-compiled caller pays a bridge round trip when calling a `promise*` builtin: not confirmed (the builtins are not in the JIT whitelist, so such callers are Tier 0).
- Prelude `async.zun` async-generator wrapper allocs (`Promise.withResolvers()` per request at L90): not measured.

## Cross-module notes

See the final report sent to the caller for items shared with other builtin modules (native name dispatch, `invoke_callback`, `Any` boxing, GC rooting of async queues, interp frame copies).
