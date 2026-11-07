# Date — optimization audit (read-only, verified in code)

## Summary
- Date is a prelude class (`crates/zun_prelude/builtin/date.zun`) over f64 natives (`date_f64.rs`) plus legacy i64 natives (`date.rs`); all Date work is native-call dispatch, and the new f64 natives (`field`, `localTime`, `setFields`, ...) are NOT in the JIT `date_builtin` allow-list, so any function touching local getters/setters is Tier0-only.
- Biggest costs: every local-time operation resolves `jiff::tz::TimeZone::system()` (and local setters do it twice), and every `toLocale*String` builds an ICU4X `DateTimeFormatter` from scratch.
- Pure UTC arithmetic (civil math) is already O(1) closed-form; no per-call allocation except returned Strings.

## Per-method table
| method | current path file:line | current complexity | possible complexity | allocs/call | JIT eligible/lowered | severity |
|---|---|---|---|---|---|---|
| getFullYear/Month/Date/Day/Hours/Minutes/Seconds/Milliseconds (local) | date.zun:100-107 -> `localTime` date_f64.rs:266 + `field` :110 | 2 native calls; tz resolve + jiff offset lookup (binary search over transitions) + full 8-field decode | cached zone: 1 call, O(1)/O(log n) offset; field decode single | none (f64) | No (not in `date_builtin`, eligible.rs:418) | high |
| getUTC* (field) | date_f64.rs:110 `f_field` -> `fields_of` :82 | O(1) but decodes all 8 fields to return 1 | single-field decode | none | No | low |
| getTimezoneOffset | date.zun:98 -> date_f64.rs:301 | tz resolve + offset lookup | cached zone | none | No | med |
| setX local (7 setters) | date.zun:113-131 `applyFields` -> localTime + setFields + utcFromLocal (date_f64.rs:278) | 3 native calls; 2 zone resolves (offset + to_zoned); jiff Timestamp->Zoned->civil->Zoned | 1 fused native, 1 zone lookup | `...a: any[]` rest array + `Number(a[i])` (Any unbox) per call | No | high |
| setUTCx | date.zun:134-140 -> `setFields` | 1 native, O(1) | same | rest array alloc | No | med |
| toISOString / toUTCString | date_f64.rs:173 / :203 | O(1), `format!` | write into reusable buffer | 1-2 Strings | No | low |
| toString/toDateString/toTimeString | date_f64.rs:228 | zone resolve + 2-3 `format!` | cached zone, single format | 2-3 Strings | No | med |
| toLocaleString/DateString/TimeString | date.rs:395-446 `locale_format` | ICU `DateTimeFormatter::try_new` + locale parse every call | cache formatter by (locale,kind) | Locale, formatter, String | No | high |
| Intl.DateTimeFormat.format | date.rs:161 | same: formatter built per `format()` | build once per Intl object | same | No | high |
| Date.UTC / new Date(y,m,..) | `f_make_utc` date_f64.rs:118; local ctor via utcFromLocal | O(1); local ctor 1 tz resolve + jiff conv | same | none | No | low/med |
| Date.now / new Date() | `time.nowMillis()` date.zun:31,206 | 1 native syscall | n/a | none | not date-lowered | low |
| Date.parse | date_f64.rs:315/431 `parse_iso` | O(len) hand parser | same | none | No | low |
| legacy i64 year/month/day/hour/... | date.rs:26-100 | O(1) | n/a | none | YES (date_builtin eligible.rs:418-433; allow-listed at :116,:1447) | low |

## Findings

