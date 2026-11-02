#requires -Version 7
<#
.SYNOPSIS
  Compare a retail walk trace with the port's replay of the same leg.

.DESCRIPTION
  Phase 0.5 trace v0's comparison. The retail trace (`retail-capture.ps1 -WalkTraceMs`) samples the
  body's pose and sector every `HoldMs` of **held-forward game time**; the port trace
  (`port-trace.ps1`) records one line per 1/60 s frame for the same leg. This script interpolates the
  port's path to each retail sample time and reports the per-sample delta.

  The clock is the held-key time, not wall time: reading `coords` goes through the dev-console
  dialog, and the dialog pauses the game, so only the hold segments advance the engine's clock. The
  port's replay holds forward continuously for the same total time, so `t = held_ms / 1000` is the
  common axis. This is a *measurement*, not a gate: it prints the deltas and the sector sequence, and
  only returns a verdict when `-Epsilon` is given (position delta in world units).

.EXAMPLE
  pwsh -File tools/parity/trace-compare.ps1 -Name walk_cantina
  pwsh -File tools/parity/trace-compare.ps1 -Name walk_cantina -Epsilon 0.5
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory)][string]$Name,
  [string]$Dir = 'target/parity/trace',
  # Position tolerance in world units. Omitted: report only, no PASS/FAIL.
  [double]$Epsilon = -1,
  # Retail clearance, in world units: a sample whose retail position is *stopped* (two consecutive
  # retail samples within this distance) is a wall, not a movement mismatch, and is reported apart.
  [double]$StoppedBelow = 0.02
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$DirPath = if ([System.IO.Path]::IsPathRooted($Dir)) { $Dir } else { Join-Path $Root $Dir }
$RetailPath = Join-Path $DirPath ("{0}-walk.tsv" -f $Name)
$PortPath = Join-Path $DirPath ("{0}.tsv" -f $Name)
foreach ($p in @($RetailPath, $PortPath)) {
  if (-not (Test-Path -LiteralPath $p)) { throw "missing trace $p (run retail-capture.ps1 -WalkTraceMs and port-trace.ps1 first)" }
}

# The retail trace's first column is `held_ms`; the port's is `tick` and its second is `t_s`, so the
# reader takes the time and data columns explicitly.
function Read-Tsv([string]$Path, [int]$TimeColumn, [string]$First) {
  $rows = @()
  $lines = @(Get-Content -LiteralPath $Path)
  foreach ($line in $lines) {
    if ($line -match '^\s*(#|$)') { continue }
    if ($line -match $First) { continue }
    $f = $line -split "`t"
    if ($f.Length -lt 6) { continue }
    $rows += [pscustomobject]@{
      T      = [double]$f[$TimeColumn]
      X      = [double]$f[$TimeColumn + 1]
      Y      = [double]$f[$TimeColumn + 2]
      Z      = [double]$f[$TimeColumn + 3]
      Sector = [int]$f[$TimeColumn + 4]
    }
  }
  return $rows
}

$retail = Read-Tsv $RetailPath 0 'held_ms'
# A trace with a `game_ms` column (7th) carries the engine's own clock: use it as the time axis, since
# the injected hold length is not game time at retail's few frames a second. Older fixtures keep held_ms.
$retailRows = @(Get-Content -LiteralPath $RetailPath | Where-Object { $_ -notmatch '^\s*(#|$)' -and $_ -notmatch 'held_ms' })
$clock = 'held_ms'
if ($retailRows.Count -eq $retail.Count -and @($retailRows | Where-Object { ($_ -split "`t").Length -ge 7 }).Count -eq $retailRows.Count) {
  for ($i = 0; $i -lt $retail.Count; $i++) { $retail[$i].T = [double](($retailRows[$i] -split "`t")[6]) }
  $clock = 'game_ms'
}
Write-Host ("  retail time axis: {0}" -f $clock)
$port = Read-Tsv $PortPath 1 'tick'
if ($retail.Count -eq 0 -or $port.Count -lt 2) { throw "empty trace ($($retail.Count) retail samples, $($port.Count) port frames)" }

