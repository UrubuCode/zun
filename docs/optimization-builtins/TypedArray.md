# TypedArray / ArrayBuffer / DataView - optimization audit

## Summary

- Every element access (interp and JIT thunk) pays 2 `HashMap<usize,_>` lookups (view table + buffer table) plus a `resolve()` window re-derivation and a per-byte copy loop; the JIT only emits calls to `extern "C"` thunks (never an inline load/store), and only 4 of 11 kinds (u8/u16/u32/f64) are JIT-eligible.
- All bulk methods (`set`, `slice`, `copyWithin`, `indexOf`, `join`, `from`, `of`, iterator, `ArrayBuffer.slice`) run element-at-a-time through `Value` boxing and heap-wide closures with an intermediate `Vec<Value>`; none uses `copy_from_slice`/`copy_within` on the byte store.
- Callback methods (`map/filter/forEach/reduce/find/some/every`) are interpreter-driven: per element a `read_element`, 1-2 `Any` boxes, an arg `Vec`, and (reduce) a GC holder-array round trip; they are not JIT-lowerable.

Scope: verified by reading code only (no benchmarks run). Gains are estimates; items marked "not confirmed" are hypotheses.

## Per-method table

Path abbreviations: `IR` = `crates/zun_interp/src/collections/runtime_typed_array.rs`, `TA` = `crates/zun_interp/src/typed_arrays.rs`, `TV` = `crates/zun_gc/src/typed_views.rs`, `V` = `crates/zun_core/src/views.rs`, `TH` = `crates/zun_codegen/src/thunks/buffers.rs`, `EL` = `crates/zun_codegen/src/eligible.rs`, `BT` = `crates/zun_codegen/src/translate/builtins_table.rs`. Line numbers marked "~" are approximate.

| method | current path (file:line) | current complexity | possible complexity | allocs/call | JIT eligible/lowered | severity |
|---|---|---|---|---|---|---|
| get `ta[i]` | `TA` `<native>Get` arm -> `TV` `$get` (~l.365) -> `V` `resolve` l.449 + `element_bytes` l.525 | O(1) but 2 hash lookups + resolve + per-byte `buffer.get` loop | O(1), 0 hashes, `from_le_bytes` on slice | 0 | only u8/u16/u32/f64 (`EL` list, `BT` ~l.899); i8/i16/i32/f32/i64/u64/u8clamped NOT eligible; lowered as `extern "C"` call (`TH` l.359, l.440), not inline load | High |
| set `ta[i]=v` | `TA` `<native>Set`; `TV` `$set`; `V` `set_element_bytes` l.538 | same as get | O(1) inline store | 0 | same 4 kinds only, thunk call (`TH` l.373, l.453) | High |
| length / byteLength | `TA` `measurement`; `TV` l.21 / l.34 | 2 hash lookups + resolve per call (also per loop condition) | O(1) cached field | 0 | the 4 kinds, via thunk (`BT` ~l.725-745) | Med |
| subarray | `TA` `subarray`; `TV` l.436; `V` `subview` l.497 | O(1) | O(1) | view obj + HashMap insert | not in eligible list | Low |
| fill | `TA` `<native>Fill` | O(n) x (2 hash + resolve) per element | O(n) single `slice::fill` | 0 | no | Med |
| set (bulk) | `IR` l.66 `typedArraySetFrom` | O(n) via `source_elements` Vec<Value>, then per-element `write_element` | O(n) memmove for same-kind typed source | Vec<Value>(n) | no | High |
| slice | `IR` l.92 | O(n) read to Vec<Value>, alloc, O(n) `write_element` | O(n) one memcpy | Vec<Value>(n) + view + buffer | no | High |
| copyWithin | `IR` l.115 | O(n) snapshot Vec<Value> + per-element writes | O(n) `copy_within` | Vec<Value>(n) | no | Med |
| indexOf / includes | `IR` l.141 | O(n) x `read_element` (kind lookup + hash chain + Value build) | O(n) typed scan, 1 resolve | 0 (1 needle unbox) | no | Med |
| join | `IR` l.174 | O(n), per element `box_any` + `stringify_any` + String | O(n) direct number formatting into one String | n Strings + Vec | no | Med |
| forEach/map/filter/find/findIndex/some/every | `IR` `typed_array_iterating` l.241-332 | O(n) interp callbacks; per element `read_element` + `box_any` + `vec!`(3) + `invoke_callback` + `write_element`/`is_truthy` | O(n) callbacks unavoidable, per-element overhead reducible | per elem: Vec(3) + 1-2 boxes; filter adds Vec<Value> | no | High |
| reduce / reduceRight (+NoInit) | `IR` l.342-410 | O(n); per elem read+box, `array_get`, Vec(4), callback, box result, `array_set` into 1-slot holder | same, accumulator kept as rooted `Value` | per elem Vec(4) + 2 boxes | no | High |
| from / of / fromMapped | `IR` l.207 / `typed_array_from_mapped` l.415 | O(n) Vec<Value> + per-element write | O(n) direct | Vec<Value>(n) | no | Med |
| iterator (`<native>IteratorNew`) | `TA` (in `call`, IteratorNew arm) | O(n) EAGER: reads all, boxes all into a new GC array, then array iterator | O(1) lazy | Vec<Value>(n) + GC array(n) + n boxes | no | High |
| new View(buffer,off,len) | `TA` `new_view`; `TV` l.397 | O(1) | O(1) | view obj + HashMap insert | no | Low |
| ArrayBuffer.slice | `crates/zun_gc/src/buffers.rs` l.125 | O(n): `buffer_get` (hash) per byte into `collect`ed Vec<u8>, then `buffer_set` (hash) per byte | O(n) one `to_vec` | Vec<u8>(n) + buffer | no (only `bufferLength/GetU8/SetU8/ReadU32/ReadF64/WriteU32/WriteF64` eligible) | High |
| ArrayBuffer resize / transfer | `crates/zun_core/src/byte_buffer.rs` `resize`, `detach`, `with_capacity` | resize O(delta); `detach` moves the Vec, but `with_capacity` zero-fills a new Vec that `transfer` then copies into (per its doc comment) | transfer could reuse the detached Vec | 1 Vec | no | Low |
| DataView getXxx / setXxx | `V` `get_bytes` l.266 / `set_bytes` l.283; heap `data_view_*` | O(1): `data_views` hash + `object.field(0)` + buffer hash + resolve + per-byte loop + `reverse()` | O(1) via `from_le/be_bytes` on slice | 0 | partly: U32/U16/F64 and Int8/Uint8/Int16/Int32/Float32 get+set eligible (`EL` l.367-384) | Med |
| Buffer length / getU8 / readU32 / readF64 / writes | `TH` l.1-80 | 1 hash lookup + bounds | O(1) inline | 0 | eligible via thunk | Low |

