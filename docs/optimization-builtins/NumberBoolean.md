# Number and Boolean: optimization audit

## Summary
- The instance formatters (`toFixed`/`toExponential`/`toPrecision`/`toString(radix)`) are Tier0-only. They box the argument to `Any` and are formatted via a name-string `match` in the interpreter.
- `Number.isNaN`/`isFinite`/`isInteger`/`isSafeInteger` and `Boolean(x)` take an `Opaque` argument, which forces `Any` marshaling. They are not JIT-lowered, even though the typed `isNaN`/`isFinite` variants are.
- `parseFloat` is O(n^2) (it re-parses every prefix) and `exact_decimal` formats 1100 digits per call. Both are cheap to fix without touching any frozen layout.

(All rows below were verified by reading the code. JIT claims rest on `eligible.rs::math_builtin` and `builtins_table.rs`. I did not run benchmarks, so every "est. gain" is an estimate and is marked "not confirmed".)

## Per-method table

| method | current path (file:line) | current complexity | possible complexity | allocs/call | JIT eligible/lowered | severity |
|---|---|---|---|---|---|---|
| `n.toFixed(d)` | mcall_number.rs:37-79 -> interp string_num.rs:328-357 -> number.rs:533 `js_to_fixed` | O(1100) digit expansion + Vec copies | O(d + digits) via an exact algorithm with no 1100-digit string | about 8: `format!{:.1100}` String, `all` Vec, `digits` Vec, `kept`/`n` Vecs, 2 Strings, result String, plus the `Any` box and the `name` match | no: `managed_only` (number.rs:678) and not in `math_builtin` | High |
| `n.toExponential(d?)` | same as above, number.rs:581 | O(1100) with d given, O(17) without | O(d) | about 5 to 7 | no | Medium |
| `n.toPrecision(p?)` | same as above, number.rs:626 | O(1100) | O(p) | about 7 | no | Medium |
| `n.toString(radix?)` | mcall_number.rs:37-79 (radix-10 `toString()` also goes here), number.rs:664 -> lib.rs:626 `format_float` | O(17) at radix 10, O(digits) otherwise | same | about 4 at radix 10 (`format!{:e}`, the `digits` String, the `format!` body, the result String) | no | Medium (hot: `n.toString()`) |
| `n.toLocaleString(loc?)` | mcall_number.rs:221-292 -> number.rs:416 `intl_number_format` | `format!("{:.3}")`, then Decimal parse, then ICU formatter built per call | cache the `DecimalFormatter` per locale | about 5 plus the ICU formatter construction | no | Medium |
| `n.valueOf()` | mcall_number.rs:296-301 | O(1) identity | n/a | 0 | n/a (erased) | none |
| `Number.isNaN/isFinite/isInteger/isSafeInteger(x)` | builtins/number.rs:337-362 -> number.rs:33-62 (`Opaque` param) | O(1) plus marshaling | O(1) inline for typed `number` | `NativeArg` Vec per call (interp builtins.rs:49) | no: the `*Strict` names are not in `math_builtin` | Medium |
| `n.isNaN()/isFinite()/isSafeInteger()` (instance extern) | number.zun:77-82 -> JIT table builtins_table.rs:459-486 | O(1) | n/a | 0 | yes (lowered to a call to `zun_jit_number_is_*`) | low (still an out-of-line call, not inline fcmp) |
| `Number.parseFloat(s)` | builtins/number.rs:364-388 -> number.rs:67 `parse_float_prefix` | O(n^2): `parse::<f64>` of every prefix | O(n) with a hand-written prefix scanner | about 0 (no `String` allocation, but n parse calls) | no | High for long input |
| `Number.parseInt(s, r)` | builtins/number.rs:389-479 -> number.rs:90 | O(n), but the radix expression is built from `Rem`, `Convert`, an `Any` coerce and a `numberCoerce` call | O(n) | `Str` arg copied into `NativeArg::Str` | no | Low |
| `Boolean(x)` | call_coerce.rs:64-92 -> interp builtins.rs:42-48 | O(1) | inline when the type is known (`number`: `!= 0 && !NaN`; `string`: length) | `Vec<Value>` args plus the heap lookup | no (`booleanCoerce` is not lowered) | Medium |
| `b.toString()` | mcall_number.rs:321-334 -> boolean.rs:47 | O(1) | return an interned constant | 1 String alloc ("true"/"false" `.to_string()`) plus the args Vec | no | Low |
| `new Boolean(x)` / `BooleanObject` | boolean.zun | O(1) | n/a | 1 object | prelude class | Low |
| Number constants | not examined in depth | n/a | n/a | n/a | not confirmed | n/a |

