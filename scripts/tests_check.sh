#!/usr/bin/env bash
# Cross-runtime conformance harness for Zun.
#
# Runs every fixture under `tests/` through Bun + Node + Zun in parallel and
# compares their stdout byte-for-byte. Exits 0 when Zun matches the reference
# runtimes on every reachable fixture; 1 when Zun diverges or errors.
#
# This is the evolved form of the `cross_runtime_check` harness from the RTS
# prototype (urubucode/rts): same comparison discipline, rewritten in English,
# with a per-fixture `category` field in the report so a single tracking issue
# per category can show conformance as a percentage.
#
# Two levels of parallelism:
#   1. Per fixture: the three runtimes are launched together (background +
#      wait), so a fixture's latency is max(N,M) instead of N+M.
#   2. Across fixtures: `xargs -P JOBS` processes JOBS fixtures at a time.
#
# Environment:
#   ZUN_BIN          - path to the zun binary (auto-detected if unset)
#   REPORT_FILE      - output JSON report
#   FIXTURES_DIR     - fixtures root (default: tests)
#   JOBS             - cross-fixture parallelism (default: nproc/2, clamped 2..16)
#   FIXTURE_TIMEOUT  - seconds before a hung fixture is killed (default: 10)
#   ERROR_TEXT_MAX   - max bytes of a failing runtime's stderr kept in the report

set -uo pipefail

# Color OFF in all three runtimes. This is correctness, not cosmetics.
#
# The harness compares stdout byte-for-byte. Bun and Node colorize console.log
# output when they think a TTY wants it, and they disagree on the escape
# spelling: for the same `10` Bun writes `\e[33m10\e[0m` and Node writes
# `\e[33m10\e[39m`. Zun colors nothing. With color on, the harness would report
# `bun_node_diverge` ("the two reference runtimes disagree") for anything that
# prints a number, and the pass rate would collapse without the engine changing.
# NO_COLOR is honored by all three; FORCE_COLOR=0 cancels an inherited
# FORCE_COLOR. Exported here (not per-invocation) so no code path can forget it.
export NO_COLOR=1
export FORCE_COLOR=0

# All three runtimes run in the SAME timezone, and that timezone is UTC. Without
# this, a machine at -03:00 makes `new Date(2024, 0, 15, 10, 30)` diverge by
# three hours across any date fixture, and what gets measured is the measurer's
# timezone rather than the engine. CI runs in UTC, so this also aligns local
# measurement with CI.
export TZ=UTC

FIXTURES_DIR="${FIXTURES_DIR:-tests}"
REPORT_FILE="${REPORT_FILE:-/tmp/zun_tests_report.json}"

# Each fixture uses three processes (Bun + Node + Zun); aim for ~nproc/2 workers.
if [ -z "${JOBS:-}" ]; then
    if command -v nproc >/dev/null 2>&1; then
        cpus=$(nproc)
    else
        cpus=8
    fi
    JOBS=$(( cpus / 2 ))
    [ "$JOBS" -lt 2 ] && JOBS=2
    [ "$JOBS" -gt 16 ] && JOBS=16
fi

# Auto-detect the zun binary.
if [ -z "${ZUN_BIN:-}" ]; then
    if [ -x "target/release/zun.exe" ]; then
        ZUN_BIN="target/release/zun.exe"
    elif [ -x "target/release/zun" ]; then
        ZUN_BIN="target/release/zun"
    elif command -v zun >/dev/null 2>&1; then
        ZUN_BIN="zun"
    else
        echo "error: no zun binary found (try: cargo run -p xtask -- package)" >&2
        exit 2
    fi
fi

if ! command -v bun  >/dev/null 2>&1; then echo "error: bun not installed"  >&2; exit 2; fi
if ! command -v node >/dev/null 2>&1; then echo "error: node not installed" >&2; exit 2; fi

# JSON writer: jq > python3 > python > node. `command -v` is not enough on
# Windows: the Microsoft Store `python` alias EXISTS but only opens the store
# (non-zero exit), which would emit empty entries. Test real execution.
if command -v jq >/dev/null 2>&1; then
    JSON_TOOL=jq
elif python3 -c 1 >/dev/null 2>&1; then
    JSON_TOOL=python3
elif python -c 1 >/dev/null 2>&1; then
    JSON_TOOL=python
else
    JSON_TOOL=node
fi
export JSON_TOOL

if [ -t 1 ]; then
    RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; BOLD='\033[1m'; NC='\033[0m'
else
    RED=''; GREEN=''; YELLOW=''; BOLD=''; NC=''
fi
export RED GREEN YELLOW NC