## Findings

### F1. Two HashMap lookups + window re-resolve on every element access

Evidence (`TV` ~l.365):
```rust
pub fn $get(&self, object: Obj, index: usize) -> Option<$repr> {
    let (buffer, view) = self.typed_views.get(&(object.addr() as usize))?;
    let bytes = self.buffers.get(&(buffer.addr() as usize))?;
    view.resolve(bytes.len())?.$read(bytes, index)
}
```
`typed_views` and `buffers` are `HashMap<usize,...>` (`crates/zun_gc/src/lib.rs` l.195-199; hasher type not confirmed beyond the default). `resolve` (`V` l.449) recomputes the window on every call and `element_bytes` (`V` l.525) copies bytes one at a time via `buffer.get(base+step)?`. `typed_array_len` (`TV` l.21) repeats the chain, so a `for (i < ta.length)` loop pays it for the loop condition too.
Why: typed-view metadata lives in a side table keyed by object address instead of in the object (DataView does read its buffer from `object.field(0)`, but its window is still in a side map).
Proposal: (a) no layout change: use a non-cryptographic pointer hasher for both maps; replace the per-byte loops with a single slice + `from_le_bytes`; (b) larger: put buffer ref, kind, offset, length, tracking flag in the view object's fields so a get is loads + a compare, with the slow `resolve` path only when the buffer is resizable/detached.
Est. gain: (a) 2-4x on element get/set; (b) more, and the prerequisite for F2. Risk: (a) low; (b) medium (GC relocation; see F6). Touches: (a) none; (b) frozen layout yes, GC yes, .zm no, ABI no. (b) requires approval.

### F2. JIT reaches only 4 of 11 kinds, only through opaque `extern "C"` calls

