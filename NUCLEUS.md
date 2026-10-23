# NUCLEUS.md — Bucket A completeness audit

This file **defines what "nucleus 100%" means** for Zun (the current `GOAL.md` goal:
"complete the nucleus — bucket A → 100%"). The nucleus is the built-in type/function
surface listed in `PLAN.md` §3.2 ("A — nucleus"). It is a living checklist: each item
below records its real state in the code — the method/property surface and semantics,
not merely "a class exists".

**Audit date:** 2026-10-23 · commit `2ae36bd` (clean `main`).
**Scope:** read-only audit of `std/*.zun`, `crates/zun_natives/src/*.rs`,
`crates/zun_sema/src/{check.rs,native_types.rs,types.rs}`, `crates/zun_core/src/*.rs`,
`crates/zun_il/src/opcode.rs`.

**JSON is NOT nucleus.** PLAN §3.2 lists JSON under A, but GOAL.md overrides this: JSON
ser/de lives in `@urubucode/common` (bucket C, ZIL), so it is **excluded** from this
audit.

Legend: **done** = standard-common surface present; **partial** = exists but material
gaps; **missing** = not implemented. "verify" marks claims not fully confirmed from the
read.

---

## Summary table

| # | Item | Status | One-line gap |
|---|------|--------|--------------|
| 1 | String | partial | Core slicing/case/search present; missing `split/match/matchAll/replaceAll/slice/at/concat/codePointAt/normalize/localeCompare`, statics `fromCharCode/fromCodePoint/raw`, iterator |
| 2 | Number | partial | Instance helpers present; **no statics** (`Number.isNaN/isInteger/isFinite/parseFloat/parseInt`, `MAX_SAFE_INTEGER` …); no `toPrecision/toExponential/toLocaleString`; no radix on `toString` |
| 3 | Boolean | missing | No wrapper class/type; only the primitive `Ty::Boolean`. No `Boolean()`, no `.toString()` object surface |
| 4 | BigInt | partial | Only `add/sub/mul/compare/toString`; missing `div/rem/pow/neg/bitwise/shift`, `BigInt()` coercion, `asIntN/asUintN`, operator integration |
| 5 | Symbol | partial | Only `Symbol.iterator` wired as a dispatch point; `SymbolRegistry` in core unused. No `Symbol()`, `Symbol.for/keyFor`, other well-known symbols, user symbol-keyed members |
| 6 | Object | partial | Enumeration/reflection surface landed (`keys/values/entries/assign/freeze/isFrozen/create/getPrototypeOf/setPrototypeOf/fromEntries/is` + instance `hasOwnProperty/isPrototypeOf`); descriptor-based statics deferred (no property-descriptor model) |
| 7 | Array | partial | Rich custom surface; missing standard `forEach/find/some/every/findLast/flatMap/join/splice/shift/unshift/keys/values/entries`, statics `Array.from/of/isArray`, callback `sort`, iterator-returning methods |
| 8 | Function | missing | **No `call/apply/bind`**; no `.name/.length` object surface, no `Function` constructor |
| 9 | Promise | partial | Instance `state/result/isReady/resolve/reject`; no `then/catch/finally`, no statics `Promise.resolve/reject/all/allSettled/race/any`. `await` works via opcode |
| 10 | Proxy / Reflect | partial | `proxyNew` + `reflect_get/reflect_set` exist; no full trap set, no `Reflect` global object surface |
| 11 | Map | partial | `set/get/has/size/delete/clear/values/keys/entries` + iterator; missing `forEach`, `size` as property (is a method), constructor-from-iterable |
| 12 | Set | partial | `add/has/size/delete/clear/values/keys/entries` + iterator; missing `forEach`, constructor-from-iterable |
| 13 | WeakMap | partial | `set/get/has/delete`; semantics OK (ephemeron floor). No iteration (correct) |
| 14 | WeakSet | missing | Not implemented (GC base ready via WeakMap) |
| 15 | WeakRef | missing | Not implemented |
| 16 | FinalizationRegistry | partial | `register/unregister` present over GC finalization queue; `unregister` keyed by numeric id, not token object (verify vs spec) |
| 17 | ArrayBuffer | partial | `new ArrayBuffer(len)` intrinsic; no `byteLength/slice/isView/resize` |
| 18 | DataView | partial | Only `getUint32/setUint32/getUint16/setUint16/getFloat64/setFloat64`; missing Int8/16/32, Uint8, Float32, BigInt64/Uint64, `byteLength/byteOffset/buffer` |
| 19 | TypedArrays | partial | Only `Uint8Array/Uint16Array/Uint32Array/Float64Array`, each only `get/set`. Missing Int8/16/32, Uint8Clamped, Float32, BigInt64/Uint64 Array, and the whole method surface (`length/subarray/set/slice/fill/map/forEach/iterator`) |
| 20 | Date | partial | UTC getters + millis + ISO + parse; missing local-time getters, **all setters** (`setFullYear/…`), component `getMonth/getDate/getDay/getHours…` JS-named (only `getUTC*`), `Date.now/parse/UTC` statics, `getTimezoneOffset`, `toLocale*`, `toDateString` |
| 21 | Buffer | partial | Native `zun:buffer` surface exists; verify Node `Buffer` API coverage (`from/alloc/write/read*/toString/slice/concat`) |
| 22 | Error (+ subclasses) | partial | `Error` + 6 subclasses (`Eval/Range/Reference/Syntax/Type/URI`); `name/message`. Missing `AggregateError`, `cause`, `.stack` capture wiring (sequence-point debug info landed per GOAL, verify stack population), `Error.captureStackTrace` |
| 23 | globalThis / global | done | `globalThis`/`global` → `RealmGlobal` opcode (`Ty::DynamicObject`), realm singleton per §3.1 |
| 24 | Typed numeric lattice | done | `i8..i64/u8..u64/f32/f64` in `IntKind`/`Ty`; int/float TS types + coercions |
| 25 | Decorators | missing | Planned in A; not implemented (no parse/sema/lowering) |
| 26 | ZIL math opcodes | done | 15 ops at `0x96-0xA4`: float `sqrt/abs/neg/floor/ceil/trunc/min/max/round/sign`, int `min/max/clz/ctz/popcnt`. Transcendentals stay on libm per PLAN §3 |

