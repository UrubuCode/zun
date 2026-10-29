# Zun conformance tests (cross-runtime)

Every fixture in this tree is a standalone TypeScript program that must produce
**identical stdout** on three runtimes: **Bun**, **Node**, and **Zun**. The
harness ([`scripts/tests_check.sh`](../scripts/tests_check.sh)) runs all three,
compares their output byte-for-byte, and classifies each fixture. This is how we
measure, as a percentage, how close Zun's observable behavior is to the de-facto
JavaScript/TypeScript standard.

This is the evolved form of the cross-runtime harness from the **RTS prototype**
([urubucode/rts](https://github.com/urubucode/rts)): same comparison discipline,
rewritten in English, with a `category` recorded per fixture so a single
tracking issue per category can report conformance instead of hundreds of
one-off issues.

## Layout

```
tests/
  <category>/*.ts     # one fixture per file, grouped by category
  support/            # shared helpers, never run in isolation (skipped)
```

The category is simply the first directory component. Add a fixture by dropping
a `.ts` file into the matching category folder (or a new one). No registry to
edit — the harness discovers fixtures by walking the tree.

## Writing a fixture

- **Print deterministic output** with `console.log`. No timestamps, no random
  values, no iteration order that depends on hashing you don't control.
- **Stay portable.** A fixture must be runnable by Bun and Node as-is. Imports
  from the `"zun"` module and runtime-only globals (`Bun`, `Deno`) are rejected
  — they can't produce a comparable result across all three.
- **Errors are fine.** A fixture that throws on purpose is valid: if Bun and
  Node fail identically and Zun does too, that's a `pass`.

## Status classification

| Status             | Meaning                                                             | Counts against Zun? |
| ------------------ | ------------------------------------------------------------------- | ------------------- |
| `pass`             | Zun's output matches the reference runtimes                         | —                   |
| `zun_diverge`      | Bun and Node agree; Zun's output differs                            | yes                 |
| `zun_error`        | Zun crashed or timed out while the references succeeded             | yes                 |
| `bun_node_diverge` | Bun and Node disagree — no canonical answer, excluded from the rate | no (excluded)       |
| `upstream_defect`  | Failure proven to be a third-party defect (marker in the fixture)   | no (excluded)       |
| `rejected`         | Fixture is not portable (Zun-only import or global)                 | no (excluded)       |

**Conformance rate** = `pass / reachable`, where `reachable = total −
bun_node_diverge − upstream_defect`. The denominator is what actually has a
correct answer to match, so the rate's ceiling is a real 100%.

### Canonical runtime

When Bun runs a fixture and Node cannot (usually TypeScript syntax Node cannot
erase, e.g. `constructor(public name: string)`), **Bun is canonical** and Zun is
compared against Bun. This keeps a Node limitation from hiding a real Zun result.

### Upstream defects

If a failure is proven to live outside this repository — bisected to a single
dependency, where changing only that dependency fixes it — mark the fixture with
a `cross-runtime:upstream-defect` comment. It is then excluded from the rate, but
**only while it fails**: once it passes it is a normal `pass`, so the marker can
never keep a broken thing green.

### Color and timezone

The harness forces `NO_COLOR=1`, `FORCE_COLOR=0`, and `TZ=UTC`. Colored
`console.log` output differs between Bun and Node escape spellings, and local
time would make date fixtures diverge by the measurer's offset — both would
corrupt the measurement rather than reveal an engine difference.

## Running locally

```bash
# Build the zun binary first (produces target/release/zun[.exe]):
cargo run -p xtask -- package

# Run the whole suite:
scripts/tests_check.sh

# Narrow it down / tune it:
FIXTURES_DIR=tests/strings scripts/tests_check.sh
JOBS=8 FIXTURE_TIMEOUT=20 scripts/tests_check.sh
ZUN_BIN=/path/to/zun scripts/tests_check.sh
```

The report is written to `$REPORT_FILE` (default `/tmp/zun_tests_report.json`).
CI consumes the same JSON to refresh the badge and table in the top-level
[`README.md`](../README.md) and to maintain the per-category tracking issues.
