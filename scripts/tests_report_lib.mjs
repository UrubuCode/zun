// Shared helpers for the cross-runtime test tooling: load the JSON report and
// roll it up per category. Used by tests_readme.mjs and tests_issues.mjs.

import { readFileSync } from "node:fs";

/** Statuses that are excluded from the conformance denominator. */
export const SKIPPED = new Set([
  "bun_node_diverge",
  "upstream_defect",
  "rejected",
]);

/** Statuses that count against Zun (a reachable fixture that is not passing). */
export const FAILING = new Set(["zun_diverge", "zun_error"]);

export function loadReport(path) {
  const raw = JSON.parse(readFileSync(path, "utf8"));
  const results = raw.results ?? [];
  return { results, summary: raw.summary ?? rollupSummary(results) };
}

function emptyBucket() {
  return {
    total: 0,
    pass: 0,
    zun_diverge: 0,
    zun_error: 0,
    bun_node_diverge: 0,
    upstream_defect: 0,
    rejected: 0,
  };
}

/** Group results by category, each with per-status counts and a rate. */
export function byCategory(results) {
  const map = new Map();
  for (const r of results) {
    const cat = r.category || "root";
    if (!map.has(cat)) map.set(cat, emptyBucket());
    const b = map.get(cat);
    b.total += 1;
    if (b[r.status] !== undefined) b[r.status] += 1;
  }
  const out = [];
  for (const [category, b] of [...map.entries()].sort()) {
    const skipped = b.bun_node_diverge + b.upstream_defect + b.rejected;
    const reachable = b.total - skipped;
    const rate = reachable > 0 ? (b.pass * 100) / reachable : null;
    out.push({ category, ...b, skipped, reachable, rate });
  }
  return out;
}

export function rollupSummary(results) {
  const b = emptyBucket();
  for (const r of results) {
    b.total += 1;
    if (b[r.status] !== undefined) b[r.status] += 1;
  }
  const skipped = b.bun_node_diverge + b.upstream_defect + b.rejected;
  const reachable = b.total - skipped;
  const rate = reachable > 0 ? (b.pass * 100) / reachable : null;
  return { ...b, reachable, share: rate === null ? "n/a" : rate.toFixed(1) };
}

export function fmtRate(rate) {
  return rate === null ? "n/a" : `${rate.toFixed(1)}%`;
}

/** A 20-cell text progress bar, e.g. `[██████████░░░░░░░░░░]`. */
export function progressBar(rate, width = 20) {
  if (rate === null) return "n/a";
  const filled = Math.round((rate / 100) * width);
  return "[" + "█".repeat(filled) + "░".repeat(width - filled) + "]";
}