Evidence: `EL` names only `u8Array*`, `u16Array*`, `u32Array*`, `f64Array*` (Get/Set/Length/ByteLength); `BT` ~l.899 maps them to `zun_jit_*_array_get/set`; thunk (`TH` l.359-369):
```rust
pub extern "C" fn zun_jit_u8_array_get(address: i64, index: i64) -> i8 {
    if index < 0 { return -1; }
    ... heap.u8_array_get(object, index as usize).map(|value| value as i8).unwrap_or(-1)
```
i8, i16, i32, f32, i64, u64, u8-clamped exist in `typed_element_table!` and the interpreter, but have no JIT mapping, so any function touching an `Int32Array`/`Float32Array` stays at Tier 0 (eligible implies lowerable). Even for the 4 kinds Cranelift sees an opaque call: no CSE/LICM of the lookups and a `with_current_heap` access per element (cost not confirmed). The thunks and `BT`/`EL` entries are hand-written per kind, a second copy of knowledge that the master table was meant to remove.
Observation (signature verified, translator behavior not inspected): the u8 getter returns `-1` as its out-of-range sentinel through an `i8` ABI typed as `NumKind::U8`; a stored `0xFF` byte is `-1 as i8`, so the sentinel is indistinguishable at the ABI. Whether the translator disambiguates is not confirmed.
Proposal: (1) generate thunks, `BT` rows and `EL` entries for all kinds from `typed_element_table!`; (2) lower get/set inline: load element count + data pointer from the stable view struct (needs F1b), unsigned `icmp` bounds check branching to a cold slow block that calls the existing thunk (handles resizable/detached), then a `load`/`store` with `MemFlags::trusted()` only on the post-check path (per `docs/guides/cranelift.md`: trusted only for known-valid runtime structures) and a Heap alias region; hoist base/length out of loops only when the buffer is fixed-length.
Est. gain: 5-20x on numeric inner loops (estimate). Risk: medium-high (buffer relocation keys, detach/resize invalidation). Touches: frozen layout yes (F1b), ABI: add new thunk symbols only, never change existing (requires approval for layout), GC: no stack maps needed for numeric loads, .zm no.

### F3. Bulk methods are element-at-a-time through `Value` with intermediate Vecs

Evidence (`IR` l.84-89):
```rust
for (index, value) in elements.iter().enumerate() {
    if crate::typed_arrays::write_element(view, offset + index, value) != Some(true) {
```
`write_element` (`TA`, generated) does a heap borrow for the kind (hash), `from_numeric`, then a second heap-mut borrow doing both hashes + resolve + byte loop, per element. `slice` (l.102-112), `copyWithin` (l.129-138), `set` from a typed source, `from`, `of` all build `Vec<Value>` first. `ArrayBuffer.slice` (`crates/zun_gc/src/buffers.rs` l.125-137) does `buffer_get` (hash) per byte and `buffer_set` (hash) per byte. `join` boxes each element to `Any` then `stringify_any` (l.186-190).
Proposal: same-kind typed source -> memcpy of byte ranges via `ByteBuffer::as_slice`; `copyWithin` -> `Vec::copy_within` (memmove semantics also cover overlap, replacing the snapshot); `slice` -> allocate + `extend_from_slice`; `ArrayBuffer.slice` -> `bytes[begin..end].to_vec()`; `fill` -> `slice::fill` / `chunks_exact_mut`; cross-kind or array-source `set` keeps a typed loop but resolves view+buffer once per call (a `Heap` range-write method). Needs a `ByteBuffer` mutable-slice accessor (only `as_slice` exists, `byte_buffer.rs`).
Est. gain: 10-50x on large slice/set/fill/ArrayBuffer.slice (estimate). Risk: low-medium (clamped/float conversions, overlap, shrink mid-call). Touches layout/.zm/ABI/GC: no.

### F4. Callback methods: per-element boxing, arg Vec, holder-array accumulator

