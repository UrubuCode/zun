---
name: zun-conformance
description: Workflow for the cross-runtime conformance suite under `tests/` (fixtures compared against Bun and Node) - how a fixture passes, how to find the exact failing operation, how to fix it at the right layer without breaking the concept, and how to check for regressions. Use this whenever a task mentions conformance, the pass rate or percentage, a failing fixture in `tests/`, `scripts/tests_check.sh`, "make test X pass", or raising compatibility with JS behaviour.
---

# Conformance fixtures (`tests/`)

`tests/<category>/*.ts` are comprehensive probe programs. The harness
`scripts/tests_check.sh` runs each through Bun, Node and Zun and compares stdout
byte for byte (with `NO_COLOR=1`, `FORCE_COLOR=0`, `TZ=UTC` so the measurement is
about the engine, not the terminal or the timezone). It needs `bun`, `node` and a
built `zun` (`ZUN_BIN` overrides auto-detection) and writes a JSON report.

The tracked pass count uses a weaker local bar: **a fixture passes when
`zun run <file>` exits 0**. The fixtures wrap edge cases in `try/catch` helpers,
so a fixture fails only on a compile error or an uncaught throw. Keep both bars in
mind: exit 0 moves the counter, but matching Bun/Node output is the real goal, so
never make a fixture exit 0 by producing output you know is wrong.

## Locate the failure

```sh
target/release/zun.exe run tests/<category>/<file>.ts 2>&1 | cat -n | tail -20
```

- **Compile error**: the diagnostic names the construct. Go to sema or the OXC
  adapter.
- **Runtime throw**: the failing operation is the one right after the last line
  printed. Copy that statement and its setup into a small probe in the scratchpad
  and shrink it until it is a few lines.

Run the probe under Node and Bun to learn the expected behaviour. Fix against the
probe, then re-run the fixture; fixtures usually have several independent
failures in sequence, so repeat.

## Fix at the right layer

| Symptom | Usual cause | Fix |
|---|---|---|
| `SEM…` rejecting valid dynamic JS | Check stricter than the language | Accept `any`/dynamic and defer to the native's run-time coercion or throw |
| "unknown variable" for a standard global | Global not registered | Register it (see `zun-add-builtin`) |
| "has no method" on a primitive | Missing dispatch-table entry | Add the entry, not a class |
| Arity mismatch with optional arguments | Exact-arity table | Default the argument in the checker |
| Uncaught throw in a builtin | Missing case in the implementation | Implement it with the canonical coercions |
| OXC adapter "unsupported" | AST kind not converted | Extend `crates/zun_syntax/src/oxc/` |

Guardrails that keep conformance work from eroding the design:

- Relaxing a check for **dynamic** values is fine. Widening a **typed** slot to
  `Any`, or routing typed code through the dynamic path, to make a fixture pass is
  not; that trades the concept for a counter.
- Fix the mechanism, not the fixture: no special-casing on a test's names or
  values.
- A fix that needs a subsystem (iterator protocol, function-as-constructor,
  prototype introspection, regex features, closure conversion) is a multi-step
  project. Say so and scope it instead of landing a partial hack that makes one
  fixture exit 0.
- Local heuristics for whole-program properties are a known trap here (see the
  reverted top-level realm-mirror heuristic in `bench/README.md`).

## Regression check

A change in sema or the interpreter can silently change output elsewhere while
every fixture still exits 0. Before reporting a flip:

1. Rebuild the CLI (`cargo build -p zun_cli --release`); a stale binary gives
   false results.
2. Capture stdout for all fixtures before and after the change and diff them.
   Normalize nondeterministic text first (pointer addresses such as `Obj(0x…)`,
   thread ids in panic messages), otherwise the diff is noise.
3. Run the unit tests of the crates you touched, per crate.

Report the count as `before -> after` out of the total, list which fixtures
flipped, and list any fixture whose output changed without flipping.

## Working at scale

When several clusters of failures are attacked in one session, a head session
reviews and commits while subagents implement one cluster each, **sequentially**.
Concurrent heavy cargo builds have corrupted the git index before. See
`zun-build-and-test`.
