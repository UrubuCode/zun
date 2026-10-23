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
| 1 | String | partial | Standard surface largely landed (`at/slice/concat/codePointAt/split/replaceAll`, string-vs-RegExp dispatch on `split`/`replace`, `[Symbol.iterator]`, statics `fromCharCode/fromCodePoint`); deferred `matchAll/normalize/localeCompare/toLocale*/String.raw` |
| 2 | Number | partial | Statics landed (`isNaN/isFinite/isInteger/isSafeInteger parseFloat parseInt` + all eight constants) and `toString(radix)`; remaining `toPrecision/toExponential/toLocaleString/valueOf` |
| 3 | Boolean | missing | No wrapper class/type; only the primitive `Ty::Boolean`. No `Boolean()`, no `.toString()` object surface |
| 4 | BigInt | partial | Only `add/sub/mul/compare/toString`; missing `div/rem/pow/neg/bitwise/shift`, `BigInt()` coercion, `asIntN/asUintN`, operator integration |
| 5 | Symbol | partial | Foundation landed: `Symbol()` unique primitives, `Symbol.for/keyFor`, well-known symbol **values** (`iterator/asyncIterator/hasInstance/toPrimitive/toStringTag`), `.description`, symbol-keyed `obj[sym]` get/set. Remaining: behavioral dispatch of `hasInstance/toPrimitive/toStringTag` (value-only). Object-literal computed symbol keys now land (`{ [sym]: v }`); class computed symbol-key members still deferred |
| 6 | Object | partial | Enumeration/reflection surface landed (`keys/values/entries/assign/freeze/isFrozen/create/getPrototypeOf/setPrototypeOf/fromEntries/is` + instance `hasOwnProperty/isPrototypeOf`); descriptor-based statics deferred (no property-descriptor model) |
| 7 | Array | partial | Standard callback + structural surface landed (spec `(value,index,array)` callbacks); Zun extensions kept. Missing `findLast/findLastIndex/flat/flatMap/keys/values/entries/at` iterators, statics `Array.from/of/isArray`, variadic `splice` inserts, no-initial `reduce` |
| 8 | Function | partial | `call/apply/bind` + `.name/.length` landed (receiver-as-leading-param model, Tier 0); no `Function` constructor, no bound-arg function values |
| 9 | Promise | partial | Instance `state/result/isReady/resolve/reject`; no `then/catch/finally`, no statics `Promise.resolve/reject/all/allSettled/race/any`. `await` works via opcode |
| 10 | Proxy / Reflect | partial | `proxyNew` + `reflect_get/reflect_set` exist; no full trap set, no `Reflect` global object surface |
| 11 | Map | partial | `set/get/has/size/delete/clear/values/keys/entries` + iterator; missing `forEach`, `size` as property (is a method), constructor-from-iterable |
| 12 | Set | partial | `add/has/size/delete/clear/values/keys/entries` + iterator; missing `forEach`, constructor-from-iterable |
| 13 | WeakMap | partial | `set/get/has/delete`; semantics OK (ephemeron floor). No iteration (correct) |
| 14 | WeakSet | done | `add/has/delete` over the WeakMap ephemeron floor (value-less); no iteration (correct) |
| 15 | WeakRef | done | `new WeakRef(obj)` + `deref()` over the GC weak-reference floor; cleared on collection |
| 16 | FinalizationRegistry | partial | `register/unregister` present over GC finalization queue; `unregister` keyed by numeric id, not token object (verify vs spec) |
| 17 | ArrayBuffer | partial | `new ArrayBuffer(len)` intrinsic; no `byteLength/slice/isView/resize` |
| 18 | DataView | partial | Only `getUint32/setUint32/getUint16/setUint16/getFloat64/setFloat64`; missing Int8/16/32, Uint8, Float32, BigInt64/Uint64, `byteLength/byteOffset/buffer` |
| 19 | TypedArrays | partial | Only `Uint8Array/Uint16Array/Uint32Array/Float64Array`, each only `get/set`. Missing Int8/16/32, Uint8Clamped, Float32, BigInt64/Uint64 Array, and the whole method surface (`length/subarray/set/slice/fill/map/forEach/iterator`) |
| 20 | Date | partial | Now a global with JS statics (`now/parse/fromISO`); UTC getters present. Missing all setters, local-time getters (UTC-only calendar), `Date.UTC`, `toLocale*`; `new Date()` needs optional construction args |
| 21 | Buffer | partial | Native `zun:buffer` surface exists; verify Node `Buffer` API coverage (`from/alloc/write/read*/toString/slice/concat`) |
| 22 | Error (+ subclasses) | partial | `Error` + 6 subclasses with JS-shaped `new Error(message)`, per-class `name` getter, JS `toString()`. Not a realm global; `.stack` absent entirely; `cause`/`AggregateError` blocked on construction having no optional args |
| 23 | globalThis / global | done | `globalThis`/`global` → `RealmGlobal` opcode (`Ty::DynamicObject`), realm singleton per §3.1 |
| 24 | Typed numeric lattice | done | `i8..i64/u8..u64/f32/f64` in `IntKind`/`Ty`; int/float TS types + coercions |
| 25 | Decorators | missing | Planned in A; not implemented (no parse/sema/lowering) |
| 26 | ZIL math opcodes | done | 15 ops at `0x96-0xA4`: float `sqrt/abs/neg/floor/ceil/trunc/min/max/round/sign`, int `min/max/clz/ctz/popcnt`. Transcendentals stay on libm per PLAN §3 |

