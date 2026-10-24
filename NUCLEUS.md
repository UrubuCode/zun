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
| 7 | Array | partial | Standard callback + structural surface landed (spec `(value,index,array)` callbacks); Zun extensions kept. Missing `findLast/findLastIndex/flat/flatMap/keys/values/entries/at` iterators, statics `Array.from/of/isArray`, variadic `splice` inserts, no-initial `reduce` |
| 8 | Function | partial | `call/apply/bind` + `.name/.length` landed (receiver-as-leading-param model, Tier 0); no `Function` constructor, no bound-arg function values |
| 9 | Promise | partial | Instance `state/result/isReady/resolve/reject` + ECMAScript `then/catch/finally` (new-promise + Promise adoption), statics `Promise.resolve/reject/all/allSettled/race/any`. `await` works via opcode. Remaining: `withResolvers`, microtask deferral of a *pending*-source reaction (eager-at-settle on Tier 0), AggregateError for `any` |
| 10 | Proxy / Reflect | mostly | Property traps `get/set/has/deleteProperty` intercept with Reflect-style target fallback; `proxy.k`/`proxy[k]` read+write route through the handler; `Reflect` realm global (`get/set/has/deleteProperty/ownKeys`). `x in obj` and `delete obj.k`/`obj[k]` operators now lex/parse and lower to the has/delete mechanism (Proxy receivers route to the `has`/`deleteProperty` traps). Deferred: descriptor-dependent traps + `apply`/`construct` |
| 11 | Map | partial | `set/get/has/size/delete/clear/values/keys/entries/forEach` + iterator + iterable constructor (`new Map([[k,v],…])`, array only); missing only `size` as property (is a method), general (non-array) iterable construction |
| 12 | Set | partial | `add/has/size/delete/clear/values/keys/entries/forEach` + iterator + iterable constructor (`new Set([…])`, array only); missing only `size` as property, general (non-array) iterable construction |
| 13 | WeakMap | partial | `set/get/has/delete`; semantics OK (ephemeron floor). No iteration (correct) |
| 14 | WeakSet | done | `add/has/delete` over the WeakMap ephemeron floor (value-less); no iteration (correct) |
| 15 | WeakRef | done | `new WeakRef(obj)` + `deref()` over the GC weak-reference floor; cleared on collection |
| 16 | FinalizationRegistry | partial | `register/unregister` present over GC finalization queue; `unregister` keyed by numeric id, not token object (verify vs spec) |
| 17 | ArrayBuffer | partial | `new ArrayBuffer(n)`, `byteLength()`, copying `slice()` with JS clamping. Missing `isView`, resizable buffers |
| 18 | DataView | partial | All element widths now present, signed included (`Int8`..`Int64`, `Float32`), plus `byteLength()`. 64-bit named `getInt64`/`getUint64` since Zun has native 64-bit ints. Missing `byteOffset`/`buffer` |
| 19 | TypedArrays | partial | Constructor length now counts elements as JS does (it counted bytes); `length()`/`byteLength()` reachable. Still only 4 kinds, and no `subarray`/`fill`/`set`/iteration |
| 20 | Date | partial | JS-correct `getUTCMonth`/`getUTCDay` (both were off by one), all UTC setters, `Date.UTC`, `getTimezoneOffset`. Missing local-time getters (UTC-only), `toLocale*`, `new Date()` |
| 21 | Buffer | missing | No `Buffer` class at all — the previous "partial" pointed at the ArrayBuffer/view floor by mistake. Needs the whole Node surface plus encodings |
| 22 | Error (+ subclasses) | partial | `Error` + 6 subclasses with JS-shaped `new Error(message)`, per-class `name` getter, JS `toString()`. Not a realm global; `.stack` absent entirely; `cause`/`AggregateError` blocked on construction having no optional args |
| 23 | globalThis / global | done | `globalThis`/`global` → `RealmGlobal` opcode (`Ty::DynamicObject`), realm singleton per §3.1 |
| 24 | Typed numeric lattice | done | `i8..i64/u8..u64/f32/f64` in `IntKind`/`Ty`; int/float TS types + coercions |
| 25 | Decorators | missing | Planned in A; not implemented (no parse/sema/lowering) |
| 26 | ZIL math opcodes | done | 15 ops at `0x96-0xA4`: float `sqrt/abs/neg/floor/ceil/trunc/min/max/round/sign`, int `min/max/clz/ctz/popcnt`. Transcendentals stay on libm per PLAN §3 |

