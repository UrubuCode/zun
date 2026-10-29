# JIT closure carrier — first-class function values with environments (design)

Status: design, not yet implemented. This is the remaining piece of first-class
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

## Slices (each independently verifiable in-process)

1. **Carrier plumbing + GC, no new source reach.** Add `fn_abi.rs`; make
   `abi_word_count(Function) = 2`; thread two words through `clif_ty` use sites,
   `thread_edge`, `shim`, and the bridge pack/unpack; declare the environment
   word as a stack-map root. Keep ilgen mapping unchanged (still only concrete
   no-closure functions reach the JIT, now with `environment == 0`). Verify:
   the existing stored-no-closure test still passes with the wider carrier, and a
   handcrafted-IL equivalence test exercises a carrier with a non-zero env word
   under GC stress. **No behavior change from source yet** — pure representation
   widening. This is the risky GC slice; land it alone and reviewed.
2. **`CallIndirect` with env dispatch.** Reconstruct the handle (both words) at
   the indirect call site and bridge it. Verify a closure value *constructed in
   the interpreter* and passed across the bridge is called correctly from JIT.
3. **`MakeClosure` lowering (bridged env).** JIT assembles the carrier from a
   bridged env construction. Verify a closure created *and* called inside a JIT
   method reads its captures.
4. **Eligibility relaxation + source end-to-end.** Drop the gate exclusions
   (function param, call-returns-function, `MakeClosure`) now that the carrier is
   sound; map dynamic function types to `Function` where safe. Verify a full
   closure program (capture, return a closure, call it) runs JIT-native and
   matches the interpreter.

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
at once. Per the project's corruption-risk discipline, land **slice 1 (carrier +
GC) as one reviewed piece on its own**, GC-stress-verified, before building
dispatch on top. Do not fold the four slices into a single marathon change.
