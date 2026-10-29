# JIT closure carrier — first-class function values with environments (design)

> **Update 2026-10-29 — dispatch landed, carrier simplified to one word, producer
> remains.** The indirect-dispatch half is implemented (commit `334cadd7`): a JIT
> `CallIndirect` now reconstructs a `FunctionHandle` through a new
> `zun_jit_call_indirect` bridge entry, backed by the interpreter primitive
> `run_function_on_current_heap_bare` (invokes a handle with its environment on
> the live heap). Two corrections to the plan below:
>
> 1. **The carrier is one word, not two.** A non-dynamic `CallIndirect` is
>    monomorphic — the runtime pins `handle.method_rid() == token.rid()` — so the
>    JIT takes the rid from the signature token and the function value needs to
>    carry only the **environment** word (0 = no-closure), rooted exactly like an
>    `Object` SSA value. The two-word identity (rid + bound discriminant) in
>    §"Representation" is only needed for *dynamic* function values, which stay on
>    Tier 0. `fn_abi.rs` and `FunctionHandle::to_carrier_words` remain correct and
>    are reused by the bridge entry, but the operand-stack carrier does not need
>    the identity word.
> 2. **The remaining work is the *producer*, and it cannot be sliced smaller.**
>    A concrete `Ty::Function(index)` closure (environment != 0) is only
>    producible and consumable *inside* a method that runs `MakeClosure`: crossing
>    a method boundary (a return value or a function-typed parameter) erases to
>    `DYNAMIC_FUNCTION` (Tier 0). So there is no cheap "closure consumer" slice;
>    lighting up env != 0 end-to-end requires JIT-lowering/bridging the producer
>    (`MakeClosure` + its `NewDynamic` environment + `StoreDynamic` captures) and
>    relaxing the eligibility gate. Today the `CallIndirect` path passes
>    environment 0, so the landed change is behavior-preserving and verified by a
>    JIT-compiled native stored-function indirect call plus a closure-invocation
>    test that feeds a real captured environment through the bridge primitive.

Status: dispatch implemented; producer (MakeClosure lowering) not yet implemented. This is the remaining piece of first-class
functions in the JIT. Slice 1 (transient `(expr)(args)`) and the stored
no-closure slice (`let f = inc; f(n)`, commit `36bc2b2e`) are done; both carry a
function value as a **one-word method rid**. This document specifies the
**two-word carrier** that lets a function value that may hold a **closure
environment** (or be a bound function, or a dynamic/erased function) be
JIT-native, and the GC integration that keeps it sound.

## Why

After the no-closure slice, the gate in `crates/zun_codegen/src/eligible.rs`
still refuses any method that could put a non-trivial function value into a
`Function` slot:

- a **function-typed parameter** (the caller could pass a closure or bound fn),
- a **call that returns a function** (`call_returns_function`),
- any **`MakeClosure`** (excluded by `opcode_supported`),
- any **dynamic** (`DYNAMIC_FUNCTION`/`_PREDICATE`/`_REDUCER`) function value
  (mapped to `Other` → Tier 0 in `zun_ilgen::helpers::jitkind_byte`).

Each of these is excluded for exactly one reason: a one-word rid cannot carry a
closure environment or a bound record. The runtime already models the full value
as `zun_core::FunctionHandle`:

```
struct FunctionHandle { method_rid: u32, reserved: u32, environment: usize }
```

`reserved == BOUND_TAG` marks a bound function (its `environment` is a bound
record, not a closure env); `environment != 0` is a managed GC address.
`relocate_environment` moves that address on GC compaction. A JIT carrier that
mirrors this handle removes every exclusion above.

## Representation: a two-word function carrier

Add a dedicated carrier module `crates/zun_codegen/src/fn_abi.rs`, mirroring
`any_abi.rs` (do **not** reuse the `Any` carrier — `Value::Function` is
deliberately excluded from `Any` precisely so environments stay visible to
precise GC rooting; see `zun_core::Any::box_any`):