**Counts:** done 4 · partial 15 · missing 4 (of 26 nucleus items, JSON excluded).

### Declarative realm-global registration (PLAN §3.1)

Base types are seeded onto `globalThis` from the declarative registry
(`builtin/realm.zun` → `zun_sema::realm`); the registry is the single source of
truth for each name's annotation type, descriptor value, and `new` lowering (the
frozen runtime representation is unchanged — the registry only rebinds the name).

- **Registered declaratively:** `Map`, `Set`, `WeakMap`, `FinalizationRegistry`,
  `Promise`, `RegExp`, `BigInt`, `Array`, `ArrayBuffer`, `DataView`,
  `Uint8Array`, `Uint16Array`, `Uint32Array`, `Float64Array`, `String`,
  `Number`, `Boolean`, `Object`, `Symbol`. (`Map` keeps token 0.)
- **Still on the hardcoded fallback:** `Date` and `Error` (+ subclasses
  `EvalError/RangeError/ReferenceError/SyntaxError/TypeError/URIError`) — their
  annotation resolves to `Ty::Instance(<program-assigned class index>)`, which a
  static `base_type_facets` entry cannot name; and `Proxy` (not in bucket A).
  The hardcoded `new`/annotation matches also remain for the lowercase aliases
  (`map`, `array`, …) and `Proxy`, so they are not yet removable.

---

## 1. String — partial
Primitive-backed class, statically bound (`std/string.zun`, dispatch in
`check.rs` `managed_method` + `check.rs:2831+`; native impls `zun_natives/src/string.rs`).

- **Present (extern, native):** `length() contains(needle) count(needle) compare(other)
  toInt() toFloat() toBoolean() includes(needle) indexOf(needle) lastIndexOf(needle)
  replace(needle,repl) substring(start,end) padStart(len,fill) padEnd(len,fill)
  toLowerCase() toUpperCase() trimStart() trimEnd() trim() charCodeAt(index)
  startsWith(prefix) endsWith(suffix) repeat(count) reverse()`. Plus a large
  non-standard validator surface (`isAlpha/isDigit/isHex/isSnakeCase/…` — Zun extras, not
  ECMAScript).
