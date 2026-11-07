# String built-ins: optimization audit

Scope: String methods only. Read-only audit; line numbers as of commit da5d265a. Only facts verified in code are stated; hypotheses are marked "not confirmed". Cross-module issues (native-call marshaling, WTF-16-over-UTF-8 representation, interpreter decode loop) are tracked in `_shared.md`.

## Summary

1. Strings are stored as UTF-8 `Rc<str>` with lone surrogates as plane-15 stand-ins, so `length` and every index operation (`charCodeAt`, `charAt`, `s[i]`) is O(n) per call. `for (i < s.length) s.charCodeAt(i)` is therefore O(n^2), in the JIT as well as in Tier 0.
2. Nearly every other native (`indexOf`, `slice`, `at`, `substring`, `padStart`, `codePointAt`, `reverse`, dynamic `stringMethod`) first materializes the whole string as `Vec<u16>` and then slices it: one full O(n) copy plus an allocation per call, even for O(1) results such as `at`.
3. The D3 typed natives (`charCodeAtNum`, `charAtNum`, `charIndex`) and `slice`, `at`, `codePointAt` are not in the JIT whitelist (`eligible.rs:286`, `:543`), which matches the name `charCodeAt` but not `charCodeAtNum`. Any function using `s.charCodeAt(i)` with an `f64` index, `s[i]`, `slice` or `at` falls back to Tier 0 as a whole.

## Per-method table

"mat" = whole-string `Vec<u16>` materialization (`wtf16::encode(..).collect()`). "JIT" = in the `string_builtin` whitelist (`eligible.rs:286`) and has a lowering in `translate/strings.rs::op_call_string`. When the typed sema path (`mcall_string.rs:103-241`) is not taken, the call goes through `collections::stringMethod` (boxed `Any` args, name-string match at `string_dyn.rs:136`).

| method | current path (file:line) | current complexity | possible complexity | allocs per call | JIT eligible/lowered | severity |
|---|---|---|---|---|---|---|
| length | natives/string.rs:5; thunk thunks/string.rs:9 -> `wtf16::len16` (wtf16.rs:41) | O(n) char scan | O(1) (cached/ASCII flag; requires approval) | 0 | yes / yes (libcall) | high |
| charCodeAt (int idx) | natives/string.rs:300 `encode().nth(i)`; thunk string.rs:46 | O(i) | O(1) for ASCII/cached | 0 | yes / yes (libcall, not inlined) | high |
| charCodeAt (f64 idx, typed) | mcall_string.rs:132 -> `charCodeAtNum` natives/string.rs:323, `unit_at` :387 | O(i) | O(1) | 0 | NO (name `charCodeAtNum` not whitelisted) | high |
| charAt (f64 idx, typed) | `charAtNum` natives/string.rs:338 | O(i) + String alloc | O(1); cached 1-char strings | 1 String | NO | high |
| s[i] | member.rs:255 `charIndex` natives/string.rs:354 | O(i) + String alloc | O(1) | 1 String | NO | high |
| charAt / charCodeAt (other arg types) | `stringMethod` string_dyn.rs:266 / :274 | O(n) mat + dyn dispatch | O(1) | Vec<u16>(n), cloned args | NO | med |
| at | natives/string.rs:501; typed only if arg is F64 (mcall_string.rs:173) | O(n) mat | O(1) | Vec<u16>(n) + String | NO | med |
| codePointAt | natives/string.rs:523 (int idx only, mcall_string.rs:168) | O(n) mat | O(1) | Vec<u16>(n) | NO | med |
| slice | natives/string.rs:549 | O(n) mat + O(k) | O(k) | Vec<u16>(n) + String(k) | NO | high |
| substring | natives/string.rs:177; thunk string.rs:191 | O(n) mat + O(k) | O(k) | Vec<u16>(n) + String(k) + Rc | yes / yes | high |
| substr | `stringMethod` only (string_dyn.rs:155) | O(n) mat | O(k) | Vec<u16>(n) + String | NO (no typed native) | med |
| indexOf | natives/string.rs:109; thunk string.rs:82 | 2x mat + O(n*m) window scan | O(n) search, 0 alloc | 2 Vec<u16> | yes / yes | high |
| lastIndexOf | natives/string.rs:126; thunk string.rs:105 | same as indexOf | same | 2 Vec<u16> | yes / yes | high |
| includes / contains | natives/string.rs:73 `str::contains`; thunk :32 | O(n) | O(n) | 0 | yes / yes | low |
| startsWith / endsWith | natives/string.rs:396 / :405; thunks :56 / :69 | O(m) | same | 0 | yes / yes | low |
| padStart / padEnd | natives/string.rs:205 / :237; thunk string.rs:296 `jit_pad_string` | native: mat + cycle; JIT: `len16` rescan per loop iteration | O(target) | native: 3 Vec + String; JIT: String loop + Vec + format! | yes / yes | med |
| repeat | natives/string.rs:423 -> string_ops.rs:146; thunk string.rs:270 | O(n*count) | same | 1-2 String (`normalize` pass) | yes / yes | low |
| trim / trimStart / trimEnd | natives/string.rs:414 / :282 / :291, then `.to_string()` | O(n) copy | return receiver when nothing trimmed | 1 String (+ Rc in thunk) | yes / yes | low |
| toLowerCase / toUpperCase | natives/string.rs:268 / :275 | O(n) | O(n), skip when already in case | 1 String | yes / yes | low |
| replace (string, first only) | natives/string.rs:148 `str::find` | O(n) | same | 1 String | yes / yes | low |
| replaceAll | natives/string.rs:578; sema mcall_string.rs:396 | O(n) per piece; empty search with `` $` ``/`$'` is O(n^2) (:589-:597) | O(n) | several Strings | NO | low |
| reverse (zun ext.) | natives/string.rs:491; thunk string.rs:284 | O(n) | O(n) | Vec<u16> + String | yes / yes | low |
| concat / `+` | natives/string.rs:12 `format!`; thunk string.rs:175 | O(n+m) | rope/builder (requires approval) | 1-2 String copies | yes / yes | med |
| compare | natives/string.rs:21; thunk :156 | O(n) | O(n) | 0 | yes / yes | low |
| split | `stringMethod` string_dyn.rs:300 -> string_ops.rs:158 | O(n) | O(n) | 2 Vec<u16>(n) + Vec<String> + array + interned pieces | NO | med |
| normalize / toString / valueOf | string_dyn.rs:330 / :341 | O(n) | O(1) for toString/valueOf | eager `units(recv)` (F6) | NO | low |
| `for (c of s)` | iterators.rs:61 `stringIteratorNew` | O(n) eager | lazy O(1) start | Vec<String>(n) + array(n) + n interned | NO | med |
| toInt / toFloat / toBoolean | natives/string.rs:37 / :49 / :61 | O(n) | same | 0 | yes / yes | low |

