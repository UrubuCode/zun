# Map / Set performance audit

## Summary
- Core table (`OrderedMap`) is O(1) open addressing and every typed-key op (I32/I64/F64) has an interp arm and a JIT thunk; the real costs are the unmixed float hash, per-call heap-side `HashMap` lookups, and O(n) snapshot iterators.
- Untyped/string-key ops (`mapSet/mapHas/mapDelete/setAdd/setHas/setDelete`), `*ForEach` and iterable constructors are not in the JIT allowlist, so any function using them stays Tier0.
- Static, read-only audit (no profiling run): claims are code-verified; hypotheses are marked "not confirmed".

## Per-method table
| method | current path (file:line) | current complexity | possible | allocs/call | JIT eligible/lowered | severity |
|---|---|---|---|---|---|---|
| new Map()/Set() | collections/mapset.rs:12 `mapNew`, :372 `setNew`; zun_gc/maps.rs:4 | O(1) | O(1) | obj + OrderedMap (2 Vecs of 8) + side-table HashMap insert | yes (`zun_jit_map_new`, thunks/collections.rs:16) | low |
| new Map(array) / new Set(array) | mapset.rs:16 `mapFrom`, :376 `setFrom` | O(n) | O(n) | 3-4 TLS heap borrows per element, no reserve | no (absent from eligible.rs:329-359) | medium |
| new Map/Set(iterable) | mapset.rs:394 `setFromIterable`, :481 `mapFromIterable` | O(n) | O(n) | snapshot Vec + rooted arrays | no | low-med |
| set(k,v) typed I32/I64/F64 | mapset.rs:75-131; maps.rs:12 | O(1) avg; F64 degenerates (F1) | O(1) | 0 | yes (thunks :172, :303) | high for F64 (F1) |
| set(k,v) any/string | mapset.rs:48 `mapSet`, :57 `mapSetChain` | O(1) avg + O(len) string hash | O(1) | recv `Value` clone in Chain (:58) | NO | high (F3) |
| get typed `*Or` | mapset.rs:171-223 | O(1); F64 degenerates | O(1) | 0 | yes (thunks :187, :312) | F1 |
| get any | mapset.rs:267; thunk thunks/collections.rs:199 | O(1) | O(1) | 0 (JitAnyAbi out-param) | yes | low |
| has typed | mapset.rs:133-169 | O(1); F64 degenerates | O(1) | 0 | yes | F1 |
| has any | mapset.rs:286 | O(1) | O(1) | 0 | NO | medium (F3) |
| delete typed / any | mapset.rs:225-266, :304; ordered_map.rs:84 | O(1) + amortized compaction | O(1) | 0 (compaction reallocs) | typed yes; any NO | medium |
| size | mapset.rs:295 | O(1) | O(1) inline load | 0 | yes (full call) | low (F4) |
| clear | mapset.rs:312; ordered_map.rs:167 | O(n) drop + 2 new Vecs | O(1)-ish | 2 Vec allocs | yes | low |
| keys/values/entries (Map) | mapset.rs:319-360 | O(n) eager snapshot | O(1) setup, live cursor | Vec of entries + array + (entries: n 2-elem arrays) + iterator | yes (thunks :25-72, same snapshot) | high (F2) |
| values (Set) | mapset.rs:758 (live cursor) | O(1) | O(1) | iterator obj | yes | low |
| entries (Set) | mapset.rs:767 | O(n) snapshot + n pair arrays | O(1) | n+2 | yes | medium (F2) |
| forEach (Map/Set) | collections/mod.rs:120 `collection_for_each` | O(n), live cursor | O(n) | `vec![Value x3]` per element (mod.rs:140-144) | NO | medium (F5) |
| add/has/delete typed Set | mapset.rs:596+, :686-720 | O(1); F64 degenerates | O(1) | 0 | yes | F1 |
| add/has/delete any Set | mapset.rs:586, :722, :741 | O(1) | O(1) | 0 | NO | medium (F3) |
| `OrderedMap::entry_at` | zun_core/ordered_map.rs:132 | O(n) | n/a | iterator | n/a (no caller found outside file) | low |

## Findings

### F1 Float (and power-of-two int) key hash has no mixing (high)
- Evidence: zun_gc/src/hashing.rs:19 `canonical.to_bits() ^ 0x22`; zun_core/src/ordered_map.rs:223 `let mut slot = hash as usize & mask;` with linear probe `slot = (slot + 1) & mask` (:233). Int hash is `value as u64 ^ 0x11` (hashing.rs:7).
- Why: the bucket index uses only the low bits of the hash. Doubles holding small integral values (1.0, 2.0, 3.0 ...) have all-zero low mantissa bits, so they land in the same bucket and form one probe chain: O(n) per op, O(n^2) to build. The typed floor defaults to f64 and the JIT has dedicated F64 thunks, so this is the common typed case. Exact slowdown not measured (not confirmed); the mechanism is visible in code. Ints that are multiples of a power of two cluster similarly.
- Proposal: apply a 64-bit finalizer (e.g. `h = (h ^ (h >> 32)).wrapping_mul(0x9E3779B97F4A7C15); h ^ (h >> 29)`) in `any_hash` or inside `OrderedMap` before masking. The hash is not observable; iteration is insertion-ordered.
- Gain: O(n) -> O(1) per op for integral float keys; large above ~1k keys.
- Risk: low. Frozen layout/.zm/ABI/GC: no.