- **Missing (standard):** `charAt at(i) codePointAt slice(start,end) split(sep)
  concat(...) match(re) matchAll(re) search(re) replaceAll normalize localeCompare
  valueOf toString(radix n/a) [Symbol.iterator] padStart/padEnd` take only one fill.
  Statics: `String.fromCharCode String.fromCodePoint String.raw`.
- **Notes:** `replace` takes plain strings only (no regex/function replacer). No template
  cooking helper. `substring` uses `u64` start/end, no negative-index `slice`.

## 2. Number — partial
`std/number.zun`; native `number.rs`.

- **Present (instance):** `isNaN isFinite isNegativeZero isSafeInteger isInteger toFixed
  toString` + Zun math helpers (`abs clamp min max sign lerp …`). Free fns
  `numberParse/numberToFixed`.
- **Missing:** **all statics** — `Number.isNaN/isInteger/isFinite/isSafeInteger
  parseFloat parseInt`, constants `MAX_SAFE_INTEGER/MIN_SAFE_INTEGER/MAX_VALUE/MIN_VALUE/
  EPSILON/POSITIVE_INFINITY/NEGATIVE_INFINITY/NaN`. Instance `toPrecision toExponential
  toLocaleString valueOf`; `toString(radix)`.
- **Notes:** the predicates exist only as **instance** methods (`x.isNaN()`), not the
  idiomatic `Number.isNaN(x)` statics.

## 3. Boolean — missing
No `std/boolean.zun`, no wrapper type; only primitive `Ty::Boolean`. No `Boolean()`
coercion function, no object `.toString()/.valueOf()`. GOAL ranks this lowest.

## 4. BigInt — partial
`std/bigint.zun` (extern) + native (`collections.rs`, `zun_core/bigint.rs`).

- **Present:** `add sub mul compare toString`.
- **Missing:** `div rem pow neg` and bitwise/shift (`and or xor not shl shr`),
  `BigInt(value)` coercion, `BigInt.asIntN/asUintN`, `valueOf`, operator-level
  integration (`+ - * / %` on bigint operands), literal `1n` → confirm lowering (verify).

## 5. Symbol — partial  *(HIGH priority)*
`SymbolRegistry` (`zun_core/src/symbols.rs`) exists with `create/for_key/iterator/
description` but is **not surfaced to user code**. Only `Symbol.iterator` is recognized,
and only as a hardcoded dispatch point in `check.rs:3080-3106` (for-of / explicit
`x[Symbol.iterator]()`), mapping Array/Map/Set/Error to their `*IteratorNew` intrinsics.

- **Present:** `Symbol.iterator` as a VM dispatch point (iteration protocol only).
- **Missing:** `Symbol(desc)` unique primitives as first-class values; global registry
  `Symbol.for(key)` / `Symbol.keyFor(sym)`; the other well-known symbols as dispatch
  points — `asyncIterator hasInstance toPrimitive toStringTag isConcatSpreadable match
  replace search split species unscopables`; user-defined **symbol-keyed members**
  (`[Symbol.iterator]() {}`, computed symbol keys). GOAL notes the symbol-keyed slot
  already exists (measured 16.5x) — the *surface* is unfinished.
- **Notes:** foundational — iteration and many built-in behaviors ride on this.

## 6. Object — partial  *(HIGH priority — core surface landed)*
The `Object` global is recognized in sema (`check.rs` `check_object_static` + the
instance reflectors in the `MethodCall` arm) and lowered to `zun:collections`
`object*` managed builtins implemented in the interpreter floor
(`zun_interp/src/collections.rs`). The statics operate on **both** fixed-layout typed
objects and `DynamicObject`s.

- **Present (statics):** `keys values entries assign freeze isFrozen create
  getPrototypeOf setPrototypeOf fromEntries is`.