## Findings

### F1 (high): `length` is O(n); `charCodeAt` / `charAt` / `s[i]` are O(i)

- Evidence: `wtf16.rs:41-51` (`len16`: `text.chars().map(..).sum()`), `thunks/string.rs:9-14` (length thunk calls `len16`), `thunks/string.rs:46-53` (`encode(value).nth(index)`), `natives/string.rs:387-394` (`unit_at` uses `.nth`), `:364` (`charIndex`), `:313`.
- Why: strings are UTF-8 `Rc<str>` with no cached length and no ASCII flag, so unit count and nth unit both need a scan. `for (let i=0;i<s.length;i++) s.charCodeAt(i)` costs O(n) for the length plus O(i) for the index per iteration: O(n^2) overall. The comment at `natives/string.rs:385` ("no per-call allocation") is accurate for allocation, but the cost stays linear.
- Proposal A (no layout change): ASCII fast path. When `value.is_ascii()`, `len16 == value.len()` and `nth(i) == bytes[i]`. `is_ascii` is itself O(n) (SIMD-fast), so this reduces the constant, not the complexity. Estimated gain: 5-20x on long ASCII strings (estimate, not measured).
- Proposal B (requires approval): cache `(len16, is_ascii)` per interned string handle in the heap string table, filled at `intern_string`. Gives O(1) `length` and O(1) ASCII indexing; for non-ASCII, a lazily built UTF-16 side buffer. Loop becomes O(n).
- Risk: low for A; medium for B (GC string-table invariants, memory per string).
- Touches: A none. B the heap string table, and the frozen String layout if the cache lives in the object rather than a side table: requires approval.

### F2 (high): JIT whitelist does not cover the D3 typed natives, `slice`, `at`, `codePointAt`

