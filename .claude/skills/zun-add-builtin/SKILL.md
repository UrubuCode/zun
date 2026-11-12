---
name: zun-add-builtin
description: Recipe for adding or fixing a JS/TS builtin in Zun - a method on String/Number/Array/Map/etc., a global function, a static like `Math.x` or `Object.x`, or a `zun:core` native - across sema resolution, the `zun_natives` table, the interpreter implementation and (optionally) the JIT. Use this whenever a task mentions a missing or wrong builtin method, "unknown variable"/"has no method" diagnostics for standard JS APIs, adding to `zun:core`, or a conformance fixture failing on a standard-library call. Not for Node.js modules (those go in zun-node, see zun-node-module).
---

# Adding or fixing a builtin

A builtin crosses four layers. Knowing which layer owns which decision keeps the
change small:

| Layer | Owns | Location |
|---|---|---|
| Sema | Does the call type-check, and which native does it resolve to | `crates/zun_sema/src/check/expr/mcall_*.rs`, `call_builtins.rs`, `check/builtins/` |
| Native table | Name, parameter kinds, return kind, stable id | `crates/zun_natives/src/<group>.rs` + `MODULES` in `lib.rs` |
| Tier 0 | The behaviour | pure: the native's `imp`; heap-needing: `crates/zun_interp/src/collections/` |
| Tier 1 | Optional fast path | `crates/zun_codegen/src/thunks/`, `translate/builtins*.rs`, `eligible.rs` |

## 0. Decide where it belongs

- **Base JS/TS semantics** (ECMAScript builtins, `zun:core` primitives): here.
- **Node.js API** (`fs`, `path`, `Buffer` shaping, `process`, …): not here. The
  engine core holds only base logic; Node modules are extensions in `zun-node/`.
- **Expressible in TypeScript** over existing primitives: prefer the prelude /
  `std/` over a new native. A native is for what the language cannot express or
  what must be fast.

## 1. Start from the failing program

Write a five-line probe `.ts` in the scratchpad and run it
(`target/release/zun.exe run probe.ts`). The first diagnostic tells you the layer:

- compile error (`SEM…`, "unknown variable", "has no method", arity mismatch):
  sema does not resolve the call. Layers 1-2.
- uncaught runtime throw or wrong output: the implementation. Layer 3.
- correct interpreted, wrong or slow in a function: Layer 4.

Check what Node and Bun print for the same probe before deciding what "correct"
is. Do not implement from memory of the spec for edge cases.

## 2. Native table (`crates/zun_natives`)

Add a `NativeFn { name, params, ret, imp }` to the group it belongs to.

- **Ids are positions.** A function's id is its global index across `MODULES`, and
  compiled `.zm` files and the JIT refer to it. Append to the **end of a group's
  list only if that group is last**, otherwise add a new group at the end of
  `MODULES` (that is why `array_ext`, `timers`, `serde`, `host`, `net` exist as
  late groups). Inserting in the middle shifts every later id and silently breaks
  existing modules.
- Use the most specific parameter kinds the operation allows. `Opaque`/`Any`
  parameters push work to run time.
- A native that needs the heap sets `imp` to the shared "requires an interpreter
  heap" stub and is implemented in `zun_interp`. Reuse the existing stub helper;
  do not add another per-file `unavailable` copy.

## 3. Sema resolution

Primitive receivers (String, Number, Array, typed arrays) resolve through the sema
method-dispatch tables, not through prelude classes, so a missing method is usually
a table entry, not a class.

- Find how a sibling method of the same receiver is resolved
  (search the sibling's name under `crates/zun_sema/src/check/`) and add yours the
  same way.
- Optional or variadic arguments: the tables enforce exact arity, so default the
  omitted argument in the checker or hand-lower the call the way the sibling does.
- When the argument's static type is `any`, accept it and let the native coerce or
  throw at run time. Rejecting valid dynamic JS at compile time is the most common
  cause of conformance failures. Do not, in exchange, loosen what a *typed*
  argument accepts.
- Use the existing helpers for the argument-check and "missing intrinsic"
  patterns if they exist; if you are about to paste the
  `check_expr` + `coerce_numeric_literal` + `is_assignable` + `self.error(…)` block
  again, extract or reuse a `Checker` helper instead.

## 4. Tier 0 implementation

Reuse the canonical coercions; each of these has been re-implemented more than
once and the copies disagree:

| Need | Use |
|---|---|
| ToNumber | `Interp::dyn_to_number` (`zun_interp/src/string_dyn.rs`) |
| ToString | `Interp::dyn_to_string` (same file) |
| Unbox an `Any` argument | `unbox_arg` (`zun_interp/src/collections/mod.rs`) |
| `Value` to heap object | `collections::object` |
| ToPropertyKey | the `prop_key` path in `property_ops.rs` |
| String search / pad / repeat / substring | `zun_core::string_ops` |
| `$`-substitution in replace | `zun_core::expand_substitution` |

If the operation is pure (no heap, no interpreter state), implement it in
`zun_core` and call it from the native. That makes step 5 a three-line thunk and
guarantees the tiers agree.

Errors thrown must be catchable JS errors of the right class (`TypeError`,
`RangeError`), not Rust panics.

## 5. Tier 1 (only if it is hot and typed)

Follow `zun-jit-lowering`: thunk that calls the same `zun_core` helper, symbol
registration, cached import, eligibility allowlist entry, and an
`assert_equivalent` test. If you skip this step, do nothing in `zun_codegen`: the
method stays ineligible and runs in Tier 0, which is correct.

## 6. Tests

- A unit or integration test in the crate that owns the behaviour
  (`crates/zun_vm/tests/<area>.rs` is where end-to-end builtin surface tests live).
- Edge inputs: empty, negative and fractional indices, NaN, `undefined` argument,
  wrong receiver type.
- If a conformance fixture under `tests/` motivated the change, re-run that
  fixture (see `zun-conformance`).

Run per crate only (`cargo test -p <crate> --test <file>`); see
`zun-build-and-test`.
