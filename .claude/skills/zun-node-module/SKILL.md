---
name: zun-node-module
description: How to add or extend a Node.js compatibility module in the `zun-node` submodule - choosing native extension vs Zun TypeScript vs TS-over-natives, the `zun_abi` v1 conventions, registering descriptors, measuring against Node's own test suite, and updating the submodule pointer in the engine repo. Use this for any work under `zun-node/`, any request about `node:*` modules (fs, path, os, process, events, stream, buffer, net, crypto, zlib, vm, worker_threads, …), Node compatibility percentages, or bumping the zun-node submodule.
paths: "zun-node/**"
---

# Node.js modules live in zun-node, behind the ABI

`zun-node/` is a git submodule (`github.com/UrubuCode/zun-node`), excluded from
the Cargo workspace and built separately as a `cdylib`. The engine core holds only
base logic. **Never add a Node module to `crates/zun_natives`, `zun:core` or the
prelude.** Read `zun-node/PLAN.md` first; it defines the principles and the order
in which libraries are owed.

It is a separate repository: commits to module code happen inside `zun-node/` on
its own history. The engine repo only records which commit it points at.

## Pick the form by capability

| The module needs | Form | Example |
|---|---|---|
| Only computation | Zun TypeScript, compiled by the platform | `src/path/path.ts`, `querystring`, `url` |
| The operating system | Rust native through `zun_abi` | lower half of `fs`, `os` |
| Both | TypeScript over natives: Rust answers facts, TS builds Node's shape | `src/os/os.ts` over `node:os` natives |
| A capability already inside the engine | TypeScript importing from `zun:core` | `timers` via `import { timers } from "zun:core"` |

Draw the seam where the capability changes. Objects, arrays, constants, argument
validation and `ERR_*` errors are shape and belong in TypeScript; marshalling
shape across the ABI costs a convention and buys nothing. Syscalls cannot be
written in TypeScript and belong in Rust.

## Native side

- The extension depends on **`zun_abi` only**: no VM, interpreter or allocator
  dependency. If you need something from the engine, the ABI is missing a feature;
  raise that instead of adding a path dependency.
- No C toolchain: dependencies must be pure Rust (no `cc`, no vendored C). The
  manifest documents why each dependency was chosen; add the same kind of note.
- One folder per module (`src/<module>/mod.rs` plus focused submodules, with
  `tests.rs` beside them). A module exposes a `DESCRIPTORS` table and is
  registered in `zun_register_v1` in `src/lib.rs` under its `node:<name>` key.
- ABI v1 carrier rules:
  - a `Str` argument is a borrowed NUL-terminated C string valid for the call;
  - a `Str` result points into per-thread scratch the host copies out immediately;
  - multi-value string results join on `/`, never NUL (interior NUL is rejected);
  - failure is a non-zero status plus per-thread `(code, message)` read back
    through the module's last-error accessors, which the host turns into a
    catchable error.
- Keep platform code target-gated (`cfg(unix)` / `cfg(windows)`) so neither reaches
  the other OS.

## TypeScript side

- Reach natives with `import * as native from "node:<name>"`.
- Engine namespaces come from `zun:core` by **namespace import**
  (`import { timers as core } from "zun:core"`). Before using a namespace, confirm
  the engine exports it: it must be a group in `MODULES` in
  `crates/zun_natives/src/lib.rs`.
- When Node implements something by monkey-patching another module, use an
  explicit hook record declared by the patched module and written by exactly one
  module (see `__domainHooks`), since there is no prototype patching to lean on.
- Implement only the slice of a dependency module that the corpus actually needs,
  and never leave a module half-finished to start the next one.

## The bar: Node's own tests

A module is done when it passes 100% of the applicable Node 25 tests, not our own.
Each module has `test/<module>/NODE_TESTS.txt` (inventory), `RESULTS.md` and a
`README.md` recording what cannot pass and why.

- Run with the tools in `zun-node/tools/` (`run_node_tests.sh`,
  `bundle_node_test.sh`, harness in `tools/harness/`).
- Re-measure after every merge that touches a shared module; a change in `events`
  or `process` moves other modules' numbers.
- Update the percentage in the README from the measurement, never by hand.
- A test that cannot pass because the engine lacks a feature is recorded with the
  reason. Do not fake the behaviour in the module to turn it green.

Rust unit tests: `cargo test` inside `zun-node/` (it is its own build; the engine's
build limits still apply to the machine, so do not run it alongside an engine
build).

## Engine compatibility check

When a module starts depending on something new from the engine, verify it exists
before relying on it:

- ABI symbols used: `grep -rhoE 'zun_abi::[A-Za-z_0-9]+' zun-node/src | sort -u`
  against `crates/zun_abi/src/lib.rs`.
- `zun:core` namespaces imported against `zun_natives::MODULES`.
- Language features (`new Function`, generators, etc.) against a probe program.

A change to `zun_abi` is a contract change: bump or version it deliberately and
keep v1 extensions loading.

## Updating the submodule pointer in the engine repo

```sh
git -C zun-node fetch origin
git -C zun-node status --short          # must be clean before moving
git -C zun-node merge --ff-only origin/main
git diff --submodule=log -- zun-node     # review what came in
```

Then run the compatibility check above. The engine repo now shows `M zun-node`;
commit that pointer change on its own (`chore(zun-node): bump to <short-sha>`)
only when the user asks for a commit. Never fast-forward over local changes in the
submodule, and never rewrite its history from the engine checkout.
