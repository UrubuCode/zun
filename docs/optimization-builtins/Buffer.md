# Buffer optimization audit

## Summary
- All `nodeBuffer*` natives are `unavailable` stubs (`crates/zun_natives/src/node_buffer.rs:17`); real work is `crates/zun_interp/src/node_buffer.rs::call` (727 lines), reached by string-name matching through `call_managed_collection_inner` (`collections/mod.rs:201`). No `nodeBuffer*` appears in `zun_codegen`: nothing is JIT-eligible/lowered.
- Every op copies the whole view byte-by-byte into a `Vec<u8>` (`view_bytes`) or goes byte-by-byte through `heap.u8_array_get/set`; each access resolves a `typed_views` HashMap entry (`zun_gc/src/typed_views.rs:22`). Fixed-width read/write are per-byte loops, not a single load/bswap.
- Biggest wins: bulk slice copy for view_bytes/fill/copy/equals/concat/indexOf, name-parsed accessor dispatch, no-alloc byteLength/utf8 paths. Hypotheses (not confirmed) are labelled.

## Per-method table
All paths in `crates/zun_interp/src/node_buffer.rs` unless noted. n = view length.

| method | current path (file:line) | current complexity | possible | allocs/call | JIT | severity |
|---|---|---|---|---|---|---|
| from(string) | `call` "nodeBufferFromString" ~l.368; `encode` l.105; `buffer_from_bytes` l.62 | O(n) encode Vec + O(n) per-byte `buffer_set` (HashMap-free but per byte) | encode straight into buffer / memcpy | Vec<u8> + buffer + view; `normalize_encoding` allocs lowercase String | no | medium |
| from(array) | "nodeBufferFromArray" | O(n) array_get + Vec + per-byte set | direct write | Vec + buffer | no | low |
| from(arrayBuffer) | "nodeBufferFromArrayBuffer" | O(1) | same | view only | no | none |
| alloc / allocUnsafe | "nodeBufferAlloc"/"AllocUnsafe" | O(n) `vec![..;n]` then per-byte copy | alloc_buffer already zeroed? (hypothesis, not confirmed) skip copy | Vec of n + buffer | no | medium |
| concat | "nodeBufferConcat" | O(total) via `view_bytes` per member then per-byte copy; two copies | one pass memcpy, presize | Vec per member + total Vec | no | medium |
| isBuffer | "nodeBufferIsBuffer" | O(1) | same | 0 | no | none |
| byteLength | "nodeBufferByteLength" | full encode then `.len()` | utf8: `str.len()`; hex len/2; base64 arithmetic; utf16 count | Vec of n | no | medium |
| toString | "nodeBufferToString": `view_bytes` full copy then slice (start/end applied AFTER copying whole buffer) | O(n_total) even for small range; decode O(range) | copy only [start,end) | full Vec + decoded String + `Rc<str>` (second copy of text) ; hex uses `format!` per byte (l.~118) | no | high |
| write | "nodeBufferWrite" | O(len) encode + per-byte set | encode truncated | Vec | no | medium |
| slice/subarray | "nodeBufferSlice" -> `typed_array_subarray` (typed_views.rs:436) | O(1) | same | 1 view | no | none |
| fill | "nodeBufferFill" | O(n) `u8_array_set` per byte, each a HashMap lookup (hypothesis for per-byte lookup: `typed_views.get` l.22 pattern, u8_array_set impl not read) | memset | 0 | no | medium |
| equals | "nodeBufferEquals" | two full Vec copies + compare | length check first, slice compare | 2 Vecs | no | medium |
| copy | "nodeBufferCopy" | full source `view_bytes` then per-byte set; ignores overlap semantics (copies from a snapshot, which is safe) | memmove of range | Vec of source n | no | medium |
| readUInt8/16/32/64, readInt*, BigInt* LE/BE (family ~24) | `accessor(name)` l.283 parses the native NAME string per call (strip_prefix, ends_with, contains) then `read_uint` l.236 | O(width) per-byte u8_array_get + shift | 1 load (+bswap for BE) | 0 | no (could be a single load; endianness via `bswap`) | high (hot) |
| write* family | `write_uint` l.254, per-byte set | O(width) | 1 store (+bswap) | 0 | no | high |
| readFloat/Double LE/BE, write* | via `read_uint` then `from_bits` | per-byte | 1 load/bitcast | 0 | no | medium |
| readUIntLE/BE, readIntLE/BE, write variable width 1..6 | same helpers, width runtime | per-byte | per-width shifts | 0 | no | low |
| swap16/32/64 | per-pair `u8_array_get`x2/`set`x2 | O(n) with 4 view lookups per byte pair | in-place `chunks_exact_mut().reverse()` / bswap loop | 0 | no | low |
| indexOf / lastIndexOf / includes | `view_bytes` full copy; needle encoded; `buffer_search` l.314 naive `windows` compare | O(n*m) worst, copy O(n) | memchr / two-way; no copy | haystack Vec + needle Vec; Any unboxed | no | high |
| buf[i] get/set | typed-array path (`zun_jit_u8_array_get/set`, thunks/buffers.rs:359/373; eligible.rs:360 names `bufferGetU8` etc. for ArrayBuffer) | extern thunk call per access, not inlined | inline bounds check + load | 0 | thunk only | medium |

