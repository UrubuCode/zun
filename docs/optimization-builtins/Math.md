# Math optimization audit

## Summary

- Hot Math (sqrt/abs/floor/ceil/trunc/min/max/round/sign) is already substituted at ZIL-gen time by single machine opcodes 0x96-0xA4 and lowered inline in the JIT; the rest go through `CallBuiltin` to Rust `extern "C"` thunks that call libm.
- Verified gaps: `asin/acos/atan/fround/imul/clz32/random` are not JIT-eligible (no thunk, not in `math_builtin`), `Math.random` does not exist, and the native/thunk `round` has non-JS semantics on negative halves.
- Interpreter fallback for non-opcode Math goes through `NativeArg` slice marshaling (`float_arg` match per call); the `apply_math` path allocates a `Vec` and a `String`.

## Per-method table

Path legend: OP = ZIL opcode (ilgen substitution, `crates/zun_ilgen/src/lib.rs:48-63`), NAT = `CallBuiltin` native (`crates/zun_natives/src/math.rs`), sema entry `crates/zun_sema/src/check/builtins/number.rs:171` (`check_math_static`; routed from `mcall_namespaces.rs:139`).

| method | current path file:line | current complexity | possible complexity | allocs/call | JIT eligible / lowered | severity |
|---|---|---|---|---|---|---|
| sqrt | OP SqrtF64 (ilgen:49-50; opcode.rs:324); JIT arith.rs:247 `sqrt` | O(1) | O(1) | 0 | eligible (eligible.rs:58) / single insn | none |
| abs | OP AbsF64 (ilgen:51); arith.rs:248 `fabs` | O(1) | O(1) | 0 | yes / single insn | none |
| floor / ceil / trunc | OP (ilgen:52-56); arith.rs:250-252 | O(1) | O(1) | 0 | yes / single insn | none |
| min / max (2 args) | OP MinF64/MaxF64; arith.rs:133-134 `fmin/fmax` | O(1) | O(1) | 0 | yes / single insn | none |
| min / max (N args) | number.rs:184-222 left fold of 2-arg native, seeded with identity | O(N) | O(N) | 0 | each fold step becomes opcode | low (one extra op for identity seed, likely const-folded by Cranelift; not confirmed) |
| min / max / hypot via spread (`Math.max(...a)`) | `math_apply_expr` (number.rs:179) -> `apply_math` math.rs:74-127 | O(N) | O(N) | min/max/hypot: 0; other names: `format!` String + `Vec<NativeArg>` per call (math.rs:109-117) | not lowered (native call) | low |
| round | OP RoundF64 (ilgen:61); arith.rs:178-201 (floor, fsub, fcmp, fadd, select x2, fcopysign) | O(1) | O(1) | 0 | yes / branchless seq | none (opcode path). Native/thunk path wrong: see F2 |
| sign | OP SignF64 (ilgen:62); arith.rs:204-220 | O(1) | O(1) | 0 | yes / branchless | none |
| pow | NAT math.rs:188-202; thunk `zun_jit_math_pow` (lib.rs:622); eligible.rs:461 | O(1) | O(1) | 0 (JIT); interp builds `Vec<NativeArg>`? not confirmed | eligible / libcall via thunk | medium (libm call; unavoidable) |
| sin cos tan exp log log2 log10 expm1 log1p sinh cosh tanh asinh acosh atanh cbrt | NAT math.rs:232-286; thunks lib.rs:597-619; eligible.rs:448-483 | O(1) | O(1) | 0 | eligible / libcall via Rust thunk -> libm | medium-low (see F5) |
| atan2, hypot (2 args) | NAT math.rs:327-347; `hypot` calls `apply_math("hypot", &[l,r])` (math.rs:345) incl. Kahan loop | O(1) | O(1) | 0 | eligible (eligible.rs:474-475) / libcall | low |
| asin, acos, atan | NAT math.rs:307-317 | O(1) | O(1) | 0 | NOT eligible: absent from `math_builtin` (eligible.rs:439-485) and no `zun_jit_math_*` symbol (lib.rs:597-625) | high (see F1) |
| fround | NAT math.rs:319-321 (`x as f32 as f64`) | O(1) | O(1) | 0 | NOT eligible (same) | high; trivially lowerable as `fdemote`+`fpromote` |
| imul, clz32 | NAT math.rs:204-230 (`to_int32` via `rem_euclid`) | O(1) | O(1) | 0 | NOT eligible | high; interp `to_int32` uses float `rem_euclid` (slow-ish) |
| random | absent: not in `check_math_static` match (number.rs:224-262 -> "not supported yet"); only an arity entry arities.rs:27 | - | O(1) | - | - | medium (functional gap) |
| MinI32/MaxI32/ClzI32/CtzI32/PopcntI32 | opcodes 0x9E-0xA2 (opcode.rs:351-361); JIT `int_minmax` arith.rs:156, `int_un` arith.rs:231 | O(1) | O(1) | 0 | yes | none (no sema/ilgen producer for Math.* found in Math files; not confirmed whether other crates emit them) |