**Counts:** done 5 · partial 19 · missing 2 (of 26 nucleus items, JSON excluded).

(Derived from the table above, not tracked by hand. Both sides of a merge have
had this line wrong; recount the rows rather than taking either version.)

### Declarative realm-global registration (PLAN §3.1)

Base types are seeded onto `globalThis` from the declarative registry
(`builtin/realm.zun` → `zun_sema::realm`); the registry is the single source of
truth for each name's annotation type, descriptor value, and `new` lowering (the
frozen runtime representation is unchanged — the registry only rebinds the name).

- **Registered declaratively:** `Map`, `Set`, `WeakMap`, `WeakSet`, `WeakRef`,
  `FinalizationRegistry`,
  `Promise`, `RegExp`, `BigInt`, `Array`, `ArrayBuffer`, `DataView`,
  `Uint8Array`, `Uint16Array`, `Uint32Array`, `Float64Array`, `String`,
  `Number`, `Boolean`, `Object`, `Symbol`. (`Map` keeps token 0.)
- **Not registrable yet:** `Date` and `Error` (+ subclasses
  `EvalError/RangeError/ReferenceError/SyntaxError/TypeError/URIError`) — their
  annotation would resolve to `Ty::Instance(<program-assigned class index>)`, which
  a static `base_type_facets` entry cannot name; and `Proxy` (not in bucket A).
  The hardcoded `new`/annotation matches remain for the lowercase aliases
  (`map`, `array`, …) and `Proxy`, so those are not yet removable.
  **Corrected 2026-10-23:** `Date` and `Error` are *not* on a hardcoded fallback,
  as this previously said. They are absent from the checker entirely, so they are
  unreachable as globals and a program must declare its own class. Letting the
  registry name a class-backed base type is the open work, and it unblocks both.

---

## 1. String — partial  *(standard surface largely landed)*
Primitive-backed class, statically bound (`builtin/string.zun`, dispatch in
`check.rs` `managed_method` + class fallback at `check.rs:2800+`; native impls
`zun_natives/src/string.rs`, with array/iterator-building members in
`zun_interp/src/collections.rs`).

**Index/encoding unit: UTF-16 code units** for `at/slice/codePointAt/charCodeAt/
substring/length/padStart/padEnd/reverse` (what `charCodeAt`/`substring` already use).
Code-point iteration is the exception: `for...of`, the string `[Symbol.iterator]`, and
the `fromCodePoint` static operate on Unicode code points (surrogate pairs kept whole).

- **Present (extern/native):** `length() contains count compare toInt toFloat toBoolean
  includes indexOf lastIndexOf substring padStart padEnd toLowerCase toUpperCase
  trimStart trimEnd trim charCodeAt startsWith endsWith repeat reverse`, plus a large
  non-standard validator surface (`isAlpha/isDigit/isHex/isSnakeCase/…` — Zun extras).
