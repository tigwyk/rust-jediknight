#requires -Version 7
<#
.SYNOPSIS
  Port half of the fixed level-1 door-encounter telemetry (issue #9).

.DESCRIPTION
  Runs the port at the pose in `tools/parity/door-encounter.tsv` for the row's fixed 1/60 s frame
  count, with `BK_ENCOUNTER_OUT` set, and writes the runtime's own timeline:

      t_ms  kind  thing  template  x  y  z  yaw  health  shields  detail

  Rows are `player` and `actor` samples on the row's cadence, `shot` at the launch of a shot that
  lands on the player (`detail` names the shooter's target and `hit`), `miss` for a fired shot that
  dies without landing on the player, and `hit` at each impact with the bars after it and the split
  that hit took from each bar. `retail-capture.ps1 -EncounterMs` writes the same shape from the
  retail engine's memory; `encounter-gate.ps1` compares them.

  The step is fixed by `--shot` (the capture path pins dt to 1/60 s), the pose is the same
  `--warp`/`--shot-at` pair the screenshot captures use, and no input is injected, so two runs of
  the same command produce the same file.

.EXAMPLE
  pwsh -File tools/parity/port-encounter.ps1
  pwsh -File tools/parity/port-encounter.ps1 -Only door_encounter -Out target/parity/encounter -Suffix -a
#>
[CmdletBinding()]
param(
  [string]$Encounters = 'tools/parity/door-encounter.tsv',
  [string]$Out = 'target/parity/encounter',
  # Appended before `.tsv`, so a gate can keep two runs apart for the repeatability check.
  [string]$Suffix = '',
  # The committed engine capture whose origin block is the encounter's starting AI phase (issue
  # #16). The runtime adopts its `rand()` word and each door actor's flag/deadline; an empty value
  # leaves the port's own phase.
  [string]$Seed = 'tools/parity/engine/route_door.tsv',
  [string[]]$Only = @()
)

$ErrorActionPreference = 'Stop'

$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$RowsPath = if ([System.IO.Path]::IsPathRooted($Encounters)) { $Encounters } else { Join-Path $Root $Encounters }
$OutDir = if ([System.IO.Path]::IsPathRooted($Out)) { $Out } else { Join-Path $Root $Out }
$Exe = Join-Path $Root 'target\release\bevi-knight.exe'

if (-not (Test-Path -LiteralPath $Exe)) { throw "missing $Exe; run cargo build --release first" }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# One process at a time: a running retail capture owns the display.
$deadline = (Get-Date).AddSeconds(180)
while (Get-Process JK -ErrorAction SilentlyContinue) {
  if ((Get-Date) -gt $deadline) { throw 'JK.EXE is still running after 180 s; aborting the port encounter capture' }
  Write-Host 'JK.EXE is running (retail capture in progress); waiting...'
  Start-Sleep -Seconds 3
}

function Split-CommaList([string[]]$Values) {
  @($Values | Where-Object { $_ } | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
}

$only = @(Split-CommaList $Only)
$rows = [System.Collections.Generic.List[object]]::new()
foreach ($line in Get-Content -LiteralPath $RowsPath) {
  if ($line -match '^\s*(#|$)') { continue }
  $f = $line -split ([char]9)
  if ($f.Length -lt 9 -or $f[0].Trim() -eq 'name') { continue }
  $rows.Add([pscustomobject]@{
      Name    = $f[0].Trim()
      Level   = $f[1].Trim()
      X       = [double]$f[2].Trim()
      Y       = [double]$f[3].Trim()
      Z       = [double]$f[4].Trim()
      Pitch   = [double]$f[5].Trim()
      Yaw     = [double]$f[6].Trim()
      Frames  = [int]$f[7].Trim()
      Cadence = [int]$f[8].Trim()
    })
}
if ($only) { $rows = @($rows | Where-Object { $only -contains $_.Name }) }
if ($rows.Count -eq 0) { throw "no encounter rows matched in $RowsPath" }

foreach ($row in $rows) {
  $trace = Join-Path $OutDir ("{0}{1}.tsv" -f $row.Name, $Suffix)
  $scratch = Join-Path $OutDir ("{0}{1}-shot.png" -f $row.Name, $Suffix)
  $log = Join-Path $OutDir ("{0}{1}-port.log" -f $row.Name, $Suffix)
  $pose = '{0},{1},{2},{3}' -f $row.X, $row.Y, $row.Z, $row.Yaw
  $shotAt = '{0},{1}' -f $pose, $row.Pitch
  Write-Host ("port encounter '{0}' level={1} pose=({2}) frames={3} ({4:N1} s)" -f `
      $row.Name, $row.Level, $pose, $row.Frames, ($row.Frames / 60.0))
  $seedPath = if ($Seed) {
    if ([System.IO.Path]::IsPathRooted($Seed)) { $Seed } else { Join-Path $Root $Seed }
  } else { '' }
  if ($seedPath -and -not (Test-Path -LiteralPath $seedPath)) {
    throw "missing engine capture $seedPath; the encounter's AI phase is seeded from it"
  }
  $env:BK_ENCOUNTER_OUT = $trace
  if ($seedPath) { $env:BK_ENCOUNTER_SEED = $seedPath }
  try {
    & $Exe --start $row.Level --frames $row.Frames --shot $scratch --shot-at $shotAt --warp $pose --mute *> $log
  } finally {
    Remove-Item Env:BK_ENCOUNTER_OUT -ErrorAction SilentlyContinue
    Remove-Item Env:BK_ENCOUNTER_SEED -ErrorAction SilentlyContinue
  }
  if (-not (Test-Path -LiteralPath $trace)) { throw "the port wrote no encounter trace for $($row.Name); see $log" }
  $lines = @(Get-Content -LiteralPath $trace)
  if ($lines.Count -lt 2) { throw "the port encounter trace for $($row.Name) has no rows; see $log" }
  $samples = @($lines | Select-Object -Skip 1 | Where-Object { $_ -match "^[0-9]+\t(sample|player|actor)\t" })
  $last = $lines[-1] -split "`t"
  Write-Host ("  wrote {0}: {1} row(s) ({2} sample row(s)), ends at t={3} ms" -f `
      $trace, ($lines.Count - 1), $samples.Count, $last[0])
}
