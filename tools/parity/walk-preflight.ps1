#requires -Version 7
<#
.SYNOPSIS
  Preflight the walk-trace legs against the port's own collision, before any QA window is spent on them.

.DESCRIPTION
  The walk-trace gate compares a retail walk of a fixed pose against the port's replay. A leg whose route
  walks straight into geometry measures nothing — the retail game walks into the wall, the harness holds
  the forward key until it notices, and the capture is wasted (and, before round 251, the game was left
  running for the operator to close).

  This script answers "does the body actually move from that pose?" with the port's headless walk probe
  (`--walk <frames> --warp x,y,z,yaw`, which resolves against the level's own collision and opens no
  window). For each leg of `walk-shots.tsv` it reports:

      leg            level            frames  travel  blocked  retail  verdict

  `travel` is the horizontal distance the body reached; `blocked` counts frames the collision resolver
  stopped; `retail` is the total xy distance the committed retail fixture travelled (`tools/parity/
  walktrace/<leg>-walk.tsv`), so a leg the port cannot walk can be compared with what retail did.

  A leg PASSES when `travel >= -MinTravel` (default 0.5 units — a walk leg that ends inside the first
  half unit is a wall, not a walk). `trace-gate.ps1` runs this first and refuses to capture or compare a
  failing leg; the exit code is 1 if any leg fails, so a bad route is caught without the retail game.

.EXAMPLE
  pwsh -NoProfile -File tools/parity/walk-preflight.ps1
  pwsh -NoProfile -File tools/parity/walk-preflight.ps1 -Only walk5_spawn,walk6_spawn
#>
[CmdletBinding()]
param(
  [string]$Legs = 'tools/parity/walk-shots.tsv',
  # Port frames to walk per leg: the trace legs hold forward for 3 s (`HoldMs` 600 x `Segments` 5) and the
  # port's walk probe steps at 1/30 s, so 90 frames is the same 3 s of game time.
  [int]$Frames = 90,
  [double]$MinTravel = 0.5,
  [string[]]$Only = @()
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))

$Root = Get-Location
$LegsPath = if ([System.IO.Path]::IsPathRooted($Legs)) { $Legs } else { Join-Path $Root $Legs }
$Exe = Join-Path $Root 'target\release\bevi-knight.exe'
if (-not (Test-Path -LiteralPath $Exe)) { throw "missing $Exe; run cargo build --release first" }

$FixtureDir = Join-Path $Root 'tools/parity/walktrace'

# Total xy distance of a retail walk fixture: the file's own x/y columns, consecutive deltas summed.
function Get-FixtureTravel {
  param([string]$Path)
  if (-not (Test-Path -LiteralPath $Path)) { return $null }
  $pts = @(Get-Content -LiteralPath $Path |
    Select-Object -Skip 1 |
    ForEach-Object {
      $c = $_ -split "`t"
      if ($c.Count -lt 4) { return }
      ,@([double]$c[1], [double]$c[2])
    })
  if ($pts.Count -lt 2) { return 0.0 }
  $travel = 0.0
  for ($k = 1; $k -lt $pts.Count; $k++) {
    $dx = $pts[$k][0] - $pts[$k - 1][0]
    $dy = $pts[$k][1] - $pts[$k - 1][1]
    $travel += [math]::Sqrt($dx * $dx + $dy * $dy)
  }
  return $travel
}

$legRows = [System.Collections.Generic.List[object]]::new()
foreach ($line in Get-Content -LiteralPath $LegsPath) {
  if ($line -match '^\s*(#|$)') { continue }
  $f = $line -split ([char]9)
  if ($f.Length -lt 7 -or $f[0].Trim() -eq 'name') { continue }
  $legRows.Add([pscustomobject]@{
      Name = $f[0].Trim()
      Level = $f[1].Trim()
      X = [double]$f[2].Trim()
      Y = [double]$f[3].Trim()
      Z = [double]$f[4].Trim()
      Yaw = [double]$f[6].Trim()
    })
}
if ($Only) { $legRows = @($legRows | Where-Object { $Only -contains $_.Name }) }
if ($legRows.Count -eq 0) { throw "no legs to preflight in $LegsPath" }

Write-Host ("walk preflight: {0} leg(s), {1} frame(s) each ({2:N1} s of game time)" -f $legRows.Count, $Frames, ($Frames / 30.0))
Write-Host ("{0,-16} {1,-15} {2,9} {3,8} {4,9} {5,7}  {6}" -f 'leg', 'level', 'travel', 'blocked', 'retail', 'verdict', 'detail')

$failed = 0
$failedNames = New-Object System.Collections.Generic.List[string]
foreach ($leg in $legRows) {
  $pose = '{0},{1},{2},{3}' -f $leg.X, $leg.Y, $leg.Z, $leg.Yaw
  $out = & $Exe --start $leg.Level --walk $Frames --warp $pose --mute 2>&1
  $walkLine = $out | Select-String -Pattern 'walk: travelled ([0-9.]+) units, ([0-9]+) blocked' | Select-Object -Last 1
  if (-not $walkLine) {
    $failed++
    $failedNames.Add($leg.Name)
    Write-Host ("{0,-16} {1,-15} {2,9} {3,8} {4,9} {5,7}  {6}" -f $leg.Name, $leg.Level, '-', '-', '-', 'FAIL', 'the walk probe printed no result')
    Write-Output ("walk-preflight`t{0}`tFAIL`t-" -f $leg.Name)
    continue
  }
  $travel = [double]$walkLine.Matches[0].Groups[1].Value
  $blocked = [int]$walkLine.Matches[0].Groups[2].Value
  $retail = Get-FixtureTravel (Join-Path $FixtureDir ("{0}-walk.tsv" -f $leg.Name))
  $retailText = if ($null -eq $retail) { '-' } else { '{0:N2}' -f $retail }
  if ($travel -ge $MinTravel) {
    Write-Host ("{0,-16} {1,-15} {2,9:N2} {3,8} {4,9} {5,7}  {6}" -f $leg.Name, $leg.Level, $travel, $blocked, $retailText, 'PASS', '')
    Write-Output ("walk-preflight`t{0}`tPASS`t{1:N2}" -f $leg.Name, $travel)
  } else {
    $failed++
    $failedNames.Add($leg.Name)
    Write-Host ("{0,-16} {1,-15} {2,9:N2} {3,8} {4,9} {5,7}  {6}" -f $leg.Name, $leg.Level, $travel, $blocked, $retailText, 'FAIL', "travel < $MinTravel units: the route walks into geometry")
    Write-Output ("walk-preflight`t{0}`tFAIL`t{1:N2}" -f $leg.Name, $travel)
  }
}

Write-Host ''
if ($failed -gt 0) {
  Write-Host ("walk preflight: {0} of {1} leg(s) fail: {2}" -f $failed, $legRows.Count, ($failedNames -join ', '))
  Write-Host 'walk preflight: not worth a QA window on these; fix the pose/heading in walk-shots.tsv or drop the leg.'
  exit 1
}
Write-Host ("walk preflight: all {0} leg(s) walk in the port; a retail capture is worth running" -f $legRows.Count)