- **Present (new, native/intercept — do not need the `String` class in scope):**
  - `at(i)` — negative index from end; out-of-range → `undefined`.
  - `codePointAt(i)` — combines a surrogate pair into its astral code point.
  - `slice(start?, end?)` — negative indices from the end (distinct from `substring`);
    absent `end` → length (lowered as a past-the-end default that clamps natively).
  - `concat(...strs)` — variadic; folded in sema into a chain of the two-arg `concat`
    native (zero args → the receiver).
  - `split(separator, limit?)` and `replace(search, replacement)` — **dispatch by the
    first argument's type**: a `RegExp` keeps the existing regex-engine path
    (`regexpSplit`/`regexpReplace`, unchanged), a `string` uses the string path
    (`stringSplit` builds the array in the interpreter floor; `replace` → the `zun:string`
    `replace` native, first occurrence). `split("")` → one piece per UTF-16 unit; `split`'s
    optional `limit` truncates the result (a RegExp separator takes no limit).
  - `replaceAll(search, replacement)` — all occurrences (string search only).
  - `[Symbol.iterator]` / `for...of` — iterate by code point via `stringIteratorNew`
    (builds a code-point array and reuses the array iterator's `next`/`done`).
  - Statics `String.fromCharCode(...codes)` / `String.fromCodePoint(...codes)` —
    recognized on the `String` receiver (`check_string_static`), variadic, folded via
    single-code natives `fromCharCode1`/`fromCodePoint1` + `concat`.
- **Deferred (documented, not faked):** `matchAll` (needs global-regex iteration),
  `normalize` (Unicode normalization), `localeCompare`/`toLocaleLowerCase`/
  `toLocaleUpperCase` (needs a locale/ICU model), `String.raw` (template-tag mechanism),
  `charAt` (superseded by `at`), `valueOf`. Replace-with-a-function replacer is also not
  supported (replacement must be a string).
- **Notes:** the string-dispatched `split`/`replace`/`replaceAll`/`concat`/`slice`/`at`/
  `codePointAt` and the statics all allocate/branch only (Tier-0 floor), so they are not
  in the JIT allowlist — no second path to keep in sync.

## 2. Number — partial  *(statics + radix landed)*
`std/number.zun`; native `number.rs`. Statics and constants are recognized in sema
(`check.rs` `check_number_static` for `Number.<m>(...)` and the `Number.<CONST>` fold in
the `ast::Expr::Field` arm) and lowered to `zun:number` natives on the Tier-0 floor.

- **Present (statics):** `isNaN isFinite isInteger isSafeInteger` — the **strict**
  forms (no coercion: a non-number argument answers `false`, distinct from the global
  `isNaN`/`isFinite`), `parseFloat(s)` (longest leading numeric prefix, `NaN` on
  failure), `parseInt(s, radix?)` (radix 2..36, auto `0x`→16 / else 10 when omitted,
  `NaN` on no digit). Implemented as `isNaNStrict/isFiniteStrict/isIntegerStrict/
  isSafeIntegerStrict` (param `Opaque`, so the raw value — including an `Any` — reaches
  the native un-coerced) + `parseFloatPrefix/parseIntRadix`.
- **Present (constants):** `MAX_SAFE_INTEGER MIN_SAFE_INTEGER MAX_VALUE MIN_VALUE
  EPSILON POSITIVE_INFINITY NEGATIVE_INFINITY NaN` — folded to compile-time `number`
  literals in sema (the `Number` class carries no static fields, so `static_field_of`
  falls through to the fold).
- **Present (instance):** `isNaN isFinite isNegativeZero isSafeInteger isInteger toFixed`
  + `toString(radix?)` — base 2..36, default 10, non-finite rendered JS-style
  (`NaN`/`Infinity`/`-Infinity`); routed via a sema intercept on an `F64` receiver to the
  `toStringRadix` native ahead of class dispatch (supersedes the no-arg extern
  `toString`). Plus the Zun math helpers (`abs clamp min max sign lerp …`). Free fns
  `numberParse/numberToFixed`.
- **Missing / deferred:** instance `toPrecision toExponential valueOf`; `toLocaleString`
  (locale — deferred, needs an ICU/locale model). `toPrecision`/`toExponential` were
  deferred rather than faked: JS's precision/exponent formatting (significant-digit
  rounding, exponent thresholds) is non-trivial and no consumer needs it yet.