- **Present (instance):** `hasOwnProperty isPrototypeOf`.
- **Key enumeration:** for a typed object, own keys = its declared instance fields in
  definition order, walking base classes first and excluding `static`/private (`#`)
  fields; for a `DynamicObject`, its dynamic **string** keys in insertion order
  (`DynamicObject` now keeps string properties in an insertion-ordered vector + index,
  replacing the previous sorted `BTreeMap`).
- **Freeze semantics:** `freeze` sets a per-dynamic-object frozen flag; the dynamic
  store path (`DynamicObject::set`/`remove`, hence `stdyn`/`dynamic_set`) then rejects
  adds, overwrites, and deletes (non-throwing). Typed objects already have an immutable
  fixed layout, so `freeze` is a no-op on them and `isFrozen` reports `true`.
- **Prototype model:** `DynamicObject` carries an optional `[[Prototype]]` link;
  `create`/`getPrototypeOf`/`setPrototypeOf`/`isPrototypeOf` operate over it. Typed
  objects expose no reflective prototype (their `getPrototypeOf` is `null`).
- **`Object.is`:** ECMAScript `SameValue` (NaN equals NaN; +0 distinct from −0;
  numbers compared by IEEE-754 bits after the NaN check; strings by content; objects by
  identity).
- **Deferred (documented follow-up — need a property-descriptor model that Zun does not
  have yet):** `defineProperty defineProperties getOwnPropertyDescriptor
  getOwnPropertyDescriptors seal isSealed preventExtensions isExtensible
  getOwnPropertyNames getOwnPropertySymbols`, and instance `propertyIsEnumerable`. These
  require configurable/enumerable/writable attribute slots and a non-extensible flag that
  is distinct from `freeze`. Implementing them soundly means first adding a descriptor
  layer to `DynamicObject` (and deciding how it interacts with the frozen typed-object
  floor) — a design decision left for the human. Not faked.
  - **Writable attribute now exists (B1):** `DynamicObject` carries a general per-entry
    **writable** bit (`non_writable` key set) with `define(name, value, writable)` /
    `is_writable(name)` (core) and `Heap::dynamic_define` / `dynamic_is_writable` (gc).
    The ordinary `set` path (`stdyn`/`dynamic_set`, string-literal and computed) rejects
    reassigning a non-writable key, silently, exactly as `freeze` rejects — so these are
    the freeze-consistent, non-throwing semantics. Introduced for `global.set(name,
    value, readonly)` (the `global` namespace in `zun:core`, PLAN §3.1), but **not**
    hard-coded to the realm global: it is a general attribute slot that the descriptor
    statics above can build on (writable first; add `enumerable`/`configurable`
    alongside it the same way).
- **Notes:** these builtins allocate/throw, so per the centralized JIT-eligibility rule
  (`zun_codegen/src/eligible.rs`) they are **not** in the Tier-1 allowlist and run on the
  always-correct Tier-0 interpreter; there is no separate JIT path to keep in sync.

## 7. Array — partial
`std/array.zun` (large custom surface) + special ZIL lowering for `push/pop`
(`ArrayPush/ArrayPop` opcodes), plus `arrayIterator*` intrinsics.

- **Present (standard):** `push pop map(cb) filter(cb) reduce(cb,init) slice(start,end)
  concat(other) fill(value) reverse copyWithin(t,s,e) flat indexOf lastIndexOf includes
  at(i) length` + `[Symbol.iterator]`. Plus many Zun extras
  (`chunk windowed rotateLeft union intersection isPalindrome sum min max sort …`).
- **Missing (standard):** `forEach find findIndex(cb) findLast findLastIndex some every
  flatMap join(sep) splice shift unshift keys() values() entries() toSorted toReversed
  with group`. `sort` takes no comparator (`sort()` only). `map/filter` callbacks do not
  receive `(value,index,array)` — verify arity. Statics `Array.from(iterable[,mapFn])
  Array.of(...) Array.isArray(x)`.
- **Notes:** `findIndex(value)` takes a *value*, not a predicate (non-standard);
  `indexOf` returns `int`. `fill` takes no range args.

