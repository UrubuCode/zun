# JIT ↔ interpreter call bridge (design)

Status: design, not yet implemented. This is the foundation that unblocks the
three subsystems still pinned to Tier 0 — first-class functions / `CallIndirect`,
the cross-frame half of exceptions, and (indirectly) general dynamic dispatch.
It is the same "frame-transfer / packed-args shim" the PLAN defers next to true
OSR.

## Why

Today JIT eligibility is a **whole-call-graph fixpoint**
(`eligible_methods_with_literals` in `crates/zun_codegen/src/eligible.rs`): a
method is eligible only if every method it can call is eligible too, because a
JIT frame has no way to call back into the interpreter. One ineligible method
(anything dynamic, throwing across a frame, or using an unlowered opcode)
therefore disqualifies every caller transitively.

A JIT→interpreter **call bridge** removes that restriction. A JIT method calls
an eligible callee directly (fast path, as now) and an ineligible callee through
the bridge, which runs it on the interpreter on the *same* heap and returns the
result. Consequences:

- Methods become **independently** eligible — no transitive closure.
- `CallIndirect` becomes lowerable: resolve the target rid at runtime; if it is
  JIT-defined call it directly, otherwise bridge to the interpreter. This is
  what first-class functions and callbacks need.
- It is the substrate for **cross-frame exceptions**: a bridged callee that
  throws returns an "exceptional" status the JIT frame routes to its handler or
  re-propagates (see §5).

## Crate-boundary constraint (non-negotiable)

`zun_codegen` (compile side) must **not** depend on `zun_interp` (runtime side);
only the driver/VM links both (see `CLAUDE.md`, PLAN §2). So the bridge cannot
be a plain `zun_jit_*` helper inside `zun_codegen` the way `zun_jit_string_concat`
is — those can call `zun_gc`, but not the interpreter.

The seam:

- **`zun_codegen`** owns the *call site*: it lowers a bridged call to an indirect
  Cranelift call through a **function pointer it is handed at JIT-build time**,
  using the existing packed-args ABI (`crate::shim` / `any_abi`). It defines the
  bridge's C-ABI signature and nothing else.
- **`zun_vm`** owns the *implementation*: it provides the bridge function (which
  calls `zun_interp::run_method_loaded_on_current_heap`) and registers it into
  the JIT. `JitHook` (`crates/zun_vm/src/jit_hook.rs`) is the natural home — it
  already bridges interp→JIT with packed-arg marshalling, so it holds the
  `loader`/module context the reverse direction needs.

Mechanically, `Jit::new` (`crates/zun_codegen/src/lib.rs`) currently hard-codes
its `builder.symbol(...)` set. Add an optional **bridge pointer** to the Jit
construction path (e.g. `Jit::with_bridge(zm, bridge_fn)` or a setter) that
registers the `zun_jit_call_interp` symbol. When no bridge is supplied (as in
`zun_codegen`'s own equivalence tests), eligibility keeps the fixpoint and no
bridged call is emitted; when a bridge is supplied (the real VM), eligibility is
relaxed and bridged calls are allowed. Eligibility therefore takes a
"bridge available" flag, parallel to today's `allow_const_str`.

## ABI

Reuse the packed-args shim already used by `JitHook` and `call_packed_with_heap`
(`crate::shim::pack_result` / `unpack_arg`, `any_abi` carrier for `Any`):

```
extern "C" fn zun_jit_call_interp(
    method_rid: u32,
    args_ptr: *const u64,   // packed argument buffer (shim layout)
    argc: u32,
    out_status: *mut u32,   // 0 = returned, 1 = threw (see §5)
) -> u64                    // packed return (bits per the callee's return kind)
```

The JIT side, at a bridged call:
1. Packs each argument into a stack-slot buffer with `pack_result` (an `Any`
   argument writes its two-word carrier), mirroring the direct-call marshalling
   at `translate.rs:879` (the `JitKind::Other` carrier case).
2. Marks every managed argument live in the call-site stack map
   (`declare_value_needs_stack_map`) — the bridge is a GC safepoint (§4).
3. Calls the bridge pointer, then unpacks the result with `unpack_arg` by the
   callee's return kind (known statically from the callee's `MethodBody`).

## GC safety across the tier boundary

The bridged call can trigger GC inside the interpreter, which may move objects
the JIT frame holds. This is **already handled in principle**: the JIT marks GC
references with Cranelift stack maps, and a call is a safepoint, so the collector
rewrites the JIT frame's live slots through the same `StackMapRegistry` path the
existing runtime helpers use. The bridge adds no new mechanism — it must only be
sure to (a) declare all managed args/live values needing stack maps at the call,
and (b) not hold raw `Obj` addresses across the call in the bridge impl without
re-reading them (the `NativeRootScope` + volatile pattern from
`namespace.rs`/`decorators.rs`). Verification: the existing GC-stress equivalence
suite, extended with a bridged call under forced collection.

