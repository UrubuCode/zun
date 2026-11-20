# Project audit — 2026-11-20

Five independent read-only analyses were run against `main` at `350b7cd7`
(conformance: 107 of 169 fixtures). This document records what they found and
the work plan derived from them. Each item is marked **confirmed** when it was
reproduced with a probe against Node, or **read** when it was inferred from
reading the code. The commits that resolve an item name its id (R1, C3, ...).

## 1. Defects in the 94 -> 107 conformance series

An adversarial review of `d0fdf6a3..350b7cd7`. The 169-fixture suite catches
none of these.

| # | Severity | Defect | Status |
|---|---|---|---|
| R1 | critical | Calling the `setTimeout` *value* (`const st = setTimeout; st(cb, 1)`) panics the engine at `value.rs:228` | confirmed |
| R2 | high | `setTimeout.name` is `__zunSetTimeout`, and its `length` differs from Node | confirmed |
| R3 | high | `__thisStringValue` identifies a String wrapper by `Object.prototype.toString`, so a plain object with `Symbol.toStringTag: "String"` is accepted | confirmed |
| R4 | high | `WEBIDL_TAGGED_CLASSES` matches by class *name*: a user `class URLSearchParams {}` gets the platform brand | confirmed |
| R5 | medium | Redeclared-field slot unification: a base constructor reading the field now sees the derived initializer's value (no base-then-derived phasing) | confirmed |
| R6 | medium | `Array.prototype.slice.call(a, Symbol())` / `1n` do not throw; computed `(Array.prototype as any)[m].call(a, fn, thisArg)` loses `thisArg` | confirmed |
| R7 | medium | `a[Symbol.iterator] === a.values` is false; an `Array` subclass overriding `[Symbol.iterator]` is ignored by spread | confirmed |
| R8 | medium | `array_values_kind()` walks the prototype-method table and allocates on every `arr[Symbol.iterator]` read | read |
| R9 | low | `encodeURIComponent("\uD800")` does not throw `URIError` | confirmed |
| R10 | low | A block-level function declaration is installed on `globalThis` | confirmed |

Checked and found correct: SHA-256 (boundary lengths, offset views), UTF-16
`TextDecoder` streaming, the `slice` coercion order, named function
expressions with `$`/unicode names.

## 2. Conformance: where the remaining failures come from

The failures are spread thin: most non-passing fixtures differ on four or more
independent causes, and no single cluster flips more than one or two fixtures.
Flip counts are estimates from label-paired diff lines.

| Step | Cluster | Effort | Est. cumulative |
|---|---|---|---|
| C1 | `structuredClone`: class instance clones as a plain object; non-index array properties | S | 108 |
| C2 | Temporal dead zone for `let`/`const`/`class` | M | 109 |
| C3 | BigInt-kind typed-array element coercion | S | 110 |
| C4 | `Array.prototype.sort`: holes/`undefined` last, `flatMap` holes | S/M | 111 |
| C5 | WeakMap/WeakSet/WeakRef brands and `instanceof` | S | 112 |
| C6 | Map/Set iterator objects: live, own prototype and tag, `m[Symbol.iterator] === m.entries` | M | 113 |
| C7 | `Boolean`/`Number`/`String` identity as values; BigInt/Symbol wrapper objects | M | 115 |
| C8 | NamedEvaluation; generator/async function metadata and prototype chain | M | 117 |
| C9 | Detached-method `this`; an own property shadows a declared method | M | 118 |
| C10 | `Error`: own `message` for `new Error("")`, borrowed `toString` on a primitive, subclass brand | S | 119 |

Out of reach for now (many independent causes each): the Proxy group, the full
typed-array/ArrayBuffer set, `symbol-wellknown-protocol-hooks`,
`regexp-object-protocols`, `unicode-sets-v-flag`, URL host parsing.

## 3. Representation designs that unblock the above