# Patterns a portable cross-runtime fixture must NOT contain: imports from the
# "zun" module, or globals that only one runtime provides. Such fixtures cannot
# produce a comparable result across all three and are rejected early.
export ZUN_ONLY_PATTERNS='(import[[:space:]]+\{[^}]*\}[[:space:]]+from[[:space:]]+["'"'"'"]zun["'"'"'"]|(^|[^A-Za-z_])(Bun|Deno)([^A-Za-z_]|$))'
export ZUN_BIN FIXTURES_DIR

TMP=$(mktemp -d -t zuntests-XXXXXX)
trap 'rm -rf "$TMP"' EXIT
export TMP

# Worker — processes one fixture.
# Argument: fixture path.
# Output: "TAG|NAME" on stdout (consumed by the progress loop) plus
# $TMP/<safe>.json (the full JSON entry) and $TMP/<safe>.status.
process_fixture() {
    local fixture="$1"
    local name; name=$(basename "$fixture" .ts)
    # Category = first path component under FIXTURES_DIR; "root" if flat.
    local rel="${fixture#"$FIXTURES_DIR"/}"
    local category="${rel%%/*}"
    [ "$category" = "$rel" ] && category="root"
    # A filesystem-safe key unique per fixture (category + name avoids clashes
    # when two categories share a fixture name).
    local key; key=$(printf '%s__%s' "$category" "$name" | tr -c 'A-Za-z0-9_.-' '_')
    local out_json="$TMP/${key}.json"
    local out_status="$TMP/${key}.status"

    # Reject early (ignore line comments so a mention in a comment is harmless).
    local stripped; stripped=$(sed 's|//.*$||' "$fixture")
    if printf '%s\n' "$stripped" | grep -qE "$ZUN_ONLY_PATTERNS"; then
        echo "rejected" > "$out_status"
        echo "!|$name"
        return
    fi

    local TO="${FIXTURE_TIMEOUT:-10}"
    timeout "$TO" bun       "$fixture" >"$TMP/${key}.bun"  2>"$TMP/${key}.bun.err"  &
    local bun_pid=$!
    timeout "$TO" node      "$fixture" >"$TMP/${key}.node" 2>"$TMP/${key}.node.err" &
    local node_pid=$!
    timeout "$TO" "$ZUN_BIN" run "$fixture" >"$TMP/${key}.zun" 2>"$TMP/${key}.zun.err" &
    local zun_pid=$!

    wait $bun_pid;  local bun_rc=$?
    wait $node_pid; local node_rc=$?
    wait $zun_pid;  local zun_rc=$?

    local bun_out;  bun_out=$(cat "$TMP/${key}.bun")
    local node_out; node_out=$(cat "$TMP/${key}.node")
    local zun_out;  zun_out=$(cat "$TMP/${key}.zun")

    # stderr is kept only for a runtime that failed (a successful run's stderr is
    # noise, e.g. deprecation warnings). Truncated so the report stays small;
    # `timeout` kills with 124 and writes nothing, so that case is labeled.
    local ERRMAX="${ERROR_TEXT_MAX:-4000}"
    local bun_err="" node_err="" zun_err=""
    [ $bun_rc  -ne 0 ] && bun_err=$(head -c "$ERRMAX" "$TMP/${key}.bun.err")
    [ $node_rc -ne 0 ] && node_err=$(head -c "$ERRMAX" "$TMP/${key}.node.err")
    [ $zun_rc  -ne 0 ] && zun_err=$(head -c "$ERRMAX" "$TMP/${key}.zun.err")
    [ $bun_rc  -eq 124 ] && bun_err="timeout after ${TO}s${bun_err:+$'\n'}$bun_err"
    [ $node_rc -eq 124 ] && node_err="timeout after ${TO}s${node_err:+$'\n'}$node_err"
    [ $zun_rc  -eq 124 ] && zun_err="timeout after ${TO}s${zun_err:+$'\n'}$zun_err"

    rm -f "$TMP/${key}.bun" "$TMP/${key}.node" "$TMP/${key}.zun" \
          "$TMP/${key}.bun.err" "$TMP/${key}.node.err" "$TMP/${key}.zun.err"

    # Classify by exit code first; stdout only compares when both sides succeeded.
    local status
    if [ $bun_rc -eq 0 ] && [ $node_rc -ne 0 ]; then
        # Node alone cannot run the file — almost always TypeScript it cannot
        # strip (e.g. `constructor(public name: string)`). That is a limitation
        # of Node, not disagreement between runtimes; Bun is canonical here.
        if   [ $zun_rc -ne 0 ];               then status="zun_error"
        elif [ "$zun_out" != "$bun_out" ];    then status="zun_diverge"
        else                                       status="pass"
        fi
    elif [ $bun_rc -ne $node_rc ] || { [ $bun_rc -eq 0 ] && [ "$bun_out" != "$node_out" ]; }; then
        status="bun_node_diverge"
    elif [ $zun_rc -ne 0 ]; then
        status="zun_error"
    elif [ $bun_rc -ne 0 ]; then
        # Bun and Node failed identically (a fixture that errors on purpose) and
        # Zun exited 0 — a real divergence: Zun should have failed too.
        status="zun_diverge"
    elif [ "$zun_out" != "$bun_out" ]; then
        status="zun_diverge"
    else
        status="pass"
    fi

    # A third-party defect, declared in the fixture itself with the marker
    # `cross-runtime:upstream-defect`. Only counts while the fixture FAILS; if it
    # passes it is `pass` like any other, so the marker can never keep a broken
    # thing green. The bar is high: the cause must be proven OUTSIDE this repo.
    if [ "$status" != "pass" ] && grep -q "cross-runtime:upstream-defect" "$fixture" 2>/dev/null; then
        status="upstream_defect"
    fi
    echo "$status" > "$out_status"

    case "$JSON_TOOL" in
        jq)
            jq -n --arg name "$name" --arg category "$category" --arg status "$status" \
                  --arg bun "$bun_out" --arg node "$node_out" --arg zun "$zun_out" \
                  --arg bun_error "$bun_err" --arg node_error "$node_err" --arg zun_error "$zun_err" \
                  --argjson bun_exit "$bun_rc" --argjson node_exit "$node_rc" --argjson zun_exit "$zun_rc" \
                  '{name:$name,category:$category,status:$status,bun:$bun,node:$node,zun:$zun,
                    bun_exit:$bun_exit,node_exit:$node_exit,zun_exit:$zun_exit,
                    bun_error:$bun_error,node_error:$node_error,zun_error:$zun_error}' > "$out_json"
            ;;
        python|python3)
            NAME="$name" CATEGORY="$category" STATUS="$status" BUN="$bun_out" NODE="$node_out" ZUN="$zun_out" \
            BUN_ERR="$bun_err" NODE_ERR="$node_err" ZUN_ERR="$zun_err" \
            BUN_RC="$bun_rc" NODE_RC="$node_rc" ZUN_RC="$zun_rc" \
                "$JSON_TOOL" -c 'import json,os; e=os.environ; print(json.dumps({"name":e["NAME"],"category":e["CATEGORY"],"status":e["STATUS"],"bun":e["BUN"],"node":e["NODE"],"zun":e["ZUN"],"bun_exit":int(e["BUN_RC"]),"node_exit":int(e["NODE_RC"]),"zun_exit":int(e["ZUN_RC"]),"bun_error":e["BUN_ERR"],"node_error":e["NODE_ERR"],"zun_error":e["ZUN_ERR"]}))' > "$out_json"
            ;;
        node)
            NAME="$name" CATEGORY="$category" STATUS="$status" BUN="$bun_out" NODE="$node_out" ZUN="$zun_out" \
            BUN_ERR="$bun_err" NODE_ERR="$node_err" ZUN_ERR="$zun_err" \
            BUN_RC="$bun_rc" NODE_RC="$node_rc" ZUN_RC="$zun_rc" \
                node -e 'const e=process.env;console.log(JSON.stringify({name:e.NAME,category:e.CATEGORY,status:e.STATUS,bun:e.BUN,node:e.NODE,zun:e.ZUN,bun_exit:+e.BUN_RC,node_exit:+e.NODE_RC,zun_exit:+e.ZUN_RC,bun_error:e.BUN_ERR,node_error:e.NODE_ERR,zun_error:e.ZUN_ERR}))' > "$out_json"
            ;;
    esac

    case "$status" in
        pass)             echo "ok|$name" ;;
        zun_diverge)      echo "xx|$name (Zun differs from Bun/Node)" ;;
        bun_node_diverge) echo "~|$name (Bun != Node - skip)" ;;
        upstream_defect)  echo "~|$name (upstream dependency defect - skip)" ;;
        zun_error)
            local first; first=$(printf '%s' "$zun_err" | grep -m1 -v '^[[:space:]]*$' | tr '|' '/' | head -c 160)
            echo "xx|$name (Zun error rc=$zun_rc${first:+: $first})"
            ;;
    esac
}
export -f process_fixture

