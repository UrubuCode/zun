# Zun against .NET 10: where the time goes, and the plan to close it

Measured 2026-11-22, same machine, release builds. The suite is
`bench/dotnet/suite.ts` and its line-for-line C# twin `bench/dotnet/Program.cs`
(`dotnet build -c Release bench/dotnet/suite.csproj`). Every line runs the same
work in both; `number` is `double` on both sides.

| line | .NET 10 | Zun | Node 22 | Zun / .NET |
| --- | --- | --- | --- | --- |
| `local` (f64 loop) | 28.5 ms | 29.0 | 49.1 | **1.0x** |
| `call` (static call in loop) | 29.3 | 29.0 | 48.7 | **1.0x** |
| `field` (`o.s` on a class) | 34.7 | 33.0 | 50.1 | **1.0x** |
| `two_hops` (`o.inner.s`) | 35.2 | 34.0 | 50.6 | **1.0x** |
| `fib30` (recursion) | 6.9 | 6.0 | 7.2 | **0.9x** |
| `virtual` (overridden call) | 3.1 | 9.0 | 2.0 | 2.9x |
| `mod_static` (module `let`) | 35.2 | 422 | 76.0 | 12x |
| `class_static` (`C.total`) | 35.8 | 425 | 77.0 | 12x |
| `obj_literal` (`{ s: 1 }`) | 64.8 (Dictionary) | 3831 | 50.1 | 59x |
| `alloc` (`new Box()` in loop) | 8.5 | 1459 | 3.4 | 170x |
| `array` (`number[]` sum) | 3.0 | 2075 | 1.9 | 690x |
| `string` (`s = s + "x"`) | 2347 | 911 | 7.0 | 0.4x |

## The headline

**Where Zun's JIT takes the code, Zun already matches .NET.** Five lines are at
parity, on typed locals, typed fields, direct calls and recursion. Cranelift is
not the gap. Every slow line is slow for a reason that sits *outside* code
generation, and each reason was located by instrument, not by deduction
(`ZUN_JIT_TRACE=1`, added with this document, prints why a hot loop stayed
interpreted and names the first offending instruction).

## The causes, one per slow line

### `array`, 690x: the type system erases element types

`zun_sema/src/types.rs:106`: "`T[]` (any element type, any depth) is an untyped
`Array`". So `a[j]` on a `number[]` is `any`, `s + a[j]` becomes `addAny`, and

    [jit] loop b_array@0x007c stays interpreted: opcode not lowered: CallBuiltin addAny

.NET's `List<double>` is a typed generic; the element load is one `movsd`. This
is the largest gap and the most common shape in real code. **Fix: carry the
element type** (`Ty::Array` gains an element, or a typed-array representation
for `number[]`), so `a[j]` is `f64` and the existing `ldelem` lowering applies.

### `alloc`, 170x: every object has a side record and a hash entry

The loop *is* compiled. Two experiments ruled out collection frequency: raising
the 256-allocation cadence changed 1459 -> 1308 ms, and an 8 MB nursery budget
changed nothing. The cost is in `Heap::alloc` itself
(`zun_gc/src/allocation.rs:50`): every allocation takes an identity hash,
consults a `free` map keyed by size, pushes a `Record`, and inserts into the
`by_addr` `HashMap` -- behind a native call, two thread-local lookups and an
`ActiveNativeFrameScope`. Roughly 450 ns per object.

.NET allocates with an inline bump of a thread-local pointer -- about ten
instructions emitted by the JIT, no call on the fast path -- and keeps **no
per-object side structure**: the heap is walkable because the object header
names the type and the type names the size. **Fix, in two steps:** (1) inline
bump allocation in JIT code with the helper as the slow path only; (2) make the
nursery linearly walkable from headers so young objects need no `Record` and no
`by_addr` entry until they survive.

### `obj_literal`, 59x: `lddyn`/`stdyn` are not lowered at all

    [jit] loop b_literal@0x0021 stays interpreted: opcode not lowered: LoadDynamic

Covered in `2026-11-22-property-access-cost.md`. The Zun answer is a fixed
layout for a literal of known shape (`ldfld`/`stfld`, already at .NET parity as
the `field` line shows). Note .NET's own dynamic equivalent, a
`Dictionary<string, object>`, costs 64.8 ms: a compiled hash lookup is 2x a
field, not 100x. So even the dynamic path has a 50x prize once it is compiled.

### `mod_static` / `class_static`, 12x: statics are found by name

Compiled, pure f64, but `ldsfld` is a native call that hashes the statics
object's address and then a string slot key. .NET's static is a load from a
fixed address. **Fix: a flat statics store indexed by FieldDef RID**, lowered to
a `load` at a constant address. The obstacles (GC scanning, an "unset" sentinel
for inherited statics) are written down in the property-access document.

### `virtual`, 2.9x: no devirtualization