## 8. Function — missing  *(HIGH priority)*
No `call/apply/bind`. PLAN §3.1 explicitly calls these "cheap reflective invokers —
implement directly". `FunctionHandle`/`ModuleBoundFunction`
(`zun_core/function_handle.rs`, `module_bound_function.rs`) and `CallIndirect` opcodes
exist, so first-class function values work, but the `.call(thisArg,...args)/.apply(
thisArg,argsArray)/.bind(thisArg,...args)` surface and `.name/.length` object properties
are absent.

## 9. Promise — partial
`std/` (none) — dispatched in `check.rs`; native `promise.rs`; `await` opcode `0x8D`.

- **Present:** `state() result() isReady() resolve(v) reject(msg)` + `await` integration
  (scheduler landed per status).
- **Missing:** `.then(onF,onR) .catch(onR) .finally(cb)`; statics `Promise.resolve/reject/
  all/allSettled/race/any/withResolvers`. `reject` takes `string`, not arbitrary value.
- **Notes:** this is a low-level promise cell, not the ECMAScript thenable surface.

## 10. Proxy / Reflect — partial
- **Present:** `new Proxy(target,handler)` → `proxyNew` (`check.rs:2590`); dynamic
  `reflect_get`/`reflect_set` recognized (`check.rs:1569-1601`).
- **Missing:** full trap set (`get set has deleteProperty apply construct ownKeys
  getOwnPropertyDescriptor defineProperty getPrototypeOf setPrototypeOf …`); a real
  `Reflect` global with `Reflect.get/set/has/deleteProperty/ownKeys/apply/construct/
  defineProperty/getPrototypeOf/…`.
- **Notes:** verify which traps `proxyNew` actually honors at runtime.

## 11. Map — partial
`std/map.zun` → `zun:collections` (`collections.rs`, `ordered_map.rs`). Ordered hash
table. Iterator intrinsics present (`mapIterator*`).
- **Present:** `set get has size() delete clear values keys entries` + `[Symbol.iterator]`.
- **Missing:** `forEach(cb)`; `size` is a method, spec is a getter property; construction
  from an iterable (`new Map([[k,v],…])`).

## 12. Set — partial
`std/set.zun` → `zun:collections`.
- **Present:** `add has size() delete clear values keys(→values) entries` + iterator.
- **Missing:** `forEach(cb)`; `size` as property; construct-from-iterable; ES2024 set
  ops (`union/intersection/difference/isSubsetOf/…`) — note Array has these as extras but
  Set does not.

## 13. WeakMap — partial / effectively done
`std/weak_map.zun`; ephemeron-backed GC floor.
- **Present:** `set get has delete`. Correctly non-iterable.
- **Missing:** nothing material for the spec surface (WeakMap has no iteration/size/clear).

## 14. WeakSet — missing  *(mid priority)*
Not implemented. GC base ready (WeakMap ephemeron floor). Needs `add/has/delete`.

## 15. WeakRef — missing  *(mid priority)*
Not implemented. Needs `new WeakRef(obj)` + `deref()`, over the GC weak-reference base.

## 16. FinalizationRegistry — partial
`std/finalization_registry.zun` → GC finalization queue.
- **Present:** `register(target,cb) unregister(id)`.
- **Notes/verify:** `register` returns a `u64` id and `unregister` takes that id;
  ECMAScript uses an **unregister token object** and `register(target,heldValue
  [,token])`. Signature diverges from spec — flag as semantic gap.

## 17. ArrayBuffer — partial
- **Present:** `new ArrayBuffer(length)` intrinsic (`check.rs:2501`), backing
  `byte_buffer.rs`.
- **Missing:** `.byteLength` (property), `.slice(begin,end)`, `ArrayBuffer.isView(x)`,
  resizable buffers (`.resize/.maxByteLength`), `SharedArrayBuffer` (deferred, OK).

## 18. DataView — partial
`BufferViewKind::DataView`; dispatch in `check.rs:2117+`; native `views.rs`.
- **Present:** `getUint32/setUint32 getUint16/setUint16 getFloat64/setFloat64` (each with
  little-endian `boolean` arg).