## Findings

### F1. Instance formatters pass the argument as `Any` through a name-string dispatch, and are Tier0-only
- Evidence:
  - mcall_number.rs:44-49 and 54-58: `native_name = "numberToFixedJs"`, `Self::box_to_any(value)`. When the argument is omitted it builds `void 0` as an `Any`.
  - number.rs:805-828 registers the four `*Js` natives with `imp: managed_only` and an `Opaque` parameter.
  - string_num.rs:328-347: `match name { "numberToFixedJs" | ... }` on a `&str`, `unbox_arg`, `to_number_value`, then another `match name`.
  - eligible.rs:439-486: `math_builtin` lists only math plus `isFinite/isNaN/isNegativeZero/isSafeInteger`. None of the formatters appear.
- Why it costs: every call boxes the argument, matches strings twice, goes through `col_string_num` (mod.rs:448-451), and cannot be JIT-lowered. A loop calling `x.toFixed(2)` leaves the JIT tier, or forces Tier0 for the whole function. Whether a single non-whitelisted `CallBuiltin` makes the whole function ineligible is not confirmed.
- Proposal:
  - (a) Resolve the string name to a native ID at compile time (it already is one) and dispatch the interpreter on the ID. Keep the name only for diagnostics.
  - (b) For a statically typed `number` or `int` argument, skip `box_to_any`. Pass an `F64` plus an explicit flag for "absent". This removes the `void 0` construction.
  - (c) Lower `toFixed` etc. in the JIT as an out-of-line call to an `extern "C"` helper that writes into a pooled string handle (see the cross-module note).
- Est. gain (not confirmed): 20-40% per call for (a)+(b); the main gain of (c) is keeping the function in Tier1.
- Risk: low for (a) and (b). Medium for (c), which needs a string-handle return convention in the JIT.
- Touches frozen layout/.zm/ABI/GC: (a) and (b) no. (c) touches the JIT-native ABI, so requires approval.

### F2. `exact_decimal` formats 1100 fractional digits per call
- Evidence: number.rs:470 `let text = format!("{x:.1100}");` followed by `all: Vec<u8>` (line 472), `digits` Vec (475), then trailing-zero popping. It is used by `js_to_fixed` (543), `js_to_exponential` with digits (602), and `js_to_precision` (647).
- Why: the result is at most ~767 significant digits, but the code always renders about 1100 fractional digits and then filters and copies. Even `(1.5).toFixed(2)` pays for that. The fast path `{value:.digits$}` that `to_fixed` (number.rs:229) uses is not applied here, though ties may be mishandled.
- Proposal:
  - (a) Fast path for ordinary values: if `x.abs() < 2^53` and `f <= 15`, compute with `format!("{x:.*}")` and fix up only the tie case (exact binary tie, where Rust rounds half-to-even but JS rounds half-up).
  - (b) Otherwise keep the exact path, but use a stack buffer (a `[u8; 1200]` plus `write!`) instead of `String` plus two `Vec`s.
  - (c) Strip leading/trailing zeros in place instead of building `all`.
- Est. gain (not confirmed): several times faster on small values.
- Risk: medium. Rounding semantics must remain bit-identical. The JS-conformance tests under `tests/` must pass for `toFixed`/`toPrecision` ties (for example 1.005, 2.5, 0.000001).
- Touches frozen layout/.zm/ABI/GC: no.

### F3. `Number.parseFloat` is O(n^2)
- Evidence: number.rs:76-83: `for end in 1..=text.len() { ... text[..end].parse::<f64>() ... }` keeps the longest successful parse.
- Why: n calls to `parse::<f64>`, each O(n). A 1 MB numeric string would take far too long. There is also a correctness smell: `"1e"` or `"Infinity"` parse differently depending on the prefix, and Rust accepts `"inf"`, `"nan"`, `"infinity"` (JS only accepts `Infinity`). Not confirmed by a test.
- Proposal: write a prefix scanner (sign, digits, optional `.` digits, optional `e[+-]digits`, or the literal `Infinity`) and call `parse::<f64>` once on the matched slice. O(n).
- Est. gain: from O(n^2) to O(n). For typical short inputs (<20 chars) about 5-10x (not confirmed).
- Risk: low; add `"inf"`, `"nan"`, `"1e"`, `".5"`, `"5."`, `"-.5e-3x"` fixtures.
- Touches frozen layout/.zm/ABI/GC: no.
- Related: `parseFloat` (number.rs:205, `parse_float`) errors on bad input via `NativeError(format!(...))`, so an invalid string allocates an error message on the hot path. That is the `zun:core` `parseFloat` import. Whether sema routes user `parseFloat` to it is not confirmed.

