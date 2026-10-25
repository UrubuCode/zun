# NUCLEUS.md — Bucket A completeness audit

This file **defines what "nucleus 100%" means** for Zun (the current `GOAL.md` goal:
"complete the nucleus — bucket A → 100%"). The nucleus is the built-in type/function
surface listed in `PLAN.md` §3.2 ("A — nucleus"). It is a living checklist: each item
below records its real state in the code — the method/property surface and semantics,
not merely "a class exists".

**Audit date:** 2026-10-24 · commit `2d395aa2` (clean `main`).
**Scope:** read-only audit of `std/*.zun`, `crates/zun_natives/src/*.rs`,
`crates/zun_sema/src/{check.rs,native_types.rs,types.rs}`, `crates/zun_core/src/*.rs`,
`crates/zun_il/src/opcode.rs`.

**The binary ser/de PRIMITIVE is nucleus (#27); the JSON surface is not.** PLAN §3.2
lists "JSON/ser-de" under A as "a core machine concern … rides on the nucleus
shape/field walk". That split is now explicit: the low-level binary
**serialize/deserialize PRIMITIVE** (Erlang `term_to_binary`-style, imported from
`"zun:core"`) IS nucleus — item **#27** — because it is the shape/field-walk machine
concern. The high-level, human-facing **JSON** text surface (`JSON.parse`/`stringify`)
stays in `@urubucode/common` (bucket C, ZIL) and remains **excluded** from this audit.

Legend: **done** = standard-common surface present; **partial** = exists but material
gaps; **missing** = not implemented. "verify" marks claims not fully confirmed from the
read.

---

## Summary table

| # | Item | Status | One-line gap |
|---|------|--------|--------------|
| 1 | String | mostly | Standard surface landed (`at/slice/concat/codePointAt/split/replaceAll/matchAll/valueOf`, string-vs-RegExp dispatch on `split`/`replace`, `[Symbol.iterator]`, statics `fromCharCode/fromCodePoint`); `String.raw` landed as the built-in template tag (tagged-template syntax now parses; raw chunks join with stringified substitutions); **the ECMA-402 / Unicode surface now lands via baked ICU4X** (`icu` crate, `compiled_data` — the modern analog of Boa's `boa_icu_provider`, no runtime data provider): `normalize(form?)` (NFC/NFD/NFKC/NFKD; default NFC), `localeCompare(that)` (returns -1/0/1), `toLocaleLowerCase()`/`toLocaleUpperCase()`. v1 uses root collation and the root (`und`) locale (locale-argument selection is a follow-up). All are pure value→value natives in `zun:string` (no second JIT path) |
| 2 | Number | mostly | Statics landed (`isNaN/isFinite/isInteger/isSafeInteger parseFloat parseInt` + all eight constants), instance `toString(radix)/toFixed/toExponential/toPrecision/valueOf`; `toLocaleString()` lands via baked ICU4X decimal formatting (root locale, grouping, max 3 fraction digits) as the `zun:number` native `numberToLocaleString` |
| 3 | Boolean | done | `Boolean(x)` truthiness coercion (primitive) + `boolean.toString()`; `globalThis.Boolean` resolves. **`new Boolean(x)` wrapper object landed** (prelude class `BooleanObject` with `valueOf`/`toString`); bare `Boolean(x)` still returns a primitive. Done |
| 4 | BigInt | mostly | Full op surface: `add/sub/mul/div/rem/pow/neg`, bitwise/shift (`and/or/xor/not/shl/shr`, two's-complement), `compare/eq/…`, `toString`, `BigInt(number\|string)` coercion (radix prefixes), `BigInt.asIntN/asUintN`. Operators `+ - * / %` and comparisons route to the natives; **`** & \| ^ << >> >>> ~` are now wired as real operator tokens** (lexer + parser precedence + sema routing to `bigIntPow/And/Or/Xor/Shl/Shr/Not`), spec-correct for BigInt, int kinds, and f64 (ToInt32/ToUint32, shift-count masking). The instance methods (`a.pow(b)`, `a.shl(b)`, …) remain as aliases. Deferred: `valueOf` |
| 5 | Symbol | mostly | Foundation + behavioral dispatch landed: `Symbol()` unique primitives, `Symbol.for/keyFor`, well-known symbol **values** (now 12: `iterator/asyncIterator/hasInstance/toPrimitive/toStringTag` + seeded `match/replace/search/split/species/isConcatSpreadable/unscopables`), `.description`, symbol-keyed `obj[sym]` get/set. Dispatch: `Symbol.hasInstance` on a **dynamic object** drives `instanceof` (via `dynamicSetSymbolFunction` + `instanceofDynamic`); **`String(x)` coercion** is the ToString site that invokes `[Symbol.toPrimitive]` (hint "string") and exposes `[Symbol.toStringTag]` as `"[object <Tag>]"` (`stringCoerce` native). **Object-literal computed Symbol methods landed** (§6b): `{ [Symbol.hasInstance](v){…} }` / `{ [Symbol.toPrimitive](h){…} }` lower to `dynamicSetSymbolFunction`, so user well-known-symbol methods can be written as literals (not only installed by hand). **Class computed symbol-key members landed**: `class C { [Symbol.hasInstance](v){…} }` parses and each such method is attached (as a function value) to a per-type symbol-method side table at class-definition time (`classSetSymbolFunction`); a typed-class `[Symbol.hasInstance]` now drives `instanceof` (forced runtime path consulting the side table). **Iteration protocol landed**: `for..of` over a non-built-in now desugars to `it[Symbol.iterator]().next()` driving, so a user object literal `{ [Symbol.iterator](){…} }`, a **typed-class `[Symbol.iterator]`** (the stored computed-symbol method is now consumed), and built-in iterator objects are all iterable; `Array`/`Map`/`Set` `keys/values/entries` return a real iterator object (`next()→{value,done}`, itself for-of-able); `new Set(x)`/`new Map(x)`/`[...x]` accept any protocol iterable. **`Number(x)` coercion landed** — the ToNumber site (`numberCoerce` native): strings parse (NaN on failure, `0x`/`0o`/`0b` radices, whitespace-trimmed), booleans map to 0/1, and an object runs OrdinaryToPrimitive(hint "number") via `[Symbol.toPrimitive]`, completing the toPrimitive dispatch surface. **Operator-position coercion landed**: `+` and template interpolation with an object operand run native `addAny` (ToPrimitive, hint "default"). **`Symbol.match/replace/search/split` now dispatch** on a dynamic-object argument from the String methods; `Symbol.species` is an intentional documented no-op (frozen Array), `Symbol.unscopables` is N/A (no `with`). Residual: object-level `toPrimitive` on a **typed class instance** in `addAny` is not honored (dynamic objects only); the `valueOf`/`toString` fallbacks are not tried (matches `stringCoerce`) |
| 6 | Object | done | Enumeration/reflection surface + **property-descriptor model** landed: `keys/values/entries/assign/freeze/isFrozen/create/getPrototypeOf/setPrototypeOf/fromEntries/is` plus `defineProperty/defineProperties/getOwnPropertyDescriptor/getOwnPropertyDescriptors/getOwnPropertyNames/seal/isSealed/preventExtensions/isExtensible` and instance `hasOwnProperty/isPrototypeOf/propertyIsEnumerable`. Full attribute set on `DynamicObject` (`writable/enumerable/configurable` + non-extensible); enumerable-skipping in `keys/values/entries/assign`; accessor get/set stored and the getter/setter is **invoked on `obj.k` read/write**. **`getOwnPropertySymbols` landed** — `Object.getOwnPropertySymbols(obj)` returns the object's own symbol keys (new `DynamicObject::symbol_keys` enumeration → heap `dynamic_symbol_keys` → `objectGetOwnPropertySymbols` native), in stable id order. Done |
| 7 | Array | done | Standard callback + structural surface landed (spec `(value,index,array)` callbacks); `findLast/findLastIndex/flat(depth)/flatMap/keys/values/entries/at` (relative), statics `Array.isArray/of/from`, variadic `splice` inserts and no-initial `reduce`/`reduceRight` now landed. Zun extensions kept. **Binding destructuring landed** (declarations + for-of heads), so `for (const [i, v] of arr.entries())` works directly. **`keys/values/entries` now return a real iterator object** (`next()→{value,done}`, itself for-of-able), and `[...x]` spread drains any protocol iterable. **Change-array-by-copy landed**: `toSorted(cmp?)/toReversed()/with(index,value)` (negative index; out-of-range throws) `/toSpliced(start,deleteCount,...items)` all return fresh arrays and leave the receiver untouched. **`group`/`groupToMap` landed**, and **`Array.prototype.concat(...items)` landed** with `Symbol.isConcatSpreadable`. Remaining: — |
| 8 | Function | mostly | `call/apply/bind` + `.name/.length` landed (receiver-as-leading-param model, Tier 0). **Function-valued `bind` partial/`this` arguments now work**: a function cannot box into `Any`, so each function-typed prepended argument is wrapped by a `functionBox` intrinsic (a GC-traced object carrying the callable in a `__boxedFn` function slot, keeping its captured environment visible to precise root handling) and `resolve_call_target` unwraps it before dispatch — order among prepended args is preserved. Deferred: **`Function` constructor** (`new Function(args, body)` evaluates a source string ⇒ needs a runtime compiler; the VM links no compiler and this conflicts with AOT reachability/trimming — structural); **a function inside `apply`'s arguments array** / functions in generic arrays (would require letting `box_any` carry functions globally — a deliberate GC-root-precision invariant, broad arg-marshalling rework); **multi-argument bound calls** (bound functions are typed `Function` and dynamic calls use the single-trailing-arg IL arity model — carrying an argument count for a dynamic `CallIndirect` is an opcode-set + verifier change) |
| 9 | Promise | mostly | Instance `state/result/isReady/resolve/reject` + ECMAScript `then/catch/finally` (new-promise + Promise adoption), statics `Promise.resolve/reject/all/allSettled/race/any/withResolvers`. `await` works via opcode. Unified micro+macrotask event loop with a `timers` surface (see §9a). `withResolvers` returns `{ promise, resolve, reject }`; **destructuring the resolver functions now works** (`const { resolve, reject } = Promise.withResolvers(); resolve(v)`) — object destructuring reads the function slot as a callable value and an `any`-typed local is now dynamically callable; `Promise.any` all-reject now carries an `AggregateError`; unhandled rejections are reported at turn end. **`new Promise(executor)` now lands** (`new Promise((res, rej) => …)`): the constructor allocates a pending promise, builds its `resolve`/`reject` settlers via the exact `withResolvers` mechanism (`make_promise_settler`/`settler_of`), and synchronously invokes the executor with them (Tier 0 callback re-entry, promise rooted); an executor throw rejects the promise with the thrown value (object reason first-class via `settle_promise_reject_value`, string reason keeps its message surface) and is swallowed; the `resolve` settler now adopts a Promise argument (chaining, not wrapping — shared with `withResolvers`). `new Promise()` zero-arg stays the empty-pending path. **Generic (non-Promise) thenable adoption landed** in `settle_target_with`; caveat: the thenable's `then` is invoked SYNCHRONOUSLY at resolve time rather than as a separate microtask job (documented ordering deviation). Remaining: the ordering deviation only (minor notes: the textbook `new Promise((res) => setTimeout(() => res(1), 0))` form: the closure gaps are now closed — **calling a captured callable by bare name** inside a closure works (sema resolves a bare-name callee against captures, not only locals), and a **function value can now be captured into a closure environment** (it rides the environment DynamicObject's GC-traced function slot, since functions do not box into `Any`; `StoreDynamic` routes a `Value::Function` to the function slot and `LoadDynamic`'s existing fallback reads it back). The UNANNOTATED textbook form now works too: **contextual parameter typing** (TS-style) was added — the `new Promise` executor site hints its unannotated `res`/`rej` as `Function`, so `(res) => setTimeout(() => res(1.0), 0.0)` is callable without an annotation (an explicit annotation still wins; with no hint the `F64` default is kept, so numeric callbacks are unchanged). Future work: thread the same hint through the other function-typed argument positions (by-name and function-value calls whose parameter is a concrete function signature check their arguments before the parameter type is known). ByCell/mutated function captures also stay deferred. See §Closures) |
| 10 | Proxy / Reflect | mostly | Property traps `get/set/has/deleteProperty` intercept with Reflect-style target fallback; `proxy.k`/`proxy[k]` read+write route through the handler; `Reflect` realm global (`get/set/has/deleteProperty/ownKeys`). `x in obj` and `delete obj.k`/`obj[k]` operators now lex/parse and lower to the has/delete mechanism (Proxy receivers route to the `has`/`deleteProperty` traps). Descriptor traps (`getOwnPropertyDescriptor`/`defineProperty`/`getPrototypeOf`/`setPrototypeOf`/`isExtensible`/`preventExtensions`/`ownKeys`) now fire via the proxy-aware `Object`/`Reflect` statics (built on #6's descriptor model); the `apply` trap + callable proxies (`proxy(...args)`, `Reflect.apply`) work via `proxyNewCallable`/`proxyApply`. **`Reflect.construct` and the Proxy `construct` trap landed for CLASS TARGETS** (via `ClassDescriptor` + `classDescriptorToken`). Limits: class targets only (no arbitrary-function constructors — Zun has no function.prototype model), `newTarget === target` only, args must supply the exact field count, and `new proxy(...)` works only when the proxy is a local variable |
| 11 | Map | done | `set/get/has/delete/clear/values/keys/entries/forEach` + `size` as a **property** (`map.size`; the `.size()` method form still resolves) + iterator + iterable constructor: array of pairs (`new Map([[k,v],…])`) plus any built-in iterable (`new Map(otherMap)`, or a Set of pair arrays) via `mapFromIterable`. **`keys/values/entries` now return a real iterator object** (for-of-able, `next()→{value,done}`); `new Map(x)` accepts arbitrary user `[Symbol.iterator]` sources through the protocol. Remaining: — |
| 12 | Set | done | `add/has/delete/clear/values/keys/entries/forEach` + `size` as a **property** (`set.size`; the `.size()` method form still resolves) + iterator + iterable constructor: array (`new Set([…])`) plus any built-in iterable (`new Set("abc")` by code point, `new Set(otherSet)`, a Map by entry pairs) via `setFromIterable`. **`keys/values/entries` now return a real iterator object** (for-of-able, `next()→{value,done}`); `new Set(x)` accepts arbitrary user `[Symbol.iterator]` sources through the protocol. Remaining: — |
| 13 | WeakMap | done | `set/get/has/delete`; semantics OK (ephemeron floor). ECMAScript WeakMap has no iteration/size/clear by design, so this is the complete surface, not a gap |
| 14 | WeakSet | done | `add/has/delete` over the WeakMap ephemeron floor (value-less); no iteration (correct) |
| 15 | WeakRef | done | `new WeakRef(obj)` + `deref()` over the GC weak-reference floor; cleared on collection |
| 16 | FinalizationRegistry | done | Spec-shaped `new FinalizationRegistry(cleanup)` / `register(target, heldValue, unregisterToken?)` / `unregister(unregisterToken)`; `unregister` keyed by unregister-token object identity (weak key); cleanup callback delivered on collection. **`heldValue` is now an arbitrary value** — boxed into `Any`, GC-traced (marked and relocated) while the registration is pending, and delivered to the cleanup callback on collection (register/queue/deliver path carries an `Any`, not a `u64`). Done |
| 17 | ArrayBuffer | done | `new ArrayBuffer(n)`, `byteLength()`, copying `slice()` with JS clamping, the static `isView(x)`, and now **resizable buffers (ES2024)**: `new ArrayBuffer(n, { maxByteLength })`, `resize(newLength)` (grow zero-fills the tail, shrink preserves the prefix), `maxByteLength()`/`resizable()`. Out-of-bounds / length-tracking view semantics are **lazy-resolved**: typed-array/DataView windows re-resolve against the buffer's current length at access time (no buffer→view back-reference registry), so a resize walks no views — a length-tracking typed array (`new Uint8Array(rab)`, no explicit length) follows the new length, and a fixed-length view that no longer fits reports length 0 and fails on access. Rejected `resize` (fixed buffer or `newLength > maxByteLength`) errs as `BadToken` (project convention, not JS `RangeError`). **`transfer`/`transferToFixedLength` landed (ES2024)**: a `detached` flag on `ByteBuffer` (checked in `resize` and surfaced via `detached()`); `transfer(newLength?)` detaches the source (clears its bytes → `byteLength` 0, every view sees length 0 through the existing lazy-resolve) and moves the surviving prefix into a fresh buffer that keeps the source's resizability; `transferToFixedLength(newLength?)` yields a fixed-length buffer. `newLength` defaults to the source length (a `-1` sentinel). **Length-tracking `DataView` over a resizable buffer landed** (see #18). Remaining: — |
| 18 | DataView | done | All element widths present, signed included (`Int8`..`Int64`, `Float32`), plus `byteLength()`/`byteOffset()`/`buffer()`. 64-bit named `getInt64`/`getUint64` since Zun has native 64-bit ints. **Endianness flag is now optional and defaults to `false` (big-endian) per spec** — `getUint32(0)` type-checks and reads big-endian; single-byte widths still take none. **Length-tracking `DataView` over a resizable `ArrayBuffer` landed**: `byteLength` and get/set re-resolve against the buffer at access time; a shrunk fixed view now correctly fails |
| 19 | TypedArrays | done | All eleven JS kinds, generated from one master table in `zun_core` (`Int8`/`Uint8`/`Uint8Clamped`/`Int16`/`Uint16`/`Int32`/`Uint32`/`Float32`/`Float64`/`BigInt64`/`BigUint64`), each with `length()`/`byteLength()`/`get`/`set`/`subarray`/`fill`/`byteOffset`/`buffer` and `[Symbol.iterator]`/`for...of`. **Bulk `set(source, offset?)`** (overloaded against the indexed `set(index, value)` by first-argument type), **copying `slice(start?, end?)`**, **in-place `copyWithin(target, start?, end?)`**, the functional methods **`map`/`filter`/`forEach`/`reduce`/`find`/`findIndex`/`some`/`every`** (spec `(value, index, array)` callback, Tier 0), **`indexOf`/`includes`/`join`**, and the statics **`from(src, mapFn?)`/`of(...)`** all landed, as kind-agnostic `typedArray*` natives (one each, not per-kind). `Uint8ClampedArray` takes a `number` and clamps per `ToUint8Clamp`; the 64-bit pair carries native `i64`/`u64`, not BigInts. No-initial-value `reduce(cb)`/`reduceRight(cb)` now landed (seed from the first/last element; empty typed array throws a TypeError-equivalent), alongside initialized `reduceRight(cb, init)`. `Uint8Array.slice` keeps Buffer (shared-type) semantics. Note: elements above 2^53 are not writable as literals — bare numeric literals are `f64` |
| 20 | Date | mostly | JS-correct `getUTCMonth`/`getUTCDay` (both were off by one), all UTC setters, `Date.UTC`, `getTimezoneOffset`, `Date.now`, `new Date()` (current time), local-time getters AND local-time setters (`setFullYear/setMonth/setDate/setHours/setMinutes/setSeconds/setMilliseconds` with ECMAScript optional trailing args, == their UTC counterparts since the calendar is UTC-only). `toLocaleString/toLocaleDateString/toLocaleTimeString` land via baked ICU4X datetime (root `und` locale, medium length, UTC) as `zun:date` natives |
| 21 | Buffer | done | Built on the `Uint8Array` byte-view floor (Buffer IS a U8 view, as in Node); realm global via `builtin/realm.zun` + `base_type_facets`. Statics `from`(string/array/ArrayBuffer)/`alloc`/`allocUnsafe`/`concat`/`isBuffer`/`byteLength`; instance `length`, indexed `buf[i]` get/set, `toString`/`slice`(negative indices)/`write`/`equals`/`copy`/`fill(value)`, common LE/BE 8/16/32/64 readers/writers, `readFloat*`/`readDouble*`/`writeFloat*`/`writeDouble*`, variable-width `read{U,}Int*`/`write{U,}Int*` (1..6 bytes, incl. 24-bit), `swap16/32/64`, and `indexOf`/`lastIndexOf`/`includes` (number/string/Buffer needle + byteOffset/encoding). Encodings utf8/hex/base64/base64url/ucs2/utf16le/latin1. Length-mismatch swap + OOB access err as `BadToken` (project convention, not JS `RangeError`). `copy(target, targetStart?, sourceStart?, sourceEnd?)` now accepts negative start/end indices (counted from the end, like `slice`); shared-memory/view semantics intact |
| 22 | Error (+ subclasses) | done | `Error` + 6 subclasses with JS-shaped `new Error(message)`, per-class `name` getter, JS `toString()`. **`.stack` captured** at construction (frame walk + sequence points; byte-span frames). **`AggregateError(errors, message)` carries its `errors`** (standalone class, not `extends Error`). **`new Error(message, { cause })` landed** — `cause` read from the options object into an `any` field (absent ⇒ `undefined`); applies to the subclasses. **`Error` + 6 subclasses are realm globals** (`globalThis.Error` resolves; class-backed `ClassFallThrough` binding). **`err.cause = x` dynamic set landed** — assigning the declared `any` `cause` field on a typed Error instance type-checks and stores through the any-field carrier (field store on `Ty::Instance`), independent of the `(message, options)` construction form. Done |
| 23 | globalThis / global | done | `globalThis`/`global` → `RealmGlobal` opcode (`Ty::DynamicObject`), realm singleton per §3.1 |
| 24 | Typed numeric lattice | done | `i8..i64/u8..u64/f32/f64` in `IntKind`/`Ty`; int/float TS types + coercions |
| 25 | Decorators | mostly | Syntax done (lexer `@`, parse on class/method/field, AST+HIR carry). **Compile-time macro-expansion hook landed** (PLAN §3.5 Stage 2, the .NET source-generator model): a `class_macro` registry keyed by decorator name, a `macros::expand` pass after parse / before sema that reads a value-model (`ClassModel`/`FieldModel`), runs the macro to an emit-model (`MacroExpansion`), and lowers it to AST via source-suffix interning (no HIR/module injection); failure contract = diagnostic (`MACRO0001`) + unchanged-target fallback; proving macro `@autoToString`. **Runtime TC39 decorators — tractable core landed** (Stage 3): a `@dec` whose callee resolves to an ordinary top-level `(any, any)` function (not a macro) is invoked once at class-definition time via the module-initializer seam with `(value, context)`, `context = { kind: "class"\|"method", name, static }`, TC39 order (members before class, bottom-to-top); callee-resolves-to-a-function is the registry-free macro-vs-runtime rule, so `@sealed`/`@trace` stay inert. Deferred (documented): return-value replacement + live `value` (needs dispatch-model change; `Value::Function` excluded from `Any`, no runtime class value → `value` is `null`); `addInitializer`; `access`; field/accessor/private-element decorators. **Decorator factories `@dec(args)` landed**: the factory call `dec(args)` (literal args) is evaluated in the initializer and its returned decorator is invoked with `(value, context)` via `applyDecorator`. Separately: in-language/user-defined macros; field/method-level built-ins; `@sealed`-style in-place runtime enforcement (needs metadata/format + runtime work). **Landed since:** a typed `DecoratorContext`; class-decorator `addInitializer`; field decorators now RUN at class-definition with `kind:"field"`. **Still deferred (architectural, see "Remaining architectural forks")**: return-value replacement (method/class) [D], instance-bound `addInitializer`, the field initializer-transform, and `context.access`. Soundness caveat to harden: the queued-initializer `Vec<FunctionHandle>` is not GC-rooted in the brief window between decorator call and drain |
| 26 | ZIL math opcodes | done | 15 ops at `0x96-0xA4`: float `sqrt/abs/neg/floor/ceil/trunc/min/max/round/sign`, int `min/max/clz/ctz/popcnt`. Transcendentals stay on libm per PLAN §3 |
| 27 | Binary ser/de primitive | done | `serialize(value): Uint8Array` / `deserialize(bytes): any`, imported from `"zun:core"` (bare names, not globals). Version-tagged CBOR-ish wire format (`zun_core::serde`) covering null/undefined/bool/int-kinds/f32/f64/bigint/string/array/dynamic object/**typed class instance (metadata type token + layout-order field walk)**/Map/Set/Date/ArrayBuffer/TypedArray(+Buffer) with a **ref/memo** tag; shared & cyclic graphs round-trip (allocate-shell-then-fill, memo on both sides). Engine on the Tier-0 floor (`zun_interp::serde`), reusing `object_own_entries` (the `Object.keys` walk). Safe-by-construction: no code exec; a function value, unknown tag, truncated stream, or unresolved type token is a clean error. **Symbol-keyed object entries landed** (registered `Symbol.for` key + well-known id; unique symbols are dropped) and **DataView landed** (length-tracking flag). Residual: cross-program/versioned type identity (type tokens are program-local) |

**Counts:** done 18 · mostly 9 · partial 0 · missing 0 (of 27 nucleus items; the JSON
text surface stays excluded — see the note at the top). Recounted from the table above;
it has to sum to 27.

**Intl (bucket-C-adjacent, shipped):** ECMA-402 `Intl.Collator` / `Intl.NumberFormat` /
`Intl.DateTimeFormat` landed as a core lib (prelude classes + dotted-`new` parser support +
an `Intl.X` to `IntlX` sema shim), ICU4X-backed, taking an explicit BCP-47 locale argument.
It is not one of the 27 nucleus rows and does not change the counts.

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
  a false positive). `Symbol.hasInstance` customization is now wired for both a
  **dynamic-object** right operand (`instanceofDynamic`; see §5) and a **typed
  class** (`instanceofClass` consults the per-type symbol-method side table
  populated by `classSetSymbolFunction` from the class's computed symbol-key
  members; a class carrying `[Symbol.hasInstance]` forces the runtime path).
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
  `Number`, `Boolean`, `Object`, `Symbol`, `Reflect`, `Buffer`, and **`Error` +
  6 subclasses** (`EvalError/RangeError/ReferenceError/SyntaxError/TypeError/
  URIError`). (`Map` keeps token 0; `Reflect`, `Buffer`, then the Error family
  are appended last to keep every earlier token stable. `Buffer` denotes the
  `Uint8Array` byte view — see #21.)
- **Class-backed binding (`ClassFallThrough`) landed.** `Error` + subclasses are
  class-backed: their annotation is `Ty::Instance(<program-assigned class
  index>)`, which a static `base_type_facets` entry cannot name, so the binding
  carries a `ClassFallThrough` marker and the checker resolves the name through
  `classes_by_name` at check time (both for the `: Error` annotation and for
  `new Error(...)`, which uses the ordinary class-construction path). The
  registry still owns the realm descriptor value, so `globalThis.Error` (and
  each subclass) resolves like every other base type. These classes ship via the
  prelude, so `new Error`/`throw new TypeError` already worked in-source; the
  registry adds the `globalThis` resolution and the uniform realm path.
- **Not registrable yet:** `Date` — its annotation would likewise resolve to
  `Ty::Instance(<program-assigned class index>)`, now expressible via the same
  `ClassFallThrough` mechanism (future work); and `Proxy` (not in bucket A). The
  hardcoded `new`/annotation matches remain for the lowercase aliases (`map`,
  `array`, …) and `Proxy`, so those are not yet removable.

---

## 1. String — mostly  *(standard surface landed, including the ICU4X-backed Unicode methods)*
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
  - `matchAll(regexp)` — all matches of a **global** regexp (the `g` flag is required;
    without it a runtime error is raised, matching V8/Node's `TypeError` convention).
    Intercepted on a `Ty::String` receiver with a `Ty::RegExp` argument (`check.rs`),
    lowered to the `stringMatchAll` managed builtin (`zun:collections`), serviced in
    `zun_interp/src/collections.rs` over `Heap::regex_exec_all` (which iterates every
    non-overlapping match via `RegexValue::captures_all` on both regex engines). **Returns
    an Array of match arrays** (each inner array is shaped exactly like `exec`'s result —
    index 0 the whole match, 1.. the capture groups, `undefined` for non-participating
    groups), **not** a distinct iterator object — a pragmatic v1 choice specific to
    `matchAll` (`Array`/`Map`/`Set` `keys()/values()/entries()` now DO return real
    iterator objects via the iteration protocol).
  - `valueOf()` — identity, returning the string primitive itself. No native: the sema
    intercept returns the checked receiver expression directly (like `number.valueOf()`).
- **Landed via baked ICU4X** (`icu` crate, `compiled_data`; dep on `zun_natives`):
  - `normalize(form?)` — ICU4X normalizer (NFC/NFD/NFKC/NFKD; default NFC synthesized by a
    sema hand-lowering for the optional arg; an unknown form is a runtime RangeError-equivalent).
  - `localeCompare(that)` — ICU4X collator, returns the ECMAScript -1/0/1 (v1 root collation).
  - `toLocaleLowerCase()`/`toLocaleUpperCase()` — ICU4X case mapper (v1 root `und` locale).
  - Follow-up: per-locale argument selection and `Intl` options.
- **Deferred:**
  - `String.raw` landed (built-in template tag). `charAt` (superseded by `at`).
    Replace-with-a-function replacer is not supported (replacement must be a string).
- **Notes:** the string-dispatched `split`/`replace`/`replaceAll`/`concat`/`slice`/`at`/
  `codePointAt` and the statics all allocate/branch only (Tier-0 floor), so they are not
  in the JIT allowlist — no second path to keep in sync.

## 2. Number — mostly  *(statics, radix, and instance formatting landed)*
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
- **Present (instance formatting, new):**
  - `toExponential(fractionDigits?)` — JS exponential notation (`e`±exponent, the sign
    always spelled). With no argument the shortest round-tripping mantissa is used; with
    `fractionDigits` (0..=100) exactly that many fraction digits. Non-finite values render
    JS-style. `toExponential` native (`zun:number`); the sema intercept on an `F64`
    receiver passes `-1` (out of range) as the absent-argument sentinel.
  - `toPrecision(precision?)` — significant-digit formatting. No argument behaves like
    `toString`; with `precision` (1..=100) it renders that many significant digits,
    switching to exponential when the decimal exponent is `< -6` or `>= precision` (per
    ECMAScript Number::toPrecision — the exponent is read back from a `precision-1`-digit
    exponential rendering). `toPrecision` native; the sema intercept passes `0` (out of
    range) as the absent-argument sentinel. **Rounding:** Rust's float formatting (round
    half-to-even) is used; it matches V8/.NET observable output for the common cases
    (covered by tests), differing only in rare exact-half ties.
  - `valueOf()` — identity, returning the number itself. No native: the sema intercept
    returns the checked receiver expression directly.
- **Landed via baked ICU4X:** `toLocaleString()` — `icu` decimal formatting (root locale,
  grouping, max 3 fraction digits; NaN/∞ handled) as the `zun:number` native
  `numberToLocaleString`. Follow-up: per-locale argument + `Intl` options.
- **Notes:** the instance predicates (`x.isNaN()`) remain for Zun-style call sites; the
  new statics are the idiomatic `Number.isNaN(x)` surface. All of these allocate/branch
  only and are not JIT-lowered — Tier-0 only, no second path to keep in sync.

## 3. Boolean — done
**Update:** `new Boolean(x)` landed as the prelude class `BooleanObject` (`valueOf`/`toString`);
bare `Boolean(x)` still returns a primitive. The "Deferred" bullet below is superseded.
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
- **Operator tokens landed:** `** & | ^ << >> >>> ~` are real lexer/parser
  operators with JS precedence, routed in sema to the BigInt natives (and to the
  int/f64 paths for those types), spec-correct (BigInt two's-complement; f64
  ToInt32/ToUint32 and shift-count masking). The instance methods (`a.pow(b)`,
  `a.and(b)`, `a.shl(b)`, `a.not()`, …) remain as equivalent aliases. Also
  deferred: `valueOf`.

## 5. Symbol — mostly  *(HIGH priority — foundation + coercion dispatch landed)*
**Update (later sessions):** operator-position coercion (`+`/templates, native `addAny`,
hint "default") landed, superseding the "operator-position auto-coercion deferred" bullet
below. `Symbol.match/replace/search/split` dispatch on a dynamic-object argument from the
String methods; `Symbol.species` is an intentional no-op (frozen Array); `Symbol.unscopables`
is N/A (no `with`). Residual: typed-class `toPrimitive` in `addAny` is not honored, and
`valueOf`/`toString` fallbacks are not tried (matches `stringCoerce`).
The `SymbolRegistry` (`zun_core/src/symbols.rs`) is now surfaced to user code. A symbol
is a first-class value: `Ty::Symbol` in sema, carried at runtime as an immediate `Any`
(`AnyTag::Symbol`, payload = `SymbolId`), so the GC traces straight past it. The
isolate's `Heap` owns the registry (`symbol_create/for/key_for/description`), seeded with
the twelve well-known symbols at fixed ids 0..11 (`zun_core::WellKnownSymbol`) so their
identities are stable and match the compiler's constants. Ids 0..4 are the dispatch-bearing
set (`iterator/asyncIterator/hasInstance/toPrimitive/toStringTag`); ids 5..11
(`match/replace/search/split/species/isConcatSpreadable/unscopables`) are seeded as stable
**values** only (no behavior wired yet), appended at the end so the earlier ids never move. All Symbol operations are
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
- **Behavioral dispatch — `Symbol.hasInstance` (dynamic-object path landed):**
  `value instanceof C` where `C` is a **dynamic object** carrying a
  `[Symbol.hasInstance]` method now calls `C[Symbol.hasInstance](value)` and returns
  `ToBoolean` of its result, overriding the default class check. Wiring:
  `check_instanceof_operator` (`check.rs`) recognizes a right operand that is not a
  class name but resolves to an `any`/`DynamicObject` value and lowers to the
  `instanceofDynamic(value, C)` managed builtin (`zun:typeinfo`), serviced in
  `zun_interp/src/collections.rs`: it reads the method from `C`'s symbol-function slot
  (`dynamic_get_symbol_function`, well-known id 2) and invokes it with the existing
  `invoke_function_value` machinery; absent the method a dynamic object has no class
  identity, so the answer is `false`. The method is attached with the
  `dynamicSetSymbolFunction(obj, symbol, fn)` intrinsic (`zun:collections`), the
  symbol-keyed analogue of `dynamicSetFunction` — **not** `obj[sym] = fn`, because
  function values do not box into `Any` (so they live in the dedicated symbol-function
  slot, not the symbol-value slot). Interp-only (Tier-0 floor, like the other Symbol
  ops); no JIT path to keep in sync.
- **Behavioral dispatch — `Symbol.toPrimitive` / `Symbol.toStringTag` via `String(x)` (landed):**
  `String(x)` is now a first-class coercion call, the ECMAScript ToString site —
  analogous to the existing `Boolean(x)`/`BigInt(x)` coercion calls in sema's plain-Call
  path (`check.rs`). A value already typed `string` returns unchanged; every other operand
  is boxed into `Any` and run through the heap-aware `stringCoerce` managed builtin
  (`zun:collections`, serviced in `zun_interp/src/collections.rs`, Tier-0 floor like the
  other Symbol ops). For an **object** operand it performs OrdinaryToPrimitive with hint
  `"string"`: if the object carries a `[Symbol.toPrimitive]` method (read from the
  symbol-function slot, well-known id 3, attached via `dynamicSetSymbolFunction`), it is
  called with the hint string and its result is ToString'd; otherwise, if
  `[Symbol.toStringTag]` holds a **string** in the symbol-value slot (readable via
  `dynamic_get_symbol`, well-known id 4) the result is `"[object <Tag>]"`; else the default
  `"[object Object]"`. Primitives stringify through `Value::display`. This gives both
  `toPrimitive` (string hint) and `toStringTag` an observable, spec-shaped site. Interp-only
  (no JIT path to keep in sync).
- **Deferred (documented follow-up):**
  - **`Symbol.hasInstance` on a typed class** stays deferred: it requires class
    computed-symbol-key member definitions (see below) so a class can carry the method.
    Only the dynamic-object path above is wired.
  - **Operator-position object→primitive auto-coercion** stays deferred: `+` and template
    interpolation still require matching `string`/number operand types (an object operand is
    a type error). The explicit `String(x)` call is the sufficient, spec-correct site; wiring
    `toPrimitive` into operators is a broader type-system change, not needed to observe the
    dispatch.
  - **`Number(x)` coercion landed** (`numberCoerce` native, sema plain-Call path alongside
    `String(x)`/`Boolean(x)`/`BigInt(x)`): a number → itself; a string → ToNumber (whitespace
    trim, `0x`/`0o`/`0b` radices, decimal/scientific, `NaN` on failure, `""` → 0); a boolean
    → 0/1; `null` → 0, `undefined` → NaN; an object → OrdinaryToPrimitive(hint `"number"`)
    via `[Symbol.toPrimitive]`, then ToNumber of its result (else NaN). Completes the
    toPrimitive dispatch surface.
  - **`Object.prototype.toString`** as a canonical stringification site is still absent; the
    `print` native keeps the generic `"[object]"` (it runs on a heap-free `NativeArg`).
    `String(obj)` is the heap-aware stringification site; a shared `toString` would route
    through the same `stringCoerce` logic.
  - **Seeded `match/replace/search/split/species/isConcatSpreadable/unscopables`** have stable
    values but no behavior wired (no RegExp-protocol/`concat`/`species` dispatch keys on them yet).
  - **Object-literal computed symbol-key properties** (`{ [computedSymbol]: v }`) now
    lower: an object literal builds a `DynamicObject` (`Expr::ObjectLiteral` →
    `hir::ExprKind::DynamicObjectLiteral` → `NewDynamic` + per-entry `StoreDynamic`/
    `StoreDynamicValue`), and a computed key that evaluates to a `Symbol` routes to the
    symbol-keyed slot. Still deferred: **class** computed symbol-key member definitions
    and object **method shorthand** with a computed symbol name (`{ [Symbol.iterator]() {} }`)
    — see §Object literals below.

## 6. Object — mostly  *(property-descriptor model landed)*
The `Object` global is recognized in sema (`check.rs` `check_object_static` + the
instance reflectors in the `MethodCall` arm) and lowered to `zun:collections`
`object*` managed builtins implemented in the interpreter floor
(`zun_interp/src/collections.rs`). The statics operate on **both** fixed-layout typed
objects and `DynamicObject`s.

- **Present (statics):** `keys values entries assign freeze isFrozen create
  getPrototypeOf setPrototypeOf fromEntries is defineProperty defineProperties
  getOwnPropertyDescriptor getOwnPropertyDescriptors getOwnPropertyNames seal isSealed
  preventExtensions isExtensible`.
- **Present (instance):** `hasOwnProperty isPrototypeOf propertyIsEnumerable`.

### Property-descriptor model (NUCLEUS #6 — landed)
`DynamicObject` (`zun_core/src/dynamic_object.rs`) now carries the full ECMAScript
attribute set per own string key, extending the pre-existing `writable` bit with
`enumerable` and `configurable` (stored as cleared-attribute sets, default-true) plus a
`non_extensible` flag. A key is either a **data property** `{ value, writable,
enumerable, configurable }` or an **accessor property** `{ get?, set?, enumerable,
configurable }` whose get/set are `FunctionHandle`s held in dedicated accessor slots
(function values do not box into `Any`; the GC traces their environments via
`object_references`). Default attributes follow the creation path: an ordinary
assignment (`obj.k = v`, `DynamicObject::set`) creates enumerable + writable +
configurable; `Object.defineProperty` defaults each omitted flag to **false**.

- **Enumerable semantics.** `DynamicObject::entries`/`enumerable_keys` skip
  non-enumerable keys, so `Object.keys/values/entries` and `Object.assign` observe only
  enumerable own string keys; `getOwnPropertyNames` (`keys()`) returns **all** own string
  keys (enumerable or not, data or accessor). (Object spread / `for-in` do not exist in
  the front-end yet, so there is no other enumeration site to filter.)
- **Enforcement (silent, freeze-consistent — matching the existing `freeze` model).** A
  non-writable data key rejects `obj.k = v`; a non-configurable key rejects re-
  `defineProperty` and `delete`; a non-extensible object rejects adding a new key.
  `preventExtensions` sets non-extensible; `seal` = non-extensible + every key
  non-configurable (values stay writable); `freeze` = non-extensible + every data key
  non-writable + every key non-configurable (reconciled with the pre-existing `frozen`
  fast flag). `isSealed`/`isExtensible`/`isFrozen` are computed from this state; typed
  fixed-layout objects report sealed + non-extensible (and `isFrozen` true).
- **Statics plumbing.** The descriptor argument and the
  `getOwnPropertyDescriptor`/`getOwnPropertyDescriptors` return value are ordinary
  `DynamicObject`s: `{value, writable, enumerable, configurable}` for data,
  `{get, set, enumerable, configurable}` for accessors (get/set read from / written to
  the string **function-slot** via `dynamicSetFunction`/`dynamic_get_function`; a missing
  accessor shows as `undefined`). `defineProperties` applies each enumerable key of the
  descriptors map. The new natives are appended **last** in the `collections` module so
  existing dispatch ids stay stable, and are added to the interpreter's by-name
  interception list (`builtins.rs`).
- **Accessor invocation — WIRED.** Reading `obj.y` (`LoadDynamic`) invokes the stored
  getter and assigning `obj.y = v` (`StoreDynamic`) invokes the setter, via
  `invoke_function_value` (reusing the callback re-entry machinery, Tier 0 only). This is
  reachable end-to-end today: a getter is attached to a descriptor with
  `dynamicSetFunction(desc, "get", fn)` and `Object.defineProperty(o, "y", desc)`, then
  `o.y` returns the getter's result (covered by a test). **Gap:** the *computed-key* paths
  (`LoadDynamicValue`/`StoreDynamicValue`, `o[k]`) do not yet consult accessors — only the
  named `.y` path does; wiring them is a follow-up.
- **Typed-object (class instance) targets.** `defineProperty` and friends target
  `DynamicObject`s; a typed frozen-layout instance keeps its fixed behavior
  (`getOwnPropertyDescriptor` on a declared field reports a writable + enumerable +
  non-configurable data descriptor; `defineProperty` cannot add fields to it).
- **Key enumeration:** for a typed object, own keys = its declared instance fields in
  definition order, walking base classes first and excluding `static`/private (`#`)
  fields; for a `DynamicObject`, its dynamic **string** keys in insertion order
  (`DynamicObject` now keeps string properties in an insertion-ordered vector + index,
  replacing the previous sorted `BTreeMap`).
- **Prototype model:** `DynamicObject` carries an optional `[[Prototype]]` link;
  `create`/`getPrototypeOf`/`setPrototypeOf`/`isPrototypeOf` operate over it. Typed
  objects expose no reflective prototype (their `getPrototypeOf` is `null`).
- **`Object.is`:** ECMAScript `SameValue` (NaN equals NaN; +0 distinct from −0;
  numbers compared by IEEE-754 bits after the NaN check; strings by content; objects by
  identity).
- **`getOwnPropertySymbols` landed:** `DynamicObject::symbol_keys` exposes the ordered
  symbol-key list the store already held; heap `dynamic_symbol_keys` + the
  `objectGetOwnPropertySymbols` native build the array of `Any` symbol values. This also
  unblocks symbol-key copying for object spread and serialization later.
- **Still available to build on:** descriptor-dependent **Proxy** traps / **Reflect** statics
  (§10) can build on this model; computed-key accessor access (see above).
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
- **Method shorthand `{ m() {} }` — landed.** The method value is a closure stored in the
  object's string **function slot** via `dynamicSetFunction(obj, name, fn)`; `obj.m()`
  dispatches through it (the dynamic member-call path, `dynamicCall0/1/2`, now accepts a
  `DynamicObject` receiver, not only `any`). A **computed Symbol method** `{ [Symbol.X]() {} }`
  routes to the symbol-function slot via `dynamicSetSymbolFunction` — this is the NUCLEUS #5
  enabler for user `[Symbol.hasInstance]` / `[Symbol.toPrimitive]` methods (consulted by
  `instanceof` / `String()` coercion). A computed **string** method key uses the string
  function slot; a computed key that is neither a string nor a Symbol is rejected.
- **Getters/setters `{ get x() {} }` / `{ set x(v) {} }` — landed (named keys).** Lowered to
  an accessor property: the literal builds a `{ get|set: fn }` descriptor (the accessor fn
  in its function slot) and installs it with `objectDefineProperty(obj, name, desc)` — the
  same property-descriptor model as above, so reading/assigning `obj.x` invokes the
  getter/setter. Computed/Symbol-keyed accessors in literals are **deferred** (computed-key
  accessor access is itself not yet wired — see above).
- **Spread `{ ...o }` — landed.** Lowered to `objectAssign(obj, o)` at the spread position,
  copying `o`'s own enumerable string keys (for a `DynamicObject`) or declared instance
  fields (for a typed object) into the new object; a later key overrides an earlier one
  (source order preserved). Symbol-keyed own properties are **not** copied (consistent with
  `getOwnPropertySymbols` still being deferred).
- **Lowering mechanics.** Data entries keep the `Dup` + store + `Pop` shape; method /
  symbol-method / accessor / spread entries are `hir::ObjectEntry::Install { builtin, args }`:
  the object handle is duplicated, the args pushed, the native (`dynamicSetFunction` /
  `dynamicSetSymbolFunction` / `objectDefineProperty` / `objectAssign`) called, and its
  result discarded. No new opcodes or natives (all four already exist and are appended-last
  in `collections`). Both front-ends (hand parser + OXC adapter) build the new AST nodes
  (`ObjectMember::{Property,Spread}`, `PropertyKind::{Init,Method,Get,Set}`).
- **Iteration protocol (`zun_interp/src/collections.rs`).** `for..of` over a non-built-in
  type (`DynamicObject`, a typed-class `Ty::Instance` carrying `[Symbol.iterator]`, or
  `any`) reuses the generator desugaring triple (`generatorIteratorNew/Done/Next`): `New`
  resolves the iterator via `resolve_iterator` (`GetIterator` — calls the symbol-function
  slot / class symbol-method table, or wraps a raw array/set/map, or passes a built-in
  iterator through) and returns a look-ahead **protocol wrapper** (a `DynamicObject` with
  `__zun_it`/`__zun_cur`/`__zun_done`/`__zun_has`); `Done` prefetches one step
  (`protocol_step`: native cursor for a built-in iterator, else `it.next()` reading
  `{value,done}`), `Next` hands back the cached value. Generators keep their existing
  frame-resume path untouched. `Array`/`Map`/`Set` `keys/values/entries` now return an
  **array-iterator over the materialized sequence** (a real iterator object, typed `any`),
  and `it.next()` on one is intercepted in `dynamicCall0` (`builtin_iterator_result`).
  `new Set|Map(x)` and `[...x]` drain user iterables through `drive_iterable`.
  **Compat choice:** `keys/values/entries` returning `any` (not `Array`) is spec-correct;
  the few tests that indexed the old array result were updated to `.next()`/`for..of`.
  **Known limitation (pre-existing, orthogonal):** an object-literal method that itself
  creates a capturing closure is miscompiled when invoked through the dynamic function
  slot, so a user `[Symbol.iterator]` that builds its counter inline must instead return a
  closure minted by a top-level helper (see the end-to-end tests).
- **Note on computed keys:** like `obj[k] = v` on a dynamic receiver, a computed key must be
  a string or a `Symbol` at runtime (`StoreDynamicValue` dispatches on tag); full ToString
  coercion of arbitrary computed keys is not implemented (consistent with `IndexAssign`).
- Interp fix landed alongside: `Value::as_object` now unwraps an object-tagged `Any`, so a
  nested object read back out of a dynamic property (`{ a: { b } }.a.b`) works.

## 7. Array — mostly (standard surface landed)
**Update:** `group`/`groupToMap` and `Array.prototype.concat(...items)` (honoring
`Symbol.isConcatSpreadable`) landed; no known remaining gap in the standard surface.
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
- **Missing (standard):** `group groupToMap` (deferred — null-proto object keyed by
  callback result; lower priority). `toSorted toReversed toSpliced with` landed
  (change-array-by-copy; return fresh arrays, receiver untouched; `with` resolves a
  negative index from the end and throws a RangeError-equivalent out of range).
  `fill`/`copyWithin` range refinements. `keys()`/`values()`/`entries()` return a
  fresh managed **Array** (iterable via `for...of` and indexable), not a distinct
  iterator object — pragmatic for v1; a proper iterator-protocol object is a
  follow-up. `Array.from` consumes an array or string (and anything with array
  storage); arbitrary iterator-protocol sources are deferred (documented). **Note:
  `for (const [i, v] of arr.entries())` now works — binding destructuring landed in
  the front-end (see §Destructuring). The for-of head accepts an array/object
  pattern; `ForOf` carries a `Pattern` and the loop body destructures a hidden
  element temp each iteration.**
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

### 7c. Binding destructuring — landed  *(declarations + for-of heads)*
`const`/`let` declarations and `for-of` heads accept array and object patterns.
The AST carries a `Pattern` (`Ident | Array | Object`) with `PatternElement`
(sub-pattern + optional default) and `PatternProperty` (key + sub-pattern +
default); `Stmt::Destructure` replaces the old flat `ArrayDestructure`/
`ObjectDestructure` nodes, and `ForOf` carries a `Pattern`. Both the hand parser
(`binding_pattern`/`array_pattern`/`object_pattern`) and the OXC adapter
(`convert_binding_pattern` over `ArrayPattern`/`ObjectPattern`/`AssignmentPattern`/
`BindingRestElement`) build it.
- **Supported:** array elements, holes/elisions `[, b]`, `...rest` (new `arraySlice`
  native), defaults `[a = 1]` / `{ x = 1 }` (new `coalesceUndefined` native — eager
  default evaluation, not lazy), object shorthand/rename (`{ x, y: alias }`), computed
  keys `{ [k]: v }`, object `...rest` (new `objectRest` native, remaining own
  string keys), and arbitrary nesting (`[[a], { b }]`). Object patterns read
  `DynamicObject`/`any` via `LoadDynamic` and typed `Instance` fields via `FieldGet`.
- **Lowering:** sema desugars a pattern into a hidden temp (`Let`) plus per-binding
  `Let`s inside a new `hir::Stmt::Group`; array elements read through `arrayAt`
  (out-of-range → `undefined`, so trailing holes/defaults don't trap). No new
  opcodes. for-of binds a hidden element temp and prepends the destructuring
  statements to the loop body.
- **Scope:** declarations + for-of heads only. Function-parameter and
  assignment-target destructuring are a deliberate follow-up (not implemented).

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
- **Holes `[1, , 3]` — landed.** A bare `,` is an elision (`ArrayElement::Hole`). The
  no-spread path still uses `ConstI32 N` + `NewArray` but stores `ConstUndef` at each hole
  index, so a hole counts toward `length` and **reads as `undefined`** (not `0`, which the
  default array fill would give). The true hole-vs-`undefined` distinction (observable only
  through `in` / `hasOwnProperty`) is **not** represented — a hole and an explicit
  `undefined` are indistinguishable; documented, since there is no front-end `in`/`for-in`
  over dense arrays today.
- **Spread `[...xs]` — landed.** When any element is a spread, the literal lowers to
  `hir::ExprKind::ArraySpreadLiteral`: a growable `NewArray 0` built up part-by-part — a
  plain element and a hole each `ArrayPush` one value (`undefined` for a hole), and a spread
  calls `arraySpreadAppend(target, source)` (a new `array` native, appended last so existing
  dispatch ids stay stable) to splice the iterable's elements in at its source position.
  **Supported spread sources:** a managed `Array` and a `String` (code points), matching
  `Array.from`'s supported sources. `Set`/`Map`/generator/typed-array spread sources are
  **deferred** (same limitation as `arrayFrom`).
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

## 9. Promise — mostly  *(method + static surface landed)*
**Update:** generic (non-Promise) thenable adoption landed in `settle_target_with`. Caveat:
the thenable's `then` is invoked SYNCHRONOUSLY at resolve time, not as a separate microtask
job (documented ordering deviation).
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
  descriptors), `race` (first already-settled input wins), `any` (first fulfilment, else an
  `AggregateError` rejection — see AggregateError note), and `withResolvers()` (see
  withResolvers note).
- **Recognition (sema, `check.rs`):** `Promise.X(...)` statics follow the
  `check_number_static`/`check_string_static` pattern — a dedicated `check_promise_static`
  keyed on the `Promise` receiver name (guarded by `!classes_by_name.contains_key`).
  `withResolvers` is handled first (zero args, result typed `any`).
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
- **Reason cell — widened (landed).** The managed Promise cell
  (`zun_core::ManagedPromise`) now carries an optional first-class `reason: Option<Any>`
  alongside the string `error`. A plain-string rejection sets `error` only (`reject`); an
  object/boxed reason sets both via `reject_value(reason, message)` — the value is
  delivered to `catch`/`await` (`settled_await_value` and the `then` on-rejected path), the
  string keeps the display surface (`allSettled` descriptors, the unhandled-rejection line).
  The reason object is write-barriered and traced by the GC (both mark passes in
  `collection.rs`) and rewritten on compaction (`ManagedPromise::rewrite_object_references`).
  `zun_gc` exposes `promise_reject_value`/`promise_reason`; interp has
  `settle_promise_reject_value`.
- **AggregateError for `any` (landed).** `Promise.any` collects each input's rejection
  reason into an ordered array; when all inputs reject it settles its target via
  `settle_promise_reject_value` with an **AggregateError-shaped dynamic object**
  (`{ name: "AggregateError", message: "All promises were rejected", errors: [...] }`). A
  dynamic object, not the typed `AggregateError` class (§22): that class ships in
  `builtin/error.zun` and is compiled into a module only when the program imports it, so the
  combinator floor cannot depend on it — the shape matches, so `reason.errors`/`.name`/
  `.message` read the same. `catch`/`await` receive the object.
- **`withResolvers` (landed).** `Promise.withResolvers()` → a dynamic object
  `{ promise, resolve, reject }`. `promise` is an ordinary dynamic property (read with
  `r.promise`, awaitable since `await` accepts `any`). `resolve`/`reject` are **native
  "settler" callables** stored in the object's dynamic-FUNCTION slot
  (`dynamic_set_function`) — function values do not box into `Any`, so they cannot be
  ordinary properties. A settler is a `new_bound` handle over a record naming the promise +
  mode; it is intercepted before the `bind` path (`settler_of` in `invoke_function_value`
  and the `CallIndirect` bound branch) and settles natively. They are invoked from source as
  **dynamic method calls** (`r.resolve(v)` / `r.reject(e)`): the `any`-receiver method-call
  path (`dynamicCall0/1/2`) now checks the function slot first (invoking the stored handle),
  falling back to the import-namespace loader. `reject(e)` carries an object/boxed `e` as a
  first-class reason, a string `e` as a message.
  - **Resolver destructuring (landed).** `const { promise, resolve, reject } =
    Promise.withResolvers(); resolve(v)` now works. Two front-end changes close the former
    gap: (1) `LoadDynamic` falls back to the function slot when the data slot is absent,
    returning the settler as a callable `Value::Function` (object methods read the same way);
    (2) a call of an `any`-typed local now lowers to a dynamic indirect call
    (`DYNAMIC_FUNCTION`, single arg), so the destructured `resolve`/`reject` binding is
    callable. The field/method form (`const r: any = …; r.resolve(v)`) still works too.
- **`new Promise(executor)` (landed).** Recognized in `check.rs` as a dedicated
  `CtorLowering::Promise` realm variant: `new Promise()` keeps the zero-arg empty-pending
  path (`promiseNew`); `new Promise(fn)` (one argument, asserted assignable to
  `Function`) lowers to a new `promiseNewExecutor` intrinsic (appended at the END of the
  `promise` native table so dispatch ids do not shift). The interpreter floor allocates a
  pending promise, builds its `resolve`/`reject` settlers with the **exact** `withResolvers`
  mechanism (`make_promise_settler`, reclaimed by `settler_of` on call), roots the promise
  with `NativeRootScope`, and synchronously invokes the executor via `invoke_function_value`
  with `(resolve, reject)` (Tier 0 callback re-entry, like the other native callbacks). An
  executor **throw** is caught and rejects the promise with the thrown value — an object
  reason carried first-class via `settle_promise_reject_value`, a string reason kept as the
  message surface (the same split the `reject` settler uses) — and is swallowed, per
  ECMAScript. The `resolve` settler's fulfil branch now routes through `settle_target_with`,
  so **resolving with a managed Promise adopts it** (chaining, not wrapping) from both
  `new Promise` and `withResolvers`; generic (non-Promise) thenable adoption is not yet
  detected (documented gap). One orthogonal pre-existing closure gap surfaces around it:
  calling a captured variable by **bare name inside a nested closure** is unresolved in sema
  (`cx.lookup` does not see captures in the inner arrow), so the textbook
  `new Promise((res) => setTimeout(() => res(1), 0))` form must settle through a captured
  object's function slot instead (e.g. adopting a `withResolvers` promise a timer settles).
- **Unhandled rejection at turn end (landed).** `zun_sched`'s `PromiseTable` tracks an
  `unnoticed` set: a promise that settles `Rejected` with **no waiter attached** is marked;
  any later handler attaching (`attach`/`adopt`, i.e. a `.catch`/`.then`/an adopting
  promise) clears it, as does `await` (via `notice`, since `await`'s synchronous fast path
  settles without attaching). At **turn end** — after the microtask drain, when no timer
  remains, in `run_event_loop` — `report_unhandled_rejections` drains the set and emits a
  `UnhandledPromiseRejection: <message>` line through the output sink (each reported once).
  Checked at turn end, not at settle time, so `const p = Promise.reject(x); p.catch(f)` is
  **not** reported. `Promise.reject`/`reject` settlers route through
  `settle_promise_reject`, which bridges the promise so even a fully detached rejection is
  tracked.
- **Tests:** `zun_vm/tests/promise_surface.rs` (end-to-end cases exercising the scheduler
  via `await`: then-chaining, adoption, rejection skip/propagation, catch, finally
  pass-through, all/allSettled/race/any, resolve-of-a-Promise, pending-source reactions,
  `withResolvers` resolve/reject, `any` AggregateError, unhandled-rejection reporting,
  the late-`catch` suppression, and the `new Promise(executor)` constructor — sync resolve,
  reject, executor-throw rejection, async timer-driven adoption, Promise adoption, and the
  zero-arg pending path) + a `PromiseTable` unnoticed-tracking unit test in `zun_sched`.

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
**Update:** `Reflect.construct` and the Proxy `construct` trap landed for class targets
(`ClassDescriptor` + `classDescriptorToken`). Limits: class targets only (no
arbitrary-function constructors; no function.prototype model), `newTarget === target` only,
args must supply the exact field count, `new proxy(...)` only when the proxy is a local
variable. Any "construct deferred" text below is superseded.
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
- **Descriptor-dependent traps (NUCLEUS #10 closure, built on #6's descriptor
  model):** the proxy-aware `object*` natives in `collections.rs` classify the
  receiver via `ProxyRoute` — a proxy with the matching trap invokes it (target
  as leading param, like the other traps); a proxy without it falls back to its
  target through the #6 descriptor machinery; a plain object operates on itself.
  Traps wired: `getOwnPropertyDescriptor` (via `Object.getOwnPropertyDescriptor`/
  `Reflect.getOwnPropertyDescriptor`), `defineProperty` (`Object.defineProperty`/
  `Reflect.defineProperty`), `getPrototypeOf`/`setPrototypeOf`, `isExtensible`/
  `preventExtensions`, and `ownKeys` (via `objectGetOwnPropertyNames`, which
  `Reflect.ownKeys` now routes to so all own keys — not just enumerable — are
  reported).
- **`apply` trap + callable proxies:** `new Proxy(fn, handler)` over a *function*
  target boxes the function into a GC record (`functionBox`) and builds the proxy
  with the new `proxyNewCallable` native, so the stored target stays a GC object.
  A direct call `proxy(...args)` on a `Ty::Proxy` local lowers (`check.rs`) to the
  new `proxyApply(proxy, thisArg, argsArray)` native: an `apply` trap →
  `handler.apply(target, thisArg, args)`; no trap → the underlying boxed function
  with the arguments forwarded verbatim. `Reflect.apply(target, thisArg, args)`
  routes a function target to the existing `functionApply` and a proxy target to
  `proxyApply`. Two natives appended at the end of the `collections` module so
  dispatch ids stay stable.
- **Deferred (documented, not faked):** the `construct` trap and `Reflect.construct`.
  `new` emits `NewObj <TypeDef token>` against a compile-time type; there is no
  runtime constructor value a trap could stand in for (the same "no runtime class
  value" limitation that defers class-decorator return-value replacement), so
  `Reflect.construct` raises a diagnostic rather than pretending to work. Also
  still: the `has`/`deleteProperty` traps are reachable via `Reflect.has`/
  `Reflect.deleteProperty`, the `proxyHas`/`proxyDelete` natives, and the
  `in`/`delete` operators, but `Reflect.has`/`Reflect.deleteProperty` themselves
  do not consult proxy traps (they read the receiver directly) — unchanged here.

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
- **Iteration protocol (landed):** `keys/values/entries` return a real iterator object
  (an array-iterator over the materialized key/value/entry sequence), for-of-able, with
  `next()→{value,done}`; `new Map(userIterable)` now drains an arbitrary user
  `[Symbol.iterator]` object via `drive_iterable` (reentrant Tier-0 protocol calls).

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
- **Iteration protocol (landed):** `keys/values/entries` return a real iterator object
  (for-of-able, `next()→{value,done}`); `new Set(userIterable)` and `[...set]` drain an
  arbitrary user `[Symbol.iterator]` object via the protocol.
- **Missing / deferred:** ES2024 set ops (`union/intersection/difference/isSubsetOf/…`) —
  note Array has these as extras but Set does not.

## 13. WeakMap — done
`builtin/weak_map.zun`; ephemeron-backed GC floor.
- **Present:** `set get has delete` (all verified end to end). Correctly non-iterable.
- **Missing:** nothing. ECMAScript WeakMap has no iteration, no `size`, and no
  `clear` by design (weak keys are not observable), so `set/get/has/delete` is the
  complete surface — this is `done`, not `partial`.

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

## 16. FinalizationRegistry — done
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
- **`heldValue` is now arbitrary:** the GC `FinalizerEntry` / `FinalizationRecord`
  carry a `held: Any` (was `token: u64`). A pending registration's held value is
  marked during collection (so an object held value stays alive) and its object
  address is rewritten on compaction; on collection the record is delivered to the
  cleanup callback as its unboxed runtime value. The surface boxes the argument into
  `Any` (sema) / at the `finalizationRegister` native. The vestigial `builtin/*.zun` class
  mirrors the surface so the module type-checks but is not the dispatch path — the
  checker lowers `FinalizationRegistry` methods directly against the builtin type.

## 17. ArrayBuffer — mostly (transfer landed)
**Update:** length-tracking `DataView` over a resizable buffer landed (see #18); the
"Deferred: length-tracking DataView" note below is superseded.
`Ty::Buffer` is what the realm registers under the name `ArrayBuffer`; the byte
storage lives in `zun_gc/buffers.rs` (`ByteBuffer` in `zun_core`).

- **Present:** `new ArrayBuffer(length)`, `byteLength()`, `slice(begin, end)`,
  the static `ArrayBuffer.isView(x)`, and resizable buffers (see below).
  `slice` copies rather than aliasing, as JS does, and clamps inverted or
  out-of-range bounds to an empty result instead of failing.
- **Resizable buffers (ES2024).** `new ArrayBuffer(n, { maxByteLength })` marks
  the buffer resizable (`ByteBuffer.max: Option<usize>` — `None` is fixed-length,
  unchanged byte-for-byte). `resize(newLength)` grows (zero-filling the new tail)
  or shrinks (preserving the surviving prefix); `maxByteLength()` returns the cap
  (or the current length for a fixed buffer) and `resizable()` the flag. A
  rejected resize (fixed buffer, or `newLength > maxByteLength`) errs as
  `BadToken`, the project convention (not JS `RangeError`).
- **View revalidation = lazy resolve (no back-reference registry).** The prior
  blocker — that a `resize` would have to re-validate every outstanding view —
  is dissolved by not caching an absolute window: a typed-array/DataView view
  re-resolves its window against the buffer's *current* length at access time
  (`TypedArrayView::resolve`/`DataView::resolve`), so a resize touches no views.
  A **length-tracking** typed array (`new Uint8Array(rab)` / `(rab, offset)`, no
  explicit length — a `track` flag, allocated via `typedArrayTrackingNew`)
  follows the new length; a **fixed-length** view whose window no longer fits is
  out-of-bounds — length 0, access fails — exactly as the spec requires. This is
  sound because every access already borrows the backing buffer, so re-reading
  its length costs nothing and no stale window can outlive a resize.
- **`transfer`/`transferToFixedLength` landed:** a `detached: bool` on
  `ByteBuffer` (checked in `resize`, surfaced via `detached()` / the
  `bufferDetached` native). `detach()` takes the bytes + growth cap out, leaving a
  zero-length detached husk — so the source's `byteLength` becomes 0 and every
  outstanding view sees length 0 through the **existing lazy-resolve** path (no new
  per-access check). `buffer_transfer(source, newLength?, fixed)` detaches the
  source and moves the surviving prefix into a fresh buffer: `transfer` keeps the
  source's resizability/cap, `transferToFixedLength` forces a fixed-length result.
  `newLength` defaults to the source length (`-1` sentinel from the surface). A
  detached source, or a destination exceeding a resizable cap, errs as `BadToken`
  (project convention).
- **Deferred:** length-tracking `DataView`
  (constructor still requires an explicit length; fixed DataViews do resolve for
  out-of-bounds), `SharedArrayBuffer` (fine).
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
## 18. DataView — mostly
**Update:** length-tracking `DataView` over a resizable `ArrayBuffer` landed: `byteLength`
and get/set re-resolve against the buffer at access time; a shrunk fixed view now fails.
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
- **Endianness is now optional** and defaults to `false` (big-endian), as the
  spec requires. `view.getUint32(0)` and `view.setFloat64(8, x)` type-check
  and read/write big-endian; the explicit `..., true` little-endian form is
  unchanged. Single-byte widths (`getInt8`/`getUint8`/`setInt8`/`setUint8`)
  take no flag. Lowered by a dedicated interceptor (`check_data_view_method`)
  that pads the default flag, since the generic method dispatch requires an
  exact argument count.
- **Missing:** nothing of the core surface now; `Uint8Clamped` has no DataView
  form in JS either.

## 19. TypedArrays — mostly
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
- **Second-generation surface (landed).** All of the following are now
  present, built as **kind-agnostic** `typedArray*` natives — one native per
  method rather than eleven. The element kind is a runtime property of the
  view for an instance method (read back via `Heap::typed_array_element`) and a
  table-order discriminant argument (`TypedElement::index`/`from_index`) for a
  static, so no per-kind duplication is hand-written. They live on the Tier-0
  interpreter (`call_typed_array_ext` in `zun_interp/collections.rs`) because
  the callback forms re-enter it, so none are JIT-eligible; appended last in
  the `buffer` module so no existing dispatch id shifted.
  - **Bulk `set(source, offset?)`** — the overload with the indexed
    `set(index, value)` is resolved in the checker (`check_typed_array_method`)
    **by the first argument's static type**: an `Array` or typed-array first
    argument is the bulk copy, two numeric arguments are the indexed store.
    Out-of-range reports `false` rather than throwing a RangeError (the whole
    `set` surface here yields a bool, as the indexed form already did). Source
    may be a plain array or any typed array.
  - **`slice(start?, end?)`** — copying (not aliasing), JS clamping, negative
    indices relative to the end. Returns a new view of the same kind.
  - **`copyWithin(target, start?, end?)`** — in place; snapshots the source
    range first so overlapping ranges are correct.
  - **Functional methods** `map`/`filter`/`forEach`/`reduce`/`find`/
    `findIndex`/`some`/`every`, with the spec `(value, index, array)`
    callback (extra args dropped to the callback's arity). `map`/`filter`
    return a new view of the same kind; the view and result are rooted across
    callback re-entry, mirroring the `Array` functional methods.
  - **`indexOf`/`includes`/`join`** — search (`includes` uses SameValueZero so
    NaN matches NaN; `indexOf` strict) and string join.
  - **Statics `from(src, mapFn?)` / `of(...items)`** per kind
    (`check_typed_array_static`): `of` is an array literal copied into a view
    of the kind; `from` copies an array or typed array, optionally through a
    map function.
- **Deferred:** the no-initial-value `reduce` (an initial value is currently
  required; the first-element seed is not wired). `Uint8Array.slice` keeps the
  Node Buffer (shared-type) semantics rather than the copying typed-array one,
  since a `Uint8Array` and a `Buffer` share `Ty::BufferView(U8)` and the Buffer
  intercept claims `slice` first. `BigInt64Array`/`BigUint64Array` carry native
  `i64`/`u64` rather than real BigInts, the same divergence as
  `DataView.getInt64` (#18).

## 20. Date — mostly
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
- **Local-time setters** `setFullYear/setMonth/setDate/setHours/setMinutes/
  setSeconds/setMilliseconds` now exist, mirroring the local getters. Because the
  calendar is UTC-only (local *is* UTC, `getTimezoneOffset` hardcoded 0), each
  lands on exactly the same timestamp as its `setUTC*` counterpart from the same
  instant — this is the correct behaviour in this model, **not a bug**; they will
  diverge from the UTC setters only once a tz offset lands. Each returns the new
  timestamp. They take the ECMAScript optional trailing arguments
  (`setFullYear(y, month?, date?)`, `setMonth(m, date?)`, `setHours(h, min?, sec?,
  ms?)`, `setMinutes(min, sec?, ms?)`, `setSeconds(sec, ms?)`): an omitted
  component keeps its current value, implemented as default parameter expressions
  that read the component back off `this` before the timestamp is rebuilt. No new
  natives — they compose the existing `date.fromUTC` machinery, so Date stays a
  declarative `.zun` surface and AOT dispatch ids are unchanged.
- **Present:** millis storage, `getTime/setTime valueOf isValid
  isBefore/isAfter/isEqual`, the UTC getters and UTC setters, the local getters
  AND local setters, `toISOString toJSON toString`, `getTimezoneOffset`, `new
  Date()`/`new Date(millis)`, and the statics
  `now/parse/UTC/fromMillis/fromISO/fromISODate`.
- **Landed via baked ICU4X:** `toLocaleString`/`toLocaleDateString`/`toLocaleTimeString`
  — `icu` datetime (root `und` locale, medium length, UTC) as `zun:date` natives.
  Follow-up: per-locale argument, `Intl` options, local time zones (the calendar is UTC-only).
- **Missing / deferred:** `new Date(string)` (use `Date.fromISO`);
  `toDateString toTimeString toUTCString`.

## 21. Buffer (Node) — mostly  *(built on the Uint8Array byte-view floor)*

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
  `slice(start?, end?)` (shares memory, via `typed_array_subarray`; **negative
  indices count from the end**, Node/ES semantics),
  `write(string, offset?, encoding?)`, `fill(value)` (single-arg, whole buffer),
  `equals(other)`, `copy(target, targetStart?, sourceStart?, sourceEnd?)`, the
  fixed-width readers/writers `readUInt8/readInt8`, `read{U,}Int16LE/BE`,
  `read{U,}Int32LE/BE`, `readBig{U,}Int64LE/BE` and matching `write*`, the
  **IEEE-754 `readFloatLE/BE`/`readDoubleLE/BE` + `writeFloat*`/`writeDouble*`**,
  the **variable-width `read{U,}IntLE/BE(offset, byteLength)` /
  `write{U,}IntLE/BE(value, offset, byteLength)`** (1..=6 bytes, incl. the
  24/40/48-bit cases), the in-place byte swaps **`swap16`/`swap32`/`swap64`**,
  and the search methods **`indexOf`/`lastIndexOf`/`includes`** (byte-value,
  string, or Buffer needle; optional signed `byteOffset` + `encoding`). All
  assembled from single-byte view access.
- **`subarray`/`fill(v,begin,end)`/`get`/`set`** keep their typed-array
  semantics (generic dispatch) — a Buffer shares its type with a plain
  `Uint8Array`, so only the Buffer-unique method shapes are intercepted; the
  typed-array surface is not regressed.
- **Encodings (present):** `utf8`/`utf-8`, `hex`, `base64` (standard alphabet),
  `base64url` (URL-safe `-`/`_`, unpadded; the decoder is tolerant of both
  alphabets), `ucs2`/`ucs-2`/`utf16le`/`utf-16le` (little-endian 16-bit code
  units), `latin1`/`binary`. Unknown encodings fall back to utf8 (lenient).
- **Error convention.** A `swap*` on a length that is not a multiple of the
  element width, and any out-of-range reader/writer, signal failure with
  `RunError::BadToken` — the same convention every neighbouring buffer op uses —
  rather than a JS `RangeError` (Node's choice); the interpreter floor never
  silently truncates there.
- **Deferred (documented, not faked):** negative `copy` indices (copy still
  clamps, unlike `slice`, which now counts from the end). Because a Buffer and a
  plain `Uint8Array` are the same type here, `isBuffer` answers `true` for any
  `Uint8Array`, the Buffer instance methods (including the richer `indexOf`) are
  reachable on a bare `Uint8Array` too, and `slice` is a shared-memory view
  rather than a copy — the visible consequences of the shared representation.

## 22. Error (+ subclasses) — done
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
- **Now a realm global** (`globalThis.Error`, + the 6 subclasses). A static
  `base_type_facets` entry cannot denote `Ty::Instance(<program-assigned class
  index>)`, so these use a new **`ClassFallThrough`** binding: the registry owns
  only the realm descriptor value (seeded via `global.set(...)` in
  `builtin/realm.zun`), while the checker resolves the name through
  `classes_by_name` at check time for the `: Error` annotation
  (`resolve_annotation`) and for `new Error(...)` (the ordinary class path). The
  classes still ship via the prelude, so `new Error`/`throw new TypeError`
  already worked in-source; this adds the `globalThis` resolution and the uniform
  realm path, and generalizes to **#20 Date** (same mechanism, future work).
- **`AggregateError(errors, message)` now lands**, carrying its `errors` array.
  It is declared **standalone, not `extends Error`**: construction fills fields
  inherited-first, so an `extends Error` subclass would force `message` ahead of
  `errors` and could not express JS's `(errors, message)` order. Declared on its
  own, the field order (`errors`, `message`, then the optional `stackFrames`) is
  ours to choose, so the JS constructor shape and `.stack` both work. The cost is
  that `aggregate instanceof Error` is `false`, and the shared surface
  (`name`/`toString`/`get stack`) is duplicated rather than inherited. Reachable
  by name through the prelude like the other error classes.
- **`cause` landed.** `new Error(message, { cause })` sets `err.cause` to the
  options object's `.cause` value (any type), and `err.cause` is `undefined` when
  no options object / no `cause` key is given. The `(message, options)` shape
  does not fit Zun's one-argument-per-field object model, so Error-family
  construction is lowered specially (`check.rs::check_error_new`): `message` ←
  arg0 (empty string when omitted), `stackFrames` ← its declared initializer, and
  `cause` ← `options.cause` read via `DynamicGet` (an empty synthesized object
  for the no-options case, so the read yields `undefined`). Applies to `Error`
  and the 6 subclasses (they inherit the `message`/`stackFrames`/`cause` layout).
  Construction allocates and reads a dynamic object, so it stays on the Tier-0
  interpreter (the `any` field also keeps the method off the JIT allowlist),
  matching the throwing/dynamic nature of Error values.
  - **The `any`-field carrier (enabler) landed** — see the "frozen-layout `any`
    fields" note below. `cause` is an `any` field; it now carries a boxed value
    (number/string/object/…) through the heap side table instead of being stored
    as zero, and absent-vs-present is a missing side-table entry (`undefined`)
    vs. a stored `Any`.
  - **`err.cause = x` dynamic set landed** — assigning the `cause` field on a typed
    Error instance type-checks (it is a declared `any` field) and stores through the
    any-field carrier (`FieldSet` on `Ty::Instance`; `store_field` boxes into the
    `any_fields` side table), independent of the `(message, options)` construction
    form. Any value round-trips (number/string/object).
- **Frozen-layout `any` fields (the transversal enabler).** A class field is one
  8-byte slot, but a runtime `Any` is 16 bytes (`zun_core::Any` = tag + payload),
  so an `any`-typed field could not be carried: `field_kind_byte(Ty::Any)` mapped
  to `JitKind::Other`, and `encode_field`/`decode_field` stored it as `0` and
  read it back as `undefined` — the value was silently dropped. The fix (PLAN §2:
  dynamic `Any` is the explicit, costly exception) adds a **`FIELD_ANY` kind
  (byte 14, appended after the existing kinds so stored kind bytes stay stable)**
  whose value lives **out of line in a heap side table** — `Heap.any_fields`,
  keyed by `(owner address, field slot)`, the exact analogue of `Heap.arrays`.
  `store_any_field`/`load_any_field` box/unbox the value; the GC traces,
  relocates (compaction), sweeps, and write-barriers it exactly like array
  elements (both mark passes, `rewrite_auxiliary_references`, the sweep removal).
  Absent (no entry) reads as `Value::Void` (`undefined`); a present entry unboxes
  to its concrete value. `NewObj`/`LoadField`/`StoreField` and the typed-object
  enumerator route `FIELD_ANY` slots through `value::load_field`/`store_field`.
  JIT: `FIELD_ANY` classifies as `JitKind::Other`, so `field_jit_ok` already
  excludes any method touching an `any` field — it stays Tier 0, no codegen path.
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

## 25. Decorators — mostly
**Update:** typed `DecoratorContext`, class-decorator `addInitializer`, and field decorators
(run at class-definition with `kind:"field"`) landed. Still deferred (architectural): return-value
replacement (method/class) [D], instance-bound `addInitializer`, the field initializer-transform,
and `context.access` (blocked on function-as-dynamic-arg marshalling, a per-instance
constructor hook, and method-dispatch indirection). Latent soundness caveat: the queued
initializer `Vec<FunctionHandle>` is not GC-rooted between decorator call and drain.
Listed in A ("language feature + macro"). **Stage 1 (syntax)**, **Stage 2 (the
compile-time macro-expansion hook)**, and the **tractable core of Stage 3 (runtime TC39
decorators)** are landed; the Stage-3 remainder (return-value replacement + the rest of
the `context` surface) is deferred with documented reasons.

**Stage 1 — syntax (done).** Lexer `@`, parser accepts `@expr` / `@expr(args)` on
classes and class members, AST nodes carry the decorator list, and sema lowers them to
an inert `hir::Decorator` carry (`crates/zun_sema/src/classes.rs`).

**Stage 2 — macro-expansion hook (done).** The .NET/Roslyn source-generator model
(PLAN §3.5): `crates/zun_compiler/src/macros.rs`. A `class_macro(name)` registry maps a
decorator name to a built-in `ClassMacro` (the single extension point for future
macros). `macros::expand` runs in the driver **after parse, before sema/ilgen**
(`compile_program_with_externals`): for each decorated class it builds a value-model
(`ClassModel`/`FieldModel`, free of AST/compilation references per the Parser/Emitter
discipline), looks the decorator up in the registry, and — if registered — runs the
macro to an emit-model (`MacroExpansion` / `GeneratedMethod`). The pass lowers the
emit-model to AST by interning generated identifiers into a source suffix so every
emitted span resolves in the returned source (clean AST re-emission — **no HIR/module
injection**, so the method-emitting flavor is handled without the codegen-injection
machinery). A decorator with no registered macro stays inert (Stage-1 behavior, never an
error). **Failure contract:** a macro that cannot transform its target returns a
`Diagnostic` (`MACRO0001`); the pass records it and leaves the target unchanged — never a
panic, never a silent miscompile. **Proving macro:** `@autoToString` generates a
`toString()` returning the class name, observable end-to-end at run time
(`crates/zun_vm/tests/decorator_macros.rs`).

**Stage 3 — runtime TC39 decorators, tractable core (done).** A `@decorator` whose
callee resolves to an ordinary top-level function value (not a compile-time macro) is a
runtime TC39 decorator and is now invoked for real, once, at class-definition time.
`crates/zun_sema/src/check.rs` (`append_runtime_decorator_calls` /
`runtime_decorator_call`) emits the invocations into the **module-initializer seam**
(`build_module_initializer`, run before the entry point) — the seam its own doc reserved
for a future `@module_init`. For each decorated **class** and **method** it emits
`dec(value, context)` where `context` is a `DynamicObject` `{ kind: "class"|"method",
name: <string>, static: <bool> }`. **Evaluation order** matches TC39: member decorators
before the class decorator, and each element's decorators bottom-to-top (reverse source
order). **Macro-vs-runtime disambiguation:** macro decorators (e.g. `@autoToString`) are
consumed in Stage 2 and their names are not declared functions, so they never resolve
here; a callee that resolves to a `(any, any)` top-level function takes the runtime path;
anything else (`@sealed`/`@trace`, unknown names) resolves to no function and stays inert
— the registry-free equivalent of "callee ∉ macro registry" (zun_sema cannot depend on
zun_compiler's registry). Proven end-to-end in `crates/zun_vm/tests/end_to_end.rs`
(method decorator runs with correct `kind`/`name`/`static`; class decorator runs with
`kind === "class"`; an undefined decorator stays inert).

**Deferred (Stage-3 remainder, documented).** The decorator's **return value is
ignored** — replacing a method/class with the returned value needs a mutable dispatch
indirection the static-vtable model cannot host without a dispatch-model change (would
also have to be mirrored in the JIT CallVirt path to keep the interp/JIT equivalence
invariant). Delivering a **live `value`** is deferred for the same family of reasons:
`Value::Function` is deliberately excluded from `Any` (so closure environments stay
visible to precise GC roots, see `Value::box_any`) and there is no first-class runtime
class/constructor value, so `value` is `null` in both cases. **Decorator factories
`@dec(args)` now landed** (`runtime_decorator_call`): the factory call `dec(args)` is
evaluated in the module initializer (literal arguments only — no local scope there) and
its result (a `Function` value) is invoked with `(value, context)` via the
`applyDecorator` native. Also deferred: `context.addInitializer`; `context.access`
(get/set); field and accessor decorators; private-element decorators. **Separately**
(not Stage 3):
in-language / user-defined macros (the registry is Rust-only for v1); field/method-level
built-in macros; incremental caching; and a `@sealed`-style in-place flag with runtime
enforcement (needs a TypeDef/metadata-format flag + verifier fixtures + runtime work).

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

## 27. Binary ser/de primitive — done
The ser/de PRIMITIVE (Erlang `term_to_binary`/`binary_to_term`-style), **not** a JSON
API; a future pickle-style object-graph layer and the `@urubucode/common` JSON surface
build on it. Imported from `"zun:core"` as bare names (not globals), exactly like
`mapSet`/`classDescriptorToken`:

```ts
import { serialize, deserialize } from "zun:core";
const bytes = serialize(value);   // -> Uint8Array
const value = deserialize(bytes); // Uint8Array -> reconstructed value
```

### Wire format (`zun_core::serde`)
A version byte (`SERDE_VERSION = 1`) then one tagged value. Tags (CBOR-ish, compact):
`Null, Undefined, False, True, Int` (kind byte + i64 LE), `Float64, Float32, Str`
(uvarint length + UTF-8), `BigInt` (decimal text), `Array, Object` (dynamic object),
`Instance` (typed class), `Map, Set, Date, ArrayBuffer, TypedArray`, and `Ref` (a memo
index). Lengths/counts/tokens/indices are unsigned LEB128 varints; fixed numbers are
little-endian. The format crate owns the contract (version byte + tag list + reader),
dependency-free, so the compiler and both tiers share one definition.

### Shape/field walk (metadata, not reflection)
A **typed class instance** rides on the nucleus shape walk (PLAN §3.2): the engine writes
its metadata **type token** (`type_rid`) then walks its declared instance fields in
layout order (base classes first, `static`/`private` excluded), reusing
`Interp::object_own_entries` — the exact enumeration `Object.keys` /
`getOwnPropertyDescriptor` use — and reads each field via `value::load_field`. A
**dynamic object** walks its own entry map (`heap.dynamic_entries`). TypedArray/Buffer
copy raw bytes via the backing buffer (Buffer round-trips as its `Uint8Array` kind).

### Cycles + sharing
A memo keyed by object identity gives every heap object one index at first sight;
later sights emit a `Ref`. Deserialize keeps a parallel memo and, for a container,
**allocates the shell and records it BEFORE filling children**, so a back-reference —
including a self-cycle (`o.self = o`) or a shared child (`{ x: a, y: a }`) — resolves to
the object under construction, with no infinite recursion.

### Safety model (fixes pickle/BinaryFormatter RCE)
Deserialize reconstructs **only data and metadata-known types**; it never runs code,
invokes a constructor body, or instantiates an arbitrary callable. A **function value**
on serialize, an **unknown tag**, a **truncated stream**, or a **type token that does not
resolve** in the deserializing program (`self.types.get(type_rid) == None`) is a clean
`RunError`, never undefined behavior.

### Approved decisions
Functions are not data (serialize errors on them); type tokens are **program-local** (a
typed object round-trips within the same program — cross-program/versioned identity is a
documented follow-up); output is `Uint8Array`, input a byte view.

### Wiring & tests
Natives `serialize`/`deserialize` live in a new `serde` module appended **last** in
`zun_natives` (no existing dispatch id shifts); the import + call lowering is the generic
bare-name path (no sema change); the engine is `zun_interp::serde` on the Tier-0 floor
(allocates/walks the heap → not JIT-eligible, correct), routed via `builtins.rs`.
`zun_core/src/serde.rs` tests cover the varint/tag codec; `zun_vm/tests/serde.rs` covers
end-to-end round-trips (primitives, int kinds, bigint, nested array, dynamic object,
Map/Set, Date, typed class instance with exact-type reconstruction, ArrayBuffer/
Uint8Array bytes, shared reference, self-cycle) plus the garbage-bytes and
function-value clean-error cases.

### Deferred
Landed since: symbol-keyed dynamic-object entries (registered `Symbol.for` key + well-known
id; unique symbols dropped) and DataView (with the length-tracking flag). Still open:
cross-program/versioned type identity (type tokens are program-local).

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
   user symbol-keyed `obj[sym]` get/set over the existing slot; `Symbol.hasInstance`
   dispatch for `instanceof` on dynamic objects (`dynamicSetSymbolFunction` +
   `instanceofDynamic`). **Remaining (deferred):** `toPrimitive` dispatch (no reachable
   object→primitive coercion site), `toStringTag` dispatch (no reachable object
   stringification site), typed-class `hasInstance`; class computed-key member
   definitions. See §5 for the blocker analysis.

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
     `setFromIterable`/`mapFromIterable`) landed. ✅ **Iteration protocol landed**:
     `keys/values/entries` return real iterator objects; `for..of` / `new Set|Map(x)` /
     `[...x]` drain arbitrary user `[Symbol.iterator]` objects (Tier 0).
   - Promise (#9): ✅ `then/catch/finally` (new-promise + Promise adoption) and statics
     `resolve/reject/all/allSettled/race/any` landed. Remaining: `withResolvers`,
     AggregateError for `any`, microtask deferral of a pending-source reaction.
   - TypedArrays/DataView (#18,#19): ✅ all eleven kinds, `get/set/length/
     byteLength/subarray/fill/byteOffset/buffer` and `[Symbol.iterator]`; **bulk
     `set(source, offset?)` (overload resolved by first-arg type), copying
     `slice`, in-place `copyWithin`, the functional methods
     (`map/filter/forEach/reduce/find/findIndex/some/every`),
     `indexOf/includes/join`, and the statics `from(src, mapFn?)`/`of(...)`** —
     landed as kind-agnostic `typedArray*` natives (Tier 0). DataView every
     width plus `byteOffset/buffer`, and the **endianness flag is now optional
     (defaults big-endian)**. Remaining (deferred): no-initial `reduce`, real
     BigInt elements, `Uint8Array.slice` uses Buffer semantics.
   - BigInt (#4): `div/rem/pow/neg`, bitwise/shift, `BigInt()` coercion, `asIntN/asUintN`.
   - Error (#22): ✅ `.stack` capture, `AggregateError(errors, message)`,
     **`cause` (via the frozen-layout `any`-field carrier)**, and **`Error` + 6
     subclasses as realm globals** (`ClassFallThrough`) landed, and
     **`err.cause = x` dynamic set** on a typed instance. Remaining:
     `aggregate instanceof Error`.
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
  symbol-keyed `obj[sym]` get/set all reach user code. `Symbol.hasInstance` now drives
  `instanceof` for dynamic objects; `toPrimitive`/`toStringTag` dispatch remains blocked
  on a missing coercion / object-stringification site (see §5).

## Remaining architectural forks (open decisions)

These items are not done because each needs a core design decision, not more surface work.

1. **`new Function` / `Function(src)`.** Needs ephemeral-module linking into the live isolate
   (the interpreter is bound to one immutable `&Module`). Options: (1) append/link the
   ephemeral module into the live module with token rewrite + re-verify, using an
   `Arc`/append-only module; (2) a multi-module isolate with a module-id-carrying
   `FunctionHandle`. The `EvalHost` injection design is settled: dev/JIT injects the compiler;
   AOT injects a throwing "code generation from strings disallowed" stub.

2. **Decorator return-value replacement, instance `addInitializer`, field transform, `access`.**
   Return-value replacement (method/class) [D] plus the rest need function-as-dynamic-arg
   marshalling (`functionBox`), a per-instance constructor init hook, and per-class
   method-dispatch indirection (trampoline/side-table) that stays in sync across the
   interp/JIT equivalence invariant.

3. **Multi-argument bound calls / functions inside `apply` args.** The dynamic `CallIndirect`
   IL carries a single trailing argument; multi-arg needs an opcode-set + verifier + codegen
   change.

4. **serde cross-program type identity.** Type tokens are program-local; a cross-program wire
   identity needs a design (qualified-name vs structural hash).

5. **Date local time zones.** The calendar is UTC-only; real local TZ needs a host-timezone
   source (a dependency) and breaks the UTC-only model; deterministic testing needs a fixed TZ.