.NET gets 3.1 ms from *guarded devirtualization*: dynamic PGO records the
receiver's class at Tier 0, and Tier 1 emits `if (type == Sq) <inlined body>
else <virtual call>`. Zun pays the indirect call and cannot inline.

### `string`, 0.4x -- Zun wins, and both lose to V8

.NET copies the whole string on every `+` (O(n^2)); so does Zun, with a smaller
constant. V8 uses ropes. Not on the .NET-parity path, but a rope or builder
representation for repeated concat is the known answer.

## De-optimization: what .NET actually does, and what Zun should copy

The question was whether Zun needs a de-optimizer to speculate. **.NET has
none.** RyuJIT never abandons a compiled frame. It gets the benefit of
speculation with three mechanisms that need no frame reconstruction:

1. **Guarded fast path with an in-method fallback.** Every speculation is an
   `if (guess holds) fast else slow` diamond, and the slow arm is ordinary
   compiled code (usually a helper call). Nothing is ever "undone".
2. **Tiering by re-compiling the method, not the frame.** Tier 0 is a fast,
   unoptimized JIT with instrumentation; after 30 calls the method is queued
   and re-jitted at Tier 1 using the profile. Entry points are patched; frames
   already running finish in the old code.
3. **OSR for the one case re-jit cannot reach** -- a frame stuck in a hot loop.
   Tier-0 code has patchpoints at loop back-edges; the runtime jumps *up* into
   optimized code. That is a transfer in the up direction only.

Zun has (3) built and working, and redirectable entry points for (2). So the
plan is **not** a de-optimizer. It is (1): guards with cold fallback arms. That
is also the answer to `lddyn`: a guarded inline fast path (own data property,
no accessor) whose slow arm calls the full helper. What the slow arm needs is
the *exception bridge* -- a helper that ran managed code must be able to
report a throw to the compiled frame -- and that, not de-opt, is the real
prerequisite. A de-optimizer only becomes necessary for speculation that
*removes* the fallback (V8-style), which the CLR model never does.

## Avoiding the interpreter: Tier 0 should be a JIT

The CLR has no interpreter on the hot path at all: Tier 0 **is** a JIT. Zun's
eligibility is an allowlist -- one unknown opcode, one `any` local, and the
whole method (and its OSR loop) falls to the interpreter, which is where every
100x cliff in this document comes from. RyuJIT's answer is that *every* IL
opcode is compilable, because anything it does not open-code becomes a call to
a runtime helper (`JIT_New`, `JIT_GetField`, `JIT_ChkCast`, ...).

**The strategic item: make the allowlist total.** Any opcode the translator
does not open-code is lowered as a call to the interpreter's own
implementation of that opcode, with `Any` passed in its 16-byte carrier form.
A method is then *always* compilable; "not lowered" stops being a 100x cliff
and becomes a call overhead, and each helper can later be replaced by inline
code one at a time, guided by profiles. Prerequisites: `JitKind::Other` (`any`)
locals in compiled frames with stack-map coverage, and the exception bridge.

## The interpreter itself

It will remain the fallback (cold code, startup, anything not yet bridged), so
its speed still matters. Today `zun_interp/src/lib.rs:609` calls
`decode_one(&code[pc..])` **on every dispatch**: each instruction is re-decoded
from bytes each time it executes, operands and tokens re-resolved, and values
move through a tagged `Value` enum on a `Vec`.

Mono's interpreter (the base of the new CoreCLR interpreter for iOS/WASM) does
not interpret IL. It **transforms each method once** into an internal
instruction stream and interprets that:

- **Pre-decode once per method** into a dense array; tokens resolved to direct
  pointers/slots at transform time (field offset, callee, type handle).
- **Locals as untyped fixed-offset slots**; types are known statically from the
  verifier, so no tag is checked or written per operation.
- **Superinstructions** for the hot sequences (`ldloc; ldloc; add; stloc` is
  one dispatch), and compare-and-branch fusion.
- **Quickening**: an instruction rewrites itself to a specialized form after
  its first execution (inline cache in the instruction stream).
- **Tiering inside the interpreter**: an unoptimized transform first, a
  re-transform with inlining and constant folding once hot.
- Dispatch: CPython 3.14 measured ~5% (geomean) from tail-call dispatch over
  computed goto; in Rust the equivalent needs `become` (nightly), so a dense
  `match` over a pre-decoded opcode is the practical form today.

For Zun the first step is by far the largest: **decode once, cache per
method**. It removes the byte decoding from every dispatch without changing
semantics, and it is the foundation the rest (resolved tokens,
superinstructions, quickening) builds on.

## The order of work

Ranked by measured gap times how common the shape is in real programs.

| # | item | closes | size |
| --- | --- | --- | --- |
| 1 | Typed array elements (`number[]` reads `f64`) | `array` 690x | medium |
| 2 | Interpreter: decode once per method | every interpreted path | small-medium |
| 3 | Statics at a fixed address | statics 12x | medium |
| 4 | Inline bump allocation; drop per-object records for young objects | `alloc` 170x | large |
| 5 | Exception bridge + total lowering via helpers (Tier 0 is a JIT) | every "not lowered" cliff | large |
| 6 | Fixed layout for object literals | `obj_literal` 59x | large |
| 7 | Guarded devirtualization + inlining from a call-site profile | `virtual` 2.9x | medium |
| 8 | Rope/builder for repeated string concat | `string` vs V8 | medium |

## Sources

- dotnet/runtime, `docs/design/features/tiered-compilation.md`
  (https://github.com/dotnet/runtime/blob/main/docs/design/features/tiered-compilation.md)
- dotnet/runtime issues 112748 and 121511, the CoreCLR interpreter built on the
  Mono interpreter (https://github.com/dotnet/runtime/issues/121511)
- "Bing on .NET 8: the impact of Dynamic PGO"
  (https://devblogs.microsoft.com/dotnet/bing-on-dotnet-8-the-impact-of-dynamic-pgo/)
- Jiterpreter for WebAssembly, on Mono interpreter tiering
  (https://paper.dropbox.com/doc/Jiterpreter-For-WebAssembly-mJ5TRc7xF1PNAGMgwU49K)
- CPython tail-calling interpreter (https://lwn.net/Articles/1010905) and the
  later corrected numbers (https://fidget-spinner.github.io/posts/no-longer-sorry.html)
- Local reference: `.docs-dotnet/09-memory-runtime/06-tiered-compilation-osr.md`,
  `07-inlining-otimizacoes-ryujit.md`, `01-geracoes-gc.md`
