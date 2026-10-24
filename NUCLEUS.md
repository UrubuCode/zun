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
| 3 | Boolean | mostly | `Boolean(x)` truthiness coercion + `boolean.toString()`; `globalThis.Boolean` resolves. `new Boolean()` wrapper object deferred |
| 4 | BigInt | mostly | Full op surface: `add/sub/mul/div/rem/pow/neg`, bitwise/shift (`and/or/xor/not/shl/shr`, two's-complement), `compare/eq/…`, `toString`, `BigInt(number\|string)` coercion (radix prefixes), `BigInt.asIntN/asUintN`. Operators `+ - * / %` and comparisons route to the natives; `** & \| ^ << >> ~` are method-only (those operator *tokens* are not in the lexer — a language-wide feature) |
| 5 | Symbol | partial | Foundation landed: `Symbol()` unique primitives, `Symbol.for/keyFor`, well-known symbol **values** (`iterator/asyncIterator/hasInstance/toPrimitive/toStringTag`), `.description`, symbol-keyed `obj[sym]` get/set. Remaining: behavioral dispatch of `hasInstance/toPrimitive/toStringTag` (value-only). Object-literal computed symbol keys now land (`{ [sym]: v }`); class computed symbol-key members still deferred |
| 6 | Object | partial | Enumeration/reflection surface landed (`keys/values/entries/assign/freeze/isFrozen/create/getPrototypeOf/setPrototypeOf/fromEntries/is` + instance `hasOwnProperty/isPrototypeOf`); descriptor-based statics deferred (no property-descriptor model) |
| 7 | Array | mostly | Standard callback + structural surface landed (spec `(value,index,array)` callbacks); `findLast/findLastIndex/flat(depth)/flatMap/keys/values/entries/at` (relative), statics `Array.isArray/of/from`, variadic `splice` inserts and no-initial `reduce`/`reduceRight` now landed. Zun extensions kept. Remaining: `toSorted/toReversed/with/group`, iterator-object return for `keys/values/entries`, for-of binding destructuring (front-end gap) |
| 8 | Function | partial | `call/apply/bind` + `.name/.length` landed (receiver-as-leading-param model, Tier 0); no `Function` constructor, no bound-arg function values |
| 9 | Promise | partial | Instance `state/result/isReady/resolve/reject` + ECMAScript `then/catch/finally` (new-promise + Promise adoption), statics `Promise.resolve/reject/all/allSettled/race/any`. `await` works via opcode. Unified micro+macrotask event loop with a `timers` surface (see §9a). Remaining: `withResolvers`, AggregateError for `any`, unhandled-rejection-at-turn-end reporting |
| 10 | Proxy / Reflect | mostly | Property traps `get/set/has/deleteProperty` intercept with Reflect-style target fallback; `proxy.k`/`proxy[k]` read+write route through the handler; `Reflect` realm global (`get/set/has/deleteProperty/ownKeys`). `x in obj` and `delete obj.k`/`obj[k]` operators now lex/parse and lower to the has/delete mechanism (Proxy receivers route to the `has`/`deleteProperty` traps). Deferred: descriptor-dependent traps + `apply`/`construct` |
| 11 | Map | mostly | `set/get/has/delete/clear/values/keys/entries/forEach` + `size` as a **property** (`map.size`; the `.size()` method form still resolves) + iterator + iterable constructor: array of pairs (`new Map([[k,v],…])`) plus any built-in iterable (`new Map(otherMap)`, or a Set of pair arrays) via `mapFromIterable`. Remaining: iterator-object return for `keys/values/entries`, arbitrary user `[Symbol.iterator]` sources |
| 12 | Set | mostly | `add/has/delete/clear/values/keys/entries/forEach` + `size` as a **property** (`set.size`; the `.size()` method form still resolves) + iterator + iterable constructor: array (`new Set([…])`) plus any built-in iterable (`new Set("abc")` by code point, `new Set(otherSet)`, a Map by entry pairs) via `setFromIterable`. Remaining: iterator-object return for `keys/values/entries`, arbitrary user `[Symbol.iterator]` sources |
| 13 | WeakMap | partial | `set/get/has/delete`; semantics OK (ephemeron floor). No iteration (correct) |
| 14 | WeakSet | done | `add/has/delete` over the WeakMap ephemeron floor (value-less); no iteration (correct) |
| 15 | WeakRef | done | `new WeakRef(obj)` + `deref()` over the GC weak-reference floor; cleared on collection |
| 16 | FinalizationRegistry | mostly | Spec-shaped `new FinalizationRegistry(cleanup)` / `register(target, heldValue, unregisterToken?)` / `unregister(unregisterToken)`; `unregister` keyed by unregister-token object identity (weak key); cleanup callback delivered on collection. Deferred: `heldValue` is a `u64`, not an arbitrary value |
| 17 | ArrayBuffer | partial | `new ArrayBuffer(n)`, `byteLength()`, copying `slice()` with JS clamping, and the static `isView(x)`. Missing resizable buffers (deferred) |
| 18 | DataView | partial | All element widths present, signed included (`Int8`..`Int64`, `Float32`), plus `byteLength()`/`byteOffset()`/`buffer()`. 64-bit named `getInt64`/`getUint64` since Zun has native 64-bit ints |
| 19 | TypedArrays | partial | All eleven JS kinds, generated from one master table in `zun_core` (`Int8`/`Uint8`/`Uint8Clamped`/`Int16`/`Uint16`/`Int32`/`Uint32`/`Float32`/`Float64`/`BigInt64`/`BigUint64`), each with `length()`/`byteLength()`/`get`/`set`/`subarray`/`fill`/`byteOffset`/`buffer` and `[Symbol.iterator]`/`for...of`. `Uint8ClampedArray` takes a `number` and clamps per `ToUint8Clamp`; the 64-bit pair carries native `i64`/`u64`, not BigInts. Deferred: bulk `set(source, offset)` — its name is taken by the indexed `set(index, value)`, so it needs arity overloading first — plus `slice`/`copyWithin`, functional methods, `from/of`. Note: elements above 2^53 are not writable as literals — bare numeric literals are `f64` |
| 20 | Date | partial | JS-correct `getUTCMonth`/`getUTCDay` (both were off by one), all UTC setters, `Date.UTC`, `getTimezoneOffset`, `Date.now`, `new Date()` (current time), local-time getters (== UTC, calendar is UTC-only). Missing `toLocale*`, local setters |
| 21 | Buffer | partial | Built on the `Uint8Array` byte-view floor (Buffer IS a U8 view, as in Node); realm global via `builtin/realm.zun` + `base_type_facets`. Statics `from`(string/array/ArrayBuffer)/`alloc`/`allocUnsafe`/`concat`/`isBuffer`/`byteLength`; instance `length`, indexed `buf[i]` get/set, `toString`/`slice`/`write`/`equals`/`copy`/`fill(value)` + common LE/BE 8/16/32/64 readers/writers. Encodings utf8/hex/base64/latin1. Deferred: ucs2/utf16le, base64url, 24-bit/float read*/write*, `swap16/32`, `indexOf`/`includes`, negative slice indices |
| 22 | Error (+ subclasses) | partial | `Error` + 6 subclasses with JS-shaped `new Error(message)`, per-class `name` getter, JS `toString()`. **`.stack` now captured** at construction (frame walk + sequence points; byte-span frames, not line:col). **`AggregateError(errors, message)` carries its `errors`** (standalone class, not `extends Error`). Still not a realm global; `cause` deferred (needs `any` fields + object-literal syntax) |
| 23 | globalThis / global | done | `globalThis`/`global` → `RealmGlobal` opcode (`Ty::DynamicObject`), realm singleton per §3.1 |
| 24 | Typed numeric lattice | done | `i8..i64/u8..u64/f32/f64` in `IntKind`/`Ty`; int/float TS types + coercions |
| 25 | Decorators | missing | Planned in A; not implemented (no parse/sema/lowering) |
| 26 | ZIL math opcodes | done | 15 ops at `0x96-0xA4`: float `sqrt/abs/neg/floor/ceil/trunc/min/max/round/sign`, int `min/max/clz/ctz/popcnt`. Transcendentals stay on libm per PLAN §3 |

**Counts:** done 5 · mostly 4 · partial 16 · missing 1 (of 26 nucleus items, JSON excluded). Recounted from the table above; it has to sum to 26.

(Derived from the table above, not tracked by hand. Both sides of a merge have
had this line wrong; recount the rows rather than taking either version.)

### Unary/binary operator tokens (`typeof` / `in` / `instanceof` / `delete`)

Landed as real keywords in the lexer (`typeof`/`in`/`instanceof`/`delete`), the
hand parser (prefix unary for `typeof`/`delete`; relational-precedence binary for
`in`/`instanceof`), the OXC adapter, sema, and lowering. Keyword-as-property-name
is preserved: `obj.in`, `obj.typeof`, a `delete(...)` method, etc. still parse
(class member names and `.member` access accept keywords).

- `typeof x` → for a **statically-typed** operand, a constant JS type string
  folded over its static type (`number/string/boolean/bigint/symbol/function/
  object`; arrays, maps, instances, proxies, null → `"object"`) — exact and
  unchanged. For an **`any`** operand it now lowers to the `typeofAny` intrinsic,
  which inspects the runtime value at Tier 0 (heap-refined): number/string/
  boolean/symbol/function/undefined/object, plus `bigint` (BigInt payload probed
  on the heap) and `typeof null === "object"`. The native lives in a new
  `zun:typeinfo` module appended **last** in the registry, so existing dispatch
  ids are unchanged. Functions using it stay Tier 0 (not in the JIT allowlist).
- `x in obj` → `boolean` via `reflectHas` (plain object / `any`) or the Proxy
  `has` trap (`proxyHas`). Key must be a string; symbol keys deferred.
- `delete obj.k` / `delete obj[k]` → `boolean` via `reflectDelete` / the Proxy
  `deleteProperty` trap. Always yields `true` on success (no configurable-property
  descriptor model yet). A non-member `delete` operand is a sema error.
- `x instanceof C` → folds to a constant when the static type decides it (a
  provable `Ty::Instance` subclass → `true`; a value that can never be an
  instance → `false`). Otherwise — an **`any`/dynamic** operand, or the **upcast**
  case (base-typed variable that may hold a derived instance) — it now lowers to
  the `instanceofClass` intrinsic (same new `zun:typeinfo` module), which walks
  the operand's **runtime** class hierarchy via `TypeSystem::is_assignable` over
  its `type_rid` against `C`'s metadata TypeDef rid (`class index + 1`). This
  closed the previously-unsound upcast fold (it answered `false`). **Still
  deferred (need a class↔prototype-object model that does not exist):** matching a
  plain `Object.create` dynamic object against a class (it answers `false`, never
  a false positive), and `Symbol.hasInstance` customization.
- `void x` is **deferred** (left as an identifier/type name — `void` is a common
  return-type annotation; no `undefined` value representation exists yet). The
  `for (k in obj)` statement form is also out of scope (this is only the `in`
  operator).

### Declarative realm-global registration (PLAN §3.1)

Base types are seeded onto `globalThis` from the declarative registry
(`builtin/realm.zun` → `zun_sema::realm`); the registry is the single source of
truth for each name's annotation type, descriptor value, and `new` lowering (the
frozen runtime representation is unchanged — the registry only rebinds the name).

