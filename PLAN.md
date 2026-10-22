# Zun — Architecture Plan

Zun is a **JS/TS machine-language platform** written in Rust, architected after the
.NET runtime (CLR/CoreCLR): a real compile step producing a binary **IL + metadata**
format (`.zm`), and a VM that loads, verifies and executes it — **Tier 0 interpreter
→ Tier 1 Cranelift JIT**, with **AOT** for hosts that forbid JIT (iOS).

It is **not** a V8-style runtime. Typed TS gets fixed object layouts and vtable
dispatch; dynamic JS (`any`) is the explicit, costly exception (hidden-class
*shapes* + inline caches, the .NET DLR model). Zun targets **native TS/JS** to
compete with Bun/Node, is **multi-host** (CLI, server, browser, games), and keeps a
**minimal, license-free nucleus** with everything else as fetched modules.

Decisions are grounded in `.docs-dotnet/` (a 10-area analysis of dotnet/runtime);
cited as `doc NN`. Material in `.docs-dotnet/` stays in its language; this file is
English.

---

## 1. Core invariants (never violate)

- Compiler and VM communicate **only** through the ZIL binary format (`.zm`), never
  by sharing in-memory ASTs.
- ZIL references types/members by **metadata token, never by offset**.
- The ZIL opcode set lives in **one master table** (macro) that generates the
  assembler, disassembler, verifier and codegen importer — never a second copy.
- The verifier evolves with the IL: every opcode ships with `_Valid` /
  `_Invalid_<Error>` fixtures.
- **Tier 0 interpreter is the always-correct fallback; interpreter and JIT pass the
  same equivalence suite.** An operation that cannot be lowered safely (e.g. one
  that can throw, before the Cranelift exception bridge exists) stays Tier 0.
- VM↔GC and VM↔codegen are narrow traits (ICorJitInfo-style). JIT entry points are
  redirectable (atomic patch) from day one — tiering/OSR need no redesign.
- All code, identifiers, comments and commit messages are in **English**.
- **Process discipline:** never claim "faster/better" without a worktree benchmark
  (ns/op). If an item stalls on a detail, leave its correct fallback and skip to the
  next unblocked item — only mark the goal blocked if *everything* remaining depends
  on the same decision.

## 2. Architecture (grounded in `.docs-dotnet`)

**Pipeline (doc 02, 09).** front-end → ZIL `.zm` (IL + metadata) → VM: Tier 0
interpreter → Tier 1 Cranelift, method-by-method behind an `ICorJitInfo`-style
trait. AOT is the *same* Cranelift at build time (`cranelift-object` → ELF/COFF/
Mach-O → linker), not a different backend.

**Object & values (docs 03, 04, 05; details in §4).** Unified object model: header
`[shape/class ptr][hash/flags]` + a flat slot array. A **typed class** is a shape
*frozen at compile time* → `load [slots+offset]`, no guard. A **dynamic object** gets
a **runtime shape** (hidden class) → same indexed access behind a cheap `shape == S`
guard, dictionary only as megamorphic fallback. Dynamic ops use **DLR-style call
sites** (`(guard, action)` rules, L0/L1/L2), scoped to `any`/`Proxy`/`Reflect`.
Typed values are unboxed; only `any` is boxed (compact `Any`); NaN-boxing deferred.
Measured 2026-10: shape+IC vs dictionary = **8.6x read / 45x write**; interned-id
guard vs string = **8.3x**.

**GC (docs 05, 09).** Blueprint = **Lua 5.4 generational** (not LuaJIT 2.0's
non-moving/non-arena collector, slow for the same reasons our Tier 0 GC is): a
**nursery** with bump allocation; **minor** collections over young only; rare
**stop-the-world major**; a **back-barrier** old→young feeding a **remembered set**;
an **evacuating** nursery over the forwarding map ("bump allocator ⇒ moving GC"). One
mutator thread today → no STW protocol (the mutator is parked at the allocation
safepoint). Precise roots from interpreter frames (`RootLink`) and JIT frames
(Cranelift stack maps at the allocation-helper safepoint: capture `(PC=return addr,
SP)`, walk via the map). Done: non-moving mark-sweep, moving compaction + forwarding
map, remembered-set start. Problem it fixes: Tier 0 mark-sweep is **O(n²)** per run.