- Evidence: `eligible.rs:290-318` and `:543-570` list `charCodeAt` but not `charCodeAtNum`, `charAtNum`, `charIndex`, `slice`, `at`, `codePointAt`. These natives are registered at `natives/string.rs:966-1054`. `mcall_string.rs:132-153` emits `charCodeAtNum` / `charAtNum` for an `f64` index (the usual type of `i` in TS loops); `member.rs:255` emits `charIndex` for `s[i]`. `builtins_table.rs:26-190` has no entry for them either.
- Why: `string_builtin` (`eligible.rs:286`) matches on the native name, so the function containing any such call is ineligible and runs entirely in Tier 0. Commit dc08219c made the call typed but not lowerable.
- Proposal: add lowering for `charCodeAtNum` (libcall `(handle, f64) -> f64`), `charAtNum`, `charIndex`, `slice`, `at`; keep eligibility and lowering in the same change (eligible iff lowerable). For the hot `charCodeAtNum` path, inline: `fcvt_to_sint_sat` on the index, bounds check, and a cold block for the slow thunk; combined with F1-B the fast path can load the byte directly for ASCII.
- Estimated gain: removes the Tier-0 fallback for whole functions using these (parser/tokenizer style code); magnitude not measured here.
- Risk: medium; needs interp/JIT equivalence tests for NaN, negative, fractional, +-Infinity indices.
- Touches: no frozen layout, no .zm, no ABI break (new thunk symbols; existing `call.builtin` ids reused, no new opcode).

### F3 (high): whole-string `Vec<u16>` materialization for O(1) / O(k) results

- Evidence: `natives/string.rs:198` (substring), `:513` (at), `:536` (codePointAt), `:567` (slice), `:118-119` and `:139-140` (indexOf / lastIndexOf build two Vecs), `:226` (padStart), thunks `string.rs:90-91`, `:113-114`, `:136-137`, `:199`; `string_dyn.rs:133` (`let s = units(recv)` for every dynamic method); `string_ops.rs:159` (`split_on`).
- Why: each call allocates and fills 2n bytes then discards them. `s.at(-1)`, `s.slice(0,3)`, `s.codePointAt(i)` should be O(1)/O(k).
- Proposal: (a) if `value.is_ascii()` (no 4-byte stand-ins possible), operate on `as_bytes()` and `&value[b..e]` directly; (b) otherwise a streaming pass instead of `collect()`, or collect only the needed window; (c) `indexOf` currently scans windows in O(n*m) (`thunks/string.rs:96`); use `str::find` when no stand-ins are present and convert the byte offset to a unit offset by counting.
- Estimated gain: removes 1-3 allocations and one O(n) pass per call (magnitude not measured).
- Risk: low-medium; byte-to-unit offset conversion must be exact for non-ASCII (keep slow path).
- Touches: nothing frozen (natives and thunks only).

### F4 (med): `padStart` / `padEnd` JIT thunk is quadratic and diverges from the native

- Evidence: `thunks/string.rs:316-318`: `while len16(&padding) < padding_units { padding.push_str(&fill); }` rescans O(len) every iteration, then `encode(&padding).take(..).collect::<Vec<_>>()` and `format!`. `:307-309` returns handle `0` on an empty fill, while the native (`natives/string.rs:220`) and the spec return the receiver unchanged.
- Why: O(units^2 / fill_len) for a large target with a short fill. The empty-fill difference is a Tier0/Tier1 divergence; the practical effect of handle 0 is not confirmed.
- Proposal: reuse `string_ops::pad_units` (as the native does); single allocation of `target` units.
- Estimated gain: O(n^2) to O(n); removes 3 allocations.
- Risk: low. Touches nothing frozen.

### F5 (med): JIT thunks diverge from natives on `repeat` and `concat`

- Evidence: `thunks/string.rs:270-281` `repeat` uses `value.repeat(count)` with no length cap and no `wtf16::normalize`, and returns `0` for a negative count; the native path uses `string_ops::repeat` with `REPEAT_MAX` and normalize (`string_ops.rs:146-154`). `:175-188` `concat` uses `push_str`, while `wtf16::concat` (`wtf16.rs:98`) merges a trailing high surrogate with a leading low surrogate; whether the interpreter `+` uses `wtf16::concat` was not checked (not confirmed).
- Why: tier equivalence is an invariant; unbounded repeat can attempt a multi-GB allocation.
- Proposal: route thunks through the same `string_ops::repeat` / `wtf16::concat` helpers and signal errors rather than returning 0.
- Estimated gain: correctness; perf neutral. Risk: low. Touches nothing frozen.

### F6 (med): dynamic `stringMethod`: eager `units(recv)`, arg clones, name-string match

