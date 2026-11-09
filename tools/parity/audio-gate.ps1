#requires -Version 7
<#
.SYNOPSIS
  The level-1 audio gate: the port's audio state through the scripted route must match the
  retail-derived reference, and the replay must reproduce it byte for byte.

.DESCRIPTION
  Issue #13's oracle. `--audio-trace` writes, every 600 ms of the replay (the same 36-tick cadence
  as `--route-trace`):

    tick  t_s  music  music_end  music_loop  music_vol  ambient  ambient_vol  sounds

  `music`/`music_end`/`music_loop` are `PlaySong`'s track range; `music_vol` is `SetMusicVol`'s
  current value (level 1's `01_SoundTrack.cog` fades it from a 0.1 s pulse). `ambient` is the
  sector's own `SOUND` loop (or a script's `SectorSound`). `sounds` lists every one-shot the window
  played as `wav@gain`, the gain being the mixer's attenuated volume.

  The gate checks four things:

    1. determinism — two replays of the same route produce identical traces;
    2. the committed port oracle — the replay matches `tools/parity/audiotrace/level1-audio-port.tsv`;
    3. self-check — a copy of the trace with one sound's gain moved 0.25 is **rejected**, and the
       comparator names it `attenuated` (`-Perturb` runs that check explicitly);
    4. retail — `--audio-compare` against `level1-audio-retail.tsv`, whose header documents the
       provenance of every column. `--audio-compare` classifies each sound difference as missing,
       extra, mistimed or attenuated, and reports music/ambience state differences per sample.

  `BK_AUDIO_EPSILON` is the absolute tolerance for gains and volumes (default 0.02; this gate
  passes 0.12, two 0.1 s pulse steps, because the VM clock accumulates 1/60 s in f32 and a pulse
  boundary lands one frame either side of the model's 100 ms grid).

.EXAMPLE
  pwsh -File tools/parity/audio-gate.ps1
  pwsh -File tools/parity/audio-gate.ps1 -Perturb
#>
[CmdletBinding()]
param(
  [string]$Level = '01narshadda',
  [string]$Route = 'tools/parity/routes/level1-audio-route.tsv',
  [string]$PortFixture = 'tools/parity/audiotrace/level1-audio-port.tsv',
  [string]$RetailFixture = 'tools/parity/audiotrace/level1-audio-retail.tsv',
  [string]$Out = 'target/parity/audiotrace',
  [double]$Epsilon = 0.12,
  [switch]$Perturb,
  [string]$Game = $env:BK_GAME
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
  throw "missing $Exe; run cargo build --release first (the audio gate replays the port)"
}
if (-not (Test-Path -LiteralPath $PortPath)) {
  throw "missing the committed port fixture $PortPath"
}
if (-not (Test-Path -LiteralPath $RetailPath)) {
  throw "missing the retail-derived fixture $RetailPath"
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$runA = Join-Path $OutDir 'level1-audio-a.tsv'
$runB = Join-Path $OutDir 'level1-audio-b.tsv'
$log = Join-Path $OutDir 'level1-audio-port.log'

function Invoke-Replay([string]$Trace) {
  if (Test-Path -LiteralPath $Trace) { Remove-Item -LiteralPath $Trace -Force }
  $env:BK_HEADLESS = '1'
  try {
    $arguments = @('--start', $Level, '--replay', $RoutePath, '--audio-trace', $Trace, '--mute')
    if ($Game) { $arguments = @('--game', $Game) + $arguments }
    & $Exe @arguments *> $log
  } finally {
    Remove-Item Env:BK_HEADLESS -ErrorAction SilentlyContinue
  }
  if (-not (Test-Path -LiteralPath $Trace)) { throw "the replay wrote no trace ($Trace); see $log" }
}

Write-Host 'audio gate: replaying the scripted route twice...'
Invoke-Replay $runA
Invoke-Replay $runB

$hashA = (Get-FileHash -LiteralPath $runA -Algorithm SHA256).Hash
$hashB = (Get-FileHash -LiteralPath $runB -Algorithm SHA256).Hash
if ($hashA -ne $hashB) {
  Write-Host "FAIL  determinism: two replays differ ($hashA vs $hashB)"
  exit 1
}
Write-Host ("PASS  determinism: two replays are identical ({0} rows)" -f ((Get-Content -LiteralPath $runA).Count - 1))

# 2. The committed oracle.
& $Exe --audio-compare $PortPath $runA
if ($LASTEXITCODE -ne 0) {
  Write-Host 'FAIL  the replay does not match the committed audio fixture (audio-compare above)'
  exit 1
}
Write-Host ("PASS  oracle: the replay matches {0}" -f $PortFixture)

# 3. The comparator must reject a trace whose mix changed. Move the gain of the first recorded
# sound by 0.25: that is an attenuation difference, and the comparator must call it one.
$perturbed = Join-Path $OutDir 'level1-audio-perturbed.tsv'
$lines = Get-Content -LiteralPath $runA
$header = @($lines[0] -split "`t")
$soundsColumn = [Array]::IndexOf($header, 'sounds')
if ($soundsColumn -lt 0) { throw 'the audio trace has no sounds column' }
$rows = New-Object System.Collections.Generic.List[string]
$rows.Add($lines[0])
$moved = $false
foreach ($line in $lines | Select-Object -Skip 1) {
  $cells = @($line -split "`t")
  if (-not $moved -and $cells[$soundsColumn]) {
    $entries = @($cells[$soundsColumn] -split ' ')
    $parts = @($entries[0] -split '@')
    if ($parts.Count -eq 2) {
      $gain = ([double]$parts[1] + 0.25).ToString('0.000', [cultureinfo]::InvariantCulture)
      $entries[0] = '{0}@{1}' -f $parts[0], $gain
      $cells[$soundsColumn] = $entries -join ' '
      $moved = $true
    }
  }
  $rows.Add(($cells -join "`t"))
}
if (-not $moved) { throw 'the replay played no sound; the audio trace cannot be perturbed' }
Set-Content -LiteralPath $perturbed -Value $rows -Encoding utf8

$perturbOut = & $Exe --audio-compare $PortPath $perturbed 2>&1
if ($LASTEXITCODE -eq 0) {
  Write-Host 'FAIL  self-check: a sound moved by 0.25 in gain passed the comparator'
  exit 1
}
if (-not ($perturbOut -join "`n") -match 'attenuated') {
  Write-Host 'FAIL  self-check: the changed sound was not reported as attenuated'
  $perturbOut | Write-Host
  exit 1
}
Write-Host 'PASS  self-check: a sound moved by 0.25 in gain is rejected as attenuated'
if ($Perturb) {
  Write-Host 'audio gate: the perturbed trace failed and the unmodified trace passed, as required'
}

# 4. Retail: the scripted route's audio state, derived from the level's own data and scripts.
$env:BK_AUDIO_EPSILON = $Epsilon.ToString([cultureinfo]::InvariantCulture)
try {
  & $Exe --audio-compare $RetailPath $runA
  $retailOk = $LASTEXITCODE -eq 0
} finally {
  Remove-Item Env:BK_AUDIO_EPSILON -ErrorAction SilentlyContinue
}
if (-not $retailOk) {
  Write-Host ("FAIL  retail: the replay differs from {0} (audio-compare above)" -f $RetailFixture)
  exit 1
}
Write-Host ("PASS  retail: the replay matches {0} within {1}" -f $RetailFixture, $Epsilon)

Write-Host 'audio gate: 0 failure(s)'
exit 0
