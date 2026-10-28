#requires -Version 7
<#
.SYNOPSIS
  Port-side walk traces: one TSV per leg of tools/parity/walk-shots.tsv.

.DESCRIPTION
  Phase 0.5 trace v0's port half. Each leg starts the player at the row's pose (`--warp`), holds
  forward for `-HoldMs` x `-Segments` of **game time** and writes
  target/parity/trace/<name>.tsv with one line per frame:

      tick  t_s  x  y  z  sector  vy  health  shields

  The retail half is `pwsh -File tools/parity/retail-capture.ps1 -Shots tools/parity/walk-shots.tsv
  -Only <name> -WalkTraceMs <total>`; `tools/parity/trace-compare.ps1` compares the two.

  The step is fixed at 1/60 s (`--shot` puts the app in capture mode, which pins `dt`), and forward
  is held by `BK_WALK_FORWARD=1` rather than the keyboard, so two runs of the same command are
  identical - the same policy `port-capture.ps1` states.

  This script NEVER launches the retail game; it waits for a running JK.EXE like port-capture does.

.EXAMPLE
  pwsh -File tools/parity/port-trace.ps1
  pwsh -File tools/parity/port-trace.ps1 -Only walk_cantina
#>
[CmdletBinding()]
param(
  [string]$Legs = 'tools/parity/walk-shots.tsv',
  [string]$Out = 'target/parity/trace',
  [int]$HoldMs = 600,
  [int]$Segments = 5,
  # Leg names to run (default: every row of the legs file).
  [string[]]$Only = @()
)

$ErrorActionPreference = 'Stop'

$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$LegsPath = if ([System.IO.Path]::IsPathRooted($Legs)) { $Legs } else { Join-Path $Root $Legs }
$OutDir = if ([System.IO.Path]::IsPathRooted($Out)) { $Out } else { Join-Path $Root $Out }
$Exe = Join-Path $Root 'target\release\bevi-knight.exe'

if (-not (Test-Path -LiteralPath $Exe)) { throw "missing $Exe; run cargo build --release first" }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# Wait for the retail side to finish: a running JK.EXE owns the display, and the port's windowed run
# must not fight it for the desktop (the same rule port-capture.ps1 states).
$deadline = (Get-Date).AddSeconds(120)
while (Get-Process JK -ErrorAction SilentlyContinue) {
  if ((Get-Date) -gt $deadline) { throw 'JK.EXE is still running after 120 s; aborting the port trace' }
  Write-Host 'JK.EXE is running (retail trace/capture in progress); waiting...'
  Start-Sleep -Seconds 3
}

# Parse the legs file: '#' comments, then name/level/x/y/z/pitch/yaw separated by tabs.
$legRows = [System.Collections.Generic.List[object]]::new()
foreach ($line in Get-Content -LiteralPath $LegsPath) {
  if ($line -match '^\s*(#|$)') { continue }
  $f = $line -split ([char]9)
  if ($f.Length -lt 7 -or $f[0].Trim() -eq 'name') { continue }
  $legRows.Add([pscustomobject]@{
      Name  = $f[0].Trim()
      Level = $f[1].Trim()
      X     = [double]$f[2].Trim()
      Y     = [double]$f[3].Trim()
      Z     = [double]$f[4].Trim()
      Pitch = [double]$f[5].Trim()
      Yaw   = [double]$f[6].Trim()
    })
}
if ($legRows.Count -eq 0) { throw "no legs in $LegsPath" }
if ($Only) {
  $legRows = @($legRows | Where-Object { $Only -contains $_.Name })
  if ($legRows.Count -eq 0) { throw "-Only matched no leg in $LegsPath" }
}

$totalMs = $HoldMs * $Segments
$frames = [int][math]::Round($totalMs / 1000.0 * 60.0)

foreach ($leg in $legRows) {
  $trace = Join-Path $OutDir ("{0}.tsv" -f $leg.Name)
  $scratch = Join-Path $OutDir ("{0}-shot.png" -f $leg.Name)
  $pose = '{0},{1},{2},{3}' -f $leg.X, $leg.Y, $leg.Z, $leg.Yaw
  $shotAt = '{0},{1}' -f $pose, $leg.Pitch
  Write-Host ("port trace '{0}' level={1} pose=({2}) frames={3} ({4} ms held)" -f $leg.Name, $leg.Level, $pose, $frames, $totalMs)
  $env:BK_WALK_FORWARD = '1'
  try {
    & $Exe --start $leg.Level --frames $frames --shot $scratch --shot-at $shotAt --warp $pose --trace-out $trace --mute *> (Join-Path $OutDir ("{0}-port.log" -f $leg.Name))
  } finally {
    Remove-Item Env:BK_WALK_FORWARD -ErrorAction SilentlyContinue
  }
  $lines = @(Get-Content -LiteralPath $trace)
  if ($lines.Count -lt 2) { throw "the port wrote no samples for $($leg.Name)" }
  $last = $lines[-1] -split "`t"
  Write-Host ("  wrote {0}: {1} frame(s), ends at ({2}, {3}, {4}) sector {5}" -f $trace, ($lines.Count - 1), $last[2], $last[3], $last[4], $last[5])
}