**Execution (doc 09, doc 06).** Tier 0 → Tier 1 by call-count tiering.
**eager-JIT-for-loops** (compile a method with a loop straight to Tier 1 on first
call — recovers the hot-loop win with no frame-transfer ABI; the .NET pre-OSR move).
**True OSR (mid-frame) deferred**; when done, compile a partial function whose params
are the live-in set, transferred via the existing packed-args shim. Measured: Tier 0
≈ **100x** native; eager-JIT loop Tier 0→Tier 1 ≈ **121x** (255→2.1 ns/it).

**Control flow & async (doc 06, 10).** Exceptions via handler-region tables
(token-typed `catch`, half-open IL ranges), interpreter unwind + rethrow routed to
outer same-frame handlers, Tier 0 fallback until the Cranelift exception bridge
exists. `async/await` and generators are **stackless state machines**; a suspended
generator frame is **owned** state (`method_rid, pc, locals, eval_stack`) in a
GC-traced side table, with `Output`/`CallHook` injected at each resume (never
stored).

**Modules (doc 01).** `ModuleScope` ≈ AssemblyLoadContext; `.zm` ≈ assembly
(IL+metadata container — our own `ZUNM`, not PE). Build-time import resolution into a
flattened manifest the loader follows (cache → hook → parent → error). Dynamic
`import()` returns `Promise<namespace>`; literal specifiers code-split at build,
computed specifiers resolved at runtime; the cache gives single instantiation.

**Front-end (future).** Native TS/JS adopts **OXC** (`oxc_parser`) feeding the
existing sema/HIR/ilgen (benchmarked lighter than SWC: 105 vs 122 crates). Keep the
hand-written `zun_syntax` and `.zun` during transition; write an `oxc_ast → HIR`
adapter; express Zun extensions as valid TS (`declare` + decorator), never invented
keywords.

**Backend.** **Cranelift for JIT *and* AOT** (one IR → x64/aarch64/riscv/s390x +
`cranelift-object` for native binaries, GC stack maps, fast compile) — rts uses
exactly this for both. **dynasm-rs** is a per-arch macro-assembler (no IR/regalloc):
viable only as an optional hand-tuned **baseline tier / interpreter**, never the main
backend (LuaJIT hand-writes ~7 per-arch VMs — that is the cost). **GCC/libgccjit
vetoed** (GPL contaminates the core). **LLVM/inkwell** = optional future second
backend for max-opt release AOT. Benchmarked 2026-10: Cranelift vs hand-dynasm on a
fixed loop ≈ parity execution (1.12x); dynasm ~5.4x faster *compile* (irrelevant with
background compilation).

## 3. The nucleus boundary (core vs. module)

**The ZIL is the leverage: "fast" ≠ "must be in the Rust core."** A lib written in
Zun compiles to native via the JIT, so speed needs neither the Rust nucleus *nor*
hand-written Cranelift (the maintenance trap). A thing is **nucleus** only if:

1. **(a) representation floor** — layouts the VM/JIT must know (String, Number,
   Boolean, BigInt, Symbol, Object, Array, Function, the binary family);
2. **(b) VM-privileged capability** — only the runtime can do it (Error stack capture
   via frame walking + source mapping; GC; Promise scheduler/event loop;
   Proxy/Reflect meta-protocol; the realm/global object);
3. **(c) machine-state intrinsic** — time source, raw bytes;
4. **(d) ZIL opcodes Cranelift lowers to native instructions** — notably **Math**
   (`sqrt/abs/min/max/floor/ceil/trunc/round/sign` → `fsqrt/fabs/fmin/fmax/…`; int
   `imin/imax/clz/ctz/popcnt`). Transcendentals (`sin/cos/exp/log/pow`) → libm.

