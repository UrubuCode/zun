---
name: zun-architecture
description: The Zun concept and its architectural invariants (CLR-style compile step, ZIL + metadata tokens, master opcode table, verify-then-execute, typed layouts vs explicit `any`), the crate map with "where does this code go", and the list of known drift. Use this before any design decision, before adding a crate or a cross-crate dependency, when a change touches more than one of compiler / format / runtime, when reviewing a change for concept fit, or whenever the question is "is this the right way to do it in Zun?".
---

# Zun architecture: concept, invariants, placement

Zun is a JS/TS **machine-language platform**, shaped after the .NET CLR, not after
V8. A real compile step produces a binary IL + metadata module (`.zm`); a VM loads,
verifies and executes it (Tier 0 interpreter, Tier 1 Cranelift JIT). Typed TS gets
fixed object layouts and vtable dispatch. Dynamic JS (`any`) is the explicit, costly
exception: an `Any` value with IL-level `box`/`unbox` and `DynamicObject`.

Every design choice is judged against one question: **does this keep typed code on
the fast, static path, and keep the dynamic path explicit?** A change that makes
something work by quietly routing typed code through the dynamic path is a
regression of the concept even when the tests pass.

Sources of truth, in order: `PLAN.md` (architecture, crate map §2, phases),
`NUCLEUS.md`, `GOAL.md`, `.docs-dotnet/` (how the CLR solved the same problem; check
the matching area doc before inventing a mechanism), `docs/design/*.md`.

## The invariants and what each one protects

1. **Compiler and VM share only the `.zm` format.** No in-memory AST or type table
   crosses. This is what makes the VM a real machine: anything the runtime needs
   must be expressible in metadata.
   The one sanctioned exception is `zun_vm`'s optional `eval` feature (links
   `zun_compiler` for `new Function`). Do not add a second one.
2. **IL references types and members by metadata token, never by offset.** Layout
   is derived at load time (`zun_types`), so a module survives layout changes.
3. **One master opcode table** (`define_opcodes!` in `crates/zun_il/src/opcode.rs`).
   Byte value, mnemonic, operand kind and stack effect live there and nowhere else.
4. **The verifier evolves with the IL.** A new opcode lands with `_Valid` /
   `_Invalid_<Error>` fixtures in the same change.
5. **Narrow boundary traits** between VM, GC and codegen (ICorJitInfo style).
6. **JIT entry points are redirectable** by atomic pointer patch, so tiering and
   OSR never need a redesign.
7. **The interpreter is the always-correct fallback**, and interp + JIT pass the
   same equivalence suite. A semantic exists once; both tiers call it.

## Where does this code go?

| You are adding | Crate | Notes |
|---|---|---|
| Opcode, operand kind, encoding | `zun_il` | Never renumber an existing byte |
| Metadata table / token kind | `zun_metadata` | Round-trip test mandatory |
| Module container, sections | `zun_module` | Round-trip test mandatory |
| Parsing, OXC AST to HIR | `zun_syntax` | OXC is the only TS/JS parser |
| Types, checking, layout assignment | `zun_sema` | |
| HIR to ZIL lowering | `zun_ilgen` | |
| Compile driver, trimming, prelude cache | `zun_compiler` | |
| Stack/type verification | `zun_verify` | |
| Load-time layout and vtables | `zun_types`, `zun_loader` | |
| Tier 0 execution | `zun_interp` | |
| Tier 1 lowering | `zun_codegen` | Must not depend on `zun_interp` |
| Tiering, hooks, bridge impl, FFI | `zun_vm` | Owns the JIT-to-interp bridge impl |
| Heap, roots, stack maps | `zun_gc` | |
| Frozen layouts + **shared pure semantics** | `zun_core` | The home for helpers both tiers use |
| `zun:core` native table | `zun_natives` | Append-only (ids are positions) |
| Extension ABI | `zun_abi` | Stable; extensions depend on this alone |
| Node.js modules | `zun-node/` submodule | Never in `zun_natives` or the core |
| Standard library in TS | `zun_prelude` (planned `std/`) | Compiled by the project's own compiler |

Dependency rule: the format crates (`zun_il`, `zun_metadata`, `zun_module`) are the
contract. Compile-time crates and runtime crates both depend on the format and
**never on each other**; only the driver (`zun_cli`) links both. `zun_natives` and
`zun_core` act as a shared *surface description* that both sides read; keep them
free of compiler or interpreter logic.

Before adding a dependency edge, read both `Cargo.toml` files and ask which side of
the format each crate lives on. A dev-dependency on the compiler from a runtime
crate is fine (tests compile source); a normal dependency is not.

## Deciding typed vs dynamic

- If the static type is known (class instance, numeric kind, string, typed array),
  lower to the typed opcode family and a slot/vtable access. Do not fall back to
  `lddyn`/`stdyn` or a `HashMap` lookup because it is easier.
- If the type is genuinely unknown, make the boundary visible: `box.any` /
  `unbox.any` at the edge, dynamic opcodes inside.
- Relaxing a sema check so a dynamic value is accepted and the native coerces or
  throws at run time is legitimate for JS conformance; widening a *typed* slot to
  `Any` to make a diagnostic go away is not.
- Width- or kind-specialized helper families (`read_u8`/`read_u16`/…, per-kind fast
  paths) are intentional. They are not duplication.

## Review checklist for any change

- Does it cross the compile/runtime line other than through `.zm`?
- Does it write opcode knowledge (name, arity, operand decoding) outside the table?
- Does it add a second implementation of a JS semantic that already exists? Search
  `zun_core` and the canonical helpers first (see the `zun-rust-style` skill).
- If it touches the JIT: is the Tier 0 behaviour identical, and is there an
  equivalence test proving it?
- If it adds a GC reference to JIT code or a native entry: is it rooted on every
  path, not only the common one?
- Does it change the format? Then emit, read back and disassemble in a test.

## Known drift

The code does not yet honour every invariant. Read [known-drift.md](known-drift.md)
before working in the verifier, the JIT entry/tiering code, `eligible.rs`, the
codegen/GC boundary or the Cranelift settings, so new work closes drift instead of
building on it. When you close an item, update that file in the same change.
