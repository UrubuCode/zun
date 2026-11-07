# Object + Proxy + Reflect - optimization audit

Read-only audit; claims are verified against code at the cited file:line (paths relative to `crates/`). Items marked "not confirmed" are hypotheses (no profiling was run).

## Summary
- Dynamic objects live in a GC side table `HashMap<usize, DynamicObject>` (default SipHash) and each `DynamicObject` carries `Vec`, `HashMap`, 4 `HashSet<String>` and 3+ `BTreeMap<String,_>`; every property read/write pays 2+ hashes plus several attribute-set probes and String allocations.
- `Object.keys/values/entries/assign` build `Vec<String>` / `Vec<(String, Value)>` and re-query the heap 3-5 times per key before allocating the result array; no dynamic-object opcode is JIT-lowered, so any function touching `any` objects stays on Tier 0.
- Cheap wins: single-probe own-data fast path, stop `to_owned()`-ing the property name per access, stop allocating a function per `o.hasOwnProperty` read, O(1) `freeze`, non-O(n) `delete`.

## Per-method table

| method | current path (file:line) | current complexity | possible complexity | allocs/call | JIT eligible/lowered | severity |
|---|---|---|---|---|---|---|
| dyn get `o.k` (own data) | zun_interp/src/dyn_member.rs:13 `op_load_dynamic`; collections/runtime_iter.rs:636 `dynamic_member_get`; zun_core/src/dynamic_object.rs:172 | O(1) but ~6 hash probes (object table, getter probe :654, `dynamic_get` :660 each with `accessors`/`string_functions`/`index` checks) | 1 table lookup + 1 key lookup | 1 `String` (name `to_owned`, dyn_member.rs:22) | no (not in `opcode_supported`, zun_codegen/src/eligible.rs:18) | high |
| dyn set `o.k = v` | dyn_member.rs:390 `op_store_dynamic`; zun_gc/src/dynamic.rs:26; dynamic_object.rs:185 | O(1), ~4 probes (index, accessors, non_writable, object table) | 1 slot lookup + flag byte | 2 `String` (dyn_member.rs:394, dynamic_object.rs:186 `name.into()`) | no | high |
| dyn get `o[k]` | dyn_member.rs:604 `op_load_dynamic_value` | O(1) but receiver `unbox_any` repeated 3+ times; string index is O(i) | decode once | 0-1 | no | medium |
| `Object.keys` | zun_interp/src/collections/object_core.rs:97; runtime_descriptor.rs:45 `object_own_entries_impl`; zun_gc/src/dynamic.rs:510 | O(n) but 3 probes/key (`dynamic_is_enumerable`, `dynamic_accessor_getter`, `dynamic_get`, runtime_descriptor.rs:121-125) even when `read_values=false` | O(n), one pass | `Vec<String>`, `Vec<(String,Value)>`, key cloned again for `Value::Str`+`box_any`, result array | no | high |
| `Object.values` | object_core.rs:141 | same + getter call + `unbox_any`/`box_any` per value | O(n), raw `Any` copy | as keys + per-value box | no | high |
| `Object.entries` | object_core.rs:192 | same + pair array per entry | O(n) | + 1 `alloc_array(2)` + key String per entry | no | high |
| `Object.assign` | object_core.rs:253 | O(n); 3 source probes/key (:317-322) + 5 target probes/key (:362-370) + `dynamic_set` | O(n), single flag check, bulk copy | keys Vec, owned key String per `dynamic_set`, function-entries Vec (:312) | no | high |
| `Object.freeze` | object_core.rs:581 -> dynamic_object.rs:442 | O(n): clones all keys, inserts into 1-2 HashSets | O(1) flag-only (a `frozen` bool already gates `set`/`remove`) | n String clones x2 | no | medium |
| `Object.isFrozen` | object_core.rs:599; dynamic_object.rs:428 | O(1) if flagged, else O(n) with 2 probes/key | O(1) | 0 | no | low |
| `Object.create` | object_core.rs:614 | O(1) | O(1) | new side-table entry | no | low |
| `Object.getPrototypeOf` | object_core.rs:626 (+ `proxy_route`) | O(1) | O(1) | 0-1 | no | low |
| `defineProperty` / descriptor | object_core/runtime_descriptor `define_own_property` (object_core.rs:588); dynamic_object.rs:225 `define_data`, :381 `descriptor` | O(1), 4-6 set ops | O(1) | 2-3 String, descriptor object alloc | no | low |
| `Object.fromEntries` | collections/object_descriptor.rs:136 | O(n) | O(n) | per-entry String + box | no | low-medium |
| `o.hasOwnProperty` read | dyn_member.rs:352-381 (dup at :949-963) | O(1) + linear 6-string `position()` scan | O(1) | 1 function object (`make_promise_settler`) per READ | no | medium |
| Proxy get trap | runtime_iter.rs:690 `proxy_dynamic_get`; zun_gc/src/proxies.rs:35 | trap lookup via `dynamic_get_function` (BTreeMap) per access | O(1) cached | `Vec<Value>`(3) + key String + result box | no | medium |
| `Reflect.has` | collections/reflect.rs:15-70 | linear `contains` over 12-16 names | O(1) match | 1 String | no | low |
| other `Reflect.*`, Proxy apply/construct | not examined line-by-line | - | - | - | no | not confirmed |

