#requires -Version 7
<#
.SYNOPSIS
  The level-1 scripted-route gate: the port's fixed-step replay must be deterministic and must match
  the committed route trace.

.DESCRIPTION
  Issue #8's oracle. `tools/parity/routes/level1-route.tsv` drives the player through the level's own
  retail-recorded route placements (spawn, cantina, the way up, the upper door) at the fixed 1/60 s
  capture step; the replay writes `tick t_s x y z sector vel_x vel_y vel_z health shields` plus the
  door/switch cog state every 600 ms (36 ticks) to a trace in the documented format.

  The gate checks four things:

    1. determinism — the same script replayed twice produces byte-identical traces (retail and port
       must produce the same trace from the same inputs, so the port's own repeatability is a
       precondition);
    2. the committed oracle — a replay matches `tools/parity/routetrace/level1-route-port.tsv`
       field for field;
    3. self-check — a copy of the trace with one position moved 0.25 units is **rejected** by the
       comparator, so the gate is known to fail on a perturbed trace and pass on the unmodified one.
       `-Perturb` runs that check; the gate's normal run compares the unmodified trace;
    4. retail parity — the replay matches `level1-route-retail.tsv` within the **epsilon the fixture
       states** (`# epsilon <units>` in its header). Sectors stay exact inside the comparator.

  Before 2-4, every field of both traces must be present: the comparator skips a `-`, so a required
  health, shield or cog column left as `-` would measure parity for the position columns only and
  pass. The gate rejects any `-` in either trace.

  Both sides capture with the AI **off**: the replay passes `--no-ai` (a capture switch that
  requires `--replay`), and `retail-capture.ps1 -RouteWalk` sets the `-devMode` toggles
  `disableai on` and `noaishots on` (both are toggles, so the capture sets them explicitly and
  re-applies them after every warp). The route's subject is movement and cog state; the cantina
  ambush used to kill retail's player while the port survived, so the bars could never match.
  Combat parity has its own gate (the door encounter).

  `-Epsilon` overrides the fixture's stated value for an experiment; the gate's verdict is the
  fixture's.

.EXAMPLE
  pwsh -File tools/parity/route-gate.ps1
  pwsh -File tools/parity/route-gate.ps1 -Perturb
  pwsh -File tools/parity/route-gate.ps1 -Epsilon 0.5
