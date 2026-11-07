# Misc built-ins optimization audit

Scope: Symbol, BigInt, Error family, WeakMap/WeakSet/WeakRef/FinalizationRegistry, Function (call/apply/bind), serialize/deserialize (#27). Read-only audit; facts are verified in code, hypotheses are marked "not confirmed". Paths are relative to `crates/`.

## Summary

- Biggest wins: **Error construction** (stack capture is O(frame depth x module sequence points) plus `format!` per frame, on every `new Error`) and **WeakMap/WeakSet** (every get/has/set/delete is a linear scan of the entry Vec; `set` also scans all weak maps for the write barrier; symbol weak keys scan all dynamic objects).
- Second tier: **BigInt** (every binary op deep-clones both operands' limb Vecs; division is bit-by-bit with a full BigInt allocation per bit; `toString` is O(n^2) with a String per digit) and **serialize/deserialize** (per-byte buffer copies, `object_own_entries` allocs per instance).
- Function call/apply/bind is moderate (one dynamic record plus array per bind, per-call arg Vec rebuilds); Symbol is cheap except the symbol-cell scan. Everything here is Tier0-only (no JIT lowering) — see JIT section.

JIT status (verified): `zun_codegen/src/eligible.rs:322-403` `collection_builtin` allow-list contains only map/set/buffer/dataView/typed-array names. None of `bigInt*`, `symbol*`, `weakMap*`, `weakSet*`, `weakRef*`, `finalizationRegister*`, `function*`, `captureStackTrace`, `serialize`, `deserialize` appear, so any method containing such a `CallBuiltin` stays on the interpreter (Tier0). `bigInt*` could in principle be lowered only for the small-value case, but BigInt values are heap objects with a side-table (`zun_gc/src/lib.rs:227` `bigints: HashMap<usize, BigInt>`), so no scalar lowering exists.

## Per-method table

Dispatch for all rows: `zun_interp/src/builtins.rs:34-39` `call_builtin` -> thread-local id->name cache (`MANAGED_BUILTIN_CACHE`) -> `call_managed_collection(name, &args)` -> `zun_interp/src/collections/mod.rs:600-650` string `match` on the name (sequential string compares, ~hundreds of arms) -> `col_*` sub-dispatch with a second string `match`. `args` is a freshly popped `Vec<Value>` per call. All rows are JIT: not eligible (Tier0 only).

### Symbol

| method | current path | current complexity | possible complexity | allocs/call | JIT | severity |
|---|---|---|---|---|---|---|
| Symbol(desc) `symbolNew` | `zun_interp/src/collections/object_core.rs:525` -> `zun_core/src/symbols.rs:~150` `create` | O(len desc) | same | desc `String` x1-2 (`to_string` + `Into<String>`), `descriptions` Vec push | no | low |
| Symbol.for `symbolFor` | `object_core.rs:539` -> `symbols.rs:for_key` | O(len key) hash | same | key `String` (to_string), `key.clone()` on miss, 2 HashMap inserts | no | low |
| Symbol.keyFor / description | `object_core.rs:548,569` | O(1) hash/index | same | result `Value::Str` copy | no | low |
| Symbol.<wellKnown> `symbolWellKnown` | `object_core.rs:559` | O(1) | O(1); could be folded to a constant at compile time (ids are fixed ABI, `symbols.rs:19-47`) | 0 heap, but still a full CallBuiltin dispatch | no | low |
| symbol as weak key (`weak_key`) | `zun_interp/src/collections/mod.rs:86-98` -> `zun_gc/src/dynamic.rs:613` `symbol_cell` | **O(total dynamic objects)** per call | O(1) with `HashMap<SymbolId, addr>` | none, but scan | no | high (see F2) |

### BigInt

| method | current path | current complexity | possible complexity | allocs/call | JIT | severity |
|---|---|---|---|---|---|---|
| add/sub | `zun_interp/src/collections/bigint.rs:~100-147` -> `zun_core/src/bigint.rs:150,173` | O(n) + 2 operand clones; `sub` clones `other` again to negate | O(n), no clones | 2 operand `Vec<u32>` clones + `sub` extra clone + result Vec + `alloc_bigint` (GC object + HashMap insert) = 4-5 | no | medium |
| mul | `bigint.rs:189` | O(n*m) schoolbook | Karatsuba only for large n (not confirmed needed) | 2 clones + result | no | low |
| div/rem | `bigint.rs:216` `div_rem` | **O(bits * n)** bit-serial: per bit `remainder.mul_small`, `cmp`, `remainder.sub(&divisor)` (allocates a new BigInt each subtract) | O(n*m) Knuth D; single-limb divisor fast path O(n) | 2 clones + 2 working clones + one BigInt per set-quotient bit; div and rem each recompute both | no | high |
| pow | `bigint.rs:254` | O(log e) mul/squarings (correct); clones base | same | 2 clones + `base.mul(&base)` Vec per bit | no | low |
| and/or/xor/not | `bigint.rs:276-290,374-419` | O(n) via two's-complement conversion | O(n) in-place for non-negative operands | 2 clones + `to_twos_complement` Vec x2 + combined Vec | no | medium |
| shl/shr, asIntN/asUintN | `bigint.rs:293-348` | O(n) | same | clones + result | no | low |
| compare / eq / ne / lt... | `bigint.rs:243`, `bigint.rs(interp):~108-122` | O(n) but **clones both operands first** | O(n) borrow-only, or O(1) for differing sign/length | 2 full clones (pure waste) | no | medium |
| toString | `bigint.rs:502-525` `Display` | **O(n^2)** repeated divide-by-10 (one digit per pass) + `digit.to_string()` heap String per digit | O(n^2/9) dividing by 10^9 per pass; push to a pre-sized `String` | `self.clone()`, `normalized()` per digit (moves), digits Vec, one String per digit | no | high |
| new / fromNumber | `bigint.rs(interp):10-30` -> `parse` | O(len) (parse is `mul_small`+`add_small` per digit => O(n^2)) | chunk by 9 digits | result + GC object | no | low |
| alloc_bigint | `zun_gc/src/bigints.rs:7` | GC object + `bigints.insert(addr, value)` HashMap | store limbs inline in the GC object | HashMap node + limb Vec | no | medium (F5) |

### Error family

| method | current path | current complexity | possible complexity | allocs/call | JIT | severity |
|---|---|---|---|---|---|---|
| `new Error(msg)` field init `stackFrames = captureStackTrace()` | `zun_prelude/builtin/error.zun:23` -> `zun_interp/src/collections/object_descriptor.rs:653` -> `zun_interp/src/collections/runtime_descriptor.rs:11` `render_stack_trace` | **O(D * S)** D = call depth, S = total sequence points of the module (`zun_module/src/lib.rs:156-162` `source_span_at` is a full `.iter().filter().max_by_key()` per frame) + `format!` per frame | O(D) with a per-method sorted sequence-point index; or lazy capture (store `(rid,pc)` pairs, render on first `.stack` read) O(D) copy only | `lines` String (grows), one `format!` String per frame, final `.into()` string value | no | **high** (F1) |
| `.stack` getter | `error.zun:67-72` | O(len) | cache | `toString()` + concat strings | no | low |
| `.name` / `errorName` | `error.zun:43-52` | virtual getter + `nameOverride != ""` string compare | same | 0 | n/a (managed code; getters are plain methods) | low |
| `toString` | `error.zun:56-61` | O(len) | same | concat strings | n/a | low |
| AggregateError ctor | `error.zun:123-126` | O(n) `[...errors]` copy | same | array copy | n/a | low |
| `ErrorValue` (core struct) | `zun_core/src/error_value.rs` | two owned `String`s | not on a hot path (not confirmed referenced in hot path) | 2 Strings | no | low |

### WeakMap / WeakSet / WeakRef / FinalizationRegistry

| method | current path | current complexity | possible complexity | allocs/call | JIT | severity |
|---|---|---|---|---|---|---|
| WeakMap.get/has | `zun_prelude/builtin/weak_map.zun:6-7` -> `zun_interp/src/collections/weak.rs:48-72` -> `zun_gc/src/weak.rs:46` `weak_map_get_value_object` | **O(entries)** linear `entries.iter().find` + HashMap lookup of the map id | O(1) with `HashMap<keyaddr, valaddr>` per weak map (must be rebuilt/rehashed after moving GC: keys relocate) | 0 (+ 2 `with_current_heap` closures); `get` re-reads `dynamic_get(value, "__wmv")` string-key lookup | no | **high** (F2) |
| WeakMap.set | `weak.rs:19-47` -> `zun_gc/src/weak.rs:80-105` `weak_map_set` | **O(entries + #weak maps)**: entry scan, then `weak_map_objects.iter().find(...)` reverse scan over ALL weak maps to find the owner for the write barrier | O(1): store the owner address in the `WeakMapEntry` | non-object value: `alloc_dynamic` + `box_any` + `dynamic_set("__wmv")` (hidden cell per primitive value) | no | **high** (F2) |
| WeakMap.delete | `weak.rs:73-86` -> `zun_gc/src/weak.rs:61-78` | O(entries) retain | O(1) | 0 | no | medium |
| WeakSet.add/has/delete | `weak.rs:101-130` | same as WeakMap (set stores key->key) | same | 0 | no | high (shares F2) |
| WeakRef.deref | `weak.rs:132-141` -> `zun_gc/src/weak.rs` | O(1)-ish | same | 0 | no | low |
| `new WeakRef` / `new WeakMap` | `weak.rs:11-14,95-100,143-150` | O(1) | same | GC alloc + registry insert | no | low |
| FinalizationRegistry.register/unregister | `weak.rs:152-184`, `finalization_registry.zun` | not fully audited (see "Not confirmed") | | `box_any` of held value | no | low |

### Function (call / apply / bind)

| method | current path | current complexity | possible complexity | allocs/call | JIT | severity |
|---|---|---|---|---|---|---|
| f.call(this, ...) | `zun_sema/src/function_reflect.rs:71-136` lowers to `IndirectCall` (no native call) | same as a direct indirect call | same | 0 extra | IndirectCall handling is outside this module | low |
| f.apply(this, arr) | `function_reflect.rs:137-165` -> `CallBuiltin functionApply` -> `zun_interp/src/collections/function.rs:9-36` | O(n) | O(n) with fewer copies | `read_array_values` Vec, `.map(unbox_function_box).collect()` second Vec, `call_args` third Vec (3 Vecs of n) + arg-pop Vec | no | medium |
| f(...xs) `functionCallSpread` | `function.rs:37-53` | O(n) | same | 2 Vecs of n | no | medium |
| f.bind(this, ...p) | `function_reflect.rs:166-196` `FunctionBind` -> `function.rs:54-76` | O(k) | O(1) record with inline slots | `array` (args array), dynamic record, 2 string-keyed `dynamic_set` (`"__target"`, `"__args"`), each partial arg possibly wrapped by `functionBox` (extra dynamic object) | no | medium (F4) |
| call a bound fn | `zun_interp/src/lib.rs:1259-1285` -> `zun_interp/src/collections/runtime_class_proxy.rs:238-283` `resolve_call_target` | O(chain * k): two string-key `dynamic_get`s per hop, prepended Vec rebuilt, `combined` Vec per hop, array unboxed element by element | cache resolved `(rid, prepended)` in the record; flatten bound-of-bound at bind time | 1 prepended Vec + 1 combined Vec per hop + final args Vec | no | medium (F4) |
| functionBox / functionBoxIs | `function.rs:78-104` | O(1) | n/a (see cross-module) | 1 dynamic object per box | no | medium |

### serialize / deserialize (#27)

| method | current path | current complexity | possible complexity | allocs/call | JIT | severity |
|---|---|---|---|---|---|---|
| serialize | `zun_natives/src/serde.rs` (stub) -> `zun_interp/src/serde.rs:56-71` (`serialize_value`) | O(total nodes) + per-node HashMap memo insert; output copied byte-by-byte into the buffer (`buffer_set` per byte, `serde.rs:62-65`) | memo only for nodes that can alias (arrays/objects/maps); bulk-copy output via a single slice write | `out` Vec, `memo` HashMap<usize,u32> (every object, even uniquely-referenced), per node `Vec` of elements, per-byte `buffer_set` | no | medium (F3) |
| ser Array/Map/Set/Dynamic | `serde.rs:~208-330` | O(n) with element Vec snapshot (`array_get` loop), `map_entries`/`dynamic_entries` snapshots | stream without snapshot (needs GC-safe iteration; the snapshot exists because serialization does not interleave allocation) | 1 Vec per container + `any_to_value` per element | no | medium |
| ser TypedArray/ArrayBuffer/DataView | `serde.rs:~150-200` | O(bytes) via `buffer_get` per byte into a `Vec` | memcpy of the buffer slice | Vec of bytes + a second copy into `out` | no | medium |
| ser BigInt | `serde.rs:~140` | O(n^2) via `to_string` (F5) | serialize raw limbs (format change, requires approval) | decimal String | no | medium |
| ser Instance | `serde.rs:~320`, `object_own_entries` (`runtime_descriptor.rs:36-45`) | metadata walk per instance: `Vec<(String, Value)>` with a `String` name per field and getter evaluation (`read_values=true`) | cache `(slot,kind)` list per `type_rid` (the deserialize side already has `instance_serialized_fields`, `serde.rs:627`); skip names | 1 name `String` per field per instance + entries Vec | no | high for object graphs (F3) |
| deserialize | `serde.rs:382-620` | O(bytes) + `memo: Vec<Value>` | same | Vec memo, per-instance `instance_serialized_fields` rebuilt by walking the base chain through metadata tables (`serde.rs:627-670`) -> allocates `chain` + `out` Vecs per instance | no | medium (F3) |

## Findings

### F1. Error construction does an O(depth x module-size) stack capture, eagerly, for every `new Error`

Evidence:
- `zun_prelude/builtin/error.zun:23` `stackFrames: string = captureStackTrace();` — a field default initializer, so it runs on every construction (including `Promise`/runtime-thrown errors via `native_error_to_run_error`, `zun_interp/src/builtins.rs:~60`).
- `zun_interp/src/collections/runtime_descriptor.rs:11-30` `render_stack_trace` iterates `self.call_stack`, calls `debug_frame_at` per frame, then `format!("    at {name} ({start}..{end})")` per frame.
- `zun_module/src/lib.rs:156-162`:
  ```rust
  self.sequence_points.iter()
      .filter(|point| point.method_rid == method_rid && point.pc <= pc)
      .max_by_key(|point| point.pc)
  ```
  is a full linear scan of every sequence point in the module for every frame.

Why: cost per `new Error` = depth x (all sequence points) comparisons plus one String allocation per frame plus the growing `lines` String. Code that throws/catches in a loop (control flow via exceptions, Promise rejections, `Node` style errors) pays this each time even if `.stack` is never read.
Proposal (pick in order of risk):
1. Index sequence points per method (sorted `(pc)` array, binary search) at module load; makes `source_span_at` O(log p). No format change if built lazily in memory.
2. Lazy rendering: capture only `(method_rid, pc)` pairs into a compact field (or a hidden side table keyed by the Error object) and render on first `.stack` access. The field type would change from `string` (layout of `Error` is a prelude class, not the frozen Object/String/Array/Function layout).
3. Optional depth cap (V8 default is 10 frames via `Error.stackTraceLimit`) — behavior change versus current output, byte-identical Node output not guaranteed either way since spans render as byte offsets today.
Est. gain: (1) removes the S factor, typically 10-100x on large modules; (2) removes all per-construction allocation except one small Vec. Risk: low for (1); medium for (2) (prelude change, `.stack` semantic stays). Touches frozen layout/.zm/ABI/GC: no for (1); (2) changes the prelude `Error` field type (not a frozen core layout; needs a metadata/type-token compatibility check) — "requires approval" if it adds an intrinsic or changes the field shape.

### F2. Weak collections are O(n) per operation; WeakMap.set is also O(#weak maps); symbol weak keys scan all dynamic objects

Evidence:
- `zun_gc/src/weak.rs:46-59` `weak_map_get_value_object`: `entry.entries.iter().find(|(candidate, _)| *candidate == key_address)` over `entries: Vec<(usize, usize)>` (also `:86-93` set, `:61-78` delete).
- `zun_gc/src/weak.rs:95-98` in `weak_map_set`:
  ```rust
  if let Some((&address, _)) = self.weak_map_objects.iter().find(|(_, candidate)| *candidate == &id)
  ```
  reverse-looks-up the owner object by scanning the whole `weak_map_objects` HashMap on every `set`/`add`.
- `zun_gc/src/dynamic.rs:613-627` `symbol_cell`: `self.dynamic_objects.iter().find_map(...)` scans every dynamic object to find the cell for a symbol; called from `weak_key` (`zun_interp/src/collections/mod.rs:96-97`) on each symbol-keyed weak op, and non-registered symbols allocate a cell once.
- `zun_interp/src/collections/weak.rs:25-37` a non-object WeakMap value allocates a hidden dynamic object plus `box_any` plus `dynamic_set("__wmv")`; `get` does a string-keyed `dynamic_get("__wmv")`.

Why: WeakMap used as a private-data/cache table (the main use) is O(n) per access, O(n^2) overall; `set` additionally is O(m) in the number of weak maps; each symbol key operation is O(all dynamic objects).
Proposal: give `WeakMapEntry` a `HashMap<usize, usize>` (or a sorted Vec) for key lookup, rebuilt in the GC relocation pass the same way the `bigints` side table is remapped (`zun_gc/src/lib.rs:1000`); store the owner `Obj` address in the entry (drop the reverse scan); add `symbol_cells: HashMap<SymbolId, usize>` for `symbol_cell` (also remapped at relocation); store primitive WeakMap values as `Any` directly in the entry value slot instead of a hidden dynamic cell (needs an `Any`-typed value lane in the ephemeron table).
Est. gain: O(n)->O(1) get/has/set/delete (100x+ at 10^4 entries); set no longer depends on number of weak maps. Risk: medium — touches GC relocation/ephemeron processing; must preserve ephemeron semantics (value kept alive only while key is alive). Touches frozen layout/.zm/ABI: no. Touches GC: **yes** (requires approval).

### F3. serialize/deserialize: per-byte buffer copies, redundant memo, metadata walk allocations per instance

Evidence:
- `zun_interp/src/serde.rs:62-65`: `for (index, byte) in out.iter().enumerate() { heap.buffer_set(buffer, index, *byte); }` — per-byte write after the walk.
- `serde.rs:~150-200`: typed-array/ArrayBuffer/DataView read byte by byte with `heap.buffer_get(buffer, offset + i)?` into a `Vec`, then copied again into `out` via `write_bytes`.
- `serde.rs:58` `memo: HashMap<usize,u32>`: every object, including uniquely-referenced leaves, gets a HashMap insert (`serde.rs:~120-125`), required only for aliasing/cycles.
- Instances: `ObjKind::Instance` calls `self.object_own_entries(obj)` (`runtime_descriptor.rs:36-45`), which builds `Vec<(String, Value)>` with a String name per field and (read_values=true) evaluates getters, only for serialize to discard the names (`for (_name, value) in entries`). The deserialize side recomputes `instance_serialized_fields(type_rid)` per instance (`serde.rs:627-670`: chain Vec walk of `type_defs` + `field_defs` + `self.types.get_field`).
- `serde.rs:~140` BigInt goes through `to_string` (O(n^2), see F5).
- Array/Map/Set: `heap.map_entries(obj)` / `array_get` snapshot Vecs, then `any_to_value` per element (box/unbox).

Why: for a 1 MB payload the byte loops are 2 per-byte heap calls per byte (each `with_current_heap` closure + bounds check); for an object graph each instance costs one String per field plus a metadata walk. Field-walk order is shared with `Object.keys`/JSON, so the improvement must keep order and filters identical.
Proposal: (a) bulk `buffer_write_slice` / `buffer_read_slice` on the GC buffer API (one bounds check, memcpy); (b) cache `Vec<(slot, kind)>` per `type_rid` in the interpreter (`HashMap<u32, Rc<[...]>>`) and use it from both directions, with a `read_slot` path that does not materialize names; (c) skip the memo insert for scalars and strings (already) and for objects, use `FxHashMap`/pre-reserved capacity; (d) pre-size `out` using a cheap size estimate.
Est. gain: bytes paths 5-20x for large buffers; instance graphs 2-3x. Risk: low-medium; wire format unchanged (so `zun_core/src/serde.rs` and byte-compat are untouched). Touches frozen layout/.zm/ABI/GC: GC buffer API addition (narrow-trait addition, "requires approval" as an ABI addition); no change to wire format.

### F4. bind and bound-call path: dynamic record with string keys, args array, per-hop Vec rebuilds

Evidence:
- `zun_interp/src/collections/function.rs:54-76`: bind allocates `alloc_dynamic` + `dynamic_set_function(record, "__target", target)` + `dynamic_set(record, "__args", ...)` plus the `__args` array already allocated by the caller (`FunctionBind` HIR node), plus `functionBox` objects for function-valued partials (`function_reflect.rs:26-45,189`).
- Each call to a bound function: `zun_interp/src/lib.rs:1259-1262` -> `resolve_call_target` (`runtime_class_proxy.rs:238-283`): two string-keyed `dynamic_get`/`dynamic_get_function` lookups, element-wise `unbox_any` + `unbox_function_box`, one `prepended` Vec, one `combined` Vec per hop, then another args Vec if the target has an environment.
- `functionApply` (`function.rs:9-36`) builds `read_array_values` Vec, the unboxed Vec, and `call_args` Vec (3 allocations of size n); `functionCallSpread` builds 2.
- The same bind-record shape is also built by decorators (`zun_interp/src/decorators.rs:115-130` `bind_this`: array alloc + dynamic record).

Why: every `.bind()`-based callback (very common for event handlers, `Array.map(fn.bind(this))`) pays a string-keyed hash lookup x2 and 2-3 Vec allocations per invocation.
Proposal: (a) fuse the three Vec stages in apply/spread into one with-capacity Vec filled from `array_get` in a single heap closure (no behavior change); (b) flatten at bind time: if the target is itself a bound function, concatenate prepended args and store the final `(rid, env)`; (c) replace the dynamic-record + string-keys with a fixed-layout internal class (target handle, args array) so the lookup is a field read — this changes the representation traced via `FunctionHandle.environment` but not the frozen `FunctionHandle` (`zun_core/src/function_handle.rs:2-8`, `#[repr(C)]`, `BOUND_TAG`).
Est. gain: (a) -30-50% apply/spread overhead for small arrays; (b)+(c) bound call about 2x faster. Risk: (a) none; (b) low; (c) medium (GC tracing of the record, `json.rs:171` and `decorators.rs` consumers also read the shape). Touches frozen layout: **no** for (a)(b); (c) leaves FunctionHandle untouched but changes the record layout (not the frozen Function layout) — requires approval due to GC tracing.

### F5. BigInt: operand clones on every op, bit-serial division, O(n^2) toString, side-table storage

Evidence:
- `zun_interp/src/collections/bigint.rs:30-38` and `:100-107`:
  ```rust
  Some((heap.bigint_value(left)?.clone(), heap.bigint_value(right)?.clone()))
  ```
  deep-clones both limb Vecs for every binary op, including `compare`/`eq`/`lt`, where only a borrow is needed. `heap.bigint_value` returns `&BigInt` so a single `with_current_heap` closure computing the result (no clone) is possible; `alloc_bigint` needs `&mut` heap, hence the clone — fixable by computing in the read closure, then allocating.
- `zun_core/src/bigint.rs:173-179` `sub` clones `other` to flip the sign, `mul` allocates `n+m` zeroed limbs then `normalized()`.
- `bigint.rs:216-240` `div_rem`: loop over `bit_count = limbs*32`; each iteration `remainder.mul_small(2)`, `cmp`, and `remainder = remainder.sub(&divisor)` (allocates). `bigIntDiv` and `bigIntRem` each call `div_rem` and discard half the result.
- `bigint.rs:502-525` `Display`: `value.normalized()` per digit and `formatter.write_str(&digit.to_string())` (one String per digit); one division pass per decimal digit.
- `zun_gc/src/bigints.rs:7-11` and `zun_gc/src/lib.rs:227`: BigInt payload lives in `bigints: HashMap<usize, BigInt>`, outside the GC heap: one HashMap insert per allocation, a rehash/remap on every compaction (`lib.rs:1000-1002`), no accounting of limb bytes in GC pressure (not confirmed whether `bigints` entries are freed on sweep).
- Also used by serialize (F3) and `typeofAny` (`object_descriptor.rs:~685` does a `with_current_heap(bigint_value)` probe to classify any object as bigint).

Why: factorial/fibonacci-style loops allocate ~5 Vecs per op; large division/printing is O(n^2)-O(n^3) in practice.
Proposal: borrow-only compare; compute in the read closure and allocate only the result; `Option<(q,r)>` caching not needed once Knuth D (word-by-word, 32x32->64) replaces the bit loop with a single-limb fast path; `to_string` dividing by 10^9 per pass into a pre-sized `String`; small-value inline representation (e.g. `Any` int tag for i64-range values, fall back to heap) would also let JIT lower add/sub/cmp (not confirmed feasible within the current `Any` tag set).
Est. gain: compare/eq 10x+; add/sub 2x (fewer clones); div 10-100x for large operands; toString about 9x. Risk: low for algorithmic fixes (pure `zun_core`, covered by `bigint.rs` unit tests at `:537-628`); medium for storage move into the GC object. Touches frozen layout/.zm/ABI/GC: algorithmic fixes no; moving storage into the heap object **touches GC** (requires approval); a small-int inline representation touches the `Any` ABI (requires approval).

### F6. Name-keyed dispatch for every misc builtin (minor, shared)

Evidence: `zun_interp/src/builtins.rs:34-39` + `zun_interp/src/collections/mod.rs:600-650` — one thread-local HashMap lookup for id->name, then a string `match` through up to two levels (`call_managed_collection_inner` then `col_*`), and a `Vec<Value>` for `args`. Symbol, BigInt, weak, function, serde and Error all go through it.
Proposal: resolve to a function pointer / enum once per call site (id -> `fn(&mut Interp, &[Value])` table). Est. gain: removes tens of string compares per call; matters for tiny ops (`symbolWellKnown`, `weakMapHas`, `bigIntCompare`). Risk: low; touches nothing frozen. See cross-module section.

## Cross-module issues (for the shared doc)

- Native-call cost: all misc builtins pay `pop_n` Vec + id->name map + string `match` chain (F6); same applies to every `col_*` builtin.
- Any box/unbox + `functionBox`: function values cannot ride the `Any` lane, so `functionBox`/`unbox_function_box` wrapper objects are allocated/unwrapped at apply/bind/spread sites (`function_reflect.rs:26-60`, `function.rs`, `runtime_class_proxy.rs:264`); a proper `Any` tag for function handles would remove this machinery (touches `Any` ABI).
- GC side tables keyed by address (`bigints`, `weak_map_objects`, `dynamic_objects`, symbol cells, `finalization` tables) require relocation remapping; address-keyed linear scans (F2) and HashMaps outside the heap (F5) are a recurring pattern.
- Field-walk shared with JSON/`Object.keys`: `object_own_entries` allocates name Strings and evaluates getters; serde (F3) and JSON should share a cached per-`type_rid` `(slot, kind)` shape.
- Source-span lookup (`Module::source_span_at`, linear) is shared by any diagnostics consumer (stack traces, error reporting).
- JIT: none of the misc builtins are in the `eligible.rs` allow-lists; any Tier1 function that constructs an `Error`, uses BigInt, WeakMap or bind falls back to Tier0.

## Not confirmed

- Whether GC sweep removes dead `bigints` / `weak_maps` / `symbol cell` entries (not inspected).
- `FinalizationRegistry.register/unregister` cost (`weak.rs:152-184` read only partially).
- Exact instruction counts per call (no profiling run; this audit is static).
- `col_object_core` / `col_object_descriptor` secondary `match` cost vs. the first-level match (not measured).