## Findings

### F1. asin/acos/atan/fround/imul/clz32 make the whole function ineligible for the JIT
- Evidence: `crates/zun_codegen/src/eligible.rs:439-485` `math_builtin` whitelists by native name (`"cbrtNumber" | "sinNumber" | ... | "atanhNumber"`) and contains no `asinNumber`, `acosNumber`, `atanNumber`, `froundNumber`, `imulNumber`, `clz32Number`. `crates/zun_codegen/src/lib.rs:597-625` registers symbols only through `zun_jit_math_atanh`/`tan`/`min`/`max`/`pow`/`atan2`/`hypot`/`fmod`; none for those six. Yet sema routes them to `CallBuiltin` (number.rs:251-256) and the natives exist (math.rs:307-321, 213-230).
- Why: eligibility is whole-function; one such call keeps a hot loop in Tier 0. Typical code: `Math.imul` in hash loops, `Math.fround`, `Math.atan`/`asin` in geometry.
- Proposal: add thunks + `builtins_table.rs` rows + `math_builtin` entries (the "add a lowered builtin" recipe). Better, lower `fround` as `fdemote`/`fpromote` and `imul` as `fcvt_to_sint_sat` -> `ireduce`-wrap `imul` using the saturating form plus ToInt32 wrap (per CLAUDE.md; note `fcvt_to_sint_sat` of values beyond i64 needs the separate modulo-2^32 wrap, so the 2^32 wrap must be done on a path that handles |x| >= 2^63, e.g. cold fallback to the thunk). `clz32` same conversion + `clz`.
- Est. gain: from Tier 0 interpretation of the whole function to native (order of 10x+ for affected loops; not measured).
- Risk: low for thunks; medium for inline ToInt32 (edge semantics). Touches frozen layout/.zm/ABI/GC/opcode-set: no for thunks; inline `imul`/`clz32` via new opcodes would be opcode-set change (requires approval, and must go through `define_opcodes!`).

### F2. `round` native and JIT thunk use `f64::round` (half away from zero), JS rounds half toward +Infinity
- Evidence: math.rs:180-182 `unary_float(args, "round", f64::round)`; thunks/misc.rs:435-437 `value.round()`. Correct semantics exist only in the opcode (opcode.rs:366-368, arith.rs:171-201, interp `js_round`). `round(-2.5)` is -3 under `f64::round`, JS gives -2. Reachable through `apply_math` (math.rs:109-114 -> `roundNumber`, e.g. `Math.round(...args)` spread path) and by any CallBuiltin not substituted by ilgen (argc guard, ilgen:84). Whether the thunk is reachable in practice: not confirmed.
- Why: correctness divergence between paths violates the interp/JIT equivalence invariant; it is a latent bug, not a performance issue.
- Proposal: make the native and thunk call the same shared `js_round` the interpreter uses (single source of truth). Also fix stale comment opcode.rs:317-321 which says `round`/`sign` are "deliberately absent" while 0xA3/0xA4 define them, and ilgen:44-47 contradicts it.
- Est. gain: correctness. Risk: very low. Frozen/ABI/opcode: no.

### F3. Math.random is missing
- Evidence: number.rs:224-262 has no `"random"` arm (falls to "`Math.random` is not supported yet"); math.rs has no `random`; arities.rs:27 lists it for arity only. Randomness native exists elsewhere (`zun_natives/src/host/mod.rs:88` `crypto_random_u64`, OS call per invocation).
- Why: functional gap; if added naively via `getrandom` per call it would be a syscall per `Math.random()` (slow).
- Proposal: add `randomNumber` backed by a per-isolate xorshift128+ state seeded once from OS randomness; mark it non-const (no CSE). JIT: libcall thunk with state pointer from vmctx, or inline xorshift (3 shifts+xors) with state load/store.
- Est. gain: O(1) ~1-2 ns vs ~100s of ns syscall. Risk: low; state in vmctx would touch the runtime context layout (requires approval if vmctx layout is frozen).

