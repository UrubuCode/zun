# Builtin performance audit

Read-only performance audit of Zun's default nucleus builtin modules. One file per
module, plus [`_shared.md`](./_shared.md) for cross-cutting issues (`S#` tags).

**Method:** static code reading only — no profiling, no builds. Every gain figure
is an estimate, not a measurement. Findings tagged **requires approval** would
change a frozen layout, the `.zm` format, the native-table ABI, or GC behaviour
(per CLAUDE.md invariants).

**Date:** 2026-11-07. Audited against `main` (working tree had local edits to
`zun_codegen/{eligible,lib,translate/mod}.rs`).

## Module index

| Module | File | Headline |
|--------|------|----------|
| String | [String.md](./String.md) | UTF-8 `Rc<str>`, no len16 cache → O(n) length/index (F1) |
| Array | [Array.md](./Array.md) | `sort` is O(n²) insertion sort (F1); no array method JIT-lowered (F3) |
| Map / Set | [MapSet.md](./MapSet.md) | weak hash mixing → int/float keys collide, lookup O(n) (F1) |
| Number / Boolean | [NumberBoolean.md](./NumberBoolean.md) | formatters Tier-0-only + `format!("{:.1100}")` per call (F1/F2) |
| Math | [Math.md](./Math.md) | `Math.round` rounds the wrong way (correctness, F2); `Math.random` missing (F3) |
| JSON | [JSON.md](./JSON.md) | `JSON.parse` is a Zun prelude parser, char-by-char (F1) |
| RegExp | [RegExp.md](./RegExp.md) | no compiled-pattern cache; recompiled per call (F1) |
| Promise | [Promise.md](./Promise.md) | `reconcile_settlements` scans all promises ever bridged (F1) |
| Date | [Date.md](./Date.md) | host zone + ICU formatter re-resolved every call (F1/F3) |
| TypedArray / ArrayBuffer / DataView | [TypedArray.md](./TypedArray.md) | two hash lookups per element access (F1); 6 of 11 kinds Tier-0 (F2) |
| Buffer | [Buffer.md](./Buffer.md) | `view_bytes` copies byte-by-byte (F1); read/write by name-string match (F3) |
| Object / Proxy / Reflect | [Object.md](./Object.md) | DynamicObject = many string-keyed side structures (F1) |
| Misc (Symbol/BigInt/Error/Weak/Function/serde) | [Misc.md](./Misc.md) | `new Error` captures stack every time (F1); weak collections linear-scan (F2) |

## Top 15 optimizations (by gain / risk)

Ordered best gain-for-risk first. "Approval" = touches frozen layout / `.zm` / ABI / GC.

| # | What | Where | Gain | Risk | Approval |
|---|------|-------|------|------|----------|
| 1 | Correctness: make `Math.round` round half toward +∞ (share one `js_round` with the opcode path) | Math F2 — `natives/math.rs:180`, `thunks/misc.rs:435` | correctness | low | no |
| 2 | Dispatch native builtins by id, not by nested string match | [S1](./_shared.md#s1--native-dispatch-is-name-string-based-two-levels-deep) | large, every call | low | no |
| 3 | Fix hash mixing so int/float Map/Set keys don't collide (64-bit finalizer) | MapSet F1 — `zun_gc/src/hashing.rs:7,19` | large (O(n)→O(1) lookup) | low | no |
| 4 | Promise: scan `pid_by_obj` once/turn and free settled/dead entries | Promise F1 — `interp/src/state.rs:463` | large (removes O(n·all-promises)) | low-med | no |
| 5 | Array `sort`/`toSorted`: merge sort + read-once/write-once, stringify keys once | Array F1 — `collections/array_iterate.rs:197` | large (O(n²)→O(n log n)) | low-med | no |
| 6 | Cache ICU4X formatters by (locale, kind) | [S9](./_shared.md#s9--icu4x-formatters-rebuilt-on-every-call) — Date F3, Number F6 | large on locale loops | low | no |
| 7 | Bulk byte/element APIs (memcpy/fill/slice) for Buffer & TypedArray & ArrayBuffer | Buffer F1, TypedArray F3, [S3](./_shared.md#s3) | large (per-byte→memcpy) | low | no |
| 8 | Promise timers: `BinaryHeap` + slab/tombstones instead of linear `Vec` | Promise F4 — timers | med-large (O(T²)→O(T log T)) | low-med | no |
| 9 | ASCII fast paths for String length/index/slice/indexOf | String F1/F3, [S5](./_shared.md#s5) | large on ASCII text | low | no |
| 10 | Generate JIT eligibility + table rows + thunks from one master table; add census test | [S2](./_shared.md#s2) | removes silent Tier-0 fallbacks | low-med | no |
| 11 | Cache decoded body/target in `invoke_callback`; reuse arg buffer | [S6](./_shared.md#s6) — Array F5, TypedArray F4 | med-large on callbacks | low-med | no |
| 12 | Swap SipHash → fast integer hasher for address-keyed heap side tables | [S3](./_shared.md#s3) | med, every access | low | no |
| 13 | Rust-native `JSON.parse` replacing the prelude char-by-char parser | JSON F1 — `builtin/json.zun:353` | large on parse | med | yes (adds native) |
| 14 | BigInt: borrow operands instead of deep-cloning limb Vecs; sub-quadratic `toString` | Misc F5 — `zun_core` bigint | med on BigInt code | low | no |
| 15 | Error: lazy `.stack` rendering + sorted sequence-point index (don't walk on every `new`) | Misc F1 — `builtin/error.zun:26`, `runtime_descriptor.rs:11` | med (every `new Error`) | low-med | no |

### Deeper, approval-gated structural wins (high ceiling, high risk)
- **Inline element/metadata pointer in object headers** so hot paths + the JIT skip
  the address-keyed side-table lookup entirely ([S3](./_shared.md#s3)). Touches
  frozen layout + GC.
- **Cache `(len16, is_ascii)` per interned string** for O(1) UTF-16 length/index
  ([S5](./_shared.md#s5)). Touches String layout + GC.
- **Lower dynamic-object get/set + per-site inline caches** (shape ids); today no
  dynamic op is JIT-eligible so any `any`-object code stays Tier 0 (Object S-notes).
- **`AnyTag::Function`** to drop `functionBox` wrappers ([S8](./_shared.md#s8)).
  Touches the `Any` ABI + GC root precision.
