---
name: zun-rust-style
description: Rust code organization and hygiene rules for the Zun workspace - crate and module layout, thin `lib.rs` facades, when and how to split a large file, visibility, workspace dependency and lint inheritance, test placement, and the canonical shared helpers to reuse instead of duplicating logic. Use this when creating a crate, module or file, when a file or `impl` block is getting large, when refactoring or deduplicating, when adding a dependency to any `Cargo.toml`, when writing a helper that might already exist, and when reviewing a change for structure.
---

# Rust organization in Zun

The workspace already follows the layout recommended for large Rust projects: a
flat `crates/` directory, internal crates and package fields inherited from the
workspace, shared lints, uniform module style and named re-exports. These rules
keep new code consistent with that, and name the places where the repo still
falls short so new work improves them.

## Workspace and Cargo

- New crates go under `crates/zun_<name>/`, are added to `members` and to
  `[workspace.dependencies]` in the root `Cargo.toml`, and are consumed with
  `zun_<name> = { workspace = true }`.
- Every crate inherits `version`, `edition`, `license`, `repository` with
  `<field>.workspace = true` and opts into lints with `[lints] workspace = true`.
  Lint inheritance is not implicit; a crate without that line is unlinted.
- **External dependencies are declared once**, in `[workspace.dependencies]`, and
  referenced with `workspace = true`. Several are still pinned per crate (oxc,
  `zstd`, `libloading`, `icu`, `jiff`, `getrandom`, `memchr`); when you touch one
  of those manifests, move the version to the workspace table.
- Be conservative with new dependencies. Each one is build time and link size on a
  machine that is already constrained. Put small shared utilities in `zun_common`
  rather than pulling a micro-crate.
- Respect the layering in the `zun-architecture` skill before adding an edge.

## Modules

- The repo uses the `foo/mod.rs` style everywhere. Stay consistent: a module that
  grows children becomes `foo/mod.rs`, not `foo.rs` + `foo/`. Do not mass-rename;
  uniformity is the rule the community agrees on, the choice of style is taste.
- **`lib.rs` is a facade**: `mod` declarations, deliberate `pub use` re-exports,
  crate docs. Logic lives in named modules. Several crate roots currently carry
  logic (`zun_codegen`, `zun_interp`, `zun_compiler`, `zun_natives`, `zun_gc`);
  add new code to a module, not to those roots.
- Re-export by name (`pub use runner::{Runner, RunOptions};`). No glob re-exports
  of internal modules.
- Modules are private by default (`mod x;`), items are `pub(crate)` unless another
  crate needs them. A `pub` item is an API commitment across crates.
- Fields are public only when every value is valid. If a type has an invariant,
  keep fields private behind a constructor and getters.

## File and function size

There is no line limit in Rust, but a file is too big when it holds more than one
responsibility. Practical triggers here: a file past roughly 1,000 lines, an
`impl` block past roughly 500, or a function you cannot see on two screens.

Split along a seam that already exists in the code:

- by **phase** (`compile.rs` / `osr.rs` / `object.rs` for the JIT driver),
- by **operation family** (define / get-set / array specializations for property
  ops),
- by **receiver type** (one file per builtin type, as `collections/` does).

A split is a pure move: same items, same behaviour, no logic edits in the same
commit, so the diff can be reviewed as a move. Record any defect noticed along the
way in `docs/refactor-findings.md` instead of fixing it inside the move.

Multiple `impl Type` blocks across files of the same crate are the normal way to
split a large type in this codebase.

## Reuse before writing

Before writing a helper, search for it. These canonical helpers exist and have
each been re-implemented by accident:

| Need | Canonical helper |
|---|---|
| ToNumber on a dynamic value | `Interp::dyn_to_number` |
| ToString on a dynamic value | `Interp::dyn_to_string` |
| Unbox an `Any` carrier | `unbox_arg` in `zun_interp/src/collections/mod.rs` |
| `Value` to `zun_gc::Obj` | `collections::object` |
| Class subtype test in sema | `is_subclass` in `zun_sema/src/class_layout.rs` |
| String find / pad / repeat / substring on UTF-16 units | `zun_core::string_ops` |
| Replacement-pattern expansion | `zun_core::expand_substitution` |
| Exception type caught by a handler | one base-chain walk, shared by interp bridge and JIT |
| Opcode name / arity / operand | `Opcode::{mnemonic, stack_effect, operand}` |

Several of these are `pub(super)` today. When a caller outside the module needs
one, widen it to `pub(crate)` and import it; do not copy the body because the
visibility was in the way.

Where to put a new shared helper:

- pure value semantics used by both tiers: `zun_core`;
- needs the interpreter or heap state: a method on `Interp`, in the module that
  owns the concept;
- sema checks repeated across call sites: a method on `Checker`;
- generic utilities: `zun_common`.

What is **not** duplication, and should be left alone:

- width- or kind-specialized families (`read_u8`/`read_u16`/`read_u32`,
  per-numeric-kind opcodes and fast paths, per-arity call paths) written for speed
  or compatibility;
- macro-generated repetition;
- the same opcode implemented once per tier, as long as both call the same
  semantic helper;
- a deliberate test oracle that restates data (verifier `SHAPES`, pinned opcode
  bytes).

When two copies of a semantic exist and disagree, that is a bug, not a cleanup:
find out which is right against Node/Bun, keep one, and add a test for the case
that differed.

## Unsafe

- `unsafe_op_in_unsafe_fn` is on: every unsafe operation gets its own `unsafe {}`
  with a `// SAFETY:` comment saying why the precondition holds.
- Wrap a repeated unsafe cast in one safe helper instead of repeating the
  `unsafe { … }` expression at each call site; that shrinks the audit surface.
- `#[allow(dead_code)]` on staged code needs a comment naming the design doc and
  what will consume it. Unreferenced code with no such plan is deleted.

## Tests

- Unit tests live in the file they test, in `#[cfg(test)] mod tests`.
- Integration tests that go through a crate's public API live in
  `crates/<crate>/tests/`, with shared setup in `tests/common/mod.rs`.
- Do not grow a multi-thousand-line test module inside `src/`; add a file per
  feature under `tests/`.
- Whole-program JS/TS behaviour belongs in the root `tests/` fixtures.

## Language

All code, identifiers, comments, commit messages and documentation are in
English. `PLAN.md` and `.docs-dotnet/` are existing Portuguese reference material
and are extended in Portuguese only.
