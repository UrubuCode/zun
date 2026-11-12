---
name: zun-add-opcode
description: End-to-end recipe for adding or changing a ZIL opcode in Zun - master table row, encode/decode round trip, verifier fixtures, ilgen emission, Tier 0 interpreter arm, Tier 1 Cranelift lowering, eligibility and the equivalence test. Use this whenever a task adds an instruction, changes an opcode's operands or stack effect, introduces a new numeric/typed opcode family, or touches `crates/zun_il/src/opcode.rs`, even if the request only says "lower X natively" or "add support for X in the IL".
---

# Adding or changing a ZIL opcode

An opcode is part of the on-disk format and is implemented by two tiers that must
agree bit for bit. The work is small per step, but skipping a step leaves the
format, the verifier or one tier out of sync, and that kind of bug surfaces far
from its cause. Do the steps in order; each one has a cheap check.

Before starting, ask whether a new opcode is the right tool. Prefer an opcode when
the operation is typed and hot (the JIT can lower it to a few instructions). Prefer
a `zun:core` native (see the `zun-add-builtin` skill) when it is a library call
with a slow path, because natives do not grow the format.

## 1. Master table row (`crates/zun_il/src/opcode.rs`)

Add one row inside `define_opcodes!`:

```rust
/// One-line meaning, including what is popped and pushed.
Name = 0xNN, OperandKind, "mnemonic", StackEffect::Fixed { pops: P, pushes: Q };
```

- Pick an unused byte in the family's range. **Never renumber or reuse a byte**:
  the discriminant is the format.
- Reference types and members with a `Token` operand, not an index or offset.
  A raw `Imm32` that indexes a Rust table is the mistake `call.builtin` already
  made; do not repeat it.
- The row is the only place the mnemonic, operand kind and stack effect are
  written. The assembler, disassembler and verifier read it. If you find yourself
  typing the arity anywhere else in `src/`, stop: read it from the table.

## 2. Format round trip

Add or extend a test that encodes the instruction, decodes it and disassembles it
(`crates/zun_il/tests/`). Byte values are pinned literally there on purpose, as a
test oracle. Run `cargo test -p zun_il`.

## 3. Verifier (`crates/zun_verify`)

- Add the opcode to `SHAPES` in `tests/opcode_coverage.rs`. That table restates
  each arity independently of `stack_effect()` **by design**: a fixture generated
  from the same table it checks proves nothing. The meta-test fails until every
  opcode in `Opcode::ALL` has an entry.
- If the opcode has rules beyond arity (branch target, region placement, operand
  range), add hand-written `<name>_valid` and `<name>_invalid_<error>` fixtures in
  `src/tests.rs`, and a new `ErrorKind` if no existing one describes the failure.
- Run `cargo test -p zun_verify`.

## 4. Emission (`crates/zun_ilgen`)

Emit the opcode from the HIR node that needs it. Choose the opcode from the static
type that sema assigned; do not emit a dynamic opcode when the type is known.
If the opcode needs a new HIR node or type rule, that goes in `zun_sema` first.

## 5. Tier 0 (`crates/zun_interp`)

Add the arm to the dispatch match. Keep the arm thin: put the actual semantic in a
function that the JIT host thunk can also call, in `zun_core` when it is pure (no
heap) so both tiers share one implementation. This is the single most effective
way to keep the tiers from drifting.

## 6. Tier 1 (`crates/zun_codegen`)

Follow the `zun-jit-lowering` skill. In short: lower inline in
`src/translate/` when it is a few Cranelift instructions, otherwise add a host
thunk in `src/thunks/` that **calls the same shared helper as Tier 0**, register
its symbol, and add the opcode to eligibility in `src/eligible.rs`.

If you intentionally leave the opcode Tier-0-only, leave it out of
`opcode_supported` so methods using it stay interpreted, and say so in the row's
doc comment. Never mark it supported without a lowering.

## 7. Equivalence test

Add an `assert_equivalent(src)` case to the matching
`crates/zun_codegen/tests/equivalence_*.rs` file. It asserts the entry is
JIT-eligible **and** that JIT and interpreter return the same value. Use locals in
the source so the operation is not constant-folded away, and include the edge
values (NaN, -0, overflow, empty string, out-of-range index) because that is where
tiers diverge.

Run only that test binary: `cargo test -p zun_codegen --test equivalence_<file>`.

## Done means

- `cargo test -p zun_il`, `-p zun_verify`, and the one equivalence binary are green.
- The disassembler prints the new mnemonic for a compiled sample
  (`zun disasm <file>`).
- No second copy of the opcode's arity or operand decoding exists outside the
  table and the deliberate `SHAPES` oracle.
- Build limits respected: see the `zun-build-and-test` skill (never
  `cargo test --workspace` on this machine).