# Collect fixtures. Accepts a flat layout (`tests/*.ts`) or per-category
# subfolders (`tests/<category>/*.ts`). `support/` holds shared helpers that are
# not runnable in isolation and is skipped.
shopt -s nullglob
mapfile -t FIXTURES < <(
    find "$FIXTURES_DIR" -type f -name '*.ts' \
        ! -path "$FIXTURES_DIR/support/*" \
        ! -path "*/support/*" \
        | sort
)
TOTAL=${#FIXTURES[@]}

if [ "$TOTAL" -eq 0 ]; then
    echo "No fixtures under $FIXTURES_DIR" >&2
    exit 0
fi

echo -e "${BOLD}Running $TOTAL fixtures with parallelism=$JOBS...${NC}"
start_time=$(date +%s)

printf '%s\n' "${FIXTURES[@]}" | xargs -I{} -P "$JOBS" bash -c 'process_fixture "$@"' _ {} | while IFS='|' read -r tag rest; do
    case "$tag" in
        ok)  echo -e "${GREEN}ok${NC} $rest" ;;
        xx)  echo -e "${RED}xx${NC} $rest" ;;
        '~') echo -e "${YELLOW}~${NC} $rest" ;;
        '!') echo -e "${YELLOW}!${NC} $rest" ;;
    esac