- **Missing:** `getInt8/setInt8 getUint8/setUint8 getInt16/setInt16 getInt32/setInt32
  getFloat32/setFloat32 getBigInt64/getBigUint64/set…`; properties `.byteLength
  .byteOffset .buffer`.

## 19. TypedArrays — partial
`native_types.rs` exposes only `U8Array/U16Array/U32Array/F64Array`; constructed via
`u8ArrayNew/u16ArrayNew/u32ArrayNew/f64ArrayNew` (`check.rs:2495`). Each supports only
`get(i)/set(i,v)`.
- **Present kinds:** `Uint8Array Uint16Array Uint32Array Float64Array`.
- **Missing kinds:** `Int8Array Int16Array Int32Array Uint8ClampedArray Float32Array
  BigInt64Array BigUint64Array`.
- **Missing surface (every kind):** `.length .byteLength .byteOffset .buffer`, `subarray
  slice set(array,offset) fill copyWithin` and the iteration/functional surface
  (`map forEach reduce find indexOf includes join [Symbol.iterator]`), statics
  `TypedArray.from/of`, construction from buffer/length/array.

## 20. Date — partial
`std/date.zun` over `zun:date`/`zun:time` intrinsics (`date.rs`, `time/`).
- **Present:** millis storage, `getTime/setTime valueOf isValid isBefore/isAfter/isEqual`,
  UTC getters `getUTCFullYear/Month/Date/Hours/Minutes/Seconds/Milliseconds/Day`,
  `toISOString toJSON toString`, free fns `dateNow/dateParse/dateFromMillis`.
- **Missing:** **all setters** (`setFullYear setMonth setDate setHours setMinutes
  setSeconds setMilliseconds setUTC*`); **local-time** getters (`getFullYear getMonth
  getDate getDay getHours getMinutes getSeconds getMilliseconds`); `getTimezoneOffset`;
  statics `Date.now() Date.parse() Date.UTC()`; `toDateString toTimeString toLocaleString
  toLocaleDateString toLocaleTimeString toUTCString`.
- **Notes:** JS-named instance getters exist only in `getUTC*` form; no local timezone
  support (UTC-only calendar).

## 21. Buffer — partial / verify
Native surface in `zun_natives/src/buffer.rs` (also hosts array/map/set iterator
intrinsics). Node `Buffer` API coverage not confirmed from the TS side.
- **Verify:** `Buffer.from/alloc/allocUnsafe/concat/isBuffer/byteLength`, instance
  `write readUInt*/readInt*/readFloat*/readDouble* toString(encoding) slice fill copy
  equals indexOf`. Likely largely **missing** as a user-facing `Buffer` class — flag for
  deeper read.

## 22. Error (+ subclasses) — partial
`std/error.zun`: `Error{name,message}` + `EvalError RangeError ReferenceError
SyntaxError TypeError URIError` (extends Error). `error_value.rs` in core.
- **Missing:** `AggregateError`; the `cause` option (`new Error(msg,{cause})`);
  `.stack` population (sequence-point PC→source debug info landed per GOAL status —
  **verify** the stack string is actually captured and formatted); `Error.captureStackTrace`
  / `Error.stackTraceLimit`; `toString()` → `"Name: message"` form (verify).

## 23. globalThis / global — done
`check.rs:1096`: both identifiers lower to `ExprKind::RealmGlobal` (opcode `0x94`),
typed `Ty::DynamicObject` — the per-realm singleton of §3.1.

## 24. Typed numeric lattice — done
`IntKind{I8..I64,U8..U64}`, `Ty::F32/F64` (`hir.rs`, `native_types.rs`); int/float TS
types with saturating coercions per the Cranelift rules. (Per MEMORY: JIT currently
f64-everything is scaffolding; the *type surface* is present.)

## 25. Decorators — missing
Listed in A ("language feature + macro"). No parser/sema/lowering support found. Needs
decorator syntax, evaluation order, and the metadata/macro mechanism.

## 26. ZIL math opcodes — done
Fifteen machine ops covering all of PLAN §3 criterion (d).