**Counts:** done 5 · partial 18 · missing 3 (of 26 nucleus items, JSON excluded).

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
  `Number`, `Boolean`, `Object`, `Symbol`, `Reflect`. (`Map` keeps token 0;
  `Reflect` is appended last to keep every earlier token stable.)
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
- **Known limitation (documented, not faked):** there is no microtask drain inside a plain
  `run()` — `await` of a *pending* Promise suspends to a host poll (the existing
  cooperative model). A reaction whose **source is still pending** at call time therefore
  cannot be driven here: the returned Promise stays pending (the same place a bare
  `await <pending>` ends up). In practice Zun Promises settle synchronously
  (`resolve`/`reject`/`Promise.resolve`), so the chaining surface is exercised end-to-end.
- **AggregateError note:** `Promise.any` with all-rejected inputs rejects with a plain
  error message (`"All promises were rejected"`). `AggregateError` is unavailable — `Error`
  is not a realm global and construction has no ordered/optional args (see §22), so the
  aggregated `errors` array cannot be carried yet.
- **`reject` reason** is still a `string` (the managed cell stores a string reason); a boxed
  `any` reason is unwrapped to its string before storage.
- **Tests:** `zun_vm/tests/promise_surface.rs` (12 end-to-end cases exercising the scheduler
  via `await`: then-chaining, adoption, rejection skip/propagation, catch, finally
  pass-through, all/allSettled/race/any, resolve-of-a-Promise).

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

## 11. Map — partial
`std/map.zun` → `zun:collections` (`collections.rs`, `ordered_map.rs`). Ordered hash
table. Iterator intrinsics present (`mapIterator*`).
- **Present:** `set get has size() delete clear values keys entries` + `[Symbol.iterator]`
  + `forEach(cb)` (`callback(value, key, map)`, insertion order; routed to the
  `mapForEach` native via the sema managed-method path, reusing the Array callback
  re-entry/rooting machinery) + iterable constructor `new Map(iterable)` (`mapFrom`).
- **Missing:** `size` is a method, spec is a getter property. The iterable constructor
  currently accepts only an ARRAY of `[key, value]` pairs; arbitrary iterator-protocol
  objects are deferred (no general iterator consumption yet).

## 12. Set — partial
`std/set.zun` → `zun:collections`.
- **Present:** `add has size() delete clear values keys(→values) entries` + iterator
  + `forEach(cb)` (`callback(value, value, set)` — the value twice per spec — insertion
  order; routed to the `setForEach` native, reusing the Array callback machinery) +
  iterable constructor `new Set(iterable)` (`setFrom`).
- **Missing:** `size` as property; ES2024 set ops
  (`union/intersection/difference/isSubsetOf/…`) — note Array has these as extras but
  Set does not. The iterable constructor currently accepts only an ARRAY of elements;
  arbitrary iterator-protocol objects are deferred.

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
`Ty::Buffer` is what the realm registers under the name `ArrayBuffer`; the byte
storage lives in `zun_gc/buffers.rs`.

- **Present:** `new ArrayBuffer(length)`, `byteLength()`, `slice(begin, end)`.
  `slice` copies rather than aliasing, as JS does, and clamps inverted or
  out-of-range bounds to an empty result instead of failing.
- **Methods, not properties.** JS has `buffer.byteLength`; this is
  `buffer.byteLength()`, consistent with `String.length()` and `Map.size()` here.
  The property-vs-method divergence is repo-wide and listed under Surprising
  findings rather than patched per type.
- **Missing:** `ArrayBuffer.isView(x)`, resizable buffers
  (`resize`/`maxByteLength`), `SharedArrayBuffer` (deferred, fine).

## 18. DataView — partial
`BufferViewKind::DataView`; core accessors in `zun_core/views.rs`.

- **Present:** `getInt8/setInt8`, `getUint8/setUint8`, `getInt16/setInt16`,
  `getUint16/setUint16`, `getInt32/setInt32`, `getUint32/setUint32`,
  `getInt64/setInt64`, `getUint64/setUint64`, `getFloat32/setFloat32`,
  `getFloat64/setFloat64`, each with the little-endian flag (single bytes take
  none), plus `byteLength()`.
