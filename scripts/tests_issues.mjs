// Maintain ONE tracking "super-issue" per category — never one issue per
// failing fixture. Each issue carries a progress bar, the conformance rate, and
// a checklist of every fixture in that category (checked = passing). It is
// found and updated by a hidden marker, kept open while the category is below
// 100%, and closed automatically once every reachable fixture passes (reopened
// on regression).
//
// This deliberately replaces the RTS prototype's behavior of opening a fresh
// issue per new divergence, which produced hundreds of short-lived issues.
//
// Needs: GITHUB_TOKEN and GITHUB_REPOSITORY (owner/name) in the environment —
// both provided by GitHub Actions. Usage: node scripts/tests_issues.mjs [report]

import {
  byCategory,
  fmtRate,
  loadReport,
  progressBar,
} from "./tests_report_lib.mjs";

const reportPath = process.argv[2] ?? ".github/tests_report.json";
const repo = process.env.GITHUB_REPOSITORY;
const token = process.env.GITHUB_TOKEN;
const LABEL = "conformance";
const TITLE_PREFIX = "Conformance:";

if (!repo || !token) {
  console.error("GITHUB_REPOSITORY and GITHUB_TOKEN are required");
  process.exit(2);
}

const api = `https://api.github.com/repos/${repo}`;
async function gh(path, init = {}) {
  const res = await fetch(path.startsWith("http") ? path : `${api}${path}`, {
    ...init,
    headers: {
      authorization: `Bearer ${token}`,
      accept: "application/vnd.github+json",
      "content-type": "application/json",
      "user-agent": "zun-tests-issues",
      ...(init.headers ?? {}),
    },
  });
  if (!res.ok) {
    throw new Error(`${init.method ?? "GET"} ${path} -> ${res.status} ${await res.text()}`);
  }
  return res.status === 204 ? null : res.json();
}

async function ensureLabel() {
  try {
    await gh(`/labels/${LABEL}`);
  } catch {
    await gh(`/labels`, {
      method: "POST",
      body: JSON.stringify({
        name: LABEL,
        color: "0e8a16",
        description: "Cross-runtime conformance tracking (one issue per category)",
      }),
    });
  }
}

/** All existing conformance issues, keyed by category marker. */
async function existingIssues() {
  const map = new Map();
  for (let page = 1; ; page++) {
    const batch = await gh(
      `/issues?state=all&labels=${LABEL}&per_page=100&page=${page}`,
    );
    if (!batch.length) break;
    for (const it of batch) {
      if (it.pull_request) continue;
      const m = (it.body ?? "").match(/<!-- zun-tests-category:(.+?) -->/);
      if (m) map.set(m[1], it);
    }
    if (batch.length < 100) break;
  }
  return map;
}

function renderBody(cat, results) {
  const marker = `<!-- zun-tests-category:${cat.category} -->`;
  const items = results
    .filter((r) => (r.category || "root") === cat.category)
    .sort((a, b) => a.name.localeCompare(b.name));

  const lines = [marker, ""];
  lines.push(`**${progressBar(cat.rate)} ${fmtRate(cat.rate)}**`);
  lines.push("");
  lines.push(
    `${cat.pass} passing · ${cat.zun_diverge} diverge · ${cat.zun_error} error · ` +
      `${cat.skipped} skipped · ${cat.total} total`,
  );
  lines.push("");
  lines.push("| | Fixture | Status |");
  lines.push("| --- | --- | --- |");
  const mark = {
    pass: "✅",
    zun_diverge: "❌",
    zun_error: "💥",
    bun_node_diverge: "➖",
    upstream_defect: "➖",
    rejected: "➖",
  };
  for (const r of items) {
    const box = r.status === "pass" ? "[x]" : "[ ]";
    lines.push(
      `| ${box} | \`${cat.category}/${r.name}.ts\` | ${mark[r.status] ?? ""} ${r.status} |`,
    );
  }
  lines.push("");
  lines.push(
    "<sub>Auto-generated per run from `scripts/tests_check.sh`. Checked = matches " +
      "Bun and Node. Edits to this body are overwritten; discuss in comments.</sub>",
  );
  return lines.join("\n");
}

const { results } = loadReport(reportPath);
const cats = byCategory(results);

await ensureLabel();
const existing = await existingIssues();

for (const cat of cats) {
  const title = `${TITLE_PREFIX} ${cat.category} (${fmtRate(cat.rate)})`;
  const body = renderBody(cat, results);
  const failing = cat.zun_diverge + cat.zun_error;
  const desiredState = failing > 0 ? "open" : "closed";
  const found = existing.get(cat.category);

  if (!found) {
    // Only open a new tracker when there is something to track.
    if (failing === 0) {
      console.log(`skip ${cat.category}: fully passing, no issue needed`);
      continue;
    }
    const created = await gh(`/issues`, {
      method: "POST",
      body: JSON.stringify({ title, body, labels: [LABEL] }),
    });
    console.log(`created #${created.number} for ${cat.category}`);
    continue;
  }

  const needsUpdate =
    found.title !== title ||
    (found.body ?? "") !== body ||
    found.state !== desiredState;
  if (!needsUpdate) {
    console.log(`unchanged #${found.number} (${cat.category})`);
    continue;
  }
  await gh(`/issues/${found.number}`, {
    method: "PATCH",
    body: JSON.stringify({ title, body, state: desiredState }),
  });
  console.log(`updated #${found.number} (${cat.category}) -> ${desiredState}`);
}