**Float, `0x96-0xA4`:** `sqrt abs neg floor ceil trunc min max round sign`. The first
eight are one Cranelift instruction each (`sqrt/fabs/fneg/floor/ceil/trunc/fmin/fmax`).
`round` and `sign` have no ISA equivalent with JS semantics, so each lowers to a short
branchless sequence rather than a libm call.

**Integer, `0x9E-0xA2`:** `min max clz ctz popcnt` → `smin/umin/smax/umax` by operand
signedness, and `clz/ctz/popcnt` directly. Width- and signedness-generic like the
existing integer arithmetic: `clz` of `1u8` is 7, `ctz` of `0u8` is 8, and the same bit
pattern gives opposite `min`/`max` answers as i8 and u8.

**Still native, by design:** the transcendentals (`sin cos tan exp log pow atan2 hypot
cbrt …`), which PLAN §3 assigns to libm.

**Never an op:** the machine `nearest`. It rounds halves to even, so it is not
`Math.round` — `nearest(2.5)` is 2. A test keeps it off the table.

### Semantics worth remembering
- **min/max (float)** are IEEE-754 2019 `minimum`/`maximum`: NaN propagates and `-0.0`
  sorts below `+0.0`, matching JS. This is *not* Rust's `f64::min`/`f64::max`
  (IEEE-754 2008 `minNum`, which returns the non-NaN operand and is unspecified between
  the two zeros); Tier 0 uses explicit `js_min`/`js_max`.
- **round** avoids `floor(x + 0.5)`: for `0.49999999999999994` the addition rounds up to
  exactly `1.0`, giving 1 where JS gives 0. It takes the fractional part instead, with a
  final `copysign` so `-0.4` rounds to `-0`.

### Wiring
`zun_ilgen::MATH_MACHINE_OPS` maps the `zun:math` natives onto the opcodes, so
`math.sqrtNumber(x)` emits `sqrt.f64`. The natives stay registered, making this a
substitution: a mapping that stops matching costs speed, never correctness. A test walks
the table against the registry, because a native rename would otherwise disable the
expansion silently (`.docs-dotnet/07-actions-operations/05-jit-intrinsics.md`).

