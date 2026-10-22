# GOAL — Zun implementation agent

You are the implementation agent for the Zun project. Execute the backlog below
**continuously, part by part, and stop only when it is all done** (or on a real
environment blocker, or a design decision not in the plan). Do not go idle between
slices — chain one into the next.

**Read `PLAN.md` first** — it is the authoritative architecture and the source of
every decision referenced here (invariants §1, architecture §2, nucleus boundary §3,
module distribution §4, performance §5, phases §6, ledger §7). Keep the §7 ledger
current as you land items.

## Rules (every slice)

- Atomic commit per sub-step; `cargo test --workspace` green before moving on.
- Tier 0 interpreter is the always-correct fallback; interpreter↔JIT equivalence for
  any new IL. An op that can throw (before the Cranelift exception bridge) stays
  Tier 0 — the eligibility rule is centralized, not an allowlist.
- Opcodes only via the master table; every new opcode ships `_Valid`/`_Invalid`
  fixtures. Reference types/members by metadata token, never by offset.
- Data structures with real complexity live in the Rust floor; TS classes are thin
  wrappers — but remember **the ZIL is the leverage** (PLAN §3): a Zun/ZIL lib JITs
  to native, so "needs to be fast" does NOT mean "put it in the Rust core" — only the
  §3 nucleus criteria do.
- English everywhere; mark commits touching GC, closure capture, or exception unwind
  with `GC:` / `closure-capture:` / `unwind:`.
- Ground design in `.docs-dotnet/` before inventing.
- **Do not block on a detail — skip to the next unblocked item.** Only mark the goal
  blocked if ALL remaining items depend on the same unresolved decision. Low-value
  refinement tails (e.g. JIT-lowering the dynamic `Any` carrier) must never stop the
  high-value work. The dynamic `Any` path is intentionally not JIT-fast (PLAN §2).
- **Validate before claiming "faster/better":** benchmark in an isolated worktree
  (ns/op) first. Measured beats asserted.
- **Module hygiene / file splitting is owned by the reviewer, not you** — focus on
  implementation.

## Environment

- Full repo access; edit the repo and `.git`, do `git add`/`commit` **locally**. **Do
  not `git push`** (proxy-blocked). Remove a stale `.git/index.lock` before
  committing. Never leave the tree dirty between commits.

## Status

Phases 0–4 are done (format, pipeline, runtime+GC mark-sweep/compaction, Cranelift
Tier 1, and the dynamic/language surface: closures, exceptions, iteration,
Map/Set/WeakMap, async/await, generators, BigInt, Group 2 syntax, DataView/typed
arrays, static/private/accessors, eager-JIT-for-loops, cached prelude, scalar+string
FFI). The current focus is **Phase 5** (PLAN §6).

## Backlog (in order)

1. **Generational GC — highest value** (PLAN §2 GC, §5, §6 Phase 5). The Lua 5.4
   blueprint: nursery + bump allocation; minor collections over young only; rare
   stop-the-world major; back-barrier old→young feeding the remembered set (already
   started); evacuating nursery over the forwarding map. Fixes the Tier 0 O(n²) run
   cost. Single-threaded ⇒ no STW protocol (mutator parked at the allocation
   safepoint). Commits `GC:`. Needs **stress testing** (forced GC, cyclic WeakMap
   graphs), not only unit tests.

2. **Expand JIT eligibility** (PLAN §5): keep lowering frozen-type ops (object/array/
   string/Map/Set) to direct native calls so fewer ops fall back to Tier 0. Centralized
   eligibility rule stands (throwing/unsupported → Tier 0).

3. **Nucleus-boundary refactor** (PLAN §3): move std-TS out of the always-recompiled
   bundle into a precompiled, default-installed `zun:std` module; JSON + console →
   `@urubucode/common` (Zun/ZIL); Math machine ops → **ZIL opcodes** lowering to
   Cranelift (`fsqrt/fabs/fmin/fmax/…`), transcendentals → libm; keep the floor +
   capabilities + realm namespace in core. Implement the **realm namespace** (§3.1):
   `globalThis`/`global` as the per-realm singleton, base-type bindings resolvable and
   **statically re-bindable** (`import String from "extended-string"`) + **extension
   methods** over the frozen floor (behavior, never re-layout). Add **sequence-point
   debug info** in the `.zm` (PC→source) so `Error` stacks are useful.

4. **Remove `zun_napi`** (N-API embedding is out of core scope, PLAN §4): delete the
   crate + workspace member + `node-api` feature; keep `libloading`/FFI. Build the
   **extension ABI** `zun_abi`/`zun_register_vN` + the `namespace`/`module().func`
   surface (PLAN §4) — the foundation of the separate-module model.

5. **Phase 6 — front-end & AOT binary** (PLAN §6): OXC native TS/JS front-end +
   `oxc_ast→HIR` adapter; the **AOT pipeline** (PLAN §6 Phase 6): `cranelift-object`
   `.o` + `runtime.lib` (runtime static lib w/ fallback interpreter) + startup shim +
   embedded metadata/IL data object → **external LLD** (never in-process byte emission)
   + xwin on Windows → self-contained binary, like rts's linker → iOS/arm64 AOT.
   Trimming is reachability over ZIL (§4): ZIL modules trim **per reachable function**
   (transitive closure), native ABI extensions ship as separate dynamic libs. Dynamic
   `import()`/reflection defeat static reachability → **warn at the site + fall back to
   embedded-IL interpreter (never fail the build)**; opt-in **root/preserve manifest**
   (rd.xml analogue) to declare dynamic roots so that branch trims + AOT-compiles too.

6. **Phase 7 — distribution** (PLAN §4): the module registry/fetcher (index,
   lockfile, content-addressed cache, signatures, capabilities) + `.zmlib` with type
   declarations and debug info. **Hard gate — trust anchor:** `SignedModuleLock`
   currently verifies against the public key *inside* the envelope (proves "signed by
   some key", not "signed by the official key"). Pin the root of trust **outside** the
   envelope (release-embedded root key / signed official-index key set); never trust a
   self-declared envelope key. Add key rotation/revocation over `VERSION` + signed index.

Deferred: true OSR (mid-frame transfer), SharedArrayBuffer/Atomics + multi-thread STW
GC, RegExp engine, full trimming/R2R, LLVM second backend.

When everything is done: `cargo test --workspace`, confirm `main` is green and
PLAN.md's §7 ledger is current, and write a summary. Start with item 1.