### F4. Interp native-call marshaling and `apply_math` allocations
- Evidence: every non-opcode Math call in Tier 0 goes through `fn(&[NativeArg], &mut dyn Sink) -> Result<NativeValue, NativeError>` with a runtime `match args.first()` (math.rs:5-11, 159-170) and error-string `format!` only on failure path. `apply_math` (math.rs:109-126): `name.to_string()`/`format!` + `crate::resolve` name lookup + `Vec<NativeArg>` + `NullSink` per call for every non-min/max/hypot function. Also `hypot` (math.rs:337-347) funnels into `apply_math`, running `any()`/`fold`/Kahan passes over a 2-element slice.
- Why: name-based dynamic dispatch and heap allocation on a hot-able path (spread calls such as `Math.sqrt.apply(...)`/dynamic `Math[fn]`).
- Proposal: resolve to a function pointer table keyed at sema/ilgen time (`id`), and give `hypot` a 2-arg fast path (`sqrt(a*a+b*b)` guarded for overflow/NaN/Inf, matching the Kahan result only when no scaling is needed: must keep exact V8 parity, so keep the scaled path as fallback).
- Est. gain: removes 2 allocations/call on the apply path; small otherwise. Risk: low. Frozen/ABI/opcode: no. Cross-module: see final report (native call cost).

### F5. Transcendentals are libcalls through a Rust `extern "C"` wrapper, not libm directly
- Evidence: lib.rs:605-619 registers `zun_jit_math_sin` etc.; thunks/misc.rs:443-449 `value.sin()`. The wrapper is `#[inline]` but, taken by address as a JIT symbol, it is an out-of-line call that tail-calls/inlines the libm routine. Rust `f64::sin` on Windows MSVC links the CRT; results may differ from other platforms by 1 ulp (JS permits implementation-approximated results). The callers in Cranelift use whatever call convention builtins_table.rs declares (not verified in this audit).
- Why: libm is the unavoidable cost for sin/cos/exp/log/pow; no inline alternative with IEEE-exact semantic worth the risk.
- Proposal: none required; optionally register libm symbols directly to skip the wrapper frame. Not confirmed measurable.
- Risk: low. Frozen/ABI: no.

### F6. min/max N-ary seed op and Int arguments
- Evidence: number.rs:202-221 always seeds with `Number(identity)` then folds; for `Math.min(a, b)` this emits `min(min(INF, a), b)`: two MinF64 instead of one. Each int argument goes through `to_number_for_arg` (number.rs:291-293) — an int->f64 convert per arg.
- Why: one redundant `fmin` with a constant (probably folded by Cranelift egraph; not confirmed) and, in Tier 0, one extra opcode dispatch per call (verified by structure; not measured).
- Proposal: for N>=1 seed with the first argument (`acc = arg0`, fold the rest); keep identity only for N==0. For N==1 `Math.min(x)` must still return ToNumber(x) (NaN-propagating, so just the coerced value).
- Est. gain: removes one op per call in Tier 0; negligible in JIT. Risk: very low (NaN/-0 semantics unchanged since `min(+INF, a)==a`, including -0 and NaN). Frozen/ABI/opcode: no.

### F7. ToInt32 in `imul`/`clz32` natives (interp)
- Evidence: math.rs:205-211 `value.trunc().rem_euclid(4294967296.0)` then `as u64 as u32 as i32`; two f64 ops (fmod-like `rem_euclid`) per operand.
- Why: CLAUDE.md prescribes saturating conversion then wrap; this implementation is correct (rem_euclid wraps exactly, NaN/Inf -> 0 by guard) but slower than `x as i64` fast path for |x| < 2^63.
- Proposal: fast path `if x.abs() < 9.2e18 { (x as i64) as i32 }` (Rust `as` saturates, truncates toward zero, NaN -> 0), fallback to `rem_euclid` otherwise.
- Est. gain: a few ns per call in Tier 0. Risk: very low. Frozen/ABI: no.

### Verified-correct points (no action)
- Opcode table rows are in the single `define_opcodes!` master (opcode.rs:322-372); interp (zun_interp/src/lib.rs:866-880), verifier fixtures (zun_verify/tests/math_opcodes.rs) and JIT (translate/mod.rs:429-442) all cover 0x96-0xA4.
- Eligibility vs lowering is consistent for opcodes: eligible.rs:58-67 lists the float opcodes and each has a lowering arm in translate/mod.rs. Every name in `math_builtin` has a matching thunk symbol in lib.rs:597-625 (spot-checked all listed; `isFinite/isNaN/isNegativeZero/isSafeInteger` belong to the number module, not checked here).
- Float remainder uses a libcall deliberately (arith.rs:140-152), with a documented semantic reason.
- `fmin`/`fmax` in Cranelift 0.131 implement the JS NaN/-0 semantics as asserted in arith.rs:130-132; relies on the equivalence suite (not re-run here).

## Cross-module references

- Native-call cost / Any marshaling / libm boundary: see shared final report (NativeArg slice dispatch, thunk-per-libm-function, whole-function ineligibility from one non-whitelisted builtin).