- **Notes:** the instance predicates (`x.isNaN()`) remain for Zun-style call sites; the
  new statics are the idiomatic `Number.isNaN(x)` surface. All of these allocate/branch
  only and are not JIT-lowered — Tier-0 only, no second path to keep in sync.

## 3. Boolean — missing
No `std/boolean.zun`, no wrapper type; only primitive `Ty::Boolean`. No `Boolean()`
coercion function, no object `.toString()/.valueOf()`. GOAL ranks this lowest.

## 4. BigInt — partial
`std/bigint.zun` (extern) + native (`collections.rs`, `zun_core/bigint.rs`).

- **Present:** `add sub mul compare toString`.
- **Missing:** `div rem pow neg` and bitwise/shift (`and or xor not shl shr`),
  `BigInt(value)` coercion, `BigInt.asIntN/asUintN`, `valueOf`, operator-level
  integration (`+ - * / %` on bigint operands), literal `1n` → confirm lowering (verify).

## 5. Symbol — partial  *(HIGH priority — foundation landed)*
The `SymbolRegistry` (`zun_core/src/symbols.rs`) is now surfaced to user code. A symbol
is a first-class value: `Ty::Symbol` in sema, carried at runtime as an immediate `Any`
(`AnyTag::Symbol`, payload = `SymbolId`), so the GC traces straight past it. The
isolate's `Heap` owns the registry (`symbol_create/for/key_for/description`), seeded with
the five well-known symbols at fixed ids 0..4 (`zun_core::WellKnownSymbol`) so their
identities are stable and match the compiler's constants. All Symbol operations are
allocating/interning managed builtins on the Tier-0 floor (`zun:collections`
`symbolNew/For/KeyFor/WellKnown/Description`, dispatched in
`zun_interp/src/collections.rs`) — not JIT-eligible, so there is no second path to keep
in sync.

- **Present (value foundation):**
  - `Symbol(description?)` — a unique symbol every call; `Symbol("x") !== Symbol("x")`.
  - `Symbol.for(key)` / `Symbol.keyFor(sym)` — the cross-realm registry (reverse map in
    `SymbolRegistry`); `keyFor` returns `undefined` for unique/well-known symbols.
  - Well-known symbols as **readable values** — `Symbol.iterator`, `Symbol.asyncIterator`,
    `Symbol.hasInstance`, `Symbol.toPrimitive`, `Symbol.toStringTag` — stable identities.
  - `symbol.description`.
  - User **symbol-keyed members**: `obj[sym]` get/set (computed member with a Symbol key)
    routes through the existing symbol-keyed slot (`dynamic_get_symbol`/`set_symbol`),
    kept distinct from string keys. The read path reuses `DynamicGetValue`; the store path
    now lowers `obj[key] = v` on dynamic receivers to `DynamicSetValue` (string **and**
    symbol keys) — `StoreDynamicValue`/`LoadDynamicValue` dispatch on the key's tag.
  - `Symbol.iterator` iteration dispatch for Array/Map/Set/Error preserved
    (`check.rs` ComputedCall intercept for `x[Symbol.iterator]()` → `*IteratorNew`).
  - Parser now accepts reserved keywords as member names (`Symbol.for`), via
    `TokenKind::is_keyword` + `member_name` in `zun_syntax`.
- **Deferred (documented follow-up):**
  - **Behavioral dispatch** of the non-iterator well-known symbols: making `instanceof`
    consult `Symbol.hasInstance`, coercion consult `Symbol.toPrimitive`, and
    `Object.prototype.toString` consult `Symbol.toStringTag`. These are exposed as
    **values** only; wiring their dispatch is a separate step (the iterator one already
    drives `for...of`).
  - **Object-literal computed symbol-key properties** (`{ [computedSymbol]: v }`) now
    lower: an object literal builds a `DynamicObject` (`Expr::ObjectLiteral` →
    `hir::ExprKind::DynamicObjectLiteral` → `NewDynamic` + per-entry `StoreDynamic`/
    `StoreDynamicValue`), and a computed key that evaluates to a `Symbol` routes to the
    symbol-keyed slot. Still deferred: **class** computed symbol-key member definitions
    and object **method shorthand** with a computed symbol name (`{ [Symbol.iterator]() {} }`)
    — see §Object literals below.
  - Other well-known symbols (`isConcatSpreadable match replace search split species
    unscopables`) — not seeded; add to `WellKnownSymbol` when a consumer needs them.

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