- Evidence: `string_dyn.rs:131-133` clones `a0`/`a1` and computes `let s = units(recv)` before `match name` (`:136`), so `toUpperCase`, `toString`, `valueOf`, `normalize`, `repeat`, `split` pay a Vec<u16> build they do not use. `mcall_string.rs:187-241` sends every non-static shape to `stringMethod` with the method name as a string literal plus padded `undefined` args and a `Convert` from `Any`. Static shapes (`mcall_string.rs:157-185`) are narrow: `indexOf` / `includes` / `startsWith` / `endsWith` with exactly one string argument (no position), `padStart` with exactly (int, string) so `padStart(n)` with default fill is dynamic, `slice` only with all-`f64` args (a one-arg `slice(i)` is handled by `mcall_string_slice_split`, not checked here).
- Why: ~30-arm string `match` per call, boxed `Any` args, and the eager copy.
- Proposal: make `s` lazy (compute only in arms that need it); resolve the method to a distinct native id at sema time (variants for optional args) so no runtime name match is needed.
- Estimated gain: removes one O(n) alloc for non-needing arms (clear); name-dispatch gain not measured.
- Risk: low (lazy `s`); medium (sema re-dispatch, equivalence tests). Touches nothing frozen.

### F7 (low): `for (c of s)` eagerly builds n interned strings

- Evidence: `iterators.rs:61-78`: `value.chars().map(|p| p.to_string()).collect::<Vec<String>>()`, then `alloc_array`, `intern_string` per point, then `alloc_array_iterator`. Note it iterates by code point (correct per spec) .
- Why: O(n) up-front allocation even if the loop breaks early; a temporary `String` is allocated per char even though interning dedups.
- Proposal: pre-interned table for ASCII one-char strings (no temporaries); a lazy string iterator (handle + byte cursor) needs a new heap object kind (requires approval).
- Estimated gain: removes n temporary Strings. Risk: low (table) / medium (new object kind, GC tracing). Touches GC object kinds for the lazy variant only.

### F8 (low): concat allocates and copies more than needed

- Evidence: `thunks/string.rs:177-186`: two `Rc` clones, a new `String`, then `Rc::from(value)` (copies again into the `Rc<str>`), then `intern_string`. `natives/string.rs:15` uses `format!`. Hash cost in `intern_string` was not inspected (not confirmed).
- Why: `s += x` in a loop is O(len) per iteration, O(n^2) total, with 2 copies per op.
- Proposal: build directly into an exact-size `Rc<str>` to avoid the second copy; ropes/builders for repeated `+=` would change the String representation (requires approval).
- Estimated gain: ~2x constant; O(n^2) remains without ropes. Risk: low (constant) / high (ropes). Ropes touch the frozen String layout: requires approval.

### F9 (low): trim family and case conversion always allocate

- Evidence: `natives/string.rs:282-298`, `:414-421` return `.to_string()` of the trimmed borrow; thunks `string.rs:215-245` do `Rc::from(js_trim(..))` then intern. `to_lowercase` / `to_uppercase` (`:268-280`, thunks `:248-267`) always allocate.
- Proposal: if the trimmed slice equals the input (common case), return the same handle without allocating; skip case mapping when an ASCII scan finds nothing to change.
- Estimated gain: avoids allocation + intern lookup in the no-op case. Risk: low. Touches nothing frozen.

### F10 (low, not confirmed): UTF-8 byte semantics in `includes` / `startsWith` / `endsWith` / `replace`

- Evidence: `natives/string.rs:76`, `:399`, `:408`, `:155` operate on `&str`. Since lone surrogates are stored as unique plane-15 scalars, results match for well-formed text; edge cases of a stand-in adjacent to a real surrogate half that `from_units` would have merged were not analyzed (not confirmed). Any representation change under F1-B must preserve these.

## Eligibility / lowering consistency

- The same 25 names are hand-copied in `string_builtin` (`eligible.rs:286`), the `CallVirt`/`String` list (`eligible.rs:543-570`), `builtins_table.rs:26-190`, and the `op_call_string` match (`translate/strings.rs:70`). They agree today, but a missing name in any copy silently turns a function into Tier 0 (see F2). Proposal: generate all copies from one table, in line with the CLAUDE.md "one master table" principle. No ABI / format impact.
- `op_call_string` returns `Err("... expects string arguments")` (e.g. `strings.rs:~98`) when a kind mismatches after eligibility said yes; whether eligibility validates argument kinds for string builtins was not confirmed.

## Cross-module items

Native-call marshaling (`NativeArg::Str` in, `NativeValue::Str(String)` out then re-interned), the UTF-8 + lone-surrogate-stand-in String representation (root cause of F1), and interpreter dispatch cost: see `_shared.md`.
