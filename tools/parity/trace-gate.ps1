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
    carries deceleration coast, so retail's own average runs high; the port's 3.0 drag ramp also costs
    it a little early speed. Both fixtures sit inside 17%.
  * at most 2 sector mismatches out of the samples - the samples are 600 ms apart while the traces are
    60 Hz, so a sector change can fall between samples on one side and not the other.

The gate is deliberately not a regression test of exact numbers: it fails on a rule change (speed,
basis, collision) while tolerating the harness's timing noise.

.EXAMPLE
powershell -NoProfile -File tools/parity/trace-gate.ps1
#>
param([string[]]$Only = @('walk_street45', 'walk_street90', 'walk_cantina'))

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))

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
  $travelOk = $ratio -ge 0.75 -and $ratio -le 1.25
  $sectorOk = $mismatch -le 2
  $verdict = if ($travelOk -and $sectorOk) { 'PASS' } else { 'FAIL'; $failed++ }
  Write-Host ("{0}  {1,-14} port {2,5:N2} / retail {3,5:N2} units ({4,6:P0})  sectors off {5}  travel-ok {6} sectors-ok {7}" -f `
      $verdict, $leg, $portTravel, $retailTravel, $ratio, $mismatch, $travelOk, $sectorOk)
}
Write-Host ("trace gate: {0} leg(s), {1} failure(s)" -f $Only.Count, $failed)
if ($failed -gt 0) { exit 1 }
