# JSON (stringify / parse) optimization audit

## Summary
- `JSON.stringify` is a Rust interp native (`objectJsonStringify`, `crates/zun_interp/src/collections/object_core.rs:704` -> `crates/zun_interp/src/json.rs:74`); it builds one fresh `String` per node and joins them bottom-up, so output is copied once per nesting level, and every key access is a by-name lookup.
- `JSON.parse` is NOT native: it is a recursive-descent parser written in Zun (`crates/zun_prelude/builtin/json.zun:177-505`) that appends strings one char at a time (`out = out + substring(...)`) and makes several native calls per input byte.
- JIT lowering of the prelude parser was not checked in `eligible.rs` (not confirmed); stringify is a native call and is not lowered.

## Per-method table

| method/phase | current path file:line | current complexity | possible complexity | allocs/call | JIT eligible/lowered | severity |
|---|---|---|---|---|---|---|
| stringify: per-node output | json.rs:411-457, 461-568 | O(n * depth) bytes copied (child String re-copied by every ancestor `join`/`format!`) | O(n) single buffer | 2+ Strings per node (`parts` Vec, joined String, `format!`) | native, n/a | High |
| stringify: string quoting | json.rs:36-57 | O(len); `format!` per control char / lone surrogate | O(len), bulk copy of clean runs | 1 String per string/key | native | Medium |
| stringify: cycle guard | json.rs:443-449 | O(depth) per object node (depth <= 256) | bounded, fine | 0 | native | Low |
| stringify: key enumeration | json.rs:525-543 | O(k) + `Vec<String>` per object; typed path `object_own_entries` builds (key, Value) pairs and discards values | O(k) without value materialization | 1 Vec + k Strings per object | native | Medium |
| stringify: property read | json.rs:227-275; metadata.rs:144-174 | per key: `parse::<usize>` + `to_string()` compare, 5 heap lookups by name (dynamic); typed: linear field scan comparing names | O(1) per key via slot iteration | `Value::Str(key.into())` per node (json.rs:332) | native | High |
| stringify: array elems | json.rs:468-514 | O(n); `index.to_string()` key + `Value::Str` per element + `__elemAcc` dynamic_get probe on object elements | O(n) without key strings unless toJSON/replacer | 2 Strings per element | native | High |
| stringify: callbacks (replacer/toJSON) | json.rs:189-223 | boxes every arg; `json_takes_this` string suffix test per call | cache per call | Vec + record per call | native | Low (rare path) |
| stringify: class-name checks | json.rs:393-408, 278-289 | per object node allocates a `String` for the type name | compare cached type rids | 1 String per object | native | Medium |
| parse: whole | json.zun:177-505 | O(n) calls, high constant | native O(n) | see below | Tier0 prelude (JIT status not confirmed) | High |
| parse: string literal | json.zun:353-378 | per char `substring` + concat (O(len^2) unless concat appends in place - not confirmed) | O(len) | 1 substring + 1 concat String per char | not confirmed | High |
| parse: peek/whitespace/number | json.zun:199-204, 460-504 | per byte `length` + `charCodeAt`; number = `substring` + `parseFloat` | O(1) per byte | 1 substring per number | not confirmed | Medium |
| parse: object build | json.zun:312-350 | O(k); `[key,value]` pair Array per member + `Object.fromEntries` | direct dynamic-object set | 1 Array per member + entries Array | not confirmed | Medium |
| parse: error path | json.zun:191-196 | cold | - | - | - | Low |

## Findings

### F1 (High) `parse` is a Zun-level, per-char, native-call-heavy parser
- Evidence: `json.zun:373`: `out = out + coreString.substring(this.src, this.pos, this.pos + 1);`; `json.zun:199-204`: `peek()` calls `coreString.length(this.src)` then `charCodeAt` on each call; `skipWhitespace` calls `peek` per byte.
- Why: a string allocation per character and several interpreted + native calls per byte.
- Proposal: implement `JSON.parse` as a Rust native like `objectJsonStringify`: single pass over the string buffer, `push_str` clean runs, build arrays/objects via the heap API directly. Keep SyntaxError messages/positions (json.zun:191-196) and depth limit 512 (json.zun:35).
- Est. gain: 20-100x on large documents (estimate, not measured).
- Risk: medium (error-message parity, lone surrogates via `\uD800`, `-0`, `__proto__` keys, GC-rooting of partial containers).
- Touches frozen layout/.zm/ABI/GC? No layout/.zm change; adds a native intrinsic in the `zun:core` object namespace (requires approval to extend the native table); allocation during parse must be GC-safe.