#>
[CmdletBinding()]
param(
  [string]$Route = 'tools/parity/routes/level1-route.tsv',
  [string]$Level = '01narshadda',
  [string]$PortFixture = 'tools/parity/routetrace/level1-route-port.tsv',
  [string]$RetailFixture = 'tools/parity/routetrace/level1-route-retail.tsv',
  [string]$Out = 'target/parity/routetrace',
  # 0 means "use the epsilon the retail fixture states in its header".
  [double]$Epsilon = 0,
  [switch]$Perturb
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $Root

function Resolve-PathUnderRoot([string]$Path) {
  if ([System.IO.Path]::IsPathRooted($Path)) { return $Path }
  Join-Path $Root $Path
}

$RoutePath = Resolve-PathUnderRoot $Route
$PortPath = Resolve-PathUnderRoot $PortFixture
$RetailPath = Resolve-PathUnderRoot $RetailFixture
$OutDir = Resolve-PathUnderRoot $Out
$Exe = Join-Path $Root 'target\release\bevi-knight.exe'

if (-not (Test-Path -LiteralPath $Exe)) {
  throw "missing $Exe; run cargo build --release first (the route gate replays the port)"
}
if (-not (Test-Path -LiteralPath $PortPath)) {
  throw "missing the committed port fixture $PortPath"
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$runA = Join-Path $OutDir 'level1-route-a.tsv'
$runB = Join-Path $OutDir 'level1-route-b.tsv'
$log = Join-Path $OutDir 'level1-route-port.log'

function Invoke-Replay([string]$Trace) {
  if (Test-Path -LiteralPath $Trace) { Remove-Item -LiteralPath $Trace -Force }
  $env:BK_HEADLESS = '1'
  try {
    # `--no-ai`: the route measures movement, not combat. Retail runs the same route under its
    # `-devMode` `disableai` (see `retail-capture.ps1 -RouteWalk`), so both sides fight nothing and
    # the cantina ambush cannot kill one side and not the other.
    & $Exe --start $Level --replay $RoutePath --route-trace $Trace --no-ai --mute *> $log
  } finally {
    Remove-Item Env:BK_HEADLESS -ErrorAction SilentlyContinue
  }
  if (-not (Test-Path -LiteralPath $Trace)) { throw "the replay wrote no trace ($Trace); see $log" }
}

Write-Host 'route gate: replaying the scripted route twice...'
Invoke-Replay $runA
Invoke-Replay $runB

$hashA = (Get-FileHash -LiteralPath $runA -Algorithm SHA256).Hash
$hashB = (Get-FileHash -LiteralPath $runB -Algorithm SHA256).Hash
if ($hashA -ne $hashB) {
  Write-Host "FAIL  determinism: two replays differ ($hashA vs $hashB)"
  exit 1
}
Write-Host ("PASS  determinism: two replays are identical ({0} rows)" -f ((Get-Content -LiteralPath $runA).Count - 1))

# 1b. Required fields. A `-` is a field the comparator would skip, so the gate rejects it before any
# comparison: health, shields and the two cogs are the trace's required fields, and level 1 has both
# cogs. Leaving any of them `-` would measure parity for the position columns only (issue #8 shipped
# that way).
if (-not (Test-Path -LiteralPath $RetailPath)) {
  Write-Host "FAIL  retail: missing $RetailPath"
  exit 1
}
function Find-MissingField([string]$Path) {
  $header = $null
  $lineNo = 0
  foreach ($line in Get-Content -LiteralPath $Path) {
    $lineNo++
    if ($line -match '^\s*(#|$)') { continue }
    $cells = @($line -split "`t")
    if (-not $header) { $header = $cells; continue }
    for ($i = 1; $i -lt $header.Count -and $i -lt $cells.Count; $i++) {
      if ($cells[$i] -eq '-') { return ("line {0}, column '{1}'" -f $lineNo, $header[$i]) }
    }
  }
  return $null
}
$retailMissing = Find-MissingField $RetailPath
if ($retailMissing) {
  Write-Host ("FAIL  retail: {0} leaves a required field as '-' ({1})" -f $RetailFixture, $retailMissing)
  exit 1
}
$portMissing = Find-MissingField $runA
if ($portMissing) {
  Write-Host ("FAIL  port: the replay leaves a required field as '-' ({0})" -f $portMissing)
  exit 1
}
Write-Host 'PASS  required fields: health, shields and both cog states are recorded on both sides'

# 2. The committed oracle. The comparator is the app's own `--route-compare`; exit code is the gate.
& $Exe --route-compare $PortPath $runA
if ($LASTEXITCODE -ne 0) {
  Write-Host 'FAIL  the replay does not match the committed route fixture (route-compare above)'
  exit 1
}
Write-Host ("PASS  oracle: the replay matches {0}" -f $PortFixture)

# 3. The comparator must reject a trace with one field moved. `-Perturb` is the explicit proof; it is
# also what the roadmap's acceptance asks the gate to demonstrate.
$perturbed = Join-Path $OutDir 'level1-route-perturbed.tsv'
$lines = Get-Content -LiteralPath $runA
$header = @($lines[0] -split "`t")
$xColumn = [Array]::IndexOf($header, 'x')
if ($xColumn -lt 0) { throw 'the route trace has no x column' }
$rows = New-Object System.Collections.Generic.List[string]
$rows.Add($lines[0])
$moved = $false
foreach ($line in $lines | Select-Object -Skip 1) {
  $cells = @($line -split "`t")
  if (-not $moved -and $cells[$xColumn] -ne '-') {
    $cells[$xColumn] = ([double]$cells[$xColumn] + 0.25).ToString('0.0000', [cultureinfo]::InvariantCulture)
    $moved = $true
  }
  $rows.Add(($cells -join "`t"))
}
Set-Content -LiteralPath $perturbed -Value $rows -Encoding utf8

& $Exe --route-compare $PortPath $perturbed 2>&1 | Out-Null
if ($LASTEXITCODE -eq 0) {
  Write-Host 'FAIL  self-check: a trace with x moved 0.25 units passed the comparator'
  exit 1
}
Write-Host 'PASS  self-check: a trace with x moved 0.25 units is rejected'
if ($Perturb) {
  Write-Host 'route gate: the perturbed trace failed and the unmodified trace passed, as required'
}

# 4. Retail parity, at the epsilon the fixture states. The fixture is the oracle, so the tolerance is
# part of it: `# epsilon <units>` is the harness's own resolution (see the file's header).
$stated = Select-String -LiteralPath $RetailPath -Pattern '^\s*#\s*epsilon\s+([0-9.]+)' | Select-Object -First 1
if (-not $stated) {
  Write-Host "FAIL  retail: $RetailFixture states no '# epsilon <units>' tolerance"
  exit 1
}
$fixtureEpsilon = [double]$stated.Matches[0].Groups[1].Value
$tolerance = if ($Epsilon -gt 0) { $Epsilon } else { $fixtureEpsilon }
if ($Epsilon -gt 0 -and $Epsilon -ne $fixtureEpsilon) {
  Write-Host ("note: comparing at {0} (the fixture states {1})" -f $Epsilon, $fixtureEpsilon)
}
$env:BK_ROUTE_EPSILON = $tolerance.ToString([cultureinfo]::InvariantCulture)
try {
  & $Exe --route-compare $RetailPath $runA
  $retailOk = $LASTEXITCODE -eq 0
} finally {
  Remove-Item Env:BK_ROUTE_EPSILON -ErrorAction SilentlyContinue
}
if (-not $retailOk) {
  Write-Host ("FAIL  retail: the replay differs from {0} beyond {1}" -f $RetailFixture, $tolerance)
  exit 1
}
Write-Host ("PASS  retail: the replay matches {0} within {1}" -f $RetailFixture, $tolerance)

Write-Host 'route gate: 0 failure(s)'
exit 0