```
WORD0_OFFSET = 0   // packed identity: (method_rid as u64) | ((reserved as u64) << 32)
WORD1_OFFSET = 8   // environment: managed GC address, or 0
CARRIER_SIZE = 16
CARRIER_ALIGN_SHIFT = 3
```

`abi_word_count(JitKind::Function)` becomes **2** (today it is 1). This is the
central change: `JitKind::Function` is promoted to a two-word carrier, with
`environment == 0` representing the no-closure case the one-word slice handled.
The one-word slice is thus **subsumed**, not replaced — same source, wider
carrier.

Threaded exactly like the `Any` carrier:

- `clif_ty(Function)` stays `I64` for the *identity* word; the carrier lives in a
  stack slot addressed by `WORD0/WORD1_OFFSET`, like `any_abi` (see
  `translate.rs` around the `CARRIER_SIZE`/`TAG_OFFSET` stack-slot code).
- `thread_edge` must thread **two** values for a `Function` operand (identity +
  environment), as it already does for `Other`/`Any`.
- `shim.rs` (pack/unpack) and the bridge's `jit_hook::{pack_value,unpack_value}`
  marshal two words and reconstruct a `FunctionHandle`.

## GC integration (the hard part)

The environment word is a **managed reference**. It must be a precise GC root
whenever a safepoint can run (every bridged call, every allocation slow path):

- The value holding the environment word is declared with
  `builder.declare_value_needs_stack_map(env_value)` — the same call used for
  every `Object`/`Array` reference in `translate.rs`.
- On compaction the environment address moves; the carrier must be updated with
  the relocated address. The precedent is `JitAnyAbi::append_managed_root` (roots
  an object payload by inspecting the tag first). For the function carrier the
  rule is simpler: **word1 is a managed root iff it is non-zero** (no tag needed;
  a bound record and a closure env are both traced the same way, exactly as
  `FunctionHandle::environment` + `relocate_environment` already do on the
  interpreter side).
- A bound function (`reserved == BOUND_TAG`) traces its `environment` (the bound
  record) identically — no special case in the stack map, matching the handle.

This is the single highest-risk area: a missed root or a non-relocated carrier
word is a use-after-free that only shows under GC pressure. It must land with a
GC-stress equivalence test (allocate in the callee across the safepoint, force a
collection, verify the environment survives and the closure still reads its
captures).

## `CallIndirect` with an environment

The interpreter side already does the right thing (`zun_interp/src/lib.rs`
`CallIndirect`): it reads `Value::Function(handle)`, reconstructs
`handle.environment()` as an `Obj`, and dispatches (closure, bound, or plain).
The JIT lowers an indirect call by reconstructing the two-word carrier into a
`FunctionHandle` and dispatching **through the existing bridge**
(`emit_bridged_dispatch`): the bridge marshals the handle to the interpreter,
which runs the target on the same heap. No new runtime entry point is needed —
the bridge already carries a function value; it only needs the environment word
alongside the rid.

The monomorphic-token invariant (a non-dynamic `CallIndirect` faults `BadToken`
unless `handle.method_rid() == token.rid()`) is unaffected: a closure of a
specific function still has that function's rid, and a dynamic call uses token
rid 0 (the `dynamic` path skips the pin and dispatches purely on the handle).

## `MakeClosure` lowering

`MakeClosure` is currently built in ilgen as `NewDynamic` (an environment
dynamic-object) + per-capture `Dup`/store + `MakeClosure(token)`
(`zun_ilgen/src/lib.rs` ~978). To JIT it, the environment allocation and capture
stores (`NewDynamic`/`StoreDynamic`) must either be lowerable or bridged. The
pragmatic first step: **bridge the environment construction** (it is an
allocation + dynamic stores, already ineligible opcodes) and have `MakeClosure`
in the JIT assemble the two-word carrier from (token rid, bridged env address).
Lowering `NewDynamic` natively is a separate, later optimization.