### F2 Map.keys/values/entries (and Set.entries) are eager O(n) snapshots (high)
- Evidence: mapset.rs:322 `heap.map_entries(object)` (collects a Vec, maps.rs:88) then `alloc_array(entries.len())`, one `with_current_heap_mut` per `array_set` (:326, :352), then `alloc_array_iterator` (:331). `entries` allocates one 2-element array per entry (:346). JIT thunks repeat the same (thunks/collections.rs:25-72). Contrast `setValues` which uses a live `alloc_set_iterator` (mapset.rs:763), and zun_core/collection_iterators.rs:15 already has a `physical_next` cursor for maps.
- Why: `map.keys().next()` or `for..of` with early break costs O(n) + 2-3 allocations. Snapshot also deviates from JS live-iteration semantics (entries added during iteration must be visited); not verified against Node by test (not confirmed).
- Proposal: use the cursor iterator for Map keys/values/entries; allocate the `[k,v]` pair lazily at `next()`. JIT thunks keep working by calling the same constructor.
- Gain: O(n) -> O(1) setup, no snapshot Vec; entries drops n small allocs for consumers that destructure immediately (needs escape handling, not confirmed).
- Risk: medium. Cursor tracing in GC already exists for Set; no .zm/ABI change. May touch GC only if a new iterator object kind is added.

### F3 Untyped/string-key ops, forEach and constructors are outside the JIT allowlist (high)
- Evidence: codegen/src/eligible.rs:329-359 lists only typed `*I32/I64/F64`, `mapGet`, size/clear/keys/values/entries, `setValues/setEntries`. Absent: `mapSet`, `mapSetChain`, `mapHas`, `mapDelete`, `setAdd`, `setAddChain`, `setHas`, `setDelete`, `mapForEach`, `setForEach`, `mapFrom`, `setFrom`, `*FromIterable`. The interpreter implements all of them (mapset.rs).
- Why: string-keyed Maps (word counts, caches) have no JIT path; one such call makes the whole function Tier0 (eligibility is per function, see project JIT-eligibility notes).
- Proposal: add `zun_jit_map_set_any / map_has_any / map_delete_any / set_add_any / set_has_any / set_delete_any` thunks following the `zun_jit_map_get_any` pattern (thunks/collections.rs:199, tag+payload `JitAnyAbi`) and extend eligible.rs + translate/builtins_table.rs together (eligible iff lowerable). How string keys are carried as Any in JIT code and their GC rooting must be checked (not confirmed).
- Gain: removes the Tier0 fallback for the whole containing function.
- Risk: medium (write barrier / safepoint around `map_insert`). No format change.

### F4 Per-op overhead around the table (medium)
- Evidence: maps live in `HashMap<usize, OrderedMap>` keyed by object address (zun_gc/lib.rs:215, sets :217); each op does `self.maps.get(&addr)` (maps.rs:44) plus a thread-local `with_current_heap` borrow. `size` is a full thunk call (`zun_jit_map_size`, thunks :11) for a field read. `OrderedMap::new` allocates two Vecs of 8 (ordered_map.rs:30-31); `tombstones` is a second bucket-sized array of `Option<(Any,u64)>`, and `entries` holds `Option<(Any,Any,u64)>`.
- Why: default SipHash on a `usize` per op adds hashing on top of the key hash (magnitude not measured; not confirmed). Memory per empty Map and per entry is high.
- Proposal: switch side tables to an identity/Fx hasher (no layout change); allocate buckets lazily on first insert; longer term hold the table pointer in the object (frozen layout: requires approval).
- Gain: constant factor per op. Risk: low (hasher) / high (layout). GC: relocation of side tables via `rewrite_object_references` (ordered_map.rs:112) must stay correct.

### F5 forEach: allocation and TLS borrow per element (medium)
- Evidence: mod.rs:132-149: per element one `with_current_heap` for `collection_cursor`, a fresh `vec![Value::Any(..) x3]` (:140-144), then `invoke_callback`. Deleted slots are skipped with `continue` (:138), each still paying a borrow.
- Proposal: reuse one args buffer, advance past tombstones inside a single borrow (`physical_next` exists, ordered_map.rs:140). Not JIT-callable anyway (F3).
- Gain: one allocation per element. Risk: low. No layout/ABI change.

### F6 Name-string dispatch in the interpreter (cross-module)
- Evidence: collections/mod.rs:311-357 `match name { "mapNew" => self.col_mapset(name, args) ...}` followed by a second `match name` at mapset.rs:11. Whether the natives table resolves id -> function pointer earlier is not confirmed. Referenced in the cross-cutting report.

### F7 Smaller items
- `remove` rescans buckets to place a tombstone (ordered_map.rs:89-95): O(probe), fine. Compaction condition (`dead_len >= 8 && dead_len > live_len`, :99) is amortized O(1).
- `clear` while pinned does `entries.resize(slots)` (ordered_map.rs:170-172): O(slots) each time.
- `mapFrom`/`setFrom` have no capacity reservation; `OrderedMap` has no `with_capacity`, so growth rehashes repeatedly at 75% load (`rehash`, :186). Proposal: `reserve(n)`. Low risk.
- Typed `mapSetI32/I64/F64` return `is_none()` (new-key flag) as Bool (mapset.rs:86-91) where JS `Map.set` returns the map; whether sema discards or reuses it is not confirmed.
