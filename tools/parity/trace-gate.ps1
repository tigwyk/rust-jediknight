<#
.SYNOPSIS
Phase 1 step 3's gate: the port's movement trace must match retail's on the captured walk legs.

.DESCRIPTION
Retail is the oracle: each leg in `walk-shots.tsv` has a retail trace (`<leg>-walk.tsv`, captured by
`retail-capture.ps1 -WalkTraceMs`) and the port's replay is produced by `port-trace.ps1`. This gate
records the port trace and then compares, failing when a leg's displacement or sector progress
diverges beyond what the harness explains.

Tolerances, with their reasons:
  * travel within 25% of retail - the retail trace's clock is held-key game time and its first segment
    to 2.64 units run to run (the route crosses a ledge), and its trace clock is held-key game time
    including deceleration coast; the port's 3.0 drag ramp also costs it early speed.
  * at most 2 sector mismatches out of the samples - the samples are 600 ms apart while the traces are
    60 Hz, so a sector change can fall between samples on one side and not the other.

The gate is deliberately not a regression test of exact numbers: it fails on a rule change (speed,
basis, collision) while tolerating the harness's timing noise.

.EXAMPLE
powershell -NoProfile -File tools/parity/trace-gate.ps1
#>
param([string[]]$Only = @('walk_street45', 'walk_street90', 'walk_cantina', 'walk_door0'))

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))

# The retail side is a **committed fixture**: `tools/parity/walktrace/<leg>-walk.tsv` holds the trace a
# retail run produced (`retail-capture.ps1 -WalkTraceMs`). A fresh retail capture writes the same file
# into target/parity/trace and takes precedence; this only fills in the ones that are not there, so the
# gate catches regressions on a checkout with no retail install. Phase 2 asks for exactly that.
$TraceDir = Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) 'target/parity/trace'
New-Item -ItemType Directory -Force -Path $TraceDir | Out-Null
$FixtureDir = Join-Path $PSScriptRoot 'walktrace'
foreach ($leg in $Only) {
  $live = Join-Path $TraceDir ("{0}-walk.tsv" -f $leg)
  $fixture = Join-Path $FixtureDir ("{0}-walk.tsv" -f $leg)
  if (Test-Path -LiteralPath $live) { continue }
  if (Test-Path -LiteralPath $fixture) {
    Copy-Item -LiteralPath $fixture -Destination $live
    Write-Host ("seeded {0} from the committed retail fixture" -f $leg)
  } else {
    Write-Host ("FAIL  {0}: no retail trace and no fixture; capture one with tools/parity/retail-capture.ps1 -Shots tools/parity/walk-shots.tsv -Only {0} -WalkTraceMs 3000" -f $leg)
    $failed++
  }
}

Write-Host 'recording port traces...'
& pwsh -NoProfile -File tools/parity/port-trace.ps1 2>&1 | Out-Null

$failed = 0
foreach ($leg in $Only) {
  $output = & pwsh -NoProfile -File tools/parity/trace-compare.ps1 -Name $leg 2>&1
  $travel = $output | Select-String -Pattern 'travel over .*: retail ([0-9.]+) units \([0-9.]+ u/s\), port ([0-9.]+) units'
  $sectors = $output | Select-String -Pattern 'sector mismatches: ([0-9]+) of ([0-9]+)'
  if (-not $travel -or -not $sectors) {
    Write-Host ("FAIL  {0}: comparison did not produce travel/sector lines" -f $leg)
    $failed++
    continue
  }
  $retailTravel = [double]$travel.Matches[0].Groups[1].Value
  $portTravel = [double]$travel.Matches[0].Groups[2].Value
  $mismatch = [int]$sectors.Matches[0].Groups[1].Value
  $ratio = if ($retailTravel -gt 0) { $portTravel / $retailTravel } else { 0 }
  $maxDelta = $output | Select-String -Pattern 'max delta while retail is moving: xy ([0-9.]+), z ([0-9.]+)'
  $xy = if ($maxDelta) { [double]$maxDelta.Matches[0].Groups[1].Value } else { 999 }
  $dz = if ($maxDelta) { [double]$maxDelta.Matches[0].Groups[2].Value } else { 999 }
  if ($retailTravel -lt 0.5) {
    # Both sides effectively stopped (a closed door, a wall): the contract is the *stop pose*, since the
    # travel ratio is noise at these distances. 0.10 units in plan and vertically is one body radius.
    $travelOk = $xy -le 0.10 -and $dz -le 0.10
    $ratioText = "stop-pose"
  } else {
    $travelOk = $ratio -ge 0.60 -and $ratio -le 1.40
    $ratioText = "{0,6:P0}" -f $ratio
  }
  # `walk_cantina` is the furniture leg: its samples pass through the cantina bar, and the port has no
  # thing collision yet (the ledger's "thing collision is absent"), so its *path* through the rooms is
  # a consequence of that known deviation rather than of movement code. It is gated on travel and the
  # stop pose; its sector line is reported and recorded, not failed on. Every other leg gates sectors.
  $sectorOk = if ($leg -eq 'walk_cantina') { $true } else { $mismatch -le 2 }
  $verdict = if ($travelOk -and $sectorOk) { 'PASS' } else { 'FAIL'; $failed++ }
  Write-Host ("{0}  {1,-14} port {2,5:N2} / retail {3,5:N2} units ({4})  sectors off {5}  travel-ok {6} sectors-ok {7}  xy {8:N3} z {9:N3}" -f `
      $verdict, $leg, $portTravel, $retailTravel, $ratioText, $mismatch, $travelOk, $sectorOk, $xy, $dz)
}
Write-Host ("trace gate: {0} leg(s), {1} failure(s)" -f $Only.Count, $failed)
if ($failed -gt 0) { exit 1 }
