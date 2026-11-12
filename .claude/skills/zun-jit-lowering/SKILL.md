---
name: zun-jit-lowering
description: How to write and review Tier 1 code in `crates/zun_codegen` - Cranelift 0.131 lowering rules for this project, host thunks that share semantics with the interpreter, FuncRef/signature import caching, GC rooting and stack maps, eligibility in `eligible.rs`, and interp/JIT equivalence. Use this for any work under `crates/zun_codegen/`, any `zun_jit_*` host function, any change to JIT eligibility or OSR, any Cranelift IR question, and when a JIT result differs from the interpreter.
paths: "crates/zun_codegen/**"
---

# Tier 1 (Cranelift) lowering in Zun

`docs/guides/cranelift.md` is the authoritative Cranelift 0.131 reference for this
repo. Read the relevant section before emitting IR you have not emitted here
before; Cranelift's API changes between versions and memory of other versions is
unreliable.

The JIT has one job: produce the **same observable result as Tier 0**, faster.
Speed that changes a result is a bug, and the equivalence suite is how that is
caught, so every lowering ships with an equivalence test.

## Rule 1: one semantic, two callers

The audit of 2026-11-12 found the JIT string thunks had re-implemented `replace`,
`padStart`/`padEnd`, `repeat`, `indexOf` and `substring` by hand, and three of
them had drifted from the interpreter (missing `$&` expansion, wrong empty-fill
result, missing RangeError). The pattern that prevents this:

- The semantic lives in **one function**, in `zun_core` when it is pure
  (`zun_core::string_ops::{find_from, last_index_from, substring_units, pad_units,
  repeat}`, `zun_core::expand_substitution`, …).
- The Tier 0 native and the JIT thunk are both thin wrappers that marshal
  arguments and call it.
- If the helper you need exists only inside `zun_natives` or `zun_interp`, move it
  to `zun_core` first; `zun_codegen` must not depend on `zun_interp`.

Before writing a thunk body, search `zun_core` and `zun_natives` for the operation.
If a thunk cannot match Tier 0 for some input, make eligibility reject that case
rather than approximating it.

## Rule 2: inline or thunk

- **Inline** in `src/translate/` when the op is a handful of instructions on values
  already in registers (arithmetic, compares, field load/store at
  `zun_core::field_offset(slot)`, bounds-checked element access).
- **Host thunk** in `src/thunks/` (`pub extern "C" fn zun_jit_<name>`) when it
  allocates, touches the heap tables, or has a slow path.
  1. Write the thunk; marshal handles and call the shared helper.
  2. Register the symbol next to the other `builder.symbol("zun_jit_…", …)` calls
     in `src/lib.rs` (AOT resolves the same name through the runtime library).
  3. Import it in the translator with the **ISA default call convention**
     (`self.module.isa().default_call_conv()`), because the thunk is `extern "C"`.

## Rule 3: import each callee once

Building a `Signature` and calling `declare_func_in_func` at every use site bloats
the function's import table and is called out in CLAUDE.md. Route imports through
one cached helper keyed by symbol name, storing the `FuncRef` for the function
being built (the cache is per function: a `FuncRef` is only valid inside the
function it was declared in).

```rust
fn import(&mut self, name: &'static str, params: &[Type], rets: &[Type]) -> FuncRef
```

The existing `*_fref` helpers predate this and are not cached. When you touch one,
convert it to the cached helper instead of adding another copy. Do not add a new
uncached `*_fref`.

## Rule 4: Cranelift rules for this project

- **Comparisons return `I8`.** `icmp`/`fcmp` results must be extended
  (`uextend`) before being stored or used as a wider value.
- **Number to int uses the saturating conversions** `fcvt_to_sint_sat` /
  `fcvt_to_uint_sat`, then apply ToInt32 wrap separately. The trapping forms raise
  SIGILL on NaN and out-of-range input.
- **Seal every block exactly once.** Seal a loop header only after its back edge
  is emitted.
- **Mark slow paths cold** with `set_cold_block`: IC misses, error exits, the
  allocation slow path, the bridge exceptional exit.
- **`MemFlags::trusted()`** only for stack slots and runtime structures known to
  be valid. Tag alias regions only when non-aliasing is guaranteed.
- **Managed functions use `CallConv::Tail`**; `SystemV`/`WindowsFastcall` only at
  the native boundary. This is not yet adopted (see known drift); do not introduce
  a third convention, and keep any new managed-call code in one place so the
  switch stays a single change.
- **ISA flags**: `preserve_frame_pointers`, `use_egraphs`,
  `enable_alias_analysis`. The JIT constructor and the AOT object path set flags
  separately; change both.
- Cranelift does no inlining, escape analysis or vectorization. If a lowering
  needs one of those to be fast, the optimization belongs in the IL layer.

## Rule 5: GC references

A GC collection can happen at any call. Every managed reference live across a call
must be visible to the collector:

- In emitted code: `builder.declare_value_needs_stack_map(value)` for every
  `Object`, `Array` and `String` kind value (and the environment word of a
  function value).
- In Rust entry points that receive packed arguments (`call_packed*`,
  `call_osr*`): root **every GC-managed kind**. Use one predicate for "is this
  kind a GC reference" and call it everywhere; the audit found entry points that
  rooted `Object` but not `Array`. If you add a `JitKind`, update that predicate
  and nothing else.
- A thunk that allocates must not hold a raw pointer or a borrowed heap reference
  across the allocation.

Test GC-sensitive lowering with a workload that allocates inside the callee so a
collection actually happens at the safepoint.

## Rule 6: eligibility (`src/eligible.rs`)

A method runs native only if eligibility admits every opcode and callee in it.
The allowlist must match exactly what the translator lowers: admitting an opcode
without a lowering fails at compile time of the method; lowering without admitting
is silently never used.

- Add the opcode to `opcode_supported`, or the builtin name to its allowlist, in
  the same change as the lowering.
- Prefer extending an existing predicate with a `const` opcode slice over writing
  another near-identical `*_only` matcher.
- Exception regions, `CallIndirect`, closures and dynamic ops have specific gates
  tied to the JIT-to-interp bridge. Read `docs/design/jit-interp-bridge.md` and
  `docs/design/jit-closure-carrier.md` before relaxing one; several "obvious"
  relaxations are unsound (a non-dynamic `CallIndirect` is monomorphic; a
  function-typed parameter may hold a closure).

## Verifying

```sh
cargo test -p zun_codegen --test equivalence_<area>
```

This compiles one crate and runs JIT and interpreter in-process; it links no
executable, which makes it the machine-safe loop (see `zun-build-and-test`).
`assert_equivalent(src)` in `tests/common/mod.rs` checks eligibility and result
equality. Write the source with locals so nothing is constant-folded, and cover
the edge inputs of the operation.

When a JIT result differs from Tier 0: reduce to the smallest source, confirm the
interpreter result is the correct JS behaviour, then diff the thunk against the
Tier 0 native line by line. The cause is almost always a re-implemented semantic
(Rule 1) or a missing extension of an `I8` compare (Rule 4).