## Slicing — and why the carrier is one atomic change

A first draft of this plan split off "carrier plumbing + GC, env == 0" as an
independent slice 1. **That is not independently meaningful.** The current
one-word rid representation already carries `environment == 0` correctly (that is
exactly the no-closure case shipped in `36bc2b2e`). Widening to a two-word
carrier whose second word is always zero adds ABI-ripple risk (every
`JitKind::Function` site moves from one word to two) while producing *no*
observable behavior and *nothing to GC-root* (a constant zero is not a live
pointer). The GC rooting — the whole point — only becomes exercisable once
`MakeClosure` can put a **non-zero** environment into the carrier.

So the carrier is genuinely **one atomic change**: representation widening +
environment production (`MakeClosure`) + environment dispatch (`CallIndirect`) +
GC rooting must land together, because none of them is observable or testable
without the others. It is testable only end-to-end, with a real closure that
captures, survives a collection, and is called.

Implementation order *within* that single change (not separate commits):

1. Add `fn_abi.rs` (carrier constants + a `FunctionHandle` pack/unpack helper),
   fully unit-tested in isolation — the one piece that *is* safely standalone.
2. Follow the `Any` carrier model on the operand stack: a `Function` operand is a
   one-word **pointer to a 16-byte stack slot** `{identity, environment}` (the
   stack holds one `Value` per slot; `Any` already does this — see the
   `stack_addr`/`TAG_OFFSET`/`PAYLOAD_OFFSET` code in `translate.rs`), *not* two
   parallel stack entries. This confines the ABI-word-count ripple.
3. `LoadFunction` writes `{rid, 0}`; `MakeClosure` writes `{rid, bridged-env-addr}`;
   `CallIndirect` reads both words, reconstructs the handle, bridges it.
4. Declare the environment stack-slot word as a GC root and relocate it on
   compaction (mirror `relocate_environment`). **Dominant risk.**
5. Relax the eligibility gate (function param, call-returns-function,
   `MakeClosure`) and map the now-representable dynamic function types.

Verify end-to-end with a GC-stress closure test: a closure captures a local,
allocation in the callee forces a collection across the safepoint, and the
closure still reads its captured value after the move.

## Risks / open questions

- **GC correctness under compaction** (slice 1) is the dominant risk. The
  two-word carrier in a stack slot must be a precise root *and* be rewritten with
  the relocated address; confirm the stack-map machinery can express a root that
  is a carrier word (vs. a bare SSA value) — check how `Any`-carrier object
  payloads are currently rooted through `StackMapInfo.reference_slots`.
- **ABI word-count change ripples.** Every site assuming `Function` is one word
  (shim, bridge, `thread_edge`, call-arg marshalling) must move to two. Grep for
  `JitKind::Function` and audit each.
- **Bound functions** share the `environment` slot with closures via
  `reserved == BOUND_TAG`; the carrier must preserve `reserved` across relocation
  (the handle's `relocate_environment` already does — mirror it).
- **`DYNAMIC_FUNCTION` return typing.** A dynamic indirect call returns `any`
  (narrowed at the use site); unchanged by this work, but the eligibility
  relaxation in slice 4 must keep dynamic indirect calls (token rid 0) on the
  bridge dynamic path.

## Guardrail

This change touches value representation, the calling convention, and GC rooting
at once, and (per the slicing note above) cannot be meaningfully split — it is
observable only end-to-end with a real closure. Per the project's
corruption-risk discipline it must therefore land as **one focused, reviewed
change in a dedicated session**, GC-stress-verified, *not* speed-run at the tail
of an unrelated session. The only safely-standalone preparatory step is the
unit-tested `fn_abi.rs` carrier module (step 1 above); everything after it is
coupled and should be built and reviewed together.