# The retail `held_ms` is milliseconds; the port's first column is ticks at 1/60 s.
function Port-At([double]$seconds) {
  if ($seconds -le $port[0].T) { return $port[0] }
  if ($seconds -ge $port[-1].T) { return $port[-1] }
  for ($i = 1; $i -lt $port.Count; $i++) {
    if ($port[$i].T -ge $seconds) {
      $a = $port[$i - 1]; $b = $port[$i]
      $span = $b.T - $a.T
      $u = if ($span -le 0) { 0.0 } else { ($seconds - $a.T) / $span }
      return [pscustomobject]@{
        T = $seconds
        X = $a.X + ($b.X - $a.X) * $u
        Y = $a.Y + ($b.Y - $a.Y) * $u
        Z = $a.Z + ($b.Z - $a.Z) * $u
        Sector = if ($u -lt 0.5) { $a.Sector } else { $b.Sector }
      }
    }
  }
  return $port[-1]
}

Write-Host ("trace: {0}" -f $Name)
Write-Host ("  retail {0}: {1} sample(s) over {2:N0} ms ({3})" -f (Split-Path -Leaf $RetailPath), $retail.Count, $retail[-1].T, $clock)
Write-Host ("  port   {0}: {1} frame(s) over {2:N3} s at 1/60 s" -f (Split-Path -Leaf $PortPath), $port.Count, $port[-1].T)
Write-Host ""
Write-Host ("  {0,8}  {1,24}  {2,24}  {3,8}  {4,8}  {5,6}  {6,6}" -f $clock, 'retail (x, y, z)', 'port (x, y, z)', 'd_xy', 'd_z', 'sect', 'sect')
Write-Host ("  " + ('-' * 100))

$maxXy = 0.0
$maxZ = 0.0
$sectorMismatch = 0
$stopped = 0
$prevRetail = $null
foreach ($r in $retail) {
  $p = Port-At ($r.T / 1000.0)
  $dxy = [math]::Sqrt(($r.X - $p.X) * ($r.X - $p.X) + ($r.Y - $p.Y) * ($r.Y - $p.Y))
  $dz = [math]::Abs($r.Z - $p.Z)
  $moving = $true
  if ($null -ne $prevRetail) {
    $step = [math]::Sqrt(($r.X - $prevRetail.X) * ($r.X - $prevRetail.X) + ($r.Y - $prevRetail.Y) * ($r.Y - $prevRetail.Y))
    if ($r.T -gt $prevRetail.T -and $step / (($r.T - $prevRetail.T) / 1000.0) -lt $StoppedBelow * 10.0) { $moving = $false }
  }
  $sectorOk = $r.Sector -eq $p.Sector
  if ($moving) {
    if ($dxy -gt $maxXy) { $maxXy = $dxy }
    if ($dz -gt $maxZ) { $maxZ = $dz }
  } else {
    $stopped++
  }
  if (-not $sectorOk) { $sectorMismatch++ }
  Write-Host ("  {0,8:N0}  ({1,8:N3}, {2,8:N3}, {3,8:N3})  ({4,8:N3}, {5,8:N3}, {6,8:N3})  {7,8:N3}  {8,8:N3}  {9,6}  {10,6}" -f `
      $r.T, $r.X, $r.Y, $r.Z, $p.X, $p.Y, $p.Z, $dxy, $dz, $r.Sector, $(if ($sectorOk) { 'ok' } else { 'MISS' }))
  $prevRetail = $r
}

$dr = $retail[-1]
$dp = Port-At ($retail[-1].T / 1000.0)
$travelRetail = [math]::Sqrt(($dr.X - $retail[0].X) * ($dr.X - $retail[0].X) + ($dr.Y - $retail[0].Y) * ($dr.Y - $retail[0].Y))
$travelPort = [math]::Sqrt(($dp.X - $port[0].X) * ($dp.X - $port[0].X) + ($dp.Y - $port[0].Y) * ($dp.Y - $port[0].Y))
$seconds = $retail[-1].T / 1000.0
Write-Host ""
Write-Host ("  travel over {0:N2} s held: retail {1:N2} units ({2:N2} u/s), port {3:N2} units ({4:N2} u/s)" -f `
    $seconds, $travelRetail, ($travelRetail / $seconds), $travelPort, ($travelPort / $seconds))
Write-Host ("  max delta while retail is moving: xy {0:N3}, z {1:N3} (world units)" -f $maxXy, $maxZ)
Write-Host ("  samples where retail is stopped: {0} of {1} (reported apart)" -f $stopped, $retail.Count)
Write-Host ("  sector mismatches: {0} of {1}" -f $sectorMismatch, $retail.Count)

if ($Epsilon -ge 0) {
  $ok = ($maxXy -le $Epsilon) -and ($maxZ -le $Epsilon) -and ($sectorMismatch -eq 0)
  Write-Host ("  verdict: {0} (epsilon {1:N3} units)" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $Epsilon)
  if (-not $ok) { exit 1 }
}