Everything else is a **Zun/ZIL lib** (JIT'd, maintainable), many in a shared
`@urubucode/common` module (browser + backend overlap).

### 3.1 Realm namespace & extensibility (a Zun advantage)

The **nucleus owns the realm namespace**: the base-type bindings (`String`, `Number`,
`Array`, …) and `globalThis`/`global` live there as the single source of resolution —
Zun **declares its own base types** there, not by name-magic. Consequences:

- **Literals resolve to the in-scope binding, at compile time.** `""` → whatever
  `String` is in scope. `import String from "extended-string"` **re-binds** `String`
  in that scope, so literals there use the extended surface. Because Zun is typed,
  this is a **static rebind (sema)**, not a runtime lookup.
- **Base-type method extension** (e.g. a `colors` lib adding `.red()` to `String`):
  modules contribute methods to a base type's surface, resolved at compile time as
  **extension methods** over the floor — no prototype monkey-patching, no runtime
  cost for typed code.
- **Constraint that keeps it sound:** rebind/extension is at the **surface / method /
  type** level over the **frozen representation**. You add behavior, you do **not**
  re-layout the primitive (the VM/JIT bake the frozen layout into every literal).
- **Why better than JS:** JS `String`/`globalThis` are mutable globals and prototype
  pollution is a hazard; Zun makes this **typed, static, VM-controlled** → safe and
  fast. `globalThis` itself is a per-realm/isolate singleton `DynamicObject` (host
  bindings like `process`/`window` attach from modules); *using* it is the dynamic
  (costly, opt-in) path, consistent with §2.

### 3.2 Catalog (first pass; refine against ECMAScript + Web/Node)

- **A — nucleus:** String, Number, Boolean, BigInt, Symbol, Object, Array, Function,
  Promise, Proxy/Reflect, Map/Set/WeakMap/WeakSet/WeakRef/FinalizationRegistry,
  ArrayBuffer/DataView/TypedArrays, **Date** (over the time/calendar intrinsic),
  **Buffer** (native binary surface — binary shines as machine), **JSON/ser-de** (a
  core machine concern — managed ser/de is a crippling delay; rides on the nucleus
  shape/field walk), **Error** (object may be managed, but **stack capture is a
  VM/JIT intrinsic** needing frame walking + sequence-point debug info),
  **`globalThis`/`global`** (realm singleton, §3.1), the **typed numeric lattice**
  (int/float, u8..u64, f32), **Decorators** (language feature + macro), the **ZIL math
  opcodes**.
- **B — `zun:std`** (TS, precompiled, default-installed, *not* recompiled per build):
  Math surface (over the ZIL opcodes), RegExp, prototype methods, global functions.
- **C — modules** (fetched): node (`fs/path/os/process/net/http/crypto`), browser
  (`DOM/fetch/Canvas/WebGL`, which also aliases `window`/`self` onto `globalThis`),
  `@urubucode/common` (console, JSON surface…), Intl, timers.

**Prerequisite surfaced by Error:** **sequence points / debug info** in the `.zm`
(PC→source), currently minimal — required for useful stack traces (the rts pain that
motivated Zun).

## 4. Module distribution (minimal core + fetched extensions)

The minimal core (§3 nucleus + `zun_abi`) ships; extensions live in separate official
repos with their own licenses, fetched on demand — Deno/npm/NuGet-shaped, over the
loader hook + `ModuleScope` + dynamic `import()`. A module is a **Zun-engine
extension**, not a standalone library: a **precompiled `.zm`/`.zmlib`** (Zun's own
output) or a **native extension** (Rust `cdylib`) built against the host **extension
ABI** (`zun_abi`/`zun_register_vN`, the napi *analogue*). Neither runs without the
engine.

**Extension ABI.** Native side registers functions/values under a module with machine
types. TS side binds by name with a typed generic inside a `namespace` (not a class):
`module("zun:node/fs").func<(p:string)=>string>("fs.readFileSync")` / `.value<T>`.
The **native registration is the source of truth** — sema verifies the generic
against it. Bindings are first-class function values; calls lower to `call.builtin`.
`extern` methods (instance methods on primitive-backed classes) and `module().func`
bindings coexist.

**Index + trust.** The release ships a refreshable **index** (JSON: logical name →
repo → release-selection) with per-release **content hash + signature**. An
official-index module is auto-fetched **once** (one module, log line, cached,
verified); unknown → **error, resolve with `zun i`**; the structural gate is the
signed official index (no human prompt). `--offline`/strict disables runtime fetch;
`zun i -g @urubucode/*` pre-installs (Docker). **Compat key is the ABI/contract
version** (`.zm` format + intrinsic ids + core-type layouts + host ABI), declared per
release. Each module declares **capabilities** (`fs/net/screen/native`). Pure-Zun
modules ship one portable `.zm`; native/FFI ship per-arch.

**Trust anchor (open gate, Phase 7).** `SignedModuleLock` today verifies the signature
against the public key *carried inside the envelope* — it proves "signed by whoever
holds this key", not "signed by the official `@urubucode/*` key". The root of trust
**must come from outside the envelope**: a release-embedded root key (or a signed
official-index key set) that the loader pins and checks the envelope key against;
self-declared envelope keys are never trusted on their own. Key rotation/revocation
policy rides on the envelope `VERSION` + the signed index. Closing this is a hard
gate for Phase 7 — without it the signature is authenticity theater.

The loader now exposes `verify_against` on signed lock and index envelopes so callers
can enforce a release-pinned root key; wiring that key into the registry/fetch path
remains the final trust-anchor integration gate.

`TrustedKeySet` now provides the release-provisioned key set, and signed-index and
`.zmlib` loading reject envelope keys outside that set before module registration.
`ModuleLoader::new_with_trusted_keys` retains the provisioned set for package loads.
`SignedModuleLock::verify` now requires that external set; embedded-key-only
verification is limited to signature parsing during envelope construction.
`load_zmlib` rejects loaders without a provisioned key set, so package loading has
no insecure self-declared-key fallback.
`TrustedKeySet::rotate`/`revoke` provide the key-set transition hooks selected by
the signed-index `VERSION` policy.

**`.zmlib` carries type declarations + debug info** (metadata-as-interface, like a
.NET assembly / `.d.ts`): imports are typed and inspectable, and errors from a
precompiled module still point at source.

**Trimming granularity (what the AOT binary can shed).** Trimming is reachability over
what the compiler *sees as IL/metadata*, exactly like .NET's ILC. Two tiers:
- **Zun/ZIL modules** (`.zm`/`.zmlib`) are fully visible: the AOT build walks the
  reachability graph from the roots (`main` + imports actually called) and **links
  only reachable functions** — unused exports and untouched modules never enter the
  binary. Same mechanism that trims `runtime.lib`. A large surface (e.g. a Node
  reimplementation) is trimmable **iff** it is shipped as a ZIL module.
- **Native ABI extensions** (`zun_abi` `cdylib`) are opaque native boxes: no
  per-function tree-shaking, and they are **loaded dynamically at runtime via
  `libloading`**, so they are **not part of the AOT binary** at all — they ship as a
  separate per-arch `.dll`/`.so` loaded on demand.

Design rule for big surfaces (Node, Web APIs): implement the bulk in **Zun/ZIL over a
thin native capability floor** (fs/net/timers = syscalls in `runtime.lib`), and use the
native ABI only where genuinely native (driver, codec). ZIL-over-thin-floor is
trimmable like the core; a monolithic native extension is not. This is a structural win
over Node: Node loads everything at runtime, the Zun AOT binary links only the reachable.
Granularity is **per reachable function**, transitively: `import { readFileSync }` that
calls only `readFileSync` links `readFileSync` + its callees (down to the capability
syscalls), not the rest of `fs`, and nothing of untouched modules.

**What defeats static reachability (and how it is handled).** The graph is static — it
must *see* the call site. Two cases break it: **dynamic `import()` with a computed
string** (`import("node:" + name)`) and **reflective access by name** (`fs[method]()`).
Neither **fails the build** (unlike .NET's ILC, which needs `rd.xml`): the unresolved
code stays as embedded IL and runs on the **fallback interpreter** — correct, just not
trimmed/AOT-fast there. The compiler **emits a build warning** at each such site naming
what could not be resolved, and the resolution: make the import static, or **declare the
dynamic roots** (an explicit root/preserve manifest — the rd.xml analogue, opt-in) so
that branch is trimmed and AOT-compiled too. Default = warn + keep-correct; opt-in =
declare and trim.

**N-API embedding (Zun-inside-Node) is out of core scope** — Zun competes with Node,
it is not a guest. Node `.node` addon compat, if ever, is a separate module.

## 5. Performance strategy (measured)

The Tier 0 **100x** is not fixed by speeding the interpreter — it is made irrelevant
by getting hot code to the JIT. Priority: (1) **generational GC** (fixes the O(n²)
run cost; nursery ~22x in a model); (2) **eager-JIT-for-loops** (121x) and expanding
JIT eligibility so fewer ops fall back to Tier 0 (**frozen-type ops lower to direct
native calls / templates**, never through interpreter dispatch). Polish: super-
instructions (~5x on Tier 0, for startup/cold code), cached std, inline of small
intrinsics in the MIR. Measured 2026-10: Map/Set O(1) (8.6x/45x vs dict); interface
IC 3.8x (slot-vtable ≈ IC for monomorphic); `switch` jump-table 11.3x; symbol-keyed
slot 16.5x; scalar `Any` box/unbox ≈ 0 (escape analysis not worth it for scalars —
heap allocation / the GC is).

## 6. Implementation phases

- **Phase 0 — format:** `.zm`, metadata tables/tokens, master opcode table,
  round-trip. **done.**
- **Phase 1 — minimal pipeline:** parser/sema (TS subset), ilgen, verifier, loader,
  interpreter running `.zm`. **done.**
- **Phase 2 — real runtime:** GC heap (mark-sweep → compaction + forwarding map +
  remembered-set start), object layouts, vtables, classes/inheritance/virtual
  dispatch. **done.**
- **Phase 3 — Cranelift Tier 1:** typed stack, `ICorJitInfo`-style trait, redirectable
  entry points, interp↔JIT equivalence, heap-touching lowering (objects, arrays,
  strings, Map/Set), eligibility rule (throwing/unsupported → Tier 0). **done /
  ongoing (expanding eligibility).**
- **Phase 4 — dynamic & language surface:** closures (capture-by-binding, boxed
  mutables, GC-traced), exceptions, iteration (`for...of`/`Symbol.iterator`),
  Map/Set/WeakMap, async/await, generators, BigInt, Group 2 syntax, DataView/typed
  arrays, static/private fields, accessors. **done.**
- **Phase 5 — performance & nucleus (current):** **generational GC** (Lua 5.4
  blueprint); eager-JIT-for-loops; the **nucleus-boundary refactor** (§3: std-TS →
  precompiled `zun:std` module; JSON/console → `@urubucode/common`; Math → ZIL
  opcodes; keep floor/capabilities + realm namespace in core); realm namespace with
  rebind/extension (§3.1); sequence-point debug info for Error stacks; `zun_abi`
  extension ABI + `.zmlib` with type declarations.
- **Phase 6 — front-end & AOT binary:** OXC native TS/JS front-end + adapter; the
  **AOT linker step** (below) → iOS/arm64 AOT.

  **AOT pipeline (`zun compile main.ts` → self-contained binary).** Mirrors .NET
  Native AOT (ILC), minus the all-or-nothing trap:
  1. **Emit `.o`** — Cranelift (`cranelift-object`) compiles the AOT-eligible functions
     to a native object (COFF/ELF/Mach-O); non-eligible bodies (dynamic `Any`, throwing
     before the exception bridge) stay as IL.
  2. **Resolve toolchain** — find/download LLD (cache → PATH → rustup `rust-lld` →
     download); on Windows provision MSVC CRT + Windows SDK import libs via **xwin**
     (no Visual Studio). Target flavor via `target-lexicon` → `lld-link`/`ld.lld`/
     `ld64.lld`.
  3. **Gather inputs:** the `.o` + **`runtime.lib`/`libruntime.a`** (the Zun runtime as
     a static lib: GC, intrinsics, scheduler, **fallback interpreter**) + a **startup
     shim** (`zun_rt_start`: init runtime → call Zun `main` → shutdown) + a **data
     object** carrying the `.zm` metadata/IL blob (generated via the `object` crate —
     *data, not executable*, so no AV heuristic) + platform CRT/SDK.
  4. **Link via EXTERNAL LLD (subprocess)** — LLD resolves relocations, lays out
     sections, writes a conventional PE/ELF/Mach-O. **Never emit executable bytes
     in-process** (in-process PE byte-writing legitimately matches AV file-infector
     heuristics — confirmed empirically in rts). This step is non-negotiable.
  5. **(optional) trim** unreachable runtime + ZIL modules (reachability graph, §4);
     **(optional) code-sign** (Authenticode) for release trust.

  Output = native code (Cranelift) + `runtime.lib` + embedded metadata/IL, AOT, **no
  runtime JIT** (iOS-safe). Reuse rts's `rts-linker` design (LLD + xwin + toolchain
  discovery). .NET parity, with one deliberate divergence: ILC ships **no IL** and
  breaks on anything it can't resolve statically (needs rd.xml); Zun **embeds the IL +
  fallback interpreter**, so non-AOT-eligible code runs interpreted instead of failing
  the build — the "Tier 0 always correct" invariant extended to the AOT binary.
- **Phase 7 — distribution:** module registry/fetcher (index, lockfile, content-
  addressed cache, signatures, capabilities), official repos (`@urubucode/*`).
- **Deferred:** true OSR (mid-frame), SharedArrayBuffer/Atomics + multi-thread STW GC,
  RegExp engine, full trimming/R2R, LLVM second backend.

## 7. Implementation ledger (2026-10-22)

On `main`, implemented and test-covered: format/metadata/verifier; interpreter with
precise roots; mark-sweep GC + moving compaction + forwarding map + remembered-set
start; generational GC (nursery bump allocation, budget-triggered evacuating minor
collections, old-to-young barriers, weak/ephemeron stress coverage, and native
stack-map root rewriting); Cranelift Tier 1 (typed stack, virtual-slot dispatch,
heap/object/array/string
+ Map/Set lowering, eager-JIT-for-loops); closures; exceptions (unwind + rethrow
routing); iteration; async/await; generators; BigInt; Group 2 syntax; DataView/typed
arrays; static/private/accessors; Map/Set/WeakMap (ordered O(1) + ephemeron tracing);
`Any`/`DynamicObject`; cached prelude plus the `zun:std` cached-module boundary;
relocatable per-realm global singleton with `globalThis`/`global` bindings;
versioned native extension ABI with owned `namespace`/`module().func` lookup;
scalar + string FFI with declared ownership contracts; multi-module loader/manifest;
SHA-256 content-addressed cache with deterministic lockfiles, Ed25519 signatures,
and capability declarations; deterministic registry index metadata with verified
loader integration; signed registry-index envelopes with HTTP decoding and
tamper coverage;
release-pinned key checks now gate signed-index loading and `.zmlib` verification;
anchored and self-declared signed-index fixtures are covered;
HTTP index fetching accepts the rotated `TrustedKeySet` path;
sequence-point PC-to-source metadata in `.zm`;
minimal `std` surfaces; static `String` rebinding with frozen-floor extension
methods; checked typed invocation for registered native extension descriptors;
VM extension loading now exposes that checked invocation boundary.
Structured method-name/source-span frame resolution is also available to
diagnostic consumers; interpreter operational faults now carry the nearest
source frame through the public runner boundary, including recursive failures.
Remaining gates:
full AOT binary (linker + runtime bundle), native TS/JS front-end, full
remote module transport/index integration (with a safe local directory
fetcher, HTTP(S) module fetcher, and HTTP registry-index fetcher now implemented), dynamic-import warnings/fallback (conservative
member-call reporting is now implemented, with affected method names emitted in
CLI warnings and dynamic instruction PCs retained by the IL decoder, with
sequence-point source spans attached when available, reflective property access
included, and regression coverage),
and true OSR. Explicit preserve-root reachability, trimming APIs, and the CLI
preserve-root manifest path—including AOT builds—are now implemented. The signed
ineligible AOT entrypoints now preserve the embedded-IL fallback without failing
the build.
 AOT builds now emit the same dynamic/reflection fallback site warnings even when
 trimming is disabled.
`.zmlib` package envelope is now implemented, including optional per-module type
declaration payloads authenticated by `<module>.d.ts` lock entries; debug information
remains embedded in each `.zm` module. Declaration lock entries are excluded from
executable `.zm` cache verification. The former
Metadata-only lock entries are also excluded from module fetch/loading.
`zun_napi` Node embedding crate has been removed (out of scope).
The loader now offers a cache-backed fetch path that verifies a module's content
identity before persisting and registering it, enabling verified offline reuse.
Lockfile and registry-index bulk loading now expose the same verified cache path.
Signed registry-index loading exposes anchored cache variants, so envelope trust
is established before any selected module is persisted or registered.
Signed lock loading now exposes the same external-key and retained-key paths,
including cache-backed variants, closing the trust gate at lock registration.
Content-addressed cache writes now repair an existing entry whose bytes no longer
match its content identity, with regression coverage for tampered cache files.
Directory and HTTP registry transports now reject parent, absolute, backslash,
query, fragment, percent-encoded, colon, and control-character module paths before
filesystem or URL interpolation; logical lock/index identities remain unchanged.
HTTP registry base URLs likewise reject query, fragment, backslash, and control
characters before endpoint construction.
The AOT launcher now resolves an explicit `ZUN_LLD` override, an LLD executable on
`PATH`, or the active toolchain's `rust-lld`, and passes it to the external final
link step when available.
The AOT build also emits a relocatable metadata object containing the serialized
`.zm` payload under the `zun_embedded_module` symbol and supplies it to the link.
It also exports `zun_embedded_module_len` for runtime initialization.
Manifest v4 records the metadata object's and runtime-library paths while retaining reads of older
manifest versions; the generated launcher references the symbol so the embedded
payload is retained by the final link.
`zun_vm` now emits both its normal `rlib` and a Cargo `staticlib` suitable for
the AOT runtime-library input. The CLI accepts `ZUN_RUNTIME_LIB` and supplies
that matching static library to the external AOT link when provisioned.
Packaged CLIs also discover `zun_vm.lib` or `libzun_vm.a` beside the executable
when no override is set, and record adjacent libraries by portable filename in
the manifest.
The runtime static library exports `zun_rt_init`/`zun_rt_shutdown`, and the
generated launcher calls those hooks around the native entry point when the
library is linked.
The runtime static library also exports a scalar embedded-IL interpreter hook for
the forthcoming ineligible-entry AOT launcher fallback.
When a matching runtime library is provisioned, the CLI now links that hook and
the embedded metadata object into a runnable fallback executable for ineligible
scalar entrypoints; without the library it retains the portable `.zm` fallback.
The fallback linker supplies the Windows cryptography system libraries required
by the runtime static library, and leaves non-scalar managed returns on `.zm` only.
The CLI has an integration fixture that provisions `zun_vm.lib` when available,
builds an ineligible scalar entrypoint, and executes the resulting fallback binary.
Signed and unsigned scalar fallback ABIs are both covered, including the generated
launcher’s `u64` path; boolean fallback output preserves native `true`/`false` text.
Floating fallback now uses an out-parameter ABI and is covered for `f64` output.
Fallback AOT manifests now record the emitted metadata object and runtime library,
while leaving `native_object` unset.
The runtime also exposes a generic display-result fallback for managed, dynamic,
and other non-scalar entrypoints, allowing a provisioned runtime library to link
an interpreter-backed executable for every entry return shape.

## 8. Risks & attention points

- Generational + evacuating nursery is the highest-value and hardest remaining
  runtime work; correctness needs **stress testing** (forced GC, cyclic WeakMap
  graphs), not just unit tests — green unit suites do not prove weak-GC correctness.
- Cranelift has no mature GC statepoints beyond stack maps, no inlining, no escape
  analysis — those belong in Zun's IL/MIR before lowering.
- Dynamic-`Any` performance is intentionally *not* a goal (costly opt-in); do not
  JIT-lower the `Any` carrier at the cost of complexity.
- The two heavy external dependencies are **Cranelift** (now) and **OXC** (future),
  both at swappable pipeline edges, both permissively licensed; the nucleus is Zun's.
