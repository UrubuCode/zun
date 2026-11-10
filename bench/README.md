# Zun execution micro-benchmarks

Representative workloads for timing the VM at runtime, used to find where
execution time actually goes (as opposed to static code hunting).

## Running

```sh
cargo build -p zun_cli --release
powershell -File bench/run.ps1
```

`run.ps1` separates **compile** time (`zun build` → `.zm`) from **execution**
time (`zun run <file>.zm`, best of 3), so the interpreter/JIT cost is isolated
from the front-end.

## Workloads

| file | stresses |
|------|----------|
| `compute.ts` | tight numeric loop at **module top level** |
| `compute_fn.ts` | the *same* loop wrapped in a function |
| `methods.ts` | typed class method dispatch + field access + allocation |
| `dynprops.ts` | dynamic `any` object property read/write |
| `instanceofb.ts` | dynamic `instanceof` in a loop |
| `gcchurn.ts` | allocation churn → GC marking / minor collections |
| `errors.ts` | `throw` / `catch` in a loop |

## Headline finding (2026-11-10)

The JIT compiles **functions/methods**, not the module top-level entry, and
eligibility is per-method all-or-nothing. The module body almost always
contains a dynamic op (`console.log`, a module `let` binding mirrored to the
realm store, etc.), so the **entire top-level — including any hot loop in it —
runs in the Tier-0 interpreter.**

Measured: the identical numeric loop runs

- `compute.ts` (top level): **~39,300 ms**
- `compute_fn.ts` (in a function): **~132 ms**

i.e. **~297× faster** once it is inside a function. Practical guidance: put hot
work in functions. The real fix (JIT the module entry / OSR hot loops out of
any frame, as .NET does for `Main`) is a large deferred foundation, not a
front-end heuristic — a heuristic that drops the realm-store mirror for
"unread" top-level bindings was prototyped and reverted because it silently
breaks self-reassigning top-level functions, nested function expressions,
exports and `eval` (the soundness analysis is whole-program, not local).

Per-iteration cost of the other (top-level, interpreted) workloads for context:
`methods` ~33 µs, `gcchurn` ~9 µs/alloc, `dynprops` ~5 µs, `errors` ~4.6 µs,
`instanceof` ~3.9 µs.