### F2 (High) stringify copies children into parents at every level and allocates a key string per node
- Evidence: `json.rs:507-514` and `555`: `format!("[{}]", parts.join(","))`, `format!("{}{separator}{text}", quote(&key))` where `text` is the child complete String; `json.rs:332` `let key_value = Value::Str(key.into());` for every node; `json.rs:504` `&index.to_string()`.
- Why: output at depth d is copied about d times; many short-lived Strings.
- Proposal: thread one `&mut String` through `json_property`, return emitted/skipped (truncate back to a saved length when a member is undefined), build the key `Value` lazily only when a replacer/toJSON exists, format indices into a stack buffer.
- Est. gain: 2-5x on nested data (estimate).
- Risk: low-medium (skip semantics, array slot -> "null").
- Touches frozen layout/.zm/ABI/GC? No.

### F3 (High) Per-key property lookup by name
- Evidence: `json.rs:229` (`key.parse::<usize>()` and `i.to_string() == key` for every key), `json.rs:246-255` (five separate heap lookups per key), `metadata.rs:157-170` (typed field search is a linear scan comparing strings; `format!("get_{name}")` fallback at line 172).
- Why: keys were just enumerated from the same object; values could be read in the same walk.
- Proposal: iterate dynamic-object (key, slot) once with flags in one heap call; iterate `field_defs` of the type directly with cached layouts instead of `object_own_entries` + name lookup; parse key as index only for array-like/typed-array objects.
- Est. gain: 2-4x on object-heavy payloads (estimate).
- Risk: medium (enumeration order/filter must stay identical to `object_own_entries`, also used by serde.rs:10,316 and object_core.rs:80,128).
- Touches frozen layout/.zm/ABI/GC? No (read-only).

### F4 (Medium) Quote path: `format!` and per-char matching
- Evidence: `json.rs:48,50` `format!("\\u{:04x}", ...)`; `json.rs:39,49` iterates `chars()` and calls `wtf16::lone_unit(c)` for every char.
- Proposal: byte-scan with a 256-entry escape table, `push_str` clean runs, hex via static table; check lone surrogates only for the relevant lead byte (assumes WTF-8 storage - not confirmed, verify in `crates/zun_core/src/wtf16.rs`).
- Est. gain: 3-8x on long ASCII strings (estimate). Risk: low. No layout/ABI/GC impact.

### F5 (Medium) Allocating `json_class_name` per object node and cloned key lists
- Evidence: `json.rs:394` `let class = self.json_class_name(obj);` (returns `String`, json.rs:287); `json.rs:526` `list.clone()` per object when a property list is given; `json.rs:529-535` collects `Vec<String>`.
- Proposal: compare `obj.type_rid()` with rids resolved once per call; borrow the list.
- Est. gain: 5-15% on object-heavy output (estimate). Risk: low. No layout/ABI/GC impact.

### F6 (Low) Guard / dead code
- Cycle guard is a linear scan over at most 256 ancestors (`json.rs:443-449`); bounded. `MAX_DEPTH = 256` (json.rs:14) reports "circular structure" for merely deep input, while the prelude uses 512 (json.zun:35) - a semantic difference from Node, not a performance issue.
- `JSON.write`/`quote`/`escapeControl`/`hexDigit` (json.zun:39-173) are not called by `stringify` (json.zun:29-31); they are still compiled into every program (other callers not checked).

## Cross-module references
- Native-call cost, String layout/escaping, `object_own_entries` machinery shared with serde (serde.rs:10,316), and interp decode: see the shared doc (final report).
