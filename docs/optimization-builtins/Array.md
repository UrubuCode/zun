# Array built-ins: optimization audit

Scope: `Array.prototype.*`, `Array.*`, array iterators. Read-only audit; every item below was verified in code unless marked "not confirmed". Paths are relative to `crates/`.

## Summary

1. `sort` / `toSorted` are O(n^2) insertion sorts that re-read the array through the heap on every step, and the default comparator allocates two `String`s per comparison (`zun_interp/src/collections/array_iterate.rs:182`, `array_mutate.rs:452`).
2. `splice` / `shift` / `unshift` / `toSpliced`-style paths do one `Vec::remove` / `Vec::insert` per element, so `splice(i, k, ...items)` costs O(n*(k+m)) instead of O(n) (`array_mutate.rs:91,97,123,130`, `array_iterate.rs:261,273,297`).
3. No Array method (callbacks, indexOf, slice, join, ...) is JIT-lowered. Only `ArrayLength/NewArray/LoadArray/StoreArray/ArrayPush/ArrayPop` are, each as an extern call without a FuncRef cache, so any function that calls an Array method falls back to Tier 0.

## Per-method table

Notation: n = array length, k = deleted count, m = inserted count, c = callback cost. "Native" = reached via `CallBuiltin` -> `call_managed_collection_inner` (name-string dispatch, see F9). JIT: "no" means the `array*` native is absent from `zun_codegen/src/eligible.rs` `collection_builtin`/`math_builtin`/etc. and from `translate/builtins_table.rs` (grep for `"array[A-Z]` in `zun_codegen/src` returns nothing), so a function containing it is not JIT-eligible.

