# Shared / cross-cutting performance issues

Issues reported here affect many builtin modules at once. The per-module files
reference these entries by their `S#` tag instead of repeating them. Severity is
"systemic": each one taxes a broad swath of the nucleus surface, so fixing one
helps every module.

Verified by code reading only; no profiling or builds were run. Gain figures are
estimates unless marked "measured" (none are, yet). Items that would change a
frozen layout, the `.zm` format, the native-table ABI, or GC behaviour are tagged
**requires approval**.

---

## S1 — Native dispatch is name-string based, two levels deep
**Where:** `zun_interp/src/builtins.rs` (`call_builtin` pops a `Vec<Value>`, resolves
id→name through a thread-local `HashMap`), then `zun_interp/src/collections/mod.rs`
(`call_managed_collection[_inner]` tries `typed_arrays::call`, `node_buffer::call`,
`call_typed_array_ext`, … in sequence, each doing its own `match name`, then the
per-group `col_*` functions `match name` a **second** time).
**Cost:** every managed builtin call pays an id→name map lookup + several failed
string-match chains + a nested string match. Reported independently by String,
Array, Map/Set, Promise, Number, Buffer, Math, TypedArray, Object, Misc.
**Fix:** resolve the native id to a handler (fn-pointer / enum) **once at load
time**; dispatch O(1) by id. No layout/.zm/ABI change (table stays append-only).
**Risk:** low. **Est. gain:** a fixed per-call overhead removed from the hottest
path in the interpreter.

## S2 — JIT eligibility & lowering are parallel hand-maintained name lists
**Where:** `zun_codegen/src/eligible.rs` (per-module name whitelists:
`math_builtin`, `collection_builtin`, `date_builtin`, string lists at ~`:286`/`:543`),
`zun_codegen/src/translate/builtins_table.rs`, the per-kind thunks, and
`translate/*.rs`. The same ~25 builtin names are hand-copied across four places.
**Cost:** (a) a name present in one list but missing from another silently makes a
function fall back to Tier 0 — **one** non-whitelisted `CallBuiltin` disqualifies
the whole function; (b) the duplication conflicts with the CLAUDE.md
"one master table" invariant. Reported by every JIT-relevant agent.
**Fix:** generate the eligibility list, the table rows, and the thunks from one
master table (the pattern `typed_element_table!` already uses). Add a census test
that flags any `NativeFn` with only JIT value-type params that is missing from the
whitelist (the ignored census in `zun_codegen` is the seed).
**Risk:** low-medium (mechanical). **Est. gain:** removes a class of silent
Tier-0 fallbacks; prerequisite for most per-module JIT findings.

## S3 — Heap objects live in address-keyed side tables (SipHash) + GC rehash
**Where:** `zun_gc/src/lib.rs` — `arrays`, `strings`, `typed_views`, `buffers`,
`maps`/`sets`, `dynamic_objects`, `promises`, `bigints`, weak tables are all
`HashMap<usize, _>` keyed by object address with the default SipHash. On every GC
relocation pass these maps are drained and rebuilt (`typed_views` ~`:787`, data
views ~`:949`, `pid_by_obj` in `state.rs:315`).
**Cost:** every element get/set/length, every Map/Set op, every dynamic property
access, every typed-array access pays ≥1 SipHash lookup + a `with_current_heap`
TLS borrow. Multi-lookup cases: typed arrays do two (`typed_views` then `buffers`);
iterators do two per step. GC cost scales with total live object count.
**Fix (cheap, no approval):** swap SipHash for a fast integer hasher (addresses are
already well-distributed pointers); add bulk-access APIs (`with_array(obj, &[Any])`,
memcpy/fill/reverse on the byte store) so natives stop going through the map per
element.
**Fix (big, requires approval):** store the metadata/elements pointer in the object
header so hot paths (and the JIT) skip the map entirely; GC `object_references` /
`rewrite_object_references` must follow. **Touches frozen layout + GC.**
**Risk:** hasher swap = low; inline storage = high. **Est. gain:** large — this is
the per-access tax under Array, TypedArray, Buffer, Map/Set and Object.

## S4 — `Any` box/unbox marshaling + NativeArg/NativeValue copies
**Where:** `Opaque`/`Any` params force `box_any` at the call site and `unbox_arg`
in the callee; `CallBuiltin` builds `Vec<Value>` then `Vec<NativeArg>`; a native's
`NativeValue::Str(String)` result is re-interned (`Rc::from` copy + `intern_string`
lookup — ~2 copies per string-returning native). `box_any`/`unbox_any` each take the
`with_current_heap` TLS.
**Cost:** per-call allocation + copy + TLS, worst in per-element callback loops.
Reported by String, Number, Math, TypedArray, Map/Set, Misc.
**Fix:** numeric fast paths that skip boxing for typed `int`/`f64` args; let
string-returning natives hand back an already-interned handle; a `Value`
accumulator for `reduce` instead of a GC holder array.
**Risk:** low-medium. A function-handle `Any` tag (see S8) is the deeper fix for
the function cases. **Est. gain:** medium, broad.