## Findings

### F1. Whole-buffer copy per call (`view_bytes`)
Evidence: node_buffer.rs:74-83 `for index in 0..len { bytes.push(heap.u8_array_get(view, index)?); }`; used by toString, equals, copy, concat, indexOf family.
Why: O(n) allocation+copy before any work; toString copies the full buffer and then slices start..end (l.~437-449); equals copies both; indexOf copies the haystack to find one byte.
Proposal: add a `Heap` method returning `&[u8]` (or running a closure on the resolved byte slice) and operate in place; for copy/concat use `copy_from_slice`. Est. gain: 5-50x on large buffers, alloc-free for equals/indexOf. Risk: low (must respect borrow of heap during closure; no allocation inside). Touches frozen layout/.zm/ABI/GC: no (new GC-side accessor; internal API only).

### F2. Per-byte access with a HashMap lookup (hypothesis, not confirmed in u8_array impl)
Evidence: `typed_array_len` does `self.typed_views.get(&(object.addr() as usize))` (zun_gc typed_views.rs:22); `u8_array_get/set` are called per byte in view_bytes, fill (l.~488), write, copy, swap, read_uint/write_uint. The macro-generated `u8_array_get` body was not read.
Why: if each call resolves the HashMap entry, fill/read/write cost a hash lookup per byte.
Proposal: resolve view once, then slice-level ops (`fill`, `copy_from_slice`, `u32::from_le_bytes`). Est. gain: 10-100x for fill/write/copy. Risk: low. Frozen layout/ABI: no.

### F3. Name-string dispatch and name parsing per call
Evidence: `call(name,&[Value])` is a long `match name` (l.367+); fixed-width read/write family falls to `_ =>` and runs `accessor(name)` (l.283) with strip_prefix/ends_with/contains on every call; reached after `typed_arrays::call` then `node_buffer::call` are tried in sequence (collections/mod.rs:198-201). `managed_node_buffer_builtin` (builtins.rs:382) is a linear string array search over ids.
Why: ~24 read*/write* natives each pay string parsing; hot for binary protocol code.
Proposal: generate the family from one master table (width, signed, endian, kind) keyed by id, resolve once at load; match on `u32` id or pre-resolved fn pointer. Est. gain: removes tens of ns per call. Risk: low-medium. Touches: native ids registered in `zun_natives` (appended registry; ids must stay stable); no .zm change.

### F4. Not JIT-lowerable today
Evidence: `grep nodeBuffer crates/zun_codegen` finds no hits; eligible.rs:360-366 lists only `bufferGetU8/ReadU32/ReadF64/SetU8/WriteU32/WriteF64` (ArrayBuffer); thunks/buffers.rs only has extern "C" thunks (e.g. `zun_jit_buffer_read_u32` l.70, `zun_jit_u8_array_get` l.359), which are calls, not inline loads.
Proposal: lower readUInt8/16/32, readInt*, Float/Double LE/BE to bounds check + Cranelift `load` (+ `bswap` for BE; native is LE on x86/ARM64) and writes to `store`; keep the thunk as slow/error path (cold block). Eligibility must equal lowerability, so add each native to eligible.rs only when lowered. Est. gain: 10-30x on tight parse loops. Risk: medium (error semantics: out-of-range currently returns `RunError::BadToken`, not Node's ERR_OUT_OF_RANGE; keep Tier0 equivalence). Touches frozen layout: reads view fields (offset/len through typed_views, which is a GC-side HashMap, so JIT needs a stable fast path or a view-layout accessor -> requires approval if layout changes).

### F5. Encoding paths allocate heavily
Evidence: `normalize_encoding` (l.90) `name.to_ascii_lowercase()` allocates a String every call; `decode` hex uses `format!("{b:02x}")` per byte (l.118); toString returns `Rc::from(decode(..).as_str())` (copies the String again); `encode_utf16le` pushes bytes with no reserve; byteLength fully encodes.
Proposal: compare encoding case-insensitively without allocation (checker already passes "utf8" default, could pass an enum tag); hex via lookup table with pre-sized String; build `Rc<str>` from the String directly; closed-form byteLength. Est. gain: 2-5x on string<->buffer. Risk: low. Touches: if a checker-passed enum tag replaces Str, native signature changes (ABI, requires approval); otherwise none.

### F6. indexOf naive scan
Evidence: buffer_search l.314-352 uses `(start..=max_start).find(|&i| &haystack[i..i+n] == needle)`.
Proposal: single-byte needle -> memchr; else `windows`+memmem (two-way). Est. gain: 4-10x. Risk: low.

### F7. swap/fill/copy minor
swap16/32/64: 4 view accesses per pair (l.~683). Replace with slice `chunks_exact_mut(w)` + `reverse`. alloc(size, fill): builds `vec![fill;size]`, then copies per byte (l.62-70) -> double work.

## Cross-module references
- Native-call cost / name-based dispatch of managed builtins: see shared doc (Any marshaling of indexOf needle via `Value::Any` unbox, l.~394).
- byte_buffer / typed_views HashMap shared with TypedArray/ArrayBuffer/DataView: see shared doc.
- String layout (`Rc<str>`, UTF-8) vs encodings (utf16/latin1 conversions): see shared doc.