- **Signed access was missing entirely** before this, so a binary format with
  signed fields could not be read: the same byte is `-1` as `Int8` and `255` as
  `Uint8`, and only the unsigned reading existed.
- **64-bit naming:** JS calls these `getBigInt64`/`getBigUint64` and returns a
  BigInt. Zun has native 64-bit integers, so they are `getInt64`/`getUint64`
  returning `i64`/`u64`. A deliberate divergence, not an oversight.
- **Missing:** `byteOffset()` and `buffer()` accessors; `Uint8Clamped` has no
  DataView form in JS either.

## 19. TypedArrays — partial
`Uint8Array`, `Uint16Array`, `Uint32Array`, `Float64Array` over a shared
`ArrayBuffer`.

- **The constructor's third argument counted bytes, where JS counts
  elements.** `new Uint32Array(b, 0, 8)` gave a two-element view in Zun and an
  eight-element one in JS, so ported code read and wrote past what it believed
  it had. Fixed: the length is an element count, and `byteLength()` is that
  times the element width.
- **`length()` and `byteLength()` were unreachable.** The natives existed for
  every kind; only `get`/`set` were in the checker's method table, so a view
  could be read but not measured.
- **Present:** `get`, `set`, `length`, `byteLength` on all four kinds.
- **Missing kinds:** `Int8Array`, `Int16Array`, `Int32Array`,
  `Uint8ClampedArray`, `Float32Array`, `BigInt64Array`, `BigUint64Array`. Each
  needs wiring in six places (the core element enum, `BufferViewKind`, the
  natives, the interpreter dispatch, the realm facets and the checker's method
  table); `zun_vm/tests/typed_array_kinds.rs` enumerates what the realm accepts
  and fails if a kind is half-wired. Signed data is readable today through
  `DataView` (#18), which has every width.
- **Missing surface:** `subarray`, `slice`, `set(array, offset)`, `fill`,
  `copyWithin`, the functional and iteration methods, and the statics
  `TypedArray.from/of`.

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
- **Present:** millis storage, `getTime/setTime valueOf isValid
  isBefore/isAfter/isEqual`, the UTC getters, `toISOString toJSON toString`, and
  the statics `now/parse/fromMillis/fromISO/fromISODate`.
- **Missing:** local-time getters (`getFullYear`, `getHours`, …), absent rather
  than wrong because the calendar is UTC-only; `toDateString toTimeString
  toUTCString toLocale*`; `new Date()` and `new Date(string)`, which need
  optional construction arguments.

## 21. Buffer (Node) — missing

Audited 2026-10-23, and the previous entry was wrong twice over. It read
"partial / verify" and pointed at `zun_natives/src/buffer.rs` as evidence of a
Buffer implementation.

- **There is no `Buffer` class.** No `builtin/buffer.zun`, no realm entry, no
  checker path; `new Buffer(4)` is an unknown class.
- **`zun_natives/src/buffer.rs` is not it.** That module is the `ArrayBuffer`,
  `DataView` and typed-array floor (`bufferNew`, `bufferGetU8`, `dataViewGetU32`,
  `u8ArrayGet`, …), which belongs to #17, #18 and #19. `Ty::Buffer` is the type
  behind the name `ArrayBuffer`, not Node's `Buffer`.
- **What a real entry needs:** `Buffer.from/alloc/allocUnsafe/concat/isBuffer/
  byteLength`, instance `write read*/toString(encoding) slice fill copy equals
  indexOf`, and the encodings (`utf8`, `hex`, `base64`) that none of the floor
  provides. PLAN §3.2 does place Buffer in bucket A, so it stays here, but it is
  a surface to build rather than one to finish.

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
   - Map/Set (#11,#12): `forEach` and array-iterable construction landed. Remaining:
     `size` as a property; general (non-array) iterable construction.
   - Promise (#9): ✅ `then/catch/finally` (new-promise + Promise adoption) and statics
     `resolve/reject/all/allSettled/race/any` landed. Remaining: `withResolvers`,
     AggregateError for `any`, microtask deferral of a pending-source reaction.
   - TypedArrays/DataView (#18,#19): missing element kinds + full method surface.
   - BigInt (#4): `div/rem/pow/neg`, bitwise/shift, `BigInt()` coercion, `asIntN/asUintN`.
   - Error (#22): `AggregateError`, `cause`, verify `.stack` capture.
   - ArrayBuffer (#17): `byteLength slice isView`.
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
