#requires -Version 7
<#
.SYNOPSIS
  The level-2 carry gate: the player state level 1 hands to level 2 must be the engine's autosave
  bin table, including the route-dependent population bin 73 = 10.5.

.DESCRIPTION
  Issue #19's oracle. The carry is written and read as the engine's own `.jks` records: one fixed
  28-byte `sithDSS_Inventory` (`0x4f9900`) record per bin in `sithGamesave_SaveCurrentWorld`
  (`0x4da770`)'s order. The gate drives the real path in two headless runs:

    1. level 1, `--replay` of the scripted route, `--carry-out`: at the level's completion the port
       stores the tally's alignment (bin 75 = 20) and writes the carried bin table;
    2. level 2, `--carry-in` that save, `--carry-out`: the level boots with the carried state (the
       engine's `jkPlayer_*` state outlives the level), runs its own `startup` scripts — which set
       `99 = 2000`, goals `100 = 1` and `102 = 1`, and the level's `71 = 8` — and writes the
       level-2 autosave on its first frames, before its own 1 s `created:` timers, exactly when
       `sithOpenPostProcess` (`0x4c4880`) writes retail's `jkauto-02narshadda.jks`.

  The level-2 save is then compared bin for bin with that committed retail autosave. The gate also
  perturbs bin 73 in a copy and requires the comparator to reject it, so a silently widened
  comparison cannot pass. Committed fixtures only: no retail launch.

  Bin 73 is 10.5 because `sithOpenPostProcess` sends `created` to every class cog at load, the actor
  scripts' `created:` arms a 1 s timer, and the credited amounts are the scripts' own (1.0 civilian,
  0.5 droid). Level 1's two `actor_fn.cog` actors arm `SetTimer(1.0, 1, 0, 0)` — the engine's single
  cog timer, which delivers `timer` with sender id 0 (`sithCog_ProcessCog`, `0x4e0300`) — so their
  id-1 credit never runs: 12.5 − 2.0 = 10.5.

.EXAMPLE
  pwsh -File tools/parity/carry-gate.ps1
#>
[CmdletBinding()]
param(
  [string]$Level = '01narshadda',
  [string]$NextLevel = '02narshadda',
  [string]$Route = 'tools/parity/routes/level1-route.tsv',
  [string]$RetailFixture = 'tools/parity/jkauto-02narshadda.jks',
  [string]$Out = 'target/parity/carry',
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
$RetailPath = Resolve-PathUnderRoot $RetailFixture
$OutDir = Resolve-PathUnderRoot $Out
$Exe = Join-Path $Root 'target\release\bevi-knight.exe'

if (-not (Test-Path -LiteralPath $Exe)) {
  throw "missing $Exe; run cargo build --release first (the carry gate boots the port)"
}
if (-not (Test-Path -LiteralPath $RetailPath)) {
  throw "missing the committed retail autosave $RetailPath"
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$Level1Save = Join-Path $OutDir 'level1-carry.jks'
$Level2Save = Join-Path $OutDir 'level2-carry.jks'

# The `.jks` prefix and record framing (`src/jks.rs`, from `sithGamesave_RestoreFile` `0x4dab70`):
# `u32 type, u32 length, payload`, after the 0x68C-byte prefix. Inventory records are type 0x10 and
# their payload's first word is the value, the second is the bin index.
$PrefixBytes = 0x62C + 0x24 + 0x20 + 4 + 0x18

function Read-CarryBins([string]$Path) {
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  $bins = @{}
  $offset = $PrefixBytes
  while ($offset + 8 -le $bytes.Length) {
    $kind = [BitConverter]::ToUInt32($bytes, $offset)
    $length = [BitConverter]::ToUInt32($bytes, $offset + 4)
    $offset += 8
    if ($kind -eq 0x10 -and $length -eq 0x1C) {
      $bin = [int][BitConverter]::ToUInt16($bytes, $offset + 2)
      $bins[$bin] = [BitConverter]::ToSingle($bytes, $offset + 4)
    }
    $offset += $length
  }
  return $bins
}

function Invoke-Port([string[]]$Arguments) {
  $env:BK_HEADLESS = '1'
  try {
    if ($Game) { $Arguments = @('--game', $Game) + $Arguments }
    & $Exe @Arguments *> (Join-Path $OutDir 'carry.log')
  } finally {
    Remove-Item Env:BK_HEADLESS -ErrorAction SilentlyContinue
  }
  if ($LASTEXITCODE -ne 0) { throw "the port exited $LASTEXITCODE; see $(Join-Path $OutDir 'carry.log')" }
}

Write-Host 'carry gate: playing level 1''s route to the level end...'
Remove-Item -LiteralPath $Level1Save -Force -ErrorAction SilentlyContinue
Invoke-Port @('--start', $Level, '--replay', $RoutePath, '--carry-out', $Level1Save, '--no-ai', '--mute')
if (-not (Test-Path -LiteralPath $Level1Save)) { throw "no level-1 carry at $Level1Save" }

$level1 = Read-CarryBins $Level1Save
if ($level1.Count -ne 82) {
  Write-Host "FAIL  level 1: the carry has $($level1.Count) bin(s), the engine's table has 82"
  exit 1
}
if ([math]::Abs($level1[73] - 10.5) -gt 1e-4) {
  Write-Host "FAIL  level 1: population bin 73 = $($level1[73]), the route credits 10.5"
  exit 1
}
if ([math]::Abs($level1[75] - 20.0) -gt 1e-4) {
  Write-Host "FAIL  level 1: the tally did not store alignment 20 in bin 75 (got $($level1[75]))"
  exit 1
}
Write-Host 'PASS  level 1: the route credits bin 73 = 10.5 and the tally stores bin 75 = 20'

Write-Host 'carry gate: booting level 2 with that carry...'
Remove-Item -LiteralPath $Level2Save -Force -ErrorAction SilentlyContinue
Invoke-Port @('--start', $NextLevel, '--carry-in', $Level1Save, '--carry-out', $Level2Save, '--exit-after', '20', '--mute')
if (-not (Test-Path -LiteralPath $Level2Save)) { throw "no level-2 carry at $Level2Save" }

& $Exe --carry-compare $RetailPath $Level2Save
if ($LASTEXITCODE -ne 0) {
  Write-Host 'FAIL  level 2: the carry differs from retail''s jkauto-02narshadda.jks'
  exit 1
}
Write-Host 'PASS  level 2: the carry matches retail''s autosave bin for bin'

# Self-check: change bin 73's value in a copy; the comparator must reject it and name the bin.
$perturbed = Join-Path $OutDir 'level2-carry-perturbed.jks'
$bytes = [System.IO.File]::ReadAllBytes($Level2Save)
$offset = $PrefixBytes
$patched = $false
while ($offset + 8 -le $bytes.Length) {
  $kind = [BitConverter]::ToUInt32($bytes, $offset)
  $length = [BitConverter]::ToUInt32($bytes, $offset + 4)
  $offset += 8
  if ($kind -eq 0x10 -and $length -eq 0x1C -and [BitConverter]::ToUInt16($bytes, $offset + 2) -eq 73) {
    $value = [BitConverter]::GetBytes([single]9.0)
    [Array]::Copy($value, 0, $bytes, $offset + 4, 4)
    $patched = $true
    break
  }
  $offset += $length
}
if (-not $patched) { throw 'the level-2 carry has no bin 73 record to perturb' }
[System.IO.File]::WriteAllBytes($perturbed, $bytes)
$output = & $Exe --carry-compare $RetailPath $perturbed 2>&1
if ($LASTEXITCODE -eq 0) {
  Write-Host 'FAIL  self-check: bin 73 moved to 9.0 and the comparator passed'
  exit 1
}
if (-not (($output -join "`n") -match 'bin 73:')) {
  Write-Host 'FAIL  self-check: the drift was not named by its bin'
  $output | Write-Host
  exit 1
}
Write-Host 'PASS  self-check: a changed population bin is rejected and named'

Write-Host '--- carry gate passed ---'
exit 0