- **Registered declaratively:** `Map`, `Set`, `WeakMap`, `WeakSet`, `WeakRef`,
  `FinalizationRegistry`,
  `Promise`, `RegExp`, `BigInt`, `Array`, `ArrayBuffer`, `DataView`,
  `Uint8Array`, `Uint16Array`, `Uint32Array`, `Float64Array`, `String`,
  `Number`, `Boolean`, `Object`, `Symbol`, `Reflect`, `Buffer`. (`Map` keeps
  token 0; `Reflect` and then `Buffer` are appended last to keep every earlier
  token stable. `Buffer` denotes the `Uint8Array` byte view — see #21.)
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

## 3. Boolean — mostly
`zun:boolean` native module (`crates/zun_natives/src/boolean.rs`) + sema intercepts.
`Boolean` stays a realm global (`builtin/realm.zun`, `FallThrough`), so
`globalThis.Boolean` resolves.

- **Present:**
  - `Boolean(x)` coercion — returns the JS truthiness of `x` as a `boolean`
    primitive (not a wrapper object). Intercepted in sema's plain-`Call` path
    (next to `BigInt(x)`); a value already typed `boolean` is returned as-is, any
    other type routes to the `booleanCoerce` native. That native mirrors the
    interpreter's `is_truthy` (used for `if`/`&&` on dynamic callback results):
    `false`, `0`/`NaN`, `""`, `null`/`undefined` falsy; objects, functions and
    non-empty strings truthy. Concrete `string`/`number` arrive marshalled by
    value (so are classified exactly).
  - `boolean.toString()` — `true`→`"true"`, `false`→`"false"`, a sema intercept on
    a `Ty::Boolean` receiver (like `number.toString(radix)`), via the
    `booleanToString` native.
- **Deferred:** the wrapper OBJECT `new Boolean(x)` (a boxed `Boolean` object) —
  rarely used; the primitive-returning `Boolean(x)` call is the important surface.
  No `builtin/boolean.zun` class, no `.valueOf()` object surface.
- **Known limitation:** a *boxed* string inside an `Any` (value whose static type
  is `any`) is treated as truthy — the native sees only the string handle and
  cannot resolve the backing text to test emptiness. Concrete `string` is exact.

## 4. BigInt — mostly
`builtin/bigint.zun` (extern) + native (`collections.rs`, `zun_core/bigint.rs`).

- **Present:**
  - Arithmetic: `add sub mul div rem pow neg` (`div`/`rem` truncate toward zero,
    remainder takes the dividend's sign; `pow` rejects a negative exponent;
    division by zero is a runtime RangeError-style error).
  - Bitwise/shift: `and or xor not shl shr`, implemented with correct infinite
    two's-complement semantics for negative operands (`~5n == -6n`, `-8n >> 1n ==
    -4n`, arithmetic/floor right shift).
  - Comparison: `compare eq ne lt le gt ge`, `toString`.
  - `BigInt(value)` coercion: from a `number` (integer-valued → bigint;
    non-integer/non-finite → runtime error) and from a `string` (decimal plus the
    `0x`/`0o`/`0b` radix prefixes). Works both as `BigInt(x)` and `new BigInt(x)`.
  - `BigInt.asIntN(bits, x)` / `BigInt.asUintN(bits, x)`.
  - Literal `1n` lowering verified (→ `bigIntNew` string parse; radix prefixes
    accepted).
  - Operator integration: `+ - * / %` and `== != < <= > >=` on two `Ty::BigInt`
    operands route to the natives in sema. BigInt mixed with Number in an operator
    does not type-check (TypeError-equivalent), per spec.
- **Deferred / method-only:** the operator *tokens* `** & | ^ << >> ~` are not in
  the lexer/parser at all (a language-wide operator feature affecting int types
  too, outside BigInt's scope), so pow/bitwise/shift are reached through the
  instance methods (`a.pow(b)`, `a.and(b)`, `a.shl(b)`, `a.not()`, …). Adding
  those tokens is a separate language decision. Also deferred: `valueOf`.

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
  findLast(cb) findLastIndex(cb) some(cb) every(cb) reduce(cb[,init])
  reduceRight(cb[,init]) sort(cmp?) join(sep) shift() unshift(v)
  splice(start,deleteCount,...items) slice(start,end) concat(other) fill(value)
  reverse copyWithin(t,s,e) flat(depth=1) flatMap(cb) keys() values() entries()
  indexOf lastIndexOf includes at(i) length` + `[Symbol.iterator]`; statics
  `Array.isArray(x) Array.of(...) Array.from(arrayLike[,mapFn])`. Every callback
  receives the ECMAScript `(value, index, array)` signature (comparator `(a, b)`);
  surplus arguments are dropped to the callback's declared arity (interp
  `invoke_callback` truncates to the callee `param_count`). `sort()` with no
  comparator is ascending **string** order (spec default). `at(i)` does relative
  indexing (negative counts from the end) and yields `undefined` out of range; a
  no-initial `reduce`/`reduceRight` seeds from the first/last element and throws a
  TypeError on an empty array. The NUCLEUS #7 extension surface lives in a
  dedicated `array` native module (`zun_natives/src/array_ext.rs`, appended last
  so every existing dispatch id stays stable; implementations in
  `zun_interp/src/collections.rs`).