### F4. `Opaque` parameters for strict predicates and `Boolean(x)` force `Any` marshaling and block the JIT
- Evidence:
  - number.rs:733-756: `isNaNStrict` etc. have `params: &[Opaque]`, and `strict_number` (number.rs:12-21) probes `Float`/`Int`/`Any`.
  - builtins/number.rs:354-361: `let arg = self.check_expr(cx, &args[0]);` is passed with no static-type specialization.
  - call_coerce.rs:77-79 already short-circuits `arg.ty == Ty::Boolean`. There is no equivalent for `F64`, `String` or `Int`.
  - eligible.rs:477-480 lists the non-strict `isNaN`/`isFinite` as lowered, but the `*Strict` names are absent.
- Why: `Number.isNaN(x)` with `x: number` goes through `Vec<Value>` -> `Vec<NativeArg>` -> `dispatch` -> `strict_number`, and is not JIT-lowered, although `x.isNaN()` is (builtins_table.rs:466). For a typed `number` receiver the strict and loose predicates are identical.
- Proposal (sema-only, no ABI change):
  - If the argument type is `Ty::F64`, rewrite to the existing typed natives (`isNaN`, `isFinite`, `isSafeInteger`) that the JIT already lowers.
  - `isInteger`: add a typed variant, or use `Trunc` plus `Eq` on `F64` in HIR.
  - If the type is a non-numeric primitive (`string`, `boolean`, `null`), constant-fold to `false` and evaluate the argument for side effects only.
  - `Boolean(x)`: for `F64` emit `x != 0 && x == x` in HIR, for `String` emit a length test, for `Int` emit `!= 0`. Only `Any` keeps the native call.
- Est. gain (not confirmed): about 10x for typed loops that use `Number.isNaN` or `Boolean()`, and it removes a function from the "Tier0 only" set.
- Risk: low. Add a differential test comparing the typed rewrite against the `Any` path.
- Touches frozen layout/.zm/ABI/GC: no.

### F5. Dead interceptor branches in `mcall_number_a/b`
- Evidence: mcall_number.rs:37-42 matches `toFixed|toExponential|toPrecision|toString` with `args.len() <= 1` and returns unconditionally. The later branches at 85 (`toStringRadix`), 133 (`toExponential`/`toPrecision`, in `mcall_number_b`) and 175 (`toFixed`) require the same conditions on the same receiver type, so they appear unreachable (assuming the interceptors run in order a, b, c, which the header comment at line 8 says).
- Why: dead code preserves an alternative semantic (`toStringRadix` native with an `I32` and a throwing `NativeError`) that differs from the JS-conformant path, and it keeps the older natives (`toFixed`, `toStringRadix`, `toExponential`, `toPrecision`, number.rs:215, 137, 309, 344) that can be both JIT-lowerable and cheap. Whether they are used elsewhere (for example from `zun:core` imports in `number.zun:3`) is not confirmed.
- Proposal: decide which path is canonical.
  - Option A: delete the dead branches and natives.
  - Option B (preferred if F1 is done): use the old typed natives when the argument is a typed `int` literal within range and the JIT lowers them, falling back to the `*Js` path only for `any` arguments. This gives a static fast path with no boxing.
- Est. gain: code size and clarity; B also yields the F1 gain for the common `toFixed(2)`.
- Risk: low. Run the conformance suite for RangeError behavior.
- Touches frozen layout/.zm/ABI/GC: native IDs are registered by position ("Appended last so existing dispatch ids stay stable", number.rs:775). Removing natives would shift IDs, so it requires approval. Keep them registered.

### F6. `toLocaleString` builds an ICU `DecimalFormatter` on every call
- Evidence: number.rs:405-409 and 444-445: `DecimalFormatter::try_new(...)` inside the function. Before that, `format!("{value:.3}")` is parsed into a `Decimal` (lines 439-442).
- Why: formatter construction (data lookup in baked ICU4X data) is repeated per call. The locale-prefs parse in `crate::string::intl_locale` also runs each time.
- Proposal: a thread-local `HashMap<locale string, DecimalFormatter>`, or at least a cached root formatter for the empty locale. Format integers via `Decimal::from(i64)` when `value.fract() == 0.0`, skipping the string round trip.
- Est. gain (not confirmed): the formatter build typically dominates; a 3-10x speedup is plausible.
- Risk: low; thread-local state with no GC refs.
- Touches frozen layout/.zm/ABI/GC: no.