## Exceptions through the bridge (phase 3)

A bridged callee that throws returns `out_status = 1` with the thrown value
packed as an object `Any`. The JIT frame then:
- if the call sits inside a protected region whose handler catches that type,
  branches to the catch block (reusing the same-frame landing-pad lowering
  already built — see `zun-jit-eligibility` memory / `exception_regions_jit_safe`);
- otherwise propagates: the JIT method itself returns the exceptional status to
  *its* caller (interp via `JitHook`, or another JIT frame via the bridge), i.e.
  every JIT method gains a hidden "may have thrown" return channel on the paths
  that can reach a bridged call. This is the cross-frame unwinding, done as an
  explicit status return rather than native stack unwinding — no platform EH,
  consistent with "Tier 0 is always correct".

This removes the current restriction that a protected region may contain no
calls, which is what makes same-frame exceptions cover little real code.

## Incremental slices (each in-process verifiable in `zun_vm`, no `.exe` link)

1. **Bridge seam + direct bridged call, no exceptions.**
   - **1a (DONE** — `crates/zun_vm/src/jit_bridge.rs`, `bridge_call`): the safe
     core. Runs an ineligible callee on the interpreter on the current heap with
     packed args, returns the packed result + a returned/threw status. Unit
     tested in isolation (scalar + throwing call). No codegen change, no unsafe.
   - **1b (next):** the `extern "C" zun_jit_call_interp` wrapper + a thread-local
     execution context (scope/loader/output) installed around JIT entry by
     `JitHook`; the `Jit` symbol-injection path; the eligibility "bridge
     available" flag (relax the fixpoint so a method with an ineligible *direct*
     callee is eligible, its call bridged); and the `translate.rs` call-site
     lowering (pack args into a stack buffer, call the pointer, unpack result).
     Verify in `zun_vm`: a JIT method calling a dynamic/ineligible callee matches
     the pure interpreter, incl. under forced GC.
2. **`CallIndirect` via the bridge.** Represent a function value as its method
   rid (+ optional environment object) in a new `JitKind::Function` carrier;
   lower `CallIndirect` to "resolve rid → direct if JIT-defined else bridge".
   Unlocks first-class functions / callbacks.
3. **Exceptional return channel (§5).** Thread the `out_status` through JIT
   frames and route to same-frame handlers or propagate. Unlocks calls inside
   `try`, `finally`, catch-all, and typed-mismatch catches.

Dynamic objects / general `Any` on the typed stack remain a separate axis (a
`JitKind::Any` carrier threaded through the stack), not covered here.

## Revision (found building 1a→1b): bridging and exception propagation are coupled

A "slice 1b with no exceptions" is **not independently correct**. A bridged
callee can throw, and the JIT frame must then propagate — but a *direct* JIT→JIT
call has no status channel, so a JIT method that can propagate a bridged throw
could be entered by another JIT method that never learns it threw. Aborting on a
bridged throw instead would diverge from the interpreter (which propagates),
breaking the equivalence invariant. So safe bridging requires the exceptional
path up front.

The correct shared mechanism is a **uniform exceptional-return channel**, not an
ABI change to every JIT function signature:
- a thread-local "exception pending" flag + the thrown value, set by the bridge
  wrapper (and by a future JIT `throw` that must leave the frame);
- after **every** call in a JIT body (direct or bridged), a check of that flag;
  if set, branch to the method's single **exceptional-exit block**, which returns
  a dummy of the return type leaving the flag set (so callers up the chain also
  branch out);
- inside a protected region, the post-call check branches to the matching catch
  block instead of the exceptional exit — this is exactly how the already-built
  same-frame `throw` lowering routes, now reachable for *calls* in a `try`;
- `JitHook`, after the top-level `call_packed_with_heap`, converts a set flag
  into `RunError::Thrown` and resumes interpreter propagation.

This folds the old slices 1b and 3 into one coherent unit ("bridged calls + the
exceptional-return channel") and makes same-frame `try` regions finally admit
calls. It is a large, cross-cutting codegen change (a post-call check and an
exceptional-exit block in every eligible body) touching the calling convention,
so it lands as one reviewed, GC-stress-verified piece — not piecemeal.

## Verification note

The whole bridge is **in-process testable** via `Jit::compile` +
`call_packed_with_heap` (JIT) and `JitHook` (interp↔JIT), exactly as the current
equivalence and tiering tests run. It does **not** require linking a native
executable — only the AOT pipeline (`zun build --aot`) does, and AOT re-uses the
*interpreter fallback*, not this bridge, for ineligible code. So this foundation
can be built and verified under the project's build-resource limits without the
heavy linker step.