Nothing emits the integer ops yet — there are no int-min/max or bit-counting natives.
They are the machine floor for Buffer and TypedArray (#19, #21) and `Math.clz32`.

### Performance
Benchmarked per PLAN §1 (`zun_codegen/tests/math_opcode_bench.rs`, ignored by default).
The measurement does **not** support a speed claim. `round` is consistently ahead of the
call it replaces (1.21-1.54x over four runs) but `sign` straddles 1.0, and the control is
the problem: a single `sqrt` instruction measures the same as a call performing it,
steady at 0.99x, so the harness does not isolate call overhead at this scale. These ops
rest on the PLAN mandate and on what the IL carries — the verifier knows the stack
effect, AOT needs no libm symbol, no native-name string coupling — with the measurement
showing only that nothing regressed.

### Fixed in passing
Float unary minus lowered as `0 - x`, which yields `+0.0` for `-0.0`, so `1 / -zero` gave
`inf`. It now uses `neg.f64`.

### Tests
`zun_il/tests/math_opcodes.rs` (table), `zun_verify/tests/math_opcodes.rs`
(`_Valid`/`_Invalid_*` fixtures), `zun_interp/tests/math_opcodes.rs` (Tier 0 edges),
`zun_codegen/tests/math_opcode_equivalence.rs` (interp↔JIT over the float edge operands,
the full matrix for binaries, f32, and the round/sign traps),
`zun_codegen/tests/int_math_opcodes.rs` (width and signedness across all eight int
kinds), `zun_ilgen/tests/math_machine_ops.rs` (lowering),
`zun_vm/tests/math_machine_ops.rs` (end to end).

(Math as a `Math` *global object* is bucket B, not audited here.)

---

## Prioritized gap list (matches GOAL.md tiers)

### HIGH — "everything uses these"
1. **`Object.*`** (#6) — ✅ core surface landed: `keys values entries assign freeze
   isFrozen create getPrototypeOf setPrototypeOf fromEntries is` + instance
   `hasOwnProperty isPrototypeOf`. **Remaining (deferred):** the descriptor-based statics
   (`defineProperty defineProperties getOwnPropertyDescriptor(s) seal isSealed
   preventExtensions isExtensible getOwnPropertyNames getOwnPropertySymbols`) + instance
   `propertyIsEnumerable` — blocked on introducing a property-descriptor model (see §6).
2. **`Function.call/apply/bind`** (#8) — reflective invokers (PLAN §3.1 says implement
   directly); add `.name/.length`. `CallIndirect` + function-handle base exists.
3. **Symbol — complete** (#5) — `Symbol()` unique primitives; `Symbol.for/keyFor` (wire
   the existing `SymbolRegistry` global map); well-known symbols as dispatch points
   (`iterator asyncIterator hasInstance toPrimitive toStringTag`); user symbol-keyed
   members + computed keys (slot already exists).

### MID
4. **WeakSet** (#14) and **WeakRef** (#15) — GC base ready (WeakMap ephemerons).
5. **Decorators** (#25) — parse + sema + lowering + macro mechanism.
6. **Method-surface tail** — ECMAScript gaps on existing classes:
   - Array (#7): `forEach find/findIndex(cb) some every flatMap join splice shift unshift
     keys values entries`, comparator `sort(cb)`, statics `from/of/isArray`.
   - String (#1): `split slice at concat match/matchAll/search replaceAll charAt
     codePointAt [Symbol.iterator]`, statics `fromCharCode/fromCodePoint`.
   - Number (#2): statics `isNaN/isInteger/isFinite/parseInt/parseFloat` + constants;
     `toString(radix) toPrecision toExponential`.
   - Date (#20): setters, local-time getters, `Date.now/parse/UTC`, `getTimezoneOffset`.
   - Map/Set (#11,#12): `forEach`, construct-from-iterable, `size` as property.
   - Promise (#9): `then/catch/finally`, statics `resolve/reject/all/allSettled/race/any`.
   - TypedArrays/DataView (#18,#19): missing element kinds + full method surface.
   - BigInt (#4): `div/rem/pow/neg`, bitwise/shift, `BigInt()` coercion, `asIntN/asUintN`.
   - Error (#22): `AggregateError`, `cause`, verify `.stack` capture.
   - ArrayBuffer (#17): `byteLength slice isView`.
   - Proxy/Reflect (#10): full trap set + `Reflect` global.
7. **Boolean wrapper object** (#3) — low (primitive suffices for most code).

### Infra (separate from the class surface)
- ~~**ZIL math opcodes** (#26)~~ — done: fifteen ops at `0x96-0xA4` (ten float, five
  integer), transcendentals left on libm. See §26.

---

## Surprising findings
- **Object and Function started with zero surface** — not even a recognized global
  namespace. `Object` now has its enumeration/reflection surface (§6); `Function.call/
  apply/bind` remains the top-tier gap.
- **Number predicates are instance-only** (`x.isNaN()`), the opposite of idiomatic JS;
  the `Number.*` statics are entirely absent.
- **Array carries a large non-standard "collection algebra" surface** (`chunk windowed
  union intersection isPalindrome isSubsetOf rotateLeft/Right …`) while lacking bread-and-
  butter standards like `forEach/find(cb)/join/splice`.
- **`sort()` takes no comparator** and **`findIndex`/`filter` predicates** diverge from
  spec arity — latent semantic traps.
- **FinalizationRegistry signature diverges** from spec (numeric id vs unregister token;
  no heldValue).
- ~~**ZIL math opcodes are not opcodes**~~ — resolved; see §26. Fixing it also exposed
  that float unary minus lowered as `0 - x`, so `-0.0` came out `+0.0`.
- **Infinities print as `inf`/`-inf`**, not JS's `Infinity`/`-Infinity` (`format_number`
  in `zun_interp/value.rs`). Belongs to the Number surface (#2).
- **Symbol infrastructure exists but is dark:** `SymbolRegistry` is fully implemented in
  `zun_core` yet only `Symbol.iterator` reaches user code.