- **Zun extensions (kept, beyond ECMAScript):** `chunk windowed rotateLeft/Right
  union intersection difference symmetricDifference isSubsetOf isSupersetOf
  isPalindrome isSorted isUnique sum product average min max transpose sortNumeric
  take drop insertAt removeAt removeFirst removeAll swap count contains …` — the
  "collection algebra" surface in `builtin/array.zun`. (`sort`'s former numeric
  default is preserved as the extension `sortNumeric()`.)
- **Missing (standard):** `toSorted toReversed toSpliced with group groupToMap`;
  `fill`/`copyWithin` range refinements. `keys()`/`values()`/`entries()` return a
  fresh managed **Array** (iterable via `for...of` and indexable), not a distinct
  iterator object — pragmatic for v1; a proper iterator-protocol object is a
  follow-up. `Array.from` consumes an array or string (and anything with array
  storage); arbitrary iterator-protocol sources are deferred (documented). **Note:
  `for (const [i, v] of arr.entries())` is blocked by a separate front-end gap —
  for-of binds a single identifier, not a destructuring pattern** (ast/parser/oxc
  `ForOf.name` is a `Name`); `entries()` itself works (iterate with one binding and
  index the pair). That loop-binding destructuring is orthogonal to the Array
  surface and left for a front-end change.
- **Notes:** these callback methods are Tier 0 (they re-enter the interpreter via
  `self.call`, like proxy traps / `functionApply`); `CallIndirect` is not
  JIT-lowered, so there is no new JIT path. The non-callback additions (`at`,
  `flat`, `keys`/`values`/`entries`, `splice` inserts, `isArray`/`of`/`from`) are
  managed `CallBuiltin` natives that are not in the codegen eligibility allowlist,
  so a function using them stays Tier 0 (interp equivalence holds trivially).
  `Array.of(...)` and `splice`'s variadic inserts lower through an array literal in
  sema (no variadic native needed); `Array.*` statics are recognized by the
  receiver name `Array` for `isArray`/`of`/`from` (the Number/String precedent).
  `indexOf`/`lastIndexOf`/`includes` stay value-based (spec) and `indexOf` returns
  `int`.

