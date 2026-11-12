---
name: zun-audit
description: Runs a structured health audit of the Zun codebase - concept and invariant compliance, duplicated or dead logic, interp/JIT divergence, and Rust organization - using read-only search subagents led by the main session, then verifies the load-bearing findings and updates the known-drift list. Use when the user asks whether the project is on the right track, asks for an audit, review or cleanup pass of the whole codebase or a crate, or asks to find duplicates or unnecessary logic.
argument-hint: "[crate or area, optional]"
---

# Zun codebase audit

The main session leads; subagents search. The lead never relays a finding it has
not checked, because a confident but wrong finding costs more than a missing one.

If an argument names a crate or area, restrict every track to it.

## 1. Orient (lead, a few commands)

- Line counts per crate, so effort goes to the large ones.
- `git status` and recent commits, so in-flight work is not reported as drift.
- Read `.claude/skills/zun-architecture/known-drift.md`: the audit reports what
  changed against it, not the same list again.

## 2. Fan out read-only subagents, in one message

Each brief states the scope, what counts as a finding, what does not, the method,
and the report format (ranked, `path:line` for every claim, a cap on the number of
findings, and an instruction not to pad). Subagents do not edit files and do not
run `cargo build`, `check`, `test` or `clippy` (see `zun-build-and-test`).

| Track | Scope |
|---|---|
| Invariants | Each invariant in `zun-architecture`, with a verdict HOLDS / PARTIAL / VIOLATED / NOT-YET-IMPLEMENTED and evidence from code, not from comments or docs |
| Duplication, compile side + interpreter | `zun_sema`, `zun_interp`, `zun_syntax`, `zun_ilgen` |
| Duplication, runtime side | `zun_codegen`, `zun_vm`, `zun_natives`, `zun_core`, `zun_gc` |
| Organization | Cargo inheritance, module style, crate-root size, visibility, test placement, CI, dependency graph, measured against current Rust guidance (web sources cited) |

Give every duplication brief the exclusions verbatim: width- or kind-specialized
helper families written for speed or compatibility, macro-generated repetition,
semantically distinct builtins that look alike, the same opcode implemented once
per tier, deliberate test oracles, and test-file similarity are **not** findings.
Require that both sites of every reported duplicate were opened and read.

Ask the runtime-side track specifically for (a) JIT thunks whose behaviour differs
from the Tier 0 native for the same operation, (b) GC rooting predicates that
differ between entry points, (c) opcode knowledge copied outside the master table.
Those are correctness findings and rank above any line-count finding.

## 3. Verify before reporting (lead)

For every finding that claims wrong behaviour, a violated invariant, or dead code:
open the cited lines yourself. Drop or downgrade what does not hold. Volume
estimates ("about N lines removable") may be relayed as the subagent's estimate.

## 4. Report

Answer the user's question first, in this order:

1. **Verdict on direction**: what of the concept is genuinely built, and where it
   is only aspirational, ranked by how much the premise depends on it.
2. **Correctness bugs**, each with the two sites and the observable difference.
3. **Duplication and dead code**, grouped by payoff, with the consolidation target
   (which copy survives, where the shared helper lives).
4. **Organization**: what follows convention, what deviates.
5. **Recommended order of work**, cheapest high-value item first.

Separate "confirmed by the lead" from "reported by a subagent, not re-checked".

## 5. Persist

Update `.claude/skills/zun-architecture/known-drift.md`: add new entries, mark
resolved ones FIXED with the commit, refresh the snapshot date. Do not fix
findings during the audit unless the user asks; an audit that also edits is hard
to review.