Evidence (`IR` l.271-281):
```rust
let value = crate::typed_arrays::read_element(roots[0], index)?;
let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))?;
let callback_args = vec![Value::Any(boxed), Value::Any(zun_core::Any::float(index as f64)), Value::Any(zun_core::Any::object(roots[0].addr() as usize))];
let produced = self.invoke_callback(callback, callback_args)?;
```
Per element: kind hash + element hash chain, a heap borrow for `box_any`, a heap-allocated `Vec` (3; 4 in reduce), `invoke_callback`, then `write_element`/`is_truthy`. Reduce additionally routes the accumulator through a 1-slot GC array each step (l.379-405: `array_get`, `box_any`, `array_set`). The method name is a `&str` matched inside the loop (l.282-313; hoisting by the compiler not confirmed). `filter` collects `Vec<Value>` then re-writes into a new view.
Correctness note (verified): a `read_element` returning `None` mid-loop (resizable buffer shrunk by the callback) becomes `RunError::BadToken` (l.273) rather than the spec behavior of reading `undefined`; length is captured once (l.259).
Proposal: select the operation once outside the loop; reuse one args buffer; keep reduce's accumulator as a rooted `Value` slot; pre-size `filter` output. Longer term: implement these as typed std code so the JIT can inline the callback (cross-module).
Est. gain: 1.5-3x (callback dispatch probably dominates; not confirmed without profiling). Risk: low for hoisting, medium for accumulator (GC rooting). Touches layout/.zm/ABI: no.

### F5. Iterator is eager O(n) with n boxes and a full GC array

Evidence (`TA`, IteratorNew arm): reads all `length` elements into `Vec<Value>`, `alloc_array(len)`, `box_any` + `array_set` each, then `alloc_array_iterator`. A `for-of` over a typed array allocates O(n) up front even when breaking early, and iterates a snapshot (writes during iteration are not observed; spec iterators are live; spec comparison from knowledge).
Proposal: lazy iterator over (view, index), or lower `for-of` over `BufferView` in sema to an indexed loop (also JIT-eligible after F2).
Est. gain: 3-10x on for-of over typed arrays (estimate). Risk: low-medium (new iterator object kind needs GC/iterator-table support: not inspected). Touches: possibly new heap object kind (GC), no .zm/ABI.

### F6. GC cost of the side tables

`crates/zun_gc/src/lib.rs` l.787-802 drains and rebuilds the whole `typed_views` map on every relocation pass; `data_views` likewise (~l.949). Cost is O(live views) per GC plus rehash, and each `subarray`/`new View` inserts a map entry. Whether dead-view entries are removed on sweep was not inspected (potential leak: not confirmed).
Proposal: view metadata inside the object (F1b) removes both the insert and the relocation rewrite. Touches: frozen layout, GC (requires approval).

### F7. Dispatch notes

`TA::call` runs `measurement`, `subarray`, `view_meta` (three name-match chains over 11 kinds x suffixes) before the per-kind `match name`, so a native like `i32ArraySet` falls through three failed string matches first. Not typed-array specific: if natives are resolved to numeric ids before dispatch this is moot (not confirmed, see cross-module). The `set` overload (indexed vs bulk) is resolved at compile time in sema (`crates/zun_sema/src/check/builtins/typed_array.rs`, `"set"` arm: array/typed-array first arg means bulk), so there is no run-time overload cost (verified).

### F8. DataView

`get_bytes`/`set_bytes` (`V` l.266-300) copy byte by byte into `[u8; N]` and `reverse()` for big-endian; each access also costs the `data_views` hash + `object.field(0)` + buffer hash + `resolve`. JIT eligibility (`EL` l.367-384): U32/U16/F64 and Int8/Uint8/Int16/Int32/Float32 get/set; I did not find BigInt64/BigUint64 variants in the list even though core has `get_i64/get_u64` (`V` l.364-399).
Proposal: slice + `from_le_bytes`/`from_be_bytes`; F1 hasher fix; add 64-bit accessors to the JIT table. Est. gain 2-3x. Risk low.

### F9. Resizable / length-tracking overhead is paid by fixed buffers too

Every access re-runs `resolve` against `bytes.len()` (`V` l.449; deliberate design, `V` l.24-29: no walk of outstanding views on resize). Correct but paid by all fixed-length buffers. Proposal: cache a "fixed-length window valid" flag (`ByteBuffer::resizable()` exists) so fixed buffers skip `resolve`, and so F2 can hoist length/base. Est. small alone, an enabler for F2. Risk low.

## Cross-module reference

Cross-cutting issues (native-call cost per element in callbacks, `Any` box/unbox marshaling, string-name native dispatch, heap side-table HashMap + GC rehash pattern, `invoke_callback` arg-Vec overhead) are listed in the shared report; see the audit's final report section "cross-cutting issues".
