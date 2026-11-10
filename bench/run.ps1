# Execution micro-benchmarks for the Zun VM.
#
# Separates COMPILE time (`zun build` -> .zm) from EXECUTION time (`zun run`
# on the prebuilt .zm), so the interpreter/JIT cost is isolated from the
# front-end. Run from the repo root after `cargo build -p zun_cli --release`:
#
#   powershell -File bench/run.ps1
#
# Each workload stresses one subsystem; iteration counts are baked into the
# .ts files. See bench/README.md for what each measures and the headline
# top-level-vs-function finding.

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$zun = Join-Path $root "target\release\zun.exe"
if (-not (Test-Path $zun)) {
    Write-Error "missing $zun - run: cargo build -p zun_cli --release"
}
$work = @("compute", "compute_fn", "methods", "dynprops", "instanceofb", "gcchurn", "errors")

"{0,-14} {1,12} {2,16}" -f "workload", "compile(ms)", "exec x3 min(ms)"
foreach ($w in $work) {
    $ts = Join-Path $PSScriptRoot "$w.ts"
    $zm = Join-Path $PSScriptRoot "$w.zm"
    $c = Measure-Command { & $zun build $ts | Out-Null }
    $best = [double]::PositiveInfinity
    for ($i = 0; $i -lt 3; $i++) {
        $e = (Measure-Command { & $zun run $zm | Out-Null }).TotalMilliseconds
        if ($e -lt $best) { $best = $e }
    }
    "{0,-14} {1,12:N1} {2,16:N1}" -f $w, $c.TotalMilliseconds, $best
}