### F7. `format_float` and small formatting helpers allocate more than needed
- Evidence: lib.rs:638-643: `format!("{:e}")` into a String, `split_once`, a `digits: String` built with `chars().filter(...).collect()`, then another `format!` for the body, then a sign concatenation.
- Why: about 4 allocations for what is the hottest conversion (`n.toString()`, number-to-string coercion in concatenation). Integer-valued floats are the common case and could short-circuit: if `n.fract() == 0.0 && n.abs() < 1e21`, print via `itoa`-style integer formatting (about 1 allocation). Also `NativeValue::Str(String)` copies again into the heap string when it crosses the boundary (cross-module note).
- Proposal: add the integer fast path, and write digits into a reusable `String` buffer. Consider the `ryu` or `zmij` shortest algorithms (not confirmed as workspace dependencies) only if the dependency policy allows.
- Est. gain (not confirmed): 2-3x for integer-valued numbers.
- Risk: low. Ensure `-0` prints `"0"` and `1e21` switches to exponent form.
- Touches frozen layout/.zm/ABI/GC: no.

### F8. Boolean `toString` and the redundant `to_string()` allocations
- Evidence: boolean.rs:49-51: `if flag { "true" } else { "false" }.to_string()`. Likewise `"NaN".to_string()` / `"Infinity".to_string()` throughout number.rs (150-155, 316-321, 361-366, 393-397).
- Why: it allocates a `String` per call for constants. Sema could emit the string literal directly for `b.toString()`, since the checker already knows the receiver is a `boolean`: `cond ? "true" : "false"` as HIR with interned constants (no native call, JIT-eligible if string constants are).
- Proposal: lower `boolean.toString()` in `mcall_number_c` to a `Conditional` of two `ExprKind::String` literals.
- Est. gain: removes an interp native call and an alloc; makes the call JIT-friendly. Not confirmed whether string literals in a conditional are lowered by the JIT.
- Risk: low.
- Touches frozen layout/.zm/ABI/GC: no.

### F9. Redundant sentinel and conversion patterns in sema
- `parseInt` radix (builtins/number.rs:403-462) builds `Rem 4294967296.0` and `Convert`, and a `numberCoerce` call for `Any`. When the radix is an integer literal (common: `parseInt(s, 10)`), `coerce_numeric_literal` makes it `I32` and no extra nodes are needed; the `Rem` is only added when the type is `F64`. That is correct, but a literal `10.0` of type `F64` would still pay `Rem`. Const-fold `Rem`/`Convert` on number literals in sema (not confirmed whether a folding pass exists later).
- `toExponential`/`toPrecision` use sentinels (`-1`/`0`) for the absent argument (mcall_number.rs:142, number.rs:324,352). The `*Js` path uses `void 0` instead. Both are workable, but the sentinels collide with legitimate input if a caller passes `0` to `toPrecision`; it is rejected as out of range by the spec (1..=100), so `0` is a valid sentinel only because it would otherwise be a RangeError. In the old path an explicit `x.toPrecision(0)` is silently treated as "absent" instead of throwing a RangeError. This is a correctness bug in the old (apparently unreachable, see F5) path. Not confirmed reachable.

## Cross-module references (for the shared doc)
- Native-call cost: every `CallBuiltin` in the interpreter builds `Vec<Value>` (pop_n) then `Vec<NativeArg>` then dispatches by ID; strings are copied into `NativeArg::Str` and the result `String` is copied again into a heap string (interp builtins.rs:37-53).
- `Any` marshaling: boxing for `Opaque` params (F1, F4); `box_to_any` at the call site and `unbox_arg` at the callee.
- Name-based dispatch: `col_string_num` matches on `&str` per call (string_num.rs:328, mod.rs:448).
- ICU4X baked data: per-call formatter construction (F6), likely shared with Date/Intl and `toLocale*` in String.
- JIT eligibility: eligibility is name-whitelisted in `math_builtin` (eligible.rs:439-486), so each new lowered native needs both an `eligible.rs` entry and a `builtins_table.rs` entry. The two must stay in sync (eligible implies lowerable).
