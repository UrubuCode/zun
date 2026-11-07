# RegExp performance audit

## Summary
- Two engines exist (`Engine::Re2(regex::Regex)` and `Engine::Backtrack(Arc<zun_regex::Regex>)`, crates/zun_core/src/regexp.rs:31-33); selection is once at compile time (`needs_backtracking`, :149), so per-call selection is free. All RegExp natives are Tier0 native calls (no JIT lowering found in crates/zun_codegen/src/eligible.rs: grep "regexp|Regex" returned no hits).
- Compilation is per `new RegExp`/literal evaluation; there is NO pattern cache (every `alloc_regex_with_flags` calls `RegexValue::with_flags`, crates/zun_gc/src/regexes.rs:15). `matchAll` and string-pattern replace/split/match paths re-compile on every call.
- Every match is O(position) to locate (UTF-16 -> byte scan from 0) and eagerly allocates owned `String`s for all groups plus UTF-16 span vectors; global loops are O(n*m).

## Per-method table
| method | current path | current complexity | possible | allocs/call | JIT | severity |
|---|---|---|---|---|---|---|
| RegExp ctor / literal | interp/collections/regexp.rs:10-28 -> gc/regexes.rs:10-15 -> core/regexp.rs:338 | compile each time (parse + regex build; \p check builds a 2nd Regex, :403) | cache by (source,flags) | source+flags Strings, Regex, parse AST | Tier0 | High (literal in loop) |
| exec / test | interp regexp.rs:30-55 -> gc regexes.rs:332-366 -> match_full core:574 | O(start16 + n) per call (prefix scan :598-611, then len16 over prefixes :632-642) | O(match) with byte-index lastIndex cache | owned String per group, group_spans16 Vec (always built, even when no `d`), match array | Tier0 | Medium |
| test (no g/y) | same as exec (builds groups then discards, gc:332-338) | as exec | use is_match (core:1054) | same as exec, wasted | Tier0 | Medium |
| match(g) / matchAll | string_dyn.rs:665-740 `match_all_iterator` | clones regex (recompile, :683) + each exec O(pos) -> O(n*m) | one pass match_all_full / spans | recompile + array per match + outer array | Tier0 | High |
| replace/replaceAll | string_dyn.rs (regexpReplaceAllGuard in regexp.rs:~75; gc regex_replace_spans :474) | per-match exec via match_full | single-pass | `Vec<char>` of input per match for Backtrack | Tier0 | Medium |
| split(RegExp) | core split :782 | one pass for Re2 | - | piece Strings | Tier0 | Low |
| split/replace/match with string-as-pattern | string_dyn.rs:505-528 builds a RegExp from the string each call | recompile per call | cache or non-regex fast path | full compile | Tier0 | High |

## Findings

### F1. No compiled-pattern cache (compile per construction, per matchAll, per string->regex coercion)
Evidence: `heap.alloc_regex_with_flags(source, flags)` at interp/string_dyn.rs:520 (string pattern coerced to RegExp every call) and :683 (`match_all_iterator` clones the regex: `let clone = heap.alloc_regex_with_flags(source, flags)?`); core/regexp.rs:347 parses, :425 builds `regex::RegexBuilder`, and :403 compiles an extra `Regex::new(\p{..})` for validation.
Why: parse + automaton build costs orders of magnitude more than a match; a regex literal inside a hot loop or a `str.match(/x/g)`/matchAll in a loop recompiles each time.
Proposal: process-wide (thread-local) LRU cache keyed by (source, flags) storing `Engine` (already `Clone`, Arc-backed for Backtrack; Re2 `regex::Regex` clone is Arc) + group_count/names; `RegexValue` keeps its own lastIndex. For matchAll, clone shares the engine instead of recompiling.
Est. gain: 10-100x on literal-in-loop and matchAll workloads (hypothesis, not confirmed by benchmark). Risk: low (immutable compiled state; the `fallback` OnceCell at :59 is Rc, not Send - keep cache thread-local). Frozen layout/.zm/ABI/GC: no (GC heap holds `regexes` map; entries just share Arc).

### F2. UTF-16 -> byte conversion is O(n) per match; global loops are O(n*m)
Evidence: core/regexp.rs:598-611 scans `input.char_indices()` from byte 0 on every `match_full`; :632-642 compute start16/end16 via `len16(&input[..whole.start()])` for the whole match and for each group (O(n) each). `match_all_full` (:719) calls `match_full` per match with `pos` in UTF-16 units.
Why: replaceAll/match(g)/matchAll on a large string with many matches is quadratic (plus G groups times O(n) each).
Proposal: carry a byte cursor across iterations (match_all_full already knows prior end_byte), fast path for ASCII input (`input.is_ascii()` => units == bytes), compute group spans incrementally/lazily (only when `d` flag). Est. gain: O(n*m) -> O(n+m). Risk: medium (surrogate/lone-unit handling must stay byte-identical; existing tests at :1131-1213 cover). Frozen layout/ABI: no.

### F3. Eager owned allocations per match (groups, group_spans16, match array)
Evidence: :634-651 `groups: captures.iter().map(|g| g.map(|m| m.as_str().to_owned())).collect()` and unconditional `group_spans16`; `captures_all` (:902-935) same; gc `regex_exec_stateful` always builds full match array (:344-366) even for `test`.
Proposal: `test` (no g/y flags) -> `is_match`; build `group_spans16` only if `d`; keep spans (byte ranges) and slice lazily when replace needs only group text; reuse capture locations buffer (`captures_read_at`). Est. gain: 2-5x on test/replace hot paths (not confirmed). Risk: low-medium. Frozen layout: no.

### F4. Backtrack engine rebuilds `Vec<char>` of the input per call
Evidence: `chars_of` (:526-531) collects chars (and `split_astral` for non-ASCII) on every `match_full` call; also every global-loop iteration (:~750) and captures_all. Backtrack is used for backrefs/lookaround so is slower anyway.
Proposal: build chars once per top-level operation (replace/matchAll/split) and pass to a `match_full_chars` variant; cache last (input ptr, chars) for lastIndex-driven exec loops. Est. gain: O(n) -> amortized O(1) per iteration. Risk: low. Frozen layout: no.

### F5. Astral-in-input twin lookup does a full-input scan per call
Evidence: core:580-583 `input.chars().any(|c| c.len_utf8() == 4)` before every Re2 match; also `other.clone()` of Engine at :592 per call.
Proposal: use `input.is_ascii()` early-out first (SIMD fast) and avoid engine clone by borrowing. Gain: small-medium on long inputs. Risk: low.

### F6. Prelude `RegExp` class is a thin wrapper (builtin/regexp.zun) - `test`/`isMatch` both call `regexp.isMatch` which is a stateless path, while interp `regexpTest` (stateful, lastIndex) is a separate native. Not confirmed which one user-level `.test` binds to; verify before optimizing so the cheaper `is_match` is used when flags lack g/y.

## JIT
No RegExp opcode/builtin is lowered (eligible.rs has no regexp entries) -> native-call from Tier0/bridge; per-call cost is dominated by regex work, not call overhead, so JIT lowering is not worthwhile; fix F1-F4 first.

## Cross-module (see final report)
- Regex pattern cache (F1) is shared by String.match/replace/split/search/matchAll string-coerced paths.
- Shared array-building for match results (alloc_match_array, gc/regexes.rs:67) overlaps with String/Array docs.
