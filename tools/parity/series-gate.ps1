#requires -Version 7
<#
.SYNOPSIS
  Live-scene parity: does the port's scene change over time at the same rate as retail's?

.DESCRIPTION
  `compare.ps1` scores one frame against one retail frame. That is the right oracle for a still scene and the
  wrong one for a room full of NPCs: two ~10 s simulations of the same room diverge in *which* patron is where,
  so a single-frame SSIM measures phase, not fidelity. Retail already records the fair measure in every sidecar:
  a **1 frame/second** series (`retail-capture.ps1 -SeriesFrames`, `$remaining = 1000 - elapsed`) and
  `series_ssim_first_last` over it - 7 s for the default eight frames.

  This gate reproduces the same measure on the port: two captures over the same span (`--shot` steps the
  simulation at a fixed 1/60 s, so 7 s is 420 frames), the HUD band trimmed the way `compare.ps1` trims it, and
  ffmpeg SSIM between them. A pair passes when the port's span scores within `-Tolerance` of retail's.

.EXAMPLE
  pwsh -NoProfile -File tools/parity/series-gate.ps1
#>
[CmdletBinding()]
param(
  # Patterns to run (default: every `<name>-series-*.png` retail set with a sidecar).
  [string[]]$Only = @('spawn'),
  # |port - retail| allowance. The default eight-frame series spans 7 s at 1 frame/s.
  [double]$Tolerance = 0.05,
  [int]$SpanSeconds = 7,
  # The HUD band is the bottom 22 % of the crop, the same share `compare.ps1` drops.
  [double]$HudBand = 0.22,
  # Frames to let the level load and fade in before the first capture: frame 15 was still a loading frame on
  # some runs (`spawn` scored 0.21 one run and 0.93 the next with the same command), so the series starts after
  # one second of settled game time.
  [int]$SettleFrames = 60
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
$Root = (Get-Location).Path
$Exe = Join-Path $Root 'target\release\bevi-knight.exe'
$RetailDir = Join-Path $Root 'target/parity/retail'
$OutDir = Join-Path $Root 'target/parity/series'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

if (-not (Test-Path -LiteralPath $Exe)) { throw "missing $Exe; run cargo build --release first" }
if (-not (Get-Command ffmpeg -ErrorAction SilentlyContinue)) { throw 'ffmpeg is required for the SSIM step' }

# The window the retail frame was compared at, so the two port captures are the same shape as usual.
function Get-RetailView([string]$Name) {
  $side = Join-Path $RetailDir ("{0}.json" -f $Name)
  if (-not (Test-Path -LiteralPath $side)) { return $null }
  $json = Get-Content -LiteralPath $side -Raw | ConvertFrom-Json
  if ($null -eq $json.series_ssim_first_last) { return $null }
  # A sidecar whose requested pose did not verify is not a comparison at that pose: `retail-capture.ps1`
  # records `pose_verified` (round 284) and refuses to write one at all when the mismatch is large, so
  # this catches fixtures captured before that guard existed.
  if ($json.requested_pose -and $null -ne $json.pose_verified -and $json.pose_verified -ne $true) {
    return [pscustomobject]@{ Ssim = $null; Level = $json.level; Note = 'retail capture did not verify its requested pose' }
  }
  # The retail side's own interval: `series_times_ms` is recorded by `retail-capture.ps1` (one offset per
  # frame). Older sidecars carry only the 1 s design spacing, so the span falls back to that.
  $times = @($json.series_times_ms | Where-Object { $null -ne $_ })
  $spanMs = if ($null -ne $json.series_span_ms -and [double]$json.series_span_ms -gt 0) {
    [double]$json.series_span_ms
  } elseif ($times.Count -ge 2) {
    [double]$times[$times.Count - 1] - [double]$times[0]
  } else { $null }
  if ($json.first_frame_is_player_camera -ne $true) {
    $when = if ($null -eq $spanMs) {
      'no recorded series times (re-capture with -SeriesFrames to record them)'
    } else {
      ('series spans {0:N0} ms inside the scripted opening' -f $spanMs)
    }
    return [pscustomobject]@{
      Ssim = $null; Level = $json.level; SpanMs = $spanMs
      Note = "series is a scripted intro camera: $when"
    }
  }
  return [pscustomobject]@{
    Ssim = [double]$json.series_ssim_first_last; Level = $json.level; SpanMs = $spanMs; Note = ''
  }
}

# The pair's pose from `shots.tsv`: the retail series was captured at the warped reference pose (a static
# camera), so the port must compare at the same pose or a level whose opening camera moves (15maw's
# `MAcamera`) measures cutscene motion instead of the room.
function Get-RetailPose([string]$Name) {
  $shots = Join-Path $Root 'tools/parity/shots.tsv'
  foreach ($line in Get-Content -LiteralPath $shots) {
    if ($line -match '^\s*(#|$)') { continue }
    $f = $line -split ([char]9)
    if ($f.Length -lt 7 -or $f[0].Trim() -ne $Name) { continue }
    if ($f[2].Trim() -eq '-') { return $null }
    # shots.tsv columns: name, level, x, y, z, pitch, yaw; `--shot-at` wants x,y,z,yaw,pitch, so yaw ($f[6])
    # is argument 4 and pitch ($f[5]) argument 3.
    return '{0},{1},{2},{4},{3}' -f $f[2].Trim(), $f[3].Trim(), $f[4].Trim(), $f[5].Trim(), $f[6].Trim()
  }
  return $null
}

# ffmpeg SSIM over the world band of two PNGs.
function Get-WorldBandSsim([string]$A, [string]$B) {
  $probe = & ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0 $A
  $parts = "$probe".Trim() -split ','
  $w = [int]$parts[0]; $h = [int]$parts[1]
  $band = [int][math]::Round($h * (1.0 - $HudBand))
  $filter = "[0]crop=${w}:${band}:0:0[a];[1]crop=${w}:${band}:0:0[b];[a][b]ssim"
  $out = & ffmpeg -hide_banner -i $A -i $B -lavfi $filter -f null - 2>&1
  $line = ($out | Select-String -Pattern 'All:([0-9.]+)' | Select-Object -Last 1)
  if (-not $line) { throw "ffmpeg produced no SSIM for $A vs $B" }
  return [double]$line.Matches[0].Groups[1].Value
}

$failed = 0
$graded = 0
foreach ($name in $Only) {
  $retail = Get-RetailView $name
  if ($null -eq $retail) {
    Write-Host ("{0,-16} SKIP  no retail sidecar (or no series_ssim_first_last)" -f $name)
    continue
  }
  if ($null -eq $retail.Ssim) {
    Write-Host ("{0,-16} SKIP  {1}" -f $name, $retail.Note)
    continue
  }
  # The port's span is the retail series' **measured** interval when the sidecar has one, at the capture
  # step's fixed 1/60 s; otherwise the design spacing (`-SpanSeconds`). Two runs of the same engine can cover
  # different wall-clock windows (retail renders slowly under dgVoodoo), so the number comes from the sidecar
  # rather than from an assumption about it.
  $frames = if ($null -ne $retail.SpanMs -and $retail.SpanMs -gt 0) {
    [int][math]::Round($retail.SpanMs / 1000.0 * 60.0)
  } else {
    $SpanSeconds * 60
  }
  $a = Join-Path $OutDir ("{0}-start.png" -f $name)
  $b = Join-Path $OutDir ("{0}-end.png" -f $name)
  $env:BK_WINDOW = $null
  $pose = Get-RetailPose $name
  $extra = if ($null -eq $pose) { @() } else { @('--shot-at', $pose, '--warp', ($pose -replace ',[^,]+$', '')) }
  & $Exe --start $retail.Level --frames $SettleFrames --shot $a --mute @extra *> (Join-Path $OutDir ("{0}-start.log" -f $name))
  & $Exe --start $retail.Level --frames ($SettleFrames + $frames) --shot $b --mute @extra *> (Join-Path $OutDir ("{0}-end.log" -f $name))
  $port = Get-WorldBandSsim $a $b
  $diff = [math]::Abs($port - $retail.Ssim)
  $verdict = if ($diff -le $Tolerance) { 'PASS' } else { 'FAIL' }
  $graded++
  if ($verdict -eq 'FAIL') { $failed++ }
  Write-Host ("series {0,-14} port {1:N4} / retail {2:N4} over {3:N1} s ({4} port frames)  |diff| {5:N4} <= {6:N2}  {7}" -f `
    $name, $port, $retail.Ssim, ($frames / 60.0), $frames, $diff, $Tolerance, $verdict)
}
Write-Host ("series gate: {0} graded of {1} requested, {2} failure(s)" -f $graded, $Only.Count, $failed)
if ($failed -gt 0) { exit 1 }