### 7b. Array literals — partial  *(expression syntax landed)*
`[ e1, e2, ... ]` now parses and lowers in the front-end (both the hand parser —
`Parser::array_literal`, dispatched from `primary()` on `[` in expression position — and
the OXC adapter — `Expression::ArrayExpression`). An array literal produces a managed
`Array` (`Ty::Array`): `Expr::ArrayLiteral` → `hir::ExprKind::ArrayLiteral` → `ConstI32 N`
+ `NewArray`, then for each element a `Dup` + `ConstI32 i` + element + `StoreArray` + `Pop`
so the array stays on the stack as the result. This parallels the object-literal work
(§6b); it reuses the existing array runtime (no new opcodes).
- **Present (v1):** positional elements of any element type; `StoreArray` boxes each value
  to `Any` at the store boundary (the dynamic array element model — elements read back out
  as `Any`, exactly like `new Array(n)` + `a[i] = v`), so no explicit `BoxAny` is inserted.
  Empty `[]`; nested literals; a single **trailing comma** (`[1, 2, 3,]`). Elements are set
  in source index order. Composes with the standard Array natives (`.map`, `.sort`, …) and
  with the `Set`/`Map` array-iterable constructors (`new Set([…])`, `new Map([[k, v], …])`).
- **Deferred (documented follow-ups — the OXC adapter errors rather than miscompiling, the
  hand parser reports them as unsupported rather than silently dropping them):** elision /
  **holes** `[1, , 3]` and **spread** `[...xs]`. Both need element kinds the v1 lowering
  does not model (a hole is a distinct "no element at this index" vs. `undefined`, and
  spread needs iteration/concatenation at the literal site).
- **Note on nested access:** because elements are `Any`, a direct `m[i][j]` chain needs the
  middle value cast back to `Array` (`(m[i] as Array)[j]`), consistent with every other
  array read — this is the element-typing model, not an array-literal limitation.

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

## 9. Promise — partial  *(method + static surface landed)*
`std/` (none) — dispatched in `check.rs`; natives registered in `zun_natives/src/promise.rs`,
implemented on the Tier-0 floor in `zun_interp/src/collections.rs`; `await` opcode `0x8D`.

- **Present (low-level cell):** `state() result() isReady() resolve(v) reject(msg)` +
  `await` integration.
- **Present (chaining, new):**
  - `then(onFulfilled?, onRejected?)` — returns a NEW Promise settled from the matching
    callback's result; a callback that throws rejects the new Promise; an absent callback
    propagates the original settlement (pass-through value / forwarded rejection). A
    callback returning a Promise is **adopted** (chained, its settlement mirrored), not
    wrapped — the standard thenable-adoption rule restricted to the Promise case (arbitrary
    `then`-able assimilation is deferred).
  - `catch(onRejected)` ≡ `then(undefined, onRejected)` (lowers to `promiseCatch`).
  - `finally(onFinally)` — runs `onFinally` on settlement and passes the original
    settlement through, unless `onFinally` itself throws (then the new Promise rejects).
- **Present (statics `Promise.*`):** `resolve(v)` (a Promise argument is returned
  unchanged; otherwise a resolved Promise), `reject(e)`, `all` (array of results, rejects
  on first rejection, in order), `allSettled` (array of `{status, value|reason}` dynamic
  descriptors), `race` (first already-settled input wins), `any` (first fulfilment, else a
  plain rejection — see AggregateError note).
- **Recognition (sema, `check.rs`):** `Promise.X(...)` statics follow the
  `check_number_static`/`check_string_static` pattern — a dedicated `check_promise_static`
  keyed on the `Promise` receiver name (guarded by `!classes_by_name.contains_key`).
  Instance `then/catch/finally` are a pre-block on a `Ty::Promise` receiver (like the
  Array `sort/reduce` pre-block), since they take optional function callbacks; an omitted
  callback lowers to `null`, which the floor reads as "no handler".
- **Scheduler reuse:** no second scheduler. Reactions run against the source Promise's
  current settlement and settle the returned Promise cell, so an already-settled chain is
  itself settled and `await` takes its synchronous fast path. Callbacks re-enter the
  interpreter via `invoke_callback` (like array callbacks / `functionApply`), rooted across
  the re-entry with `NativeRootScope`; not JIT-eligible (Tier 0 only).