### F1 — Host zone re-resolved on every local-time call (high)
Evidence: `date_f64.rs:224` `local_offset_minutes_at(t as i64, &zone_of(&[]))`; `date.rs:448-453`
```rust
_ => jiff::tz::TimeZone::system(),
```
`utc_from_local` (date_f64.rs:286) calls `zone_of(&[])` again. Every `getHours()` = `localTime` (zone resolve + offset) + `field`. Whether `TimeZone::system()` is cached internally by jiff: not confirmed (hypothesis; jiff has a global cache for named zones and a system-zone detection that may stat/read env/files). Proposal: cache the host `TimeZone` in a `OnceLock`/thread-local (or in the extension state), honouring any TZ-change policy explicitly. Est. gain: 5-50x on local getters/setters if system() is not already cached. Risk: low-med (TZ changed at runtime — JS engines also cache). Frozen layout/.zm/ABI/GC: no.

### F2 — Local getters = 2 native calls + full-field decode; setters = 3 calls (high)
Evidence: date.zun:100-107, 127-129 (`date.field(date.localTime(this.t), n)`; `date.utcFromLocal(date.setFields(date.localTime(...)))`). Proposal: fused natives `localField(t, idx)` and `setLocalFields(t, start, n, v0..v3)` doing a single zone lookup; single-field decode (compute only needed component, weekday from days). Est. gain 2-3x on getters, ~3x setters. Risk: low (byte-identical output testable). Touches ABI: adds native entries only (no frozen layout / .zm change; metadata token IDs for natives shift only if table order changes — append).

### F3 — ICU formatter rebuilt per call (high)
Evidence: date.rs:420-431 `DateTimeFormatter::try_new(prefs, fieldsets::YMDT::medium())` inside `locale_format`; date.rs:161-180 same for Intl.DateTimeFormat.format, plus `crate::string::intl_locale(text)` parse per call. Formatter load from baked data selects/validates data each call (cost not measured; not confirmed magnitude). Proposal: thread-local small cache keyed by (locale string, kind) -> formatter; for Intl objects hold handle in the instance. Est. gain: likely 10x+ on repeated formatting (hypothesis). Risk: low; formatters are Send-unfriendly maybe -> thread-local. No frozen layout/.zm impact.

### F4 — Date natives not JIT-lowered; Date-touching functions are Tier0-only (high for hot loops)
Evidence: eligible.rs:418-433 `date_builtin` lists only legacy i64 natives (`isValid|year|month|day|hour|minute|second|millisecond|weekday|dayOfYear`). The prelude uses `localTime`/`field`/`setFields`/`utcFromLocal`/`tzOffset` — none listed. UTC `field` is pure arithmetic and trivially lowerable (div_euclid chain). Proposal: add `field`, `valid`, `clip` (pure) to the lowered set via the recipe in memory "zun-jit-eligibility"; keep tz-dependent ones as native calls (not confirmed that eligible.rs permits non-lowered native calls). Est. gain: removes Tier0 fallback for UTC-heavy loops. Risk: med (equivalence suite interp vs JIT). Touches JIT only.

### F5 — Setters allocate rest array and Any-unbox (med)
Evidence: date.zun:136-146 `setHours(...a: any[])` -> `applyFields(..., a, ...)` with `Number(a[0])`. Proposal: sema-level fixed-arity overloads (`setHours(h, m?, s?, ms?)` with defaults NaN-sentinel) avoiding array + Any boxing. Est. gain 2x on setters. Risk: low-med (arity/undefined semantics must match: no-args means NaN). No frozen layout change.

### F6 — field() decodes all 8 components (low)
Evidence: date_f64.rs:82-100 `fields_of` builds [f64;8] incl. weekday for every `field` call. Proposal: switch on idx. Est. gain ~2x on the native itself (small vs call overhead). Risk: none.

### F7 — Formatting allocations (low)
`format!` multiple Strings in f_date_string (date_f64.rs:233-262) and combined `format!("{date} {time}")`. Proposal: single `write!` into one String with capacity. Gain minor.

## Cross-module (see shared doc)
- native-call cost (NativeArg slice / NativeValue returns, f64 round-trip via i64 casts) — shared.
- ICU4X baked-data formatter construction cost — shared (also Intl, String.localeCompare).
- jiff tz-DB / system-zone resolution cost — shared (any host-time use).
- Rest-param `any[]` boxing in prelude classes — shared with other prelude classes.