| # | Design | Effort | Unblocks |
|---|---|---|---|
| D1 | Native error allocators (`native_error.rs::make_error_value`, `builtins.rs::new_error_object`) fill a fixed positional array; make them layout-driven so `Error` can gain fields | S | C10 |
| D2 | Key `methods_by_class` by `(name, is_static)`; include staticness in the export name | S | `Response.json` |
| D3 | One identity for `String`/`Number`/`Boolean`: the class descriptor, callable through a per-type call method resolved in `callback_handle` | S/M | C7 |
| D4 | A `Brand` byte on heap objects read by `typeof`, `objectToStringTag`, `constructor_record`, `structuredClone` | M | C5, C7, R3, R4 |
| D5 | Typed `MethodFlags` + `display_name` + `length` on `MethodDef`, replacing the closure-name string markers (`@this`, `@rest`, `@generator`, ...) parsed today at 31 sites in 13 files | M/L | C8, P5 |
| D6 | Field initializers run per class level at the `super()` return point | M | R5 |
| D7 | TDZ: static proof where possible, a sentinel and a `checkinit` opcode only for unproven captured bindings | M | C2 |
| D8 | Prelude compiled once as a separate module and linked, instead of concatenated as source after the user program | L | name collisions, init order |

## 4. Performance

Measured with the release binary. The dominant cost is which functions fall
back to Tier 0, not interpreter micro-overhead: JIT eligibility is
all-or-nothing per method (`crates/zun_codegen/src/eligible.rs`).

| # | Finding | Evidence | Impact |
|---|---|---|---|
| P1 | A closure anywhere in a function disqualifies it | 200k calls: 1 ms direct vs 242 ms through a closure | high |
| P2 | `NewArray` disqualifies outside five scalar patterns | 271 ms for a loop allocating `[i, i+1]` | high |
| P3 | Module top-level code is never compiled | ~300x, per `bench/README.md` | high |
| P4 | Builtins dispatched by string name on every call (`builtins.rs`, `collections/mod.rs`) | read | medium |
| P5 | Method facts read by scanning the name string on every call (`lib.rs:492`, `runtime_iter.rs`) | read | low/medium |
| P6 | `$classBag#{token}` lookups `format!` a key on every probe | read | low/medium |
| P7 | Rest-argument packing allocates twice per call | +150 ms per 200k calls | medium |

Startup is fine (57 ms for a trivial script).

## 5. Architecture

| # | Finding | Status |
|---|---|---|
| A1 | `zun_sema` (compile time) depends on `zun_core` (runtime) and uses `zun_core::WellKnownSymbol`, against the crate rule in `CLAUDE.md` | read |
| A2 | Closure names are an untyped protocol between `zun_sema` and `zun_interp` (same as D5) | read |
| A3 | The syntax adapter decides scoping by substring search (`contains_word`, `body_text.contains`), blind to comments, strings and nested blocks; about 20 thread-locals carry conversion state | read |
| A4 | Desugaring has no consistent home (named function expressions, block functions and `yield*` are rewritten in the adapter) | read |
| A5 | Duplicated rules: receiver detection, three iteration drivers, builtin brand detection, symbol-method lookup | read |
| A6 | God functions: `col_array_construct` (~940 lines), `dynamic_call.rs` (171 match arms), `define_own_property` | read |
| A7 | `zun_vm/tests/wintercg.rs::headers_set_append_delete` is stale: it expects `"a"` where the Fetch spec, and the implementation, give `"a, b"` | read |
| A8 | Interp/JIT equivalence is only asserted for JIT-eligible programs | read |

## 6. Order of work

1. R1-R4, R8, A7: defects and a red test in work already landed.
2. D1, D2: small unblockers; then C10 and `Response.json`.
3. C1, C3, C4: small conformance clusters.
4. D5 (with A2, P5), then C8.
5. D4, then C5, C7 (with D3), and the principled versions of R3/R4.
6. D6 (R5), D7 (C2), C6, C9.
7. P4, P6, P7; then P1-P3.
8. A1, A3-A6, A8, D8.