## 6b. Object literals — partial  *(expression syntax landed)*
`{ key: value, ... }` now parses and lowers in the front-end (both the hand parser —
`Parser::object_literal`, dispatched from `primary()` on `{` in expression position — and
the OXC adapter — `Expression::ObjectExpression`). An object literal produces a
`DynamicObject` (`Ty::DynamicObject`): `Expr::ObjectLiteral` → `hir::ExprKind::DynamicObjectLiteral`
→ `NewDynamic`, then for each property a `Dup` + store + `Pop` so the object is the result.
- **Present (v1):** string keys from identifier/string/number property names (`StoreDynamic`);
  **shorthand** `{ x }` ≡ `{ x: x }`; **computed keys** `{ [expr]: v }` (`StoreDynamicValue`,
  where a `Symbol` key routes to the symbol-keyed slot and a string key to the ordinary
  property); empty `{}`; nested literals. Properties are set in source order
  (`DynamicObject` is insertion-ordered). Values box to `Any` for the dynamic store.
- **Block-vs-object:** the hand parser treats `{` as an object literal only in expression
  position; it has no bare block statement, so the ambiguity does not arise there.
- **Deferred (documented follow-ups — the OXC adapter errors rather than miscompiling, the
  hand parser does not yet accept them):** method shorthand `{ m() {} }`, getters/setters
  `{ get x() {} }` / `{ set x(v) {} }`, and spread `{ ...o }`.
- **Note on computed keys:** like `obj[k] = v` on a dynamic receiver, a computed key must be
  a string or a `Symbol` at runtime (`StoreDynamicValue` dispatches on tag); full ToString
  coercion of arbitrary computed keys is not implemented (consistent with `IndexAssign`).
- Interp fix landed alongside: `Value::as_object` now unwraps an object-tagged `Any`, so a
  nested object read back out of a dynamic property (`{ a: { b } }.a.b`) works.

## 7. Array — partial (standard surface landed)
`builtin/array.zun` (Zun-extension surface) + special ZIL lowering for `push/pop`
(`ArrayPush/ArrayPop` opcodes), plus `arrayIterator*` intrinsics. The standard
callback and structural methods are **`zun:core` `array` natives**
(`zun_natives/src/collections.rs` signatures, `zun_interp/src/collections.rs`
implementations); sema routes those method names to the natives before class
dispatch (`check.rs`, `Ty::Array` arm of the managed-method table plus the
`sort/reduce/reduceRight` pre-block).

- **Present (standard):** `map(cb) filter(cb) forEach(cb) find(cb) findIndex(cb)
  some(cb) every(cb) reduce(cb,init) reduceRight(cb,init) sort(cmp?) join(sep)
  shift() unshift(v) splice(start,deleteCount) slice(start,end) concat(other)
  fill(value) reverse copyWithin(t,s,e) flat indexOf lastIndexOf includes at(i)
  length` + `[Symbol.iterator]`. Every callback receives the ECMAScript
  `(value, index, array)` signature (comparator `(a, b)`); surplus arguments are
  dropped to the callback's declared arity (interp `invoke_callback` truncates to
  the callee `param_count`). `sort()` with no comparator is ascending **string**
  order (spec default).
- **Zun extensions (kept, beyond ECMAScript):** `chunk windowed rotateLeft/Right
  union intersection difference symmetricDifference isSubsetOf isSupersetOf
  isPalindrome isSorted isUnique sum product average min max transpose sortNumeric
  take drop insertAt removeAt removeFirst removeAll swap count contains …` — the
  "collection algebra" surface in `builtin/array.zun`. (`sort`'s former numeric
  default is preserved as the extension `sortNumeric()`.)
- **Missing (standard):** `findLast findLastIndex flatMap keys() values() entries()
  at`-iterators `toSorted toReversed with group`; statics `Array.from(iterable
  [,mapFn]) Array.of(...) Array.isArray(x)`; variadic `splice` inserts; the
  no-initial `reduce`/`reduceRight` form (throws-on-empty); `fill`/`copyWithin`
  range refinements.
