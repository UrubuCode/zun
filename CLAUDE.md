# Zun

A JS/TS machine-language platform written in Rust, architected after the .NET runtime
(CLR): a real compile step that produces a binary IL + metadata format, and a VM that
loads, verifies, and executes it natively via Cranelift (interpreter Tier 0 → JIT
Tier 1). This is **not** a V8-style runtime: typed TS code gets fixed object layouts
and vtable dispatch; dynamic JS (`any`) is the explicit, costly exception (`Any` value
with IL-level `box`/`unbox` + `DynamicObject`).

Read `PLAN.md` first — it defines the full architecture, the crate map, and the
implementation phases. Design decisions are grounded in `.docs-dotnet/` (a 10-area
analysis of dotnet/runtime); when making a runtime/compiler design choice, check the
corresponding area doc before inventing something new.

## Language rules

- **All code, identifiers, comments, commit messages, and documentation are in
  English.** No exceptions inside the repo.
- **Portuguese is used only when talking to the user** in the conversation. Never
  edit files in Portuguese (except `PLAN.md` and `.docs-dotnet/`, which are existing
  Portuguese reference material — extend them in their own language if ever needed).

## Repository layout

```
PLAN.md               # architecture plan (Portuguese, reference)
.docs-dotnet/         # .NET runtime analysis, 10 areas (Portuguese, reference)
docs/guides/cranelift.md  # Cranelift 0.131 complete reference — READ BEFORE codegen work
crates/               # Cargo workspace (see PLAN.md §2 for the crate map and dep rules)
```

Crate dependency rule: the format crates (`zun_il`, `zun_metadata`, `zun_module`) are
the contract. Compile-time crates and runtime crates both depend on the format, but
**never on each other** (only the driver/CLI links both).

## Core architectural invariants

- Compiler and VM communicate **only** through the ZIL binary format (`.zm`), never
  by sharing in-memory ASTs.
- ZIL instructions reference types/members by **metadata token, never by offset**.
- The ZIL opcode set is defined in **one master table** (macro) that generates the
  assembler, disassembler, verifier, and codegen importer. Never hand-write a second
  copy of opcode knowledge.
- The verifier evolves together with the IL: every new opcode lands with
  `_Valid` / `_Invalid_<Error>` fixtures.
- VM↔GC and VM↔codegen boundaries are narrow traits (ICorJitInfo-style). Codegen
  never reaches into GC internals or vice versa.
- JIT entry points are redirectable (atomic pointer patch) from day one — tiering
  and OSR must not require a redesign.
- Interpreter (Tier 0) is the always-correct fallback; interp and JIT must pass the
  same equivalence test suite.

## Cranelift standards

`docs/guides/cranelift.md` is the authoritative reference (version 0.131). Follow it,
plus these project rules distilled from its pitfalls section:

- Use `CallConv::Tail` for all managed (user/std) functions; `SystemV` /
  `WindowsFastcall` only at the native FFI boundary. Set `preserve_frame_pointers`,
  `use_egraphs`, `enable_alias_analysis` in the ISA flags.
- **Dedup `FuncRef`/`SigRef` imports**: cache a `FuncRef` per `FuncId` per function
  (see reference §16). Never call `declare_func_in_func` repeatedly for the same
  callee.
- JS number→int coercions use the **saturating** `fcvt_to_sint_sat`/`fcvt_to_uint_sat`
  (the trapping forms SIGILL on NaN/out-of-range), then apply ToInt32 wrap semantics
  separately.
- `icmp`/`fcmp` return `I8` — extend explicitly before storing as a wider value.
- Seal every block exactly once; loop headers are sealed only after the back-edge.
- `MemFlags::trusted()` only for stack slots and known-valid runtime structures; tag
  alias regions (`Heap`/`Table`/`Vmctx`) only when non-aliasing is guaranteed.
- Mark slow paths (IC misses, error paths, allocation slow path) as cold blocks.
- Cranelift does **no inlining, no escape analysis, no autovectorization** — those
  belong in our IL/MIR layer before lowering.
- GC integration: `declare_value_needs_stack_map` for GC references; safepoints at
  call sites via Cranelift stack maps.

## Workflow

- Rust engine, Cargo workspace at the repo root; every new crate goes under
  `crates/` and into the workspace members list.
- Tests accompany each crate (`cargo test` at workspace level must stay green).
- Round-trip tests for the format (emit → read → disassemble) are mandatory for any
  metadata/IL change.
- Standard library lives in `std/` as typed TypeScript compiled by the project's own
  compiler; Rust-side intrinsics live in `zun_core` with the frozen layouts
  (Object/String/Array/Function).