## Findings

### F1. DynamicObject storage: many String-keyed collections, SipHash side table
- Evidence: zun_core/src/dynamic_object.rs:27-65 (`properties: Vec<(String, Any)>`, `index: HashMap<String,usize>`, `non_writable/non_enumerable/non_configurable/accessors: HashSet<String>`, `accessor_get/accessor_set/string_functions: BTreeMap<String,..>`); zun_gc/src/lib.rs:185 `dynamic_objects: HashMap<usize, DynamicObject>` with `std::collections::HashMap` (lib.rs:30).
- Why: `get` (dynamic_object.rs:172-176) probes `accessors` and `string_functions` before `index`; `set` (:185) allocates a String, probes `index`, `accessors`, `non_writable`; `descriptor` (:381) does 4+ probes. Each key is stored in `properties` and `index`.
- Proposal: (a) fast hasher (FxHash/ahash) for the side table and inner maps; (b) per-property flags byte stored in the property entry so one probe yields value + writable/enumerable/configurable/accessor; (c) intern keys (the `LoadDynamic` operand is already a string-table index) and key `index` by interned id; this is also the prerequisite for shapes/inline caches (F6).
- Est. gain: 2-4x on dynamic get/set microbenchmarks (not measured; not confirmed).
- Risk: medium. Touches DynamicObject layout (requires approval); `object_references`/`rewrite_object_references` (dynamic_object.rs:663,700) must follow; no .zm/ABI change.

### F2. Property name `to_owned()` on every dynamic load/store
- Evidence: dyn_member.rs:22 `let name = self.dynamic_name(&instr.operand)?.to_owned();` and :394 (same in `op_store_dynamic`); `dynamic_name` (metadata.rs:179) already returns `&str` from the module string table.
- Why: one heap String per dynamic access in Tier 0, used only to compare against "length"/"name" and pass as `&str`.
- Proposal: borrow instead (resolve operand to an index / split borrow of module strings), allocate only on the insert path; let existing-key set take `&str`.
- Est. gain: 1-2 mallocs removed per access (~10-25%, not confirmed). Risk: low. Layout/.zm/ABI/GC: none.

### F3. keys/values/entries/assign: 3-5 heap lookups per key and intermediate Vecs
- Evidence: runtime_descriptor.rs:118-137 (`dynamic_keys` -> Vec<String>, then per key `dynamic_is_enumerable`, `dynamic_accessor_getter`, `dynamic_get`, `dynamic_is_accessor`, each re-doing `dynamic_objects.get(&addr)`); `data` is fetched before the `!read_values` early-out (:121-130); objectKeys (object_core.rs:123-135) clones each key again for `Value::Str` + `box_any`; objectAssign :317-322 (3 source probes) and :362-370 (5 target probes) per key.
- Proposal: add `DynamicObject::for_each_own_enumerable(|key, any, is_accessor|)` / snapshot returning `(&str, Any, flags)`; one table lookup; build result arrays from `Any` directly; `Object.keys` skips value work; `Object.assign` plain->plain with no accessors and an extensible target uses a bulk path (check flags once). Keep existing path as fallback for accessors/Proxy/typed objects.
- Est. gain: 3-6x on large objects (not confirmed). Risk: low-medium (getter/setter ordering and exceptions). Layout: no (API addition); .zm/ABI: no; GC: result array alloc must not hold heap borrows across getters.