- **Notes:** these callback methods are Tier 0 (they re-enter the interpreter via
  `self.call`, like proxy traps / `functionApply`); `CallIndirect` is not
  JIT-lowered, so there is no new JIT path. `indexOf`/`lastIndexOf`/`includes`
  stay value-based (spec) and `indexOf` returns `int`.

## 8. Function — partial  *(HIGH priority — core surface landed)*
`call/apply/bind` + `.name/.length` are implemented. Recognition is in sema's
member-call path when the receiver is `Ty::Function` (`zun_sema/src/function_reflect.rs`,
dispatched from `check.rs` `MethodCall`); `.name/.length` resolve from the compile-time
signature in `check.rs` `Field`.

- **`this` model:** Zun threads a method's receiver as the **leading positional
  parameter slot**, so these invokers reduce to positional-argument manipulation.
  `f.call(thisArg, ...args)` lowers to the ordinary indirect call `f(thisArg, ...args)`
  (reuses `CallIndirect`; no new runtime helper). A plain function with no declared
  parameters ignores `thisArg` (the arity check rejects passing one). There is no
  separate `this` slot: "bound this" and "bound/partial args" are uniformly prepended
  positional arguments.
- **`apply(thisArg, argsArray)`** → `functionApply` native (`zun:collections`,
  `zun_interp/src/collections.rs`): prepends `thisArg`, spreads the managed array, and
  re-enters the interpreter via `self.call`.
- **`bind(thisArg, ...partial)`** → `functionBind` native: allocates a **bound-function
  record** (a GC `DynamicObject` storing the target function via `dynamic_set_function`
  and a managed array of the prepended args `[thisArg, ...partial]`), returned as a
  first-class `Value::Function` whose `FunctionHandle` is tagged bound (`reserved`
  discriminant; `environment` points at the record). Calling it flows through
  `CallIndirect`, which intercepts bound handles, peels the `bind` chain (supports
  bind-of-bound) via `resolve_call_target`, and dispatches the target. The record and
  its contents are GC-traced through the handle's `environment` root
  (`relocate_environment` preserves the bound tag across compaction). Bound functions
  are typed `Function` (dynamic), so they are callable with exactly one trailing
  argument per the dynamic-function-value arity model.
- **`.name`** → the signature name (`""` for a dynamic/closure value);
  **`.length`** → declared required-parameter count (params minus optionals minus rest;
  `0` for a dynamic/closure value). Both are compile-time constants — no runtime
  function metadata was invented.
- **Tiering:** reflective/variadic invokers; `functionApply`/`functionBind` are **not**
  in the JIT allowlist and `CallIndirect` is not JIT-lowered, so every caller stays on
  the always-correct Tier 0 interpreter. No separate JIT path to keep in sync.
- **Still missing:** the `Function` constructor; binding a **function-typed** partial
  argument (prepended args are `Any`-boxed, and functions are deliberately kept out of
  `Any`) — rejected at runtime; multi-argument calls to a bound function beyond the
  dynamic-value single-arg model.

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

## 14. WeakSet — done
`builtin/weak_set.zun`; reuses the WeakMap ephemeron floor (each object member is
stored keyed to itself, value-less).
- **Present:** `add has delete`. Correctly non-iterable. Registered declaratively
  (`new WeakSet()` / `: WeakSet` via the realm registry).
- **Missing:** nothing material (WeakSet has no iteration/size/clear).

## 15. WeakRef — done
`builtin/weak_ref.zun`; a single weakly-held reference over the GC weak-reference
floor (`alloc_weak_ref`/`weak_ref_get_object`, cleared on the collection that
reclaims the target).
- **Present:** `new WeakRef(obj)` + `deref()` (returns the target while live,
  `undefined` once collected). Registered declaratively.
- **Missing:** nothing material.

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
`builtin/date.zun` over the `zun:date`/`zun:time` intrinsics.