done

elapsed=$(($(date +%s) - start_time))

pass=0; diverge_zun=0; diverge_bun_node=0; errors=0; rejected=0; upstream=0
for fixture in "${FIXTURES[@]}"; do
    name=$(basename "$fixture" .ts)
    rel="${fixture#"$FIXTURES_DIR"/}"; category="${rel%%/*}"; [ "$category" = "$rel" ] && category="root"
    key=$(printf '%s__%s' "$category" "$name" | tr -c 'A-Za-z0-9_.-' '_')
    s_file="$TMP/${key}.status"
    [ -f "$s_file" ] || continue
    s=$(cat "$s_file")
    case "$s" in
        pass)             pass=$((pass + 1)) ;;
        zun_diverge)      diverge_zun=$((diverge_zun + 1)) ;;
        bun_node_diverge) diverge_bun_node=$((diverge_bun_node + 1)) ;;
        upstream_defect)  upstream=$((upstream + 1)) ;;
        zun_error)        errors=$((errors + 1)) ;;
        rejected)         rejected=$((rejected + 1)) ;;
    esac
done

# The denominator is what is REACHABLE, not the raw total. A fixture where Bun
# and Node answer differently has no correct answer this harness can name, so it
# is excluded from the fraction (still counted and listed). The fraction asks:
# "of everything with a comparable answer, how much did Zun get right?"
reachable=$((TOTAL - diverge_bun_node - upstream))
if [ "$reachable" -gt 0 ]; then
    share=$(awk -v p="$pass" -v r="$reachable" 'BEGIN { printf "%.1f", p * 100 / r }')
else
    share="n/a"
fi

{
    printf '{"results":[\n'
    first=1
    for fixture in "${FIXTURES[@]}"; do
        name=$(basename "$fixture" .ts)
        rel="${fixture#"$FIXTURES_DIR"/}"; category="${rel%%/*}"; [ "$category" = "$rel" ] && category="root"
        key=$(printf '%s__%s' "$category" "$name" | tr -c 'A-Za-z0-9_.-' '_')
        entry_file="$TMP/${key}.json"
        [ -f "$entry_file" ] || continue
        if [ $first -eq 1 ]; then first=0; else printf ',\n'; fi
        cat "$entry_file"
    done
    printf '\n],"summary":{"total":%d,"pass":%d,"reachable":%d,"zun_diverge":%d,"bun_node_diverge":%d,"errors":%d,"rejected":%d,"upstream_defect":%d,"share":"%s"}}\n' \
        "$TOTAL" "$pass" "$reachable" "$diverge_zun" "$diverge_bun_node" "$errors" "$rejected" "$upstream" "$share"
} > "$REPORT_FILE"

echo ""
echo -e "${BOLD}Summary:${NC} (time: ${elapsed}s, $JOBS parallel jobs)"
echo "  Total:                  $TOTAL"
echo -e "  ${GREEN}Pass:                   $pass${NC}"
echo -e "  ${BOLD}Reachable:              $reachable  (${share}% of $reachable)${NC}"
echo -e "  ${RED}Zun divergent:          $diverge_zun${NC}"
echo -e "  ${YELLOW}Bun/Node disagree:      $diverge_bun_node  (outside the denominator)${NC}"
echo -e "  ${YELLOW}Upstream defect:        $upstream  (outside the denominator)${NC}"
echo -e "  ${RED}Zun runtime error:      $errors${NC}"
echo -e "  ${YELLOW}Rejected (Zun-only):    $rejected${NC}"
echo ""
echo "Report: $REPORT_FILE"

if [ $diverge_zun -gt 0 ] || [ $errors -gt 0 ]; then
    exit 1
fi
exit 0
