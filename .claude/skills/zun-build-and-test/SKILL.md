---
name: zun-build-and-test
description: Machine-safe build, test and benchmark commands for the Zun workspace, and how to choose the cheapest check that actually proves a change. Use this before running ANY cargo command in this repo, when deciding how to verify a change, when a build is slow, freezes the machine or gets OOM-killed, when running the benchmarks in `bench/`, and before delegating build or test work to subagents.
---

# Building and testing Zun without freezing the machine

This workspace links large static test binaries. On the development machine,
linking many of them at once saturates disk I/O and freezes the UI, and heavy
concurrent cargo runs once corrupted the git index. The rules below are not
style; they are what keeps the session usable.

## Hard limits

- **Never run `cargo test --workspace`, `cargo build --workspace` or
  `cargo clippy --workspace` locally.** Always scope with `-p <crate>`.
- **One cargo invocation at a time.** Do not run two builds in parallel, and do
  not let subagents build concurrently: run implementing subagents sequentially.
- **Skip `cargo test -p zun_vm` as a whole**; it gets OOM-killed. Run a single
  test binary instead (`--test <file>`) or `--lib` with a name filter.
- Do not change `.cargo/config.toml` (`jobs = 4`, `RUST_MIN_STACK`); it is
  machine-local and git-ignored. Lower `jobs` only if the user asks.

Workspace-wide lint and test runs belong in CI, not on this machine.

## Choosing the check

Pick the narrowest command that exercises the code you changed:

| Changed | Run |
|---|---|
| Opcode table, encoding | `cargo test -p zun_il` |
| Metadata / module format | `cargo test -p zun_metadata` / `-p zun_module` |
| Verifier | `cargo test -p zun_verify` |
| Parser / OXC adapter | `cargo test -p zun_syntax` |
| Type checking | `cargo test -p zun_sema` |
| Lowering | `cargo test -p zun_ilgen` |
| Interpreter | `cargo test -p zun_interp --lib <name_filter>` |
| JIT lowering, eligibility | `cargo test -p zun_codegen --test equivalence_<area>` |
| GC | `cargo test -p zun_gc` |
| Builtin surface end to end | `cargo test -p zun_vm --test <file>` |
| JIT-to-interp bridge | `cargo test -p zun_vm --lib jit_bridge` |
| Whole-program behaviour | build the CLI once, then run fixtures (below) |

The `zun_codegen` equivalence binaries are the best loop for engine work: they
compile one crate and run JIT and interpreter in-process, with no executable
linking.

Use a name filter (`cargo test -p <crate> <substring>`) while iterating, then run
the crate's full relevant binary once before reporting.

Type-check without linking when you only need diagnostics:
`cargo check -p <crate>`.

## Running programs

```sh
cargo build -p zun_cli --release          # one-time, a few minutes
target/release/zun.exe run file.ts        # compile + run
target/release/zun.exe build file.ts      # produce file.zm
target/release/zun.exe run file.zm        # run a compiled module
target/release/zun.exe disasm file.ts     # print the ZIL
target/release/zun.exe run --ext <lib> file.ts   # with a native extension
```

Put probe programs in the session scratchpad, not in the repo.

Staleness gotcha: the CLI binary does not rebuild itself. After changing any
engine crate, rebuild `zun_cli` before trusting a fixture result. The same applies
to AOT and `cdylib` artifacts.

## Benchmarks (`bench/`)

```sh
cargo build -p zun_cli --release
powershell -File bench/run.ps1
```

`run.ps1` separates compile time from execution time (best of 3). Read
`bench/README.md` first: the module top level runs interpreted, so a hot loop
there measures Tier 0, not the JIT. Put the workload in a function when you intend
to measure native code, and compare against the committed numbers before claiming
a speedup. A performance claim needs a before and after from this harness.

## Reporting

State exactly which commands ran and their result. If a check was skipped because
of the limits above (for example the full `zun_vm` suite), say so rather than
implying full coverage. A known pre-existing failure must be named as such, not
silently ignored.