## S5 — String storage is UTF-8 `Rc<str>` with no length/ASCII cache
**Where:** `zun_core/src/wtf16.rs` (`len16` is an O(n) `chars()` scan; lone
surrogates stored as plane-15 scalars), string heap table. No per-handle
`(len16, is_ascii)`.
**Cost:** `String.length` and every UTF-16 index (`charCodeAt`, `charAt`, `s[i]`,
`slice`, `at`, `substring`, `indexOf`) are O(n)/O(i); `for (i<len) s[i]` is O(n²) in
both tiers. RegExp index↔byte conversion is O(n) per match. Buffer utf16le/latin1/hex
and JSON escaping round-trip through this. Root cause of String F1, RegExp F2,
Buffer F5/F6.
**Fix (cheap):** ASCII byte-slice fast paths (cuts the constant, no layout change).
**Fix (big, requires approval):** cache `(len16, is_ascii)` per interned string, or
keep a UTF-16 side buffer in the heap string table — O(1) length/index. **Touches
GC and possibly the frozen String layout.**
**Risk:** fast path = low; cached length = medium/high. **Est. gain:** large for
string-indexing workloads.

## S6 — `invoke_callback` re-entry cost, paid per element
**Where:** `zun_interp/src/collections/runtime_typed_array.rs` (`invoke_callback` ~`:500`)
and the Promise/timer path. Per invocation: `resolve_call_target`, `read_body`
(re-decodes the method body header), a `name.ends_with("@rest>")` string compare,
`settler_of` string-keyed `dynamic_get`s for bound handles, and a fresh
`Vec<Value>` of args padded with `undefined`.
**Cost:** O(1) work that becomes O(n) across `map`/`filter`/`forEach`/`reduce`/`some`/
`every`/`sort` comparators on Array, TypedArray, Map/Set, String.replace, and every
Promise reaction / timer. Reported by Array, TypedArray, Promise, Map/Set.
**Fix:** cache the decoded body + resolved target per handle; precompute the
rest-arg flag; reuse a scratch arg buffer.
**Risk:** low-medium. **Est. gain:** medium-large on callback-heavy code.

## S7 — `object_own_entries` field-walk allocates a name `String` per field
**Where:** `zun_interp/src/collections/object_core.rs` (`object_own_entries` builds
`(key, Value)` pairs; keys are then discarded or re-fetched by name). A typed
instance does a linear field-name scan with a `format!("get_{name}")` fallback.
**Cost:** shared by `Object.keys/values/entries/assign`, `JSON.stringify`, and
`serde`. Each consumer re-queries the heap by name after enumeration.
**Fix:** a key-only / visitor-style own-enumeration API, plus a per-`type_rid`
cached `(slot, kind)` shape for typed instances. Enumeration order + enumerable
filter must stay byte-identical. No layout/.zm change.
**Risk:** low-medium. **Est. gain:** medium (Object, JSON, serde).

## S8 — Function values can't ride the `Any` lane → `functionBox` wrappers
**Where:** bind / apply / spread sites allocate a `functionBox` GC object and
`unbox_function_box` it back; function-value captures into closures use a dedicated
slot.
**Cost:** per-call allocation + unwrap on the function-argument paths.
**Fix (requires approval):** an `AnyTag::Function` so a function handle rides `Any`
directly. **Touches the `Any` ABI** (and GC root precision — the reason it was
excluded). **Risk:** high. **Est. gain:** medium, narrow (bind/apply-heavy code).

## S9 — ICU4X formatters rebuilt on every call
**Where:** `Date` `toLocale*` + `Intl.DateTimeFormat` (`date.rs:178`, `:419-427`),
`Number.toLocaleString` (`number.rs:405`, `:444`), String `toLocale*`/`localeCompare`.
Each call does `DateTimeFormatter::try_new` / `DecimalFormatter::new` + locale parse.
**Cost:** formatter construction dominates the actual formatting.
**Fix:** cache formatters keyed by (locale, kind) in a thread-local. **Risk:** low
(thread-local, formatters are immutable). **Est. gain:** large on locale-formatting
loops.

## S10 — Codegen redeclares `FuncRef`/`SigRef` imports per use
**Where:** `zun_codegen/src/translate/arrays.rs:156-444` — the `array_*_fref`
helpers call `declare_function` + `declare_func_in_func` on every use; no cache
(`grep fref_cache` is empty). Other `translate/*` modules likely do the same
(not confirmed).
**Cost:** violates the CLAUDE.md FuncRef-dedup rule (reference §16); redundant
import churn during lowering.
**Fix:** cache a `FuncRef` per `FuncId` per function. No layout/.zm/ABI change.
**Risk:** low. **Est. gain:** compile-time / codegen-size (not runtime-hot).