| method | current path (file:line) | current complexity | possible complexity | allocs per call | JIT eligible/lowered | severity |
|---|---|---|---|---|---|---|
| length | `ArrayLength` opcode; JIT `translate/arrays.rs:10` extern call `zun_jit_array_length` | O(1) + TLS heap + HashMap lookup | O(1) inline load (needs layout approval) | 0 | lowered (extern call) | medium |
| index get/set | `LoadArray`/`StoreArray`; `translate/mod.rs:526`, thunk `thunks/misc.rs:55` | O(1) + HashMap lookup per access | O(1) inline | 0 (carrier load uses stack slot) | lowered (extern call, FuncRef not cached) | high |
| push | `ArrayPush`; `translate/arrays.rs:100`, `zun_gc/src/arrays.rs:53` | amortised O(1) + HashMap lookup | same, inline fast path | 0 | lowered (extern call) | medium |
| pop | `ArrayPop`; sema types result `Ty::Any` (`zun_sema/.../mcall_arrays.rs:52-60`); JIT `translate/arrays.rs:40` | O(1) | same | box to Any on typed path | lowered, kind chosen from method return kind | low |
| push()/unshift() no args | sema -> `ArrayLength` (`mcall_arrays.rs:66`) | O(1) | O(1) | 0 | lowered | none |
| shift | `arrayShift`, `array_iterate.rs:254` -> `Vec::remove(0)` (`zun_core/src/arrays.rs:71`) | O(n) | O(1) amortised (offset/deque) - requires approval | 0 | no | high |
| unshift(a,b,c) | sema emits one `arrayUnshift` per arg, reversed (`mcall_arrays.rs:77-120`); each `Vec::insert(0)` (`array_iterate.rs:267`) | O(m*n) | O(n+m) | 0 | no | medium |
| splice | `arraySplice` (`array_iterate.rs:278`), `arraySpliceArgs` (`array_mutate.rs:42`), `arraySpliceInsert` (`array_mutate.rs:104`) | O(n*(k+m)) | O(n) via `Vec::splice`/`drain` | result array + (spread arg list) | no | high |
| toSpliced | `array_mutate.rs:594` | O(n) | O(n) | 1 result, per-elem hole map | no | low |
| slice | `arraySliceRange` `array_mutate.rs:167`; sema variant `arraySliceArgs` `array_mutate.rs:17` | O(count), per-elem heap lookup | O(count) memcpy (`extend_from_slice`) | result array; `args.to_vec()` (L172); spread variant builds Vec | no | medium |
| concat | `arrayConcat` `array_mutate.rs:662`; sema gathers items into temp array (`mcall_arrays.rs:676`) | O(total) but 2 copies | 1 copy, `reserve` exact | temp items array + `flat: Vec<Any>` + result | no | medium |
| indexOf/lastIndexOf/includes | `array_mutate.rs:229` | O(n) | O(n) with typed fast path | 0 heap, but per element: 2 HashMap lookups + `unbox_any` -> `Value` (may allocate/clone string) + `strict_equals` | no | high |
| join / toString | `arrayJoin` `array_iterate.rs:244` -> `array_join_dyn` `runtime_coerce.rs:368` | O(n) but per element: len lookup + get lookup + unbox + `String` alloc; `Vec<String>` then `join`; result `normalize` | O(n) single `String` with pre-reserved capacity | n `String` + parts Vec + joined + normalized + `separator.to_string()` | no | high |
| reverse | `array_mutate.rs:211` | O(n), 4 HashMap lookups per swap pair | O(n) `slice::reverse` | 0 | no | medium |
| toReversed / with | `array_mutate.rs:522`, `546` | O(n), 2 lookups per element | O(n) memcpy | 1 result | no | low |
| sort / toSorted | `array_iterate.rs:182`, `array_mutate.rs:452` | O(n^2) compares, 2+ lookups per step; default: 2 `String` allocs per compare | O(n log n) merge sort (stable), keys pre-stringified once | per compare: `vec![..]` callback args + 2 Strings (default) | no | critical |
| forEach/map/filter/find/findIndex/some/every | single arm `array_iterate.rs:17` | O(n*c) + per iteration: `array_element_value` (lookup, plus `dynamic_has("__elemAcc")` string lookup when element is an object), `vec![3 Value]`, `invoke_callback` | O(n*c) minus per-iter overhead | per iteration: 1 `Vec<Value>` (3 elems) + per-callback `resolve_call_target`/`read_body` (F5) | no | high |
| reduce / reduceRight | `array_iterate.rs:128`; NoInit `array_mutate.rs:376` | O(n*c), accumulator held in 1-slot GC array | same | per iteration: `Vec<Value>` (4) + box + heap read/write of accumulator; NoInit allocs holder array | no | high |
| findLast / findLastIndex | `array_iterate.rs:328` | O(n*c) | same | per iteration `Vec<Value>` | no | medium |
| flat | `array_iterate.rs:367` | O(total), recursion | same | result array | no | low |
| flatMap | `array_iterate.rs:414` | O(n*c) | same | per iter `Vec<Value>` | no | medium |
| group / groupToMap | `array_iterate.rs:460` | O(n*c) | same | not examined in detail (not confirmed) | no | low |
| fill | `array_iterate.rs:542` | O(range), `array_set` per slot (lookup + barrier check each) | O(range) `slice::fill` | 0 | no | medium |
| copyWithin | `array_iterate.rs:542` | O(count) | same | temp `Vec<Any>` | no | low |
| at | `array_iterate.rs:305` | O(1) | O(1), inline | 0 | no | medium (common in loops) |
| keys/values/entries | `array_iterate.rs:528` -> `alloc_array_iterator_mode`; step `zun_gc/src/iterators.rs:4` | O(1) create; per step 2 HashMap lookups; entries allocs a 2-array per step | same; entries pair unavoidable unless for-of destructuring is fused | 1 iterator obj (+2 map inserts); entries: 1 array per step | no | medium |
| Array.isArray | `array_construct.rs:16` | O(1) | O(1) | 0 | no | low |
| Array.from / fromMapped | `array_construct.rs:465` | O(n) | O(n) | not examined in detail (not confirmed) | no | low |
| spread `[...x]` | `arraySpreadAppend` `array_construct.rs:703` | O(n), per-element heap get + push; string case builds `Vec<String>` | O(n) bulk extend | string: n `String`s | no | medium |
| destructuring | `arrayDestructure/destructOpen/Step/Rest` `array_construct.rs:27,393-444`; `Step` on array -> `arrayAt` -> full native dispatch | O(1) per step but each step is a name-dispatched native call | inline index load | per step: `Vec<Value>` args | no | medium |
| dynamic (`any` receiver) calls | `ARRAY_METHODS` linear scan `zun_core/src/arrays.rs:133`, `dynamic_call.rs:457` | O(#methods) string compares per call | perfect hash / enum | arg `Vec` normalisation | n/a | low |

## Findings

### F1. sort / toSorted: quadratic insertion sort with per-step heap round-trips and per-compare String allocation (critical)
- Evidence: `zun_interp/src/collections/array_iterate.rs:182-241` comment "Insertion sort so each comparison re-enters the interpreter at most once"; inner loop `for i in 1..length { while j > 0 { left = heap.array_get(j-1); right = heap.array_get(j); ... heap.array_set x2 } }`. Default branch: `stringify_any(left, heap).unwrap_or_default()` and the same for `right` on every comparison. `array_mutate.rs:452-520` (`toSorted`) duplicates the loop after a copy.
- Why: O(n^2) comparisons and swaps (each swap = 2 gets + 2 sets, i.e. ~4 HashMap lookups + 2 write-barrier checks); default sort stringifies each element O(n) times instead of once; comparator calls allocate `vec![Value::Any(left), Value::Any(right)]` each time.
- Proposal: read the elements once into a `Vec<Any>` (rooted via the existing `NativeRootScope`/a side root), run a stable merge sort (`slice::sort_by` cannot be used directly because the comparator is fallible and can GC; use a hand-written merge sort that propagates `Err`), write back once. For the default comparator pre-compute `Vec<(String, Any)>` once and use `sort_by` (infallible, stable). Keep the "NaN/non-numeric result = 0" rule (`!(x > 0.0)`) and hole/undefined ordering of the spec. Holes and `undefined` must still sort last (not confirmed that the current code does this; verify with a test before changing).
- Estimated gain: n=10k default sort from O(n^2)=~5e7 string-allocating compares to ~1.3e5; comparator sort from 5e7 to ~1.3e5 callback invocations (hundreds of x for n >= 10k).
- Risk: comparator call order is observable (side effects in comparator, Node/V8 uses TimSort). Matching Node's exact call sequence is not required by output unless the comparator logs; a stable sort gives the same result for consistent comparators. Medium.
- Touches frozen layout/.zm/ABI/GC: no (uses existing root scope). No approval needed.

### F2. splice / shift / unshift: one Vec remove/insert per element (high)
- Evidence: `array_mutate.rs:91` (`for offset in 0..delete_count { heap.array_remove(source, start) }`), `:97` (`heap.array_insert(source, start + offset, element)` in a loop), same at `:123`, `:130` (`arraySpliceInsert`) and `array_iterate.rs:297` (`arraySplice`). `ArrayStorage::remove/insert` are `Vec::remove/insert` (`zun_core/src/arrays.rs:63-73`), each O(n - index). `shift` = `array_remove(source, 0)` (`array_iterate.rs:261`), `unshift` = `array_insert(source, 0, v)` (`:273`).
- Why: splice(i,k,m items) is O(n*(k+m)); a queue loop with `shift()` is O(n^2) in total (typical BFS/queue idiom). `unshift(a,b,c)` is lowered by sema into 3 separate natives (`mcall_arrays.rs:77-120`), i.e. 3 full shifts.
- Proposal: (a) add `ArrayStorage::splice(start, delete_count, items) -> Vec<Any>` using `Vec::splice` / `drain` (one memmove), expose `Heap::array_splice` (applying write barriers for object children), use it in the three arms; (b) for `shift`, change `ArrayStorage.elements` to a `VecDeque<Any>` or a `Vec` plus a `head: usize` offset (requires approval: it changes `ArrayStorage`, the GC-visible storage, and the JIT/thunk access paths that use `get`/`iter`); (c) for multi-arg unshift, gather items into one array and use the same bulk insert.
- Estimated gain: splice 10-1000x for large k/m; `shift` queue loops from O(n^2) to O(n) with (b).
- Risk: (a) low; (b) medium (every consumer of `elements`, including `object_references`, `rewrite_object_references`, GC). Hole semantics in removed results must stay: `arraySlice`/`toSpliced` convert holes to `undefined`, `splice` result preserves them (current code copies raw `Any` including holes) - keep as is.
- Touches frozen layout/.zm/ABI/GC: (a) no; (b) ArrayStorage internals/GC - requires approval.

### F3. No Array method is JIT-lowered; array opcodes are extern calls without FuncRef dedup (high)
- Evidence: `grep '"array[A-Z]' crates/zun_codegen/src` returns nothing; `eligible.rs` only admits `date/math/collection/allocation/string` builtins (`eligible.rs:115-121`) and the opcodes `ArrayLength|NewArray|LoadArray|StoreArray|ArrayPop|ArrayPush` (`eligible.rs:88-93`). `translate/arrays.rs:156-444` each `*_fref` helper does `declare_function(...)` + `declare_func_in_func(...)` on every call (e.g. `array_load_f64_fref`, L179-189); no cache field found (`grep funcref_cache|fref_cache` is empty). This contradicts the CLAUDE.md rule "Dedup FuncRef/SigRef imports". Every load/store/len/push/pop is `call` to a thunk that does `with_current_heap` (TLS) + `HashMap<usize, ArrayStorage>::get` (`zun_gc/src/arrays.rs:10`, `thunks/misc.rs:55`).
- Why: a hot loop `for (i<arr.length) s += arr[i]` pays an out-of-line call + TLS + hash per iteration; also the same FuncRef is re-declared once per array op in the function body (compile-time cost, plus unnecessary IR bloat). Any function that calls `.indexOf/.map/.slice/...` is excluded from the JIT entirely (eligible implies lowerable consistently, but the set is tiny).
- Proposal: (1) cache FuncRef per FuncId in `Translator` (HashMap<&'static str, FuncRef>) for all `array_*_fref` - no semantic change; (2) lower cheap pure natives as thunks: `arrayIndexOf/Includes/At/Reverse/Fill/SliceRange` for typed-numeric arrays via `CallBuiltin` entries in `builtins_table.rs` + `eligible.rs` `collection_builtin` (recipe in memory "Zun JIT eligibility"); (3) keep callback methods (`map/forEach/...`) in the interpreter until the JIT-interp bridge supports `CallIndirect` (not confirmed whether bridge slice is complete). Inline length/load fast paths need the array layout work noted in the shared doc.
- Estimated gain: (1) compile-time only; (2) removes Tier 0 fallback for functions that call these; magnitude depends on bench (not measured).
- Risk: low for (1)/(2); every new lowered builtin needs `eligible <=> lowerable` consistency and an interp-vs-JIT equivalence test.
- Touches frozen layout/.zm/ABI/GC: no for (1)/(2) (thunk signatures are the FFI contract but new ones are additive). Inline loads: requires approval.

### F4. indexOf / lastIndexOf / includes: per-element boxed Value comparison (high)
- Evidence: `array_mutate.rs:229-307`. For each index: `is_hole(index)` (a `with heap array_get` lookup, `:263`), then `element(index)` = `heap.array_get` + `Value::unbox_any(any, heap)` (`:253-258`) producing an owned `Value` (string/object variants clone), then `strict_equals(&Value, &Value, zero_aware)`. The needle is unboxed once, which is fine.
- Why: 2 HashMap lookups + a `Value` construction per element; strings may allocate on unbox (not confirmed - depends on `Value::Str` repr, check `unbox_any`).
- Proposal: take one `&ArrayStorage` borrow (add `Heap::with_array(obj, |elems: &[Any]|)`), compare `Any` bit patterns directly for numbers/bools/objects/interned strings (strings interned by id => equality by id if interning is canonical; not confirmed), and fall back to `strict_equals` only for non-interned cases. `from` argument is `F64` typed (`array_ext.rs:273`) which is fine.
- Estimated gain: 3-10x on large numeric arrays (estimate, not measured).
- Risk: low-medium (-0/+0 and NaN semantics: `includes` is SameValueZero, `indexOf` strict; keep `zero_aware` flag).
- Touches frozen layout/.zm/ABI/GC: no (a new read-only Heap accessor).

### F5. Callback iteration: per-iteration allocation and per-callback re-resolution (high)
- Evidence: `array_iterate.rs:17-127`: per element `vec![Value::Any(element), Value::Any(Any::float(index)), Value::Any(Any::object(roots[0].addr()))]` then `invoke_callback`. `runtime_typed_array.rs:500-524` `invoke_callback`: `resolve_call_target(handle, call_args)`, `method_defs.get(..)`, `read_body(&self.module.code, target.body_offset)` (decodes the method body header on every call), `self.module.strings.get(target.name).ends_with("@rest>")` (string compare on every call), `final_args.truncate`. `array_element_value` (`runtime_typed_array.rs:471-498`) for object elements runs `heap.dynamic_has(record, "__elemAcc")` (string-keyed lookup) on each element, so arrays of objects pay a string hash lookup per element per callback. `visits_holes = matches!(name, ...)` is recomputed per iteration (`:41`) and the `name` string is re-matched with `match name` for each result (`:74`).
- Why: constant overhead per element dominates cheap callbacks (n=1e6 -> 1e6 Vec allocs + 1e6 string compares + 1e6 body decodes).
- Proposal: hoist loop invariants out of the loop: resolve the callback target, `body.param_count`, and the `@rest>` flag once before the loop (cache `(method_rid, param_count, is_rest)` in a small `CallbackPlan`), reuse one `Vec<Value>` args buffer with `clear()`/push, and replace the `name` string matches by a local `enum Kind` computed once. Move the `__elemAcc` check behind a per-array flag or only for objects that are dynamic records (check `heap.is_dynamic_object` first, which is cheaper than a string hash; not confirmed which is cheaper without measuring).
- Estimated gain: 1.5-3x on cheap callbacks (estimate, not measured).
- Risk: low; must preserve the GC rooting contract (roots re-read after each callback) and the argument count truncation rule (callbacks with fewer params).
- Touches frozen layout/.zm/ABI/GC: no.

### F6. join / toString: per-element allocations and repeated length lookup (high)
- Evidence: `runtime_coerce.rs:368-409` `array_join_dyn`: re-reads `array_len` every iteration (`:379`), `array_get` + `unbox_any` per element, `Vec<String> parts`, `parts.join(separator)`; caller `array_iterate.rs:244-252` does `separator` `to_string()`, then `wtf16::normalize(joined).into()` (a full extra pass/copy; not confirmed whether `normalize` is zero-copy when valid).
- Why: n+3 allocations and 3 passes for what can be one pass into a pre-sized `String`.
- Proposal: write directly into one `String` (`push_str(separator)` between items); skip `normalize` when no lone surrogates were produced (track during the loop); fast path when all elements are strings/numbers (no `Value` unbox, call `stringify_any` directly, which `array_join_dyn` already does for the non-object case).
- Estimated gain: 2-4x (estimate).
- Risk: low; keep the cycle guard (`inner != obj`) and the `ToPrimitive` path for objects.
- Touches frozen layout/.zm/ABI/GC: no.

### F7. Bulk copy methods use element-wise get/set through the heap map (medium)
- Evidence: `slice` (`array_mutate.rs:167-209`), `toReversed` (`:522`), `with` (`:546`), `toSpliced` (`:594`), `concat` (`:662`), `reverse` (`:211`), `fill` (`array_iterate.rs:542`): each element does `heap.array_get(...)` and `heap.array_set(...)` = 2 HashMap lookups + write-barrier check per element. `concat` first copies receiver and items into a temporary `flat: Vec<Any>` and then into the result (2 copies), and `mcall_array_concat` builds an extra items array (`mcall_arrays.rs:676-712`). `slice` also `args.to_vec()` (`:172`) just to coerce objects. `alloc_array(n)` fills with holes first (`ArrayStorage::with_length`, `zun_core/src/arrays.rs:15`) then overwrites.
- Why: 3-4x more hash lookups than needed; double initialization.
- Proposal: add `Heap` bulk helpers: `array_extend_from_slice`, `array_copy_range(src, range) -> Obj`, `array_reverse_in_place`, `array_fill_range`; use `Vec::with_capacity` + `extend_from_slice` so the result is built in one pass; one write-barrier pass for object children only if the array holds any (could track "contains objects" lazily; not confirmed needed). Avoid `to_vec()` unless an arg is an object.
- Estimated gain: 2-5x on large copies (estimate).
- Risk: low; hole -> undefined mapping differs per method (`slice` keeps holes via raw get, `toReversed/with/toSpliced/arraySlice` map hole->undefined; preserve per method).
- Touches frozen layout/.zm/ABI/GC: no (new methods on existing storage); write-barrier call must be kept for object children.

### F8. Array iterators (keys/values/entries) cost two map lookups per step and allocate per entries step (medium)
- Evidence: `zun_gc/src/iterators.rs:4-17`: `array_iterators.get_mut(iterator)` then `arrays.get(array)` (2 HashMap lookups); `mode 2` calls `alloc_array(2)` + 2 `array_set` per step. `alloc_array_iterator_mode` (`allocation.rs:235`) allocates an iterator object plus a HashMap entry. `array_ext.rs:56-58` comment says these return "a fresh managed Array" but the implementation returns an iterator (comment is stale; documentation only).
- Why: for-of over `arr.entries()` allocates a pair array per element; `[i, v]` destructuring then reads it back.
- Proposal: (a) keep as is for semantics, but when `for (const [i, v] of arr.entries())` is statically seen, have sema lower to an indexed loop (no iterator, no pair). (b) store the array's raw storage pointer in the iterator state to drop one lookup (requires the GC to treat this as a root; touches GC, requires approval).
- Estimated gain: (a) removes 1 alloc + 3 lookups per element for the common pattern.
- Risk: (a) low-medium (must preserve mutation-during-iteration semantics: length is re-read each step, as `ArrayIterator::advance` does). (b) medium.
- Touches frozen layout/.zm/ABI/GC: (a) no; (b) GC - requires approval.

### F9. Name-string dispatch for every array native (medium; cross-module, see shared doc)
- Evidence: `call_managed_collection_inner` (`zun_interp/src/collections/mod.rs:191-`) runs, before reaching the array arms: `typed_arrays::call(name)`, `node_buffer::call(name)`, `call_typed_array_ext(name)`, `call_decorator_native(name)`, `guard_array_native(name)` (`integrity.rs:66`, a string match), a regexp `matches!(name, ...)` of ~20 strings, and only then the big `match name` (array arms at `mod.rs:541-586`), which then re-matches the same name inside `col_array_iterate/col_array_mutate`. `call_managed_collection` also does `name.starts_with("dynamicCall")` twice (`:161`, `:179`).
- Why: ~8 string comparisons/prefix scans per array native call, before any array work; the cost is per call, so it matters for small ops (`at`, `indexOf` on short arrays, `push` with spread).
- Proposal: resolve the native id to an enum/function pointer at load/link time (the `CallBuiltin` operand already carries the id), and call the handler directly. Out of scope for the Array doc; listed in the shared report.
- Touches frozen layout/.zm/ABI/GC: no ABI change if the id table stays internal; the id->handler table is new.

### F10. Smaller items
- `arrayFindLast` (`array_iterate.rs:328`) does not use `array_element_value` and does not convert holes to `undefined` (the forward `find` does): this is a semantic inconsistency, not a perf item (not confirmed against Node output; verify with a test).
- `arrayFill` (`array_iterate.rs:542`) boxes the fill value once (good) but then calls `heap.array_set` per slot, which re-checks the write barrier for object values every slot.
- `ARRAY_METHODS` (`zun_core/src/arrays.rs:133`) uses a linear scan of ~45 rows on the dynamic path (`dynamic_call.rs:457`); trivial but replaceable by a `match` or `phf`.
- `arrayReduce` keeps the accumulator in a one-slot GC array and does 2 heap round-trips per iteration (`array_mutate.rs:421-440`); a rooted `Any` local in the native root scope would remove them (not confirmed that `NativeRootScope` supports an updatable slot; check before changing).
- sema types `pop()` as `Ty::Any` (`mcall_arrays.rs:52-60`), so typed `number[]` pop results are boxed/unboxed (not confirmed that the unboxing is elided downstream).