### F4. `o.hasOwnProperty` (and 5 siblings) allocate a function per read
- Evidence: dyn_member.rs:352-381: when a read yields `Void` for `toString|toLocaleString|valueOf|hasOwnProperty|isPrototypeOf|propertyIsEnumerable`, a linear `position()` over the names runs and `make_promise_settler(object, mode)` creates a new function object each time; duplicated at :949-963.
- Why: `o.hasOwnProperty(k)` in hot loops allocates a closure per call and adds GC pressure.
- Proposal: lower the call to a native intrinsic in sema when the receiver is any/object (like `objectKeys`), or cache a shared function per mode and bind `this` at call time; replace `matches!` + `position` with one `match`.
- Est. gain: 1 alloc + GC pressure per call. Risk: low-medium (this-binding, `.call` interplay). No layout/.zm/ABI change on the cached-function route.

### F5. `remove` (delete) is O(n); `freeze` clones all keys
- Evidence: dynamic_object.rs:645-649 `Vec::remove(position)` then `for slot in self.index.values_mut() { if *slot > position { *slot -= 1; } }`. `freeze()` (:442-452) clones all names into a `Vec<String>` and inserts each into `non_writable`/`non_configurable`.
- Why: delete in a loop is O(n^2); freeze is O(n) allocations although the `frozen` flag already short-circuits `set` (:190) and `remove` (:635).
- Proposal: tombstone + lazy compaction for delete; flag-only freeze with lazily derived attributes (`is_writable`/`descriptor`, :357-399, do not currently consult `frozen`, so they must be updated; whether all readers stay correct is not confirmed).
- Est. gain: delete O(1) amortized, freeze O(1). Risk: medium (own-key ordering, descriptor correctness). Touches DynamicObject layout (approval); not .zm/ABI.

### F6. No JIT lowering / inline caches for dynamic property ops
- Evidence: zun_codegen/src/eligible.rs:18 `opcode_supported` whitelists `LoadField/StoreField/CallVirt/CallIndirect/BoxAny/UnboxAny` etc. but none of the Dynamic load/store opcodes; grep of "Dynamic" in zun_codegen/src finds only comments (eligible.rs:135, :1270-1273). docs/guides/cranelift.md contains no inline-cache section (not found by grep).
- Why: any function with a dynamic access is Tier-0 only, and Tier 0 pays F1/F2 every time.
- Proposal: (1) lower to a thunk call (`zun_jit_dyn_get/set(obj, name_idx)`) via the existing exception bridge so mixed functions become eligible; (2) later per-site inline caches keyed by a shape id, requiring DynamicObject to carry a shape (F1 prerequisite); miss path in a cold block per CLAUDE.md.
- Est. gain: large for object-heavy `any` code (not confirmed). Risk: high (opcode via `define_opcodes!`, verifier fixtures, interp/JIT equivalence suite). Touches DynamicObject layout (approval) and GC if shapes are heap-pointed; .zm only if IC slots enter the IL.

### F7. Repeated receiver unboxing in `op_load_dynamic_value`
- Evidence: dyn_member.rs:628-632, :647-652 and the numeric-receiver check each re-run `with_current_heap(|heap| Value::unbox_any(..))` on the receiver; string index uses `wtf16::encode(text).nth(at)` (O(i), :672-677).
- Proposal: unbox once into a local; ASCII/typed charAt fast path for string index. Est. gain 10-30% on `o[k]` (not confirmed). Risk low; no layout/.zm/ABI/GC.

### F8. Proxy traps: trap lookup, arg Vec and boxing per access
- Evidence: runtime_iter.rs:698-709 (`proxy_trap` = proxy handler lookup + `dynamic_get_function` BTreeMap lookup; `vec![Object, Str(key.into()), Object]`), result boxed at :716-720; same pattern for set/has/delete.
- Proposal: cache trap handles on the proxy record, invalidate on handler mutation; reuse a small arg buffer. Est. gain ~20-30% (not confirmed). Risk low-medium. Touches proxy record in zun_gc/src/proxies.rs only.

### F9. Reflect.has / Object.create minor costs
- `Reflect.has` (reflect.rs:20-70) allocates `name.to_string()` and linearly scans static name slices; replace with a `match`. `Object.create` (object_core.rs:614-620) allocates a full `DynamicObject` side-table entry; whether this matters is not confirmed.

## Cross-module references
Shared-doc candidates (see final report): native builtin dispatch by string name (`call_managed_collection` `match name`, collections/mod.rs:608-632), DynamicObject layout/key storage (F1), `box_any`/`unbox_any` round trips (Any marshaling), absence of inline-cache infrastructure (F6).