- **Unified event loop (ASYNC #4, PLAN §5.1):** a plain `run()` now drives a real
  micro+macrotask loop in `zun_interp` (`drive_to_completion`/`run_event_loop`): microtasks
  (Promise reactions / `await` continuations) drain to empty, then the single earliest-due
  **timer** fires as one macrotask on the scheduler's `TaskId` tier, then microtasks drain
  again, until no microtask and no pending timer remain. Reactions on a *pending* source are
  driven here (slice 3 retired eager-at-settle); the only remaining suspend-to-host path is
  the `ResumableSession` poll model, which also fires timers when microtasks go idle.
- **AggregateError note:** `Promise.any` with all-rejected inputs still rejects with a
  plain error message (`"All promises were rejected"`). `AggregateError` itself now
  exists and carries an `errors` array (see §22), but `Promise`'s reject reason is stored
  as a *string*, not a managed object, so wiring the aggregate in requires widening the
  reason cell to hold an object first — that is the remaining work, not AggregateError.
- **`reject` reason** is still a `string` (the managed cell stores a string reason); a boxed
  `any` reason is unwrapped to its string before storage.
- **Tests:** `zun_vm/tests/promise_surface.rs` (end-to-end cases exercising the scheduler
  via `await`: then-chaining, adoption, rejection skip/propagation, catch, finally
  pass-through, all/allSettled/race/any, resolve-of-a-Promise, pending-source reactions).

## 9a. Timers — unified loop (ASYNC #4)
Timers are a **core capability** (the VM owns the event loop), not a library: the macrotask
side of the same loop that runs Promise microtasks. Native signatures in
`zun_natives/src/timers.rs` (`timers` namespace, appended **last** so dispatch ids stay
stable); implemented on the Tier-0 floor in `zun_interp/src/collections.rs`; resolved as bare
global calls in `zun_sema/src/check.rs` (like `print`, shadowable by a user symbol).
- **Surface:** `setTimeout(cb, ms?)` / `setInterval(cb, ms?)` → numeric handle;
  `clearTimeout(h)` / `clearInterval(h)`; `queueMicrotask(cb)`; `setImmediate(cb)` → handle.
- **Timer heap** (`AsyncState::timers`): a `Vec`-backed min-heap keyed by due time (FIFO
  tiebreak by arm order). A due timer feeds the scheduler's `TaskId` macrotask tier
  (`schedule_task` → consumed via `scheduler.next()`), so the scheduler stays the ordering
  authority (continuations drain before any task).
- **Virtual time:** the clock is seeded from the real `nowMillis` once, then **fast-forwarded**
  to the earliest due time between macrotasks — no wall-clock sleeping, so timed programs
  terminate deterministically.
- **Ordering guarantee:** `setTimeout(cb, 0)` fires after every currently-queued microtask
  (macrotask < microtask drain), proven in tests by a `queueMicrotask`/`.then` running first.
- **Cancellation:** `clear*` cancels via the scheduler's task-queue cancellation
  (`Queues::cancel_task`) and drops the pending entry; intervals re-arm until cleared.
- **GC:** a pending timer's callback (and any forwarded args) is a heap root held by the timer
  heap, rooted in `append_parked_roots` and rewritten in `AsyncState::rewrite_after_move`,
  exactly like the reaction / parked-frame tables.
- **Tests:** `zun_vm/tests/timers_surface.rs` (9 end-to-end cases) + a rooting unit test in
  `zun_interp/src/state.rs` and task-cancellation tests in `zun_sched/src/queue.rs`.

## 10. Proxy / Reflect — mostly
- **Present:** `new Proxy(target,handler)` → `proxyNew`; dynamic
  `reflect_get`/`reflect_set` free functions recognized.
- **Proxy traps that intercept (`zun_interp/src/collections.rs` `proxyGet`/`proxySet`/
  `proxyHas`/`proxyDelete`):** `get(target,key)`, `set(target,key,value)`,
  `has(target,key)`, `deleteProperty(target,key)`. Each looks up the named trap on the
  handler (`heap.proxy_trap`); when the trap is **absent** it falls back to the target's
  default dynamic-property behavior (Reflect-style). Trap callbacks are invoked with the
  receiver-as-leading-param model (handler environment, then `target`, `key`[, `value`]).
- **Property-access syntax now routes through the handler** (`zun_sema/src/check.rs`):
  a `Ty::Proxy` receiver lowers `proxy.k` / `proxy[k]` reads to `proxyGet` and
  `proxy.k = v` / `proxy[k] = v` writes to `proxySet` (`proxy_get`/`proxy_set` helpers).
  The key must be a `string`.
- **`Reflect` realm global** — registered declaratively like `Object`
  (`builtin/realm.zun` + `zun_sema/src/realm.rs` `base_type_facets`, `FallThrough`,
  `Ty::DynamicObject`), so `globalThis.Reflect` resolves to its class descriptor.
  `Reflect.<m>(...)` statics are recognized by receiver name (`check_reflect_static`,
  dispatched next to `Object`): `get(target,key)`→`LoadDynamicValue`,
  `set(target,key,value)`→`StoreDynamicValue` (value boxed to `Any`),
  `has(target,key)`→`reflectHas`, `deleteProperty(target,key)`→`reflectDelete`,
  `ownKeys(target)`→`objectKeys`. These are the default operations the Proxy traps fall
  back to. No new natives were added (existing `proxy*`/`reflect*`/`objectKeys`
  intrinsics reused), so native dispatch ids are unchanged.
- **Deferred (documented, not faked):** the descriptor-dependent traps/statics
  (`getOwnPropertyDescriptor`/`defineProperty`/`getPrototypeOf`/`setPrototypeOf`/
  `isExtensible`/`preventExtensions`) — they need the property-descriptor model still
  deferred (NUCLEUS §6) — and the `apply`/`construct` traps. The `has`/`deleteProperty`
  traps are reachable via `Reflect.has`/`Reflect.deleteProperty` and the `proxyHas`/
  `proxyDelete` natives, but **not** via the `in`/`delete` operators: those operator
  tokens are not in the lexer (a language-wide feature, like BigInt's `** & | ^`).

## 11. Map — mostly
`std/map.zun` → `zun:collections` (`collections.rs`, `ordered_map.rs`). Ordered hash
table. Iterator intrinsics present (`mapIterator*`).
- **Present:** `set get has delete clear values keys entries` + `size` as a **property**
  (`map.size`) + `[Symbol.iterator]` + `forEach(cb)` (`callback(value, key, map)`,
  insertion order; routed to the `mapForEach` native via the sema managed-method path,
  reusing the Array callback re-entry/rooting machinery) + iterable constructor
  `new Map(iterable)`.
- **`size` as property:** `.size` is intercepted on a `Map` receiver in the sema `Field`
  arm (like `.length` on views/strings) and lowered to the `mapSize` native → `u64`,
  ahead of class dispatch. The `map.size()` method form still resolves via the method
  table (kept as trivial backward-compat).
- **Iterable constructor:** an ARRAY of `[key, value]` pairs keeps the dedicated `mapFrom`
  fast path; any other built-in iterable (another Map — its entries; a Set of pair
  arrays; an array for `any`-typed args) routes to `mapFromIterable`, which dispatches on
  the runtime kind. A string is rejected (its elements are not pairs), matching JS.
- **Missing / deferred:** iterator-object return for `keys/values/entries`; draining an
  arbitrary user `[Symbol.iterator]` object (would need reentrant iterator-protocol calls
  from a native).

## 12. Set — mostly
`std/set.zun` → `zun:collections`.
- **Present:** `add has delete clear values keys(→values) entries` + `size` as a
  **property** (`set.size`) + iterator + `forEach(cb)` (`callback(value, value, set)` —
  the value twice per spec — insertion order; routed to the `setForEach` native, reusing
  the Array callback machinery) + iterable constructor `new Set(iterable)`.
- **`size` as property:** same mechanism as Map — intercepted on a `Set` receiver in the
  `Field` arm, lowered to `setSize` → `u64`; `set.size()` still resolves.
- **Iterable constructor:** an ARRAY keeps the `setFrom` fast path; any other built-in
  iterable routes to `setFromIterable`, which dispatches on runtime kind: a string
  (iterated by Unicode **code point** — `new Set("abc")` → 3 elements), another Set (its
  elements), another Map (its `[key, value]` entry pairs).
- **Missing / deferred:** ES2024 set ops (`union/intersection/difference/isSubsetOf/…`) —
  note Array has these as extras but Set does not; iterator-object return for
  `keys/values/entries`; draining an arbitrary user `[Symbol.iterator]` object.

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

## 16. FinalizationRegistry — mostly
`builtin/finalization_registry.zun` → GC finalization queue.
- **Present (spec-shaped):** `new FinalizationRegistry(cleanup)`,
  `register(target, heldValue, unregisterToken?)`, `unregister(unregisterToken)`.
  `unregister` is now keyed by the **unregister-token object identity** (the same
  address-identity mechanism WeakMap/WeakRef use for weak keys), not by a numeric
  id. It returns `true` when at least one registration made with that token object
  was removed, `false` otherwise (including an unknown token). A registration made
  **without** an unregister token can never be removed, per spec; the checker
  synthesizes a `null` sentinel for the omitted slot and the runtime stores a `0`
  token address that never matches. The unregister token is held **weakly** (its
  address is rewritten on compaction but it is not kept alive).
- **Cleanup callback — wired.** The callback passed to `new
  FinalizationRegistry(cleanup)` is stored per-registry (keyed by the registry
  object address) and copied into each registration. When the GC sweeps a
  registered target, its registration is pushed onto the finalization queue and
  `Interp::deliver_finalization_callbacks` invokes the cleanup callback with the
  held value at the next safepoint. Callback environments are strong GC roots (both
  the per-registry callback and the per-registration copy); targets and unregister
  tokens are weak.
- **Scoping note:** `heldValue` is currently a `u64` (it reuses the existing
  `token: u64` held-value slot threaded through the GC `FinalizerEntry` /
  `FinalizationRecord`); arbitrary-value held values would require widening that
  slot to a boxed `Value` and are deferred. The vestigial `builtin/*.zun` class
  mirrors the surface so the module type-checks but is not the dispatch path — the
  checker lowers `FinalizationRegistry` methods directly against the builtin type.

## 17. ArrayBuffer — partial
`Ty::Buffer` is what the realm registers under the name `ArrayBuffer`; the byte
storage lives in `zun_gc/buffers.rs`.

- **Present:** `new ArrayBuffer(length)`, `byteLength()`, `slice(begin, end)`,
  and the static `ArrayBuffer.isView(x)`.
  `slice` copies rather than aliasing, as JS does, and clamps inverted or
  out-of-range bounds to an empty result instead of failing.
- **`ArrayBuffer.isView(x)`** — `true` for a `DataView` or any typed array,
  `false` for everything else (including a plain `ArrayBuffer`, a non-view
  object, and any non-object value). The argument is accepted as an opaque
  value and a non-view answers `false` rather than erroring. Backed by
  `Heap::is_view`, which checks the view registries; recognized as a static on
  the receiver name `ArrayBuffer`.
- **Methods, not properties.** JS has `buffer.byteLength`; this is
  `buffer.byteLength()`, consistent with `String.length()` and `Map.size()` here.
  The property-vs-method divergence is repo-wide and listed under Surprising
  findings rather than patched per type.
- **Missing:** resizable buffers (`resize`/`maxByteLength`),
  `SharedArrayBuffer` (deferred, fine). Resizable buffers stay deferred: a
  `resize` would have to re-validate every outstanding view's window against the
  new length, which the view registries do not track back-references for yet.

## 18. DataView — partial
`BufferViewKind::DataView`; core accessors in `zun_core/views.rs`.

- **Present:** `getInt8/setInt8`, `getUint8/setUint8`, `getInt16/setInt16`,
  `getUint16/setUint16`, `getInt32/setInt32`, `getUint32/setUint32`,
  `getInt64/setInt64`, `getUint64/setUint64`, `getFloat32/setFloat32`,
  `getFloat64/setFloat64`, each with the little-endian flag (single bytes take
  none), plus `byteLength()`, `byteOffset()` and `buffer()`.
- **`byteOffset()` / `buffer()`** expose the view's byte offset into its
  backing buffer and a reference to that `ArrayBuffer`. The offset is the
  stored `DataView`'s `offset()`; the buffer handle is the one saved in the
  view object's field 0 at construction (`Heap::data_view_byte_offset` /
  `data_view_buffer`). Methods, not properties, like the rest of the surface.
- **Signed access was missing entirely** before this, so a binary format with
  signed fields could not be read: the same byte is `-1` as `Int8` and `255` as
  `Uint8`, and only the unsigned reading existed.
- **64-bit naming:** JS calls these `getBigInt64`/`getBigUint64` and returns a
  BigInt. Zun has native 64-bit integers, so they are `getInt64`/`getUint64`
  returning `i64`/`u64`. A deliberate divergence, not an oversight.
- **Missing:** nothing of the core surface now; `Uint8Clamped` has no DataView
  form in JS either.

## 19. TypedArrays — partial
All eleven JS kinds over a shared `ArrayBuffer`, generated from the master
table in `zun_core/typed_elements.rs`: `Int8Array`, `Uint8Array`,
`Uint8ClampedArray`, `Int16Array`, `Uint16Array`, `Int32Array`, `Uint32Array`,
`Float32Array`, `Float64Array`, `BigInt64Array`, `BigUint64Array`.

- **The constructor's third argument counted bytes, where JS counts
  elements.** `new Uint32Array(b, 0, 8)` gave a two-element view in Zun and an
  eight-element one in JS, so ported code read and wrote past what it believed
  it had. Fixed: the length is an element count, and `byteLength()` is that
  times the element width.
- **`length()` and `byteLength()` were unreachable.** The natives existed for
  every kind; only `get`/`set` were in the checker's method table, so a view
  could be read but not measured.
- **Present:** `get`, `set`, `length`, `byteLength`, `subarray`, `fill`,
  `byteOffset`, `buffer`, and `[Symbol.iterator]` / `for...of` on every kind.
  All generated from the master table, so a kind cannot be annotatable without
  having the full method surface; `zun_vm/tests/typed_array_kinds.rs` enumerates
  the kinds and fails if one is half-wired.
- **`byteOffset` / `buffer`** mirror the DataView ones: the view's byte offset
  into its backing buffer and a reference to that `ArrayBuffer`. Kind-independent
  — a view carries its own byte offset and buffer handle regardless of element
  width — so both go through the master table and every kind gets them at once
  (`Heap::typed_array_byte_offset` / `typed_array_buffer`).
- **Iteration** builds an element sequence and reuses the array iterator, the
  same shape the string iterator uses: `<kind>IteratorNew` reads every element
  into a fresh array, then returns an array iterator whose `next`/`done` are the
  `arrayIterator*` intrinsics. `for (let x of someUint8Array)` and
  `someUint8Array[Symbol.iterator]()` both route here; `DataView` is not
  iterable and reports so. The element read happens before any allocation, so
  the view handle is never held across a move.
- **Deferred:** the bulk `set(source, offset)` — its name collides with the
  indexed `set(index, value)`, so it needs arity/overload resolution first;
  `slice` (copying), `copyWithin`, the functional methods, and the statics
  `TypedArray.from/of`. `BigInt64Array`/`BigUint64Array` carry native
  `i64`/`u64` rather than real BigInts, the same divergence as
  `DataView.getInt64` (#18).

## 20. Date — partial
`builtin/date.zun` over the `zun:date`/`zun:time` intrinsics, reachable as a
global through the prelude.

- **Two JS-named getters were returning wrong answers**, not missing ones.
  `getUTCMonth()` gave the 1-based calendar month where JS is 0-based, and
  `getUTCDay()` gave the ISO weekday (1 = Monday … 7 = Sunday) where JS is
  0-based from Sunday. For 2021-01-03, a Sunday, JS says month 0 and day 0; Zun
  said 1 and 7. Ported code computed the wrong date silently. Both now follow
  JS; the Zun-named `utcMonth()`/`utcWeekday()` keep the 1-based and ISO
  conventions.
- **Setters:** `setUTCFullYear/Month/Date/Hours/Minutes/Seconds/Milliseconds`,
  each returning the new timestamp as JS does. Out-of-range components roll
  over — month 12 is January of the next year, day 32 of January is 1 February,
  hour 25 is 1am the next day — which is JS behaviour and falls out of composing
  in absolute units rather than validating.
- **`Date.UTC(year, month, …)`** with a 0-based month, over a new
  `DateValue::from_utc` and the `date.fromUTC` native. That composition
  primitive did not exist: components could be read but never written, so no
  setter was possible before it.
- **`getTimezoneOffset()`** returns 0. With a UTC-only calendar local time *is*
  UTC, so this is accurate rather than a stub.
- **`new Date()` (no-arg) = current time.** The `millis` field carries a default
  initializer `millis = time.nowMillis()`; when no constructor argument is
  supplied the object is built from that initializer, exactly the
  trailing-default-initializer mechanism `Error.stackFrames` uses. `new Date(0)`
  still binds the epoch and `new Date(millis)` is unchanged — no overload
  dispatch was needed. `new Date(string)` is still absent (use `Date.fromISO`);
  a string positional would not type-check against the `i64` field.
- **Local-time getters** `getFullYear/getMonth/getDate/getDay/getHours/
  getMinutes/getSeconds/getMilliseconds` now exist. **The VM exposes no platform
  timezone offset** (no tz intrinsic; `getTimezoneOffset` is a hardcoded 0), so
  local time *is* UTC and each local getter delegates to its UTC-named
  counterpart (0-based month, 0-based-from-Sunday weekday). This is accurate for
  a UTC-only calendar, not a fake offset; the local getters diverge from the UTC
  getters only once a timezone intrinsic lands.
- **Present:** millis storage, `getTime/setTime valueOf isValid
  isBefore/isAfter/isEqual`, the UTC getters and UTC setters, the local getters,
  `toISOString toJSON toString`, `getTimezoneOffset`, `new Date()`/`new
  Date(millis)`, and the statics `now/parse/UTC/fromMillis/fromISO/fromISODate`.
- **Missing / deferred:** `new Date(string)` (use `Date.fromISO`); local-time
  *setters* (`setFullYear` etc. — would be identical to the UTC setters while
  local == UTC, deferred to avoid duplicating the surface until a tz offset
  exists); `toDateString toTimeString toUTCString`; `toLocale*` (locale/ICU).

## 21. Buffer (Node) — partial  *(built on the Uint8Array byte-view floor)*

Node's `Buffer` **is** a `Uint8Array` subclass, so Zun's `Buffer` is a nominal
alias for the `U8` typed-array view (`Ty::BufferView(BufferViewKind::U8)`) — it
owns no storage of its own and reuses the existing byte-view floor (#17/#19).
There is no parallel `builtin/buffer.zun` class.

- **Storage + realm registration.** A Buffer is a `U8` view over a fresh
  `ArrayBuffer` (`heap.alloc_buffer` + `heap.alloc_typed_array`). `Buffer` is a
  declarative realm global (`builtin/realm.zun` `global.set("Buffer", Buffer,
  true)` + a `base_type_facets` entry mapping it to `Ty::BufferView(U8)` with a
  `FallThrough` ctor), appended last so every earlier descriptor token stays
  stable. `globalThis.Buffer` resolves and `Buffer` works as an annotation; the
  statics are recognized by receiver name (like `ArrayBuffer.isView`), so there
  is no `new Buffer()` (deprecated in Node anyway).
- **Natives.** A new `zun:core` namespace `nodeBuffer`
  (`zun_natives/src/node_buffer.rs`), appended **last** in the `MODULES`
  registry so no existing dispatch id shifts. Implementations live on the Tier-0
  interpreter floor (`zun_interp/src/node_buffer.rs`), routed by name through
  `managed_node_buffer_builtin`. Not JIT-eligible (they allocate/throw), so no
  second path to keep in sync.
- **Statics (present):** `Buffer.from(string, encoding?)`, `from(array)`,
  `from(arrayBuffer)` (overload resolved on the argument's static type),
  `alloc(size, fill?)` (zero-filled), `allocUnsafe(size)` (zero-filled here),
  `concat(list)`, `isBuffer(x)` (true only for a `U8` view), `byteLength(string,
  encoding?)`.
- **Instance (present):** `length` (property), indexed byte get/set `buf[i]`
  (added generically for all buffer views), `toString(encoding?, start?, end?)`,
  `slice(start?, end?)` (shares memory, via `typed_array_subarray`),
  `write(string, offset?, encoding?)`, `fill(value)` (single-arg, whole buffer),
  `equals(other)`, `copy(target, targetStart?, sourceStart?, sourceEnd?)`, and
  the common fixed-width readers/writers `readUInt8/readInt8`,
  `read{U,}Int16LE/BE`, `read{U,}Int32LE/BE`, `readBig{U,}Int64LE/BE` and the
  matching `write*` (all assembled from single-byte view access).
- **`subarray`/`fill(v,begin,end)`/`get`/`set`** keep their typed-array
  semantics (generic dispatch) — a Buffer shares its type with a plain
  `Uint8Array`, so only the Buffer-unique method shapes are intercepted; the
  typed-array surface is not regressed.
- **Encodings (present):** `utf8`/`utf-8`, `hex`, `base64` (standard alphabet),
  `latin1`/`binary`. Unknown encodings fall back to utf8 (lenient).
- **Deferred (documented, not faked):** `ucs2`/`utf16le` and `base64url`
  encodings; the long tail of readers/writers (24-bit `read{U,}IntLE/BE`,
  `readFloatLE/BE`, `readDoubleLE/BE`), `swap16/swap32`, `indexOf`/`includes`,
  negative `slice`/`copy` indices (clamped, not counted from the end). Because a
  Buffer and a plain `Uint8Array` are the same type here, `isBuffer` answers
  `true` for any `Uint8Array` and the Buffer instance methods are reachable on a
  bare `Uint8Array` too — the one visible consequence of the shared
  representation.

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
- **`.stack` is captured at construction** (`zun:error` intrinsic
  `captureStackTrace`, serviced on the interpreter's managed floor —
  `zun_interp::collections` / `builtins.rs`, alongside the collection and
  `zun:typeinfo` intrinsics). It is a *trailing field with a default
  initializer* on `Error` (`stackFrames: string = captureStackTrace()`), so it
  stays optional (`new Error(msg)` is unchanged) and is evaluated at the
  construction site, where the live frame chain is exactly the methods that led
  to the `new`. The public `get stack()` renders the `name: message` header on
  top of the captured frames, so the header reflects the concrete subclass.
  - **Capture mechanism:** the interpreter keeps a `call_stack: Vec<CallFrame>`
    (method rid + pc), pushed/popped in `Interp::call` and advanced per
    instruction — a frame carries only two `u32`s, so no GC rooting. At capture
    the chain is walked innermost-first; each frame resolves its method name
    from the `MethodDef` table and its source span from the `.zm` sequence
    points.
  - **Format:** first line `<name>: <message>`, then one
    `    at <fn> (<start>..<end>)` per frame, e.g.
    `Error: deep` / `    at makeIt (1360..1379)` / `    at main (3722..3730)`.
  - **Fidelity/limits:** sequence points record **byte spans** into the source,
    and the VM holds no source text, so frames render byte offsets, not
    `file:line:col`. Only frames of the current module are walked (a call into
    another loaded module runs on its own interpreter instance). Async-resumed
    frames may be incomplete (a resumed activation re-enters `call` fresh).
    `captureStackTrace` itself is a `call.builtin` with no activation, so it
    never appears in its own trace.
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
- **`AggregateError(errors, message)` now lands**, carrying its `errors` array.
  It is declared **standalone, not `extends Error`**: construction fills fields
  inherited-first, so an `extends Error` subclass would force `message` ahead of
  `errors` and could not express JS's `(errors, message)` order. Declared on its
  own, the field order (`errors`, `message`, then the optional `stackFrames`) is
  ours to choose, so the JS constructor shape and `.stack` both work. The cost is
  that `aggregate instanceof Error` is `false`, and the shared surface
  (`name`/`toString`/`get stack`) is duplicated rather than inherited. Reachable
  by name through the prelude like the other error classes.
- **`cause` is still deferred**, on two independent blockers, not effort: (1)
  `new Error(message, { cause })` needs an **options object literal**, and object
  literals are not in the AST; (2) a `cause` field would be typed `any`, which the
  construction/GC path still stores as zero (the documented `any`-field gap below).
  Either alone blocks it; both must land first.
- **`Error.captureStackTrace` / `stackTraceLimit`** are V8 extensions rather than
  ECMAScript; worth a deliberate decision before implementing.
- **Tests:** `zun_vm/tests/error_surface.rs` runs the shipped `builtin/error.zun`
  source itself rather than a copy — now including `.stack` (header + frame line,
  subclass header, nested innermost-first ordering, thrown-and-caught) and
  `AggregateError` (`errors` round-trip + shape).

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
   - Map/Set (#11,#12): ✅ `forEach`, array-iterable construction, `size` as a property,
     and general built-in-iterable construction (string/Set/Map via
     `setFromIterable`/`mapFromIterable`) landed. Remaining: iterator-object return for
     `keys/values/entries`; draining arbitrary user `[Symbol.iterator]` objects.
   - Promise (#9): ✅ `then/catch/finally` (new-promise + Promise adoption) and statics
     `resolve/reject/all/allSettled/race/any` landed. Remaining: `withResolvers`,
     AggregateError for `any`, microtask deferral of a pending-source reaction.
   - TypedArrays/DataView (#18,#19): ✅ all eleven kinds, `get/set/length/
     byteLength/subarray/fill/byteOffset/buffer` and `[Symbol.iterator]`; DataView
     every width plus `byteOffset/buffer` — landed. Remaining (deferred): bulk
     `set(source, offset)` (needs overload resolution), `slice/copyWithin`, the
     functional methods, `TypedArray.from/of`, real BigInt elements.
   - BigInt (#4): `div/rem/pow/neg`, bitwise/shift, `BigInt()` coercion, `asIntN/asUintN`.
   - Error (#22): ✅ `.stack` capture and `AggregateError(errors, message)` landed.
     Remaining: `cause` (blocked on object-literal syntax + `any` fields), realm
     global, `aggregate instanceof Error`, `Promise.any` aggregate wiring.
   - ArrayBuffer (#17): ✅ `byteLength slice isView` landed. Remaining (deferred):
     resizable buffers (`resize/maxByteLength`), `SharedArrayBuffer`.
   - Proxy/Reflect (#10): full trap set + `Reflect` global.
7. **Boolean wrapper object** (#3) — low: `Boolean(x)` coercion + `boolean.toString()`
   landed; only the boxed `new Boolean(x)` object remains deferred (primitive suffices).

### Infra (separate from the class surface)
- ~~**ZIL math opcodes** (#26)~~ — done: fifteen ops at `0x96-0xA4` (ten float, five
  integer), transcendentals left on libm. See §26.

---

- **Fixed 2026-10-23: unsigned literals above `i32::MAX` saturated.** Lowering
  built every integer constant of 32 bits or less with `f64 as i32`, which
  saturates in Rust, so `let a: u32 = 4294967295` silently held 2147483647.
  Found while testing `DataView.setUint32`. Now lowered through `i64` so the
  bit pattern survives for the following `conv` to reinterpret.

## Language gaps blocking the method-surface tail

These are not per-class work; they are the common causes behind several
"missing" entries above.

- **`any`-typed class fields silently store zero.** `encode_field` maps `Any` to
  `JitKind::Other => 0`, so `class H { v: any; }` with `new H(42)` reads back as
  `Void`, and `new H(null)` does not compare equal to `null`. Making it work needs
  boxing on store, unboxing on load, and the GC tracing the slot — so it touches
  collector correctness and belongs in a `GC:` commit. This still blocks
  `Error.cause` (#22), which wants an `any` field. `AggregateError.errors` (#22)
  sidesteps it by typing the field as `Array` (a managed reference the GC already
  traces) rather than `any`.
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