- **Now reachable as a global**, through the prelude, for the same reason as
  `Error` (#22): a plain class exists only where its source does, and the
  `zun:std:date` module registration it had was unreachable from user code. That
  registration is removed so the class is not declared twice.
- **JS statics added:** `Date.now()` and `Date.parse(iso)` return a timestamp, as
  they do in JS, plus `Date.fromMillis`, `Date.fromISO` and `Date.fromISODate` for
  the object-returning forms. These replace the former top-level `dateNow`,
  `dateParse`, `dateParseDate` and `dateFromMillis`, which could not live in the
  prelude: top-level functions there become expected exports of the cached `zun:std`
  module, which does not have them.
- **Present:** millis storage, `getTime/setTime valueOf isValid isBefore/isAfter/
  isEqual`, UTC getters `getUTCFullYear/Month/Date/Hours/Minutes/Seconds/
  Milliseconds/Day`, `toISOString toJSON toString`.
- **Construction is `new Date(millis)` only.** JS also has `new Date()` and
  `new Date(string)`; Zun builds an object from exactly one positional argument per
  field, with no constructors or overloads, so those are the statics above. Same
  blocker as `cause`/`AggregateError` in #22.
- **Missing:** all setters (`setFullYear setMonth …`); local-time getters
  (`getFullYear getHours …`) and `getTimezoneOffset`, which are absent rather than
  wrong because the calendar is UTC-only; `Date.UTC`; `toDateString toTimeString
  toLocale*` `toUTCString`.

## 21. Buffer — partial / verify
Native surface in `zun_natives/src/buffer.rs` (also hosts array/map/set iterator
intrinsics). Node `Buffer` API coverage not confirmed from the TS side.
- **Verify:** `Buffer.from/alloc/allocUnsafe/concat/isBuffer/byteLength`, instance
  `write readUInt*/readInt*/readFloat*/readDouble* toString(encoding) slice fill copy
  equals indexOf`. Likely largely **missing** as a user-facing `Buffer` class — flag for
  deeper read.

## 22. Error (+ subclasses) — partial
`builtin/error.zun` (module `zun:std:error`): `Error` + `EvalError RangeError
ReferenceError SyntaxError TypeError URIError`. `error_value.rs` in core.

- **Construction is now the JS shape.** `new Error(message)` takes the message alone;
  `name` is a getter each subclass overrides, resolved by virtual dispatch even
  through a base-typed reference. It was previously a field, which made the
  constructor `new Error(name, message)` and forced every caller to repeat what the
  class already knows. The cost is that `name` is read-only, where JS allows
  `e.name = "..."`; that is a dynamic-path concern.
- **`toString()`** follows JS, including the empty-message case: `new Error("")`
  prints `Error`, not `Error: `.
- **`.stack` is absent** — not merely unverified, which is what this entry used to
  say. There is no `stack` property and no capture anywhere; the sequence-point
  debug info that landed is a prerequisite, not the feature.
- **`Error` is now a global**, in both compile paths. It reaches programs through
  the prelude (`zun_prelude::ERRORS`), not as a module: **a class cannot cross a
  module boundary in Zun**, so `builtin/error.zun` as the `zun:std:error` module was
  unreachable from user code and programs declared their own `class Error {}`.
  It is no longer registered as a module, to avoid declaring each class twice.
- **Why not linked instead of inlined.** Making classes importable needs a
  program-wide type identity first. The object header stores a per-module type RID
  and the GC reads it (`ref_slots(obj.type_rid())`) to find reference slots, while
  `TypeSystem` is built per module — so an object built from a foreign `TypeRef`
  would make the collector trace the wrong fields. That is heap corruption, not a
  wrong answer, which is why the cheap inlining is the safe option and the linking
  work is a project of its own. It would also unblock **#20 Date** and the
  precompiled `zun:std` of PLAN §3.2.
- **Still not a realm global** (`globalThis.Error`): a static `base_type_facets`
  entry cannot denote `Ty::Instance(<program-assigned class index>)`. The class is
  in scope by name, which is what user code needs; the realm binding waits on the
  same type-identity work.
- **`cause` and `AggregateError` are blocked on construction, not on effort.** Object
  construction requires exactly one positional argument per declared field, with no
  constructors, defaults or optional arguments. So `cause` cannot be an optional
  second field without breaking `new Error(msg)`, and `AggregateError` cannot take
  JS's `(errors, message)` order, since inherited fields come first and would force
  `(message, errors)`.
- **`Error.captureStackTrace` / `stackTraceLimit`** are V8 extensions rather than
  ECMAScript; worth a deliberate decision before implementing.
- **Tests:** `zun_vm/tests/error_surface.rs` runs the shipped `builtin/error.zun`
  source itself rather than a copy.

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
2. **`Function.call/apply/bind`** (#8) — ✅ landed (receiver-as-leading-param model,
   Tier 0) with `.name/.length`. **Remaining:** `Function` constructor; function-typed
   bound args.
3. **Symbol — foundation complete** (#5) — ✅ `Symbol()` unique primitives;
   `Symbol.for/keyFor`; well-known symbols as **values**
   (`iterator asyncIterator hasInstance toPrimitive toStringTag`); `.description`;
   user symbol-keyed `obj[sym]` get/set over the existing slot. **Remaining (deferred):**
   behavioral dispatch of `hasInstance/toPrimitive/toStringTag` (value-only for now);
   object-literal/class computed-key member definitions (no object literals in the AST).

### MID
4. ~~**WeakSet** (#14) and **WeakRef** (#15)~~ — done (reuse the WeakMap ephemeron / GC weak-reference floors).
5. **Decorators** (#25) — parse + sema + lowering + macro mechanism.
6. **Method-surface tail** — ECMAScript gaps on existing classes:
   - Array (#7): remaining — `findLast findLastIndex flatMap keys values entries at`,
     statics `from/of/isArray`, variadic `splice` inserts, no-initial `reduce`.
     (Done: `forEach find findIndex(cb) some every map filter reduce reduceRight join
     splice shift unshift`, comparator `sort(cmp?)` — all spec `(value,index,array)`.)
   - String (#1): ✅ `at slice concat codePointAt split replaceAll`, string-vs-RegExp
     dispatch on `split`/`replace`, `[Symbol.iterator]`, statics
     `fromCharCode/fromCodePoint` — landed. Remaining (deferred):
     `matchAll normalize localeCompare toLocale* String.raw`.
   - Number (#2): ✅ statics `isNaN/isInteger/isFinite/isSafeInteger/parseInt/parseFloat`
     + constants, and `toString(radix)` — landed. Remaining: `toPrecision toExponential
     toLocaleString valueOf`.
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

## Language gaps blocking the method-surface tail

These are not per-class work; they are the common causes behind several
"missing" entries above.

- **`any`-typed class fields silently store zero.** `encode_field` maps `Any` to
  `JitKind::Other => 0`, so `class H { v: any; }` with `new H(42)` reads back as
  `Void`, and `new H(null)` does not compare equal to `null`. Making it work needs
  boxing on store, unboxing on load, and the GC tracing the slot — so it touches
  collector correctness and belongs in a `GC:` commit. This is what blocks
  `Error.cause` and `AggregateError.errors` (#22): both want an `any` field.
- **Construction has no constructors.** An object is built from exactly one
  positional argument per declared field, inherited fields first. Trailing fields
  with a default initializer are now optional (`class P { x: i64; y: i64 = 5; }`
  accepts `new P(1)`), which covers the common case, but there is still no way to
  reorder arguments or overload. That is why `AggregateError` cannot take JS's
  `(errors, message)` and `new Date()` cannot mean "now".
  `.docs-dotnet/05-object-creation/01-newobj-e-construtores.md` recommends the
  split the CLR makes: `newobj` as one opcode that the backend decomposes into
  allocation plus a constructor call.

## Surprising findings
- **Object and Function started with zero surface** — not even a recognized global
  namespace. `Object` now has its enumeration/reflection surface (§6) and `Function`
  now has `call/apply/bind` + `.name/.length` (§8), both over the frozen floor.
- ~~**Number predicates are instance-only**~~ — resolved: the idiomatic strict
  `Number.isNaN/isFinite/isInteger/isSafeInteger` statics, `parseFloat/parseInt`, the
  eight constants, and `toString(radix)` now land (§2). The instance predicates remain.
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
- **Symbol infrastructure is now surfaced:** the `SymbolRegistry` (`zun_core`) backs a
  first-class `Ty::Symbol` value (immediate `AnyTag::Symbol`). `Symbol()`,
  `Symbol.for/keyFor`, the five well-known symbols as values, `.description`, and
  symbol-keyed `obj[sym]` get/set all reach user code. The behavioral dispatch of
  `hasInstance/toPrimitive/toStringTag` remains a value-only follow-up (see §5).
