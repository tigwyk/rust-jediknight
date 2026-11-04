#requires -Version 7
<#
.SYNOPSIS
  Step 8's frame pair: retail's opening film against the port's, at one pinned frame.

.DESCRIPTION
  The roadmap's next rung for stage 8 asks for "the same movie frame on both sides". The port's
  film path is deterministic now (`--cutscene <file> <start>` plus `BK_FILM_HOLD=<frame>` pins the
  decoded frame), so the pair needs no wall-clock alignment: the retail side is the opening
  cutscene captured by `retail-capture.ps1 -IntroShotMs` (640x480, the game's own display mode),
  and the port renders the same 640x480 screen with `BK_WINDOW=640x480`.

  The crawl frame is static for ~90 film frames (91..178), so the retail capture's own clock does
  not have to hit an exact frame: any frame in that hold shows the same picture. The floor is high
  because both sides are the install's own 8-bit palette art drawn by a palette blit - the number
  measures the port's film path, not content matching.

  Both sides are captured on demand and then scored with ffmpeg's SSIM on grayscale. The retail
  side takes over the display, so its capture only runs when the fixture is missing or -Recapture
  is passed.

.EXAMPLE
  pwsh -File tools/parity/film-gate.ps1
  pwsh -File tools/parity/film-gate.ps1 -Floor 0.999 -FilmFrame 147
#>
[CmdletBinding()]
param(
  [double]$Floor = 0.99,
  [switch]$Recapture,
  # A frame inside the crawl's static hold (film frames 91..178).
  [int]$FilmFrame = 147,
  [string]$RetailPng = 'target/parity/retail/intro-cutscene.png',
  [string]$PortPng = 'target/parity/port-intro-640.png'
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $Root
$Game = 'G:/Games/Star Wars Jedi Knight - Dark Forces 2'

function Invoke-Cmd([string]$File, [string[]]$ArgList) {
  $out = & $File @ArgList 2>&1
  return [pscustomobject]@{ ExitCode = $LASTEXITCODE; Text = ($out -join "`n") }
}

if ($Recapture -or -not (Test-Path -LiteralPath $RetailPng)) {
  Write-Host ("capturing the retail opening film -> {0}" -f $RetailPng)
  $r = Invoke-Cmd 'pwsh' @('-NoProfile', '-File', 'tools/parity/retail-capture.ps1',
    '-IntroShotMs', '7000')
  if ($r.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $RetailPng)) {
    Write-Host 'FAIL  the retail opening film could not be captured'
    Write-Host ($r.Text -split "`n" | Select-Object -Last 8 | Out-String).Trim()
    exit 1
  }
}

Write-Host ("capturing the port's film frame {0} -> {1}" -f $FilmFrame, $PortPng)
$env:BK_WINDOW = '640x480'
$env:BK_FILM_HOLD = "$FilmFrame"
try {
  $p = Invoke-Cmd 'target/release/bevi-knight.exe' @('--game', $Game, '--cutscene', '01-02A.SMK',
    '0', '--shot', $PortPng)
} finally {
  Remove-Item Env:BK_WINDOW -ErrorAction SilentlyContinue
  Remove-Item Env:BK_FILM_HOLD -ErrorAction SilentlyContinue
}
if ($p.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $PortPng)) {
  Write-Host 'FAIL  the port film capture failed'
  Write-Host ($p.Text -split "`n" | Select-Object -Last 8 | Out-String).Trim()
  exit 1
}

# SSIM on grayscale, scaled to the smaller of the two sizes so a display-mode change cannot throw.
$chain = '[0:v]scale=640:480:flags=bilinear,setsar=1,format=gray[a];' +
  '[1:v]scale=640:480:flags=bilinear,setsar=1,format=gray[b];[a][b]ssim'
$ff = Invoke-Cmd 'ffmpeg' @('-hide_banner', '-loglevel', 'info', '-i', $RetailPng, '-i', $PortPng,
  '-filter_complex', $chain, '-f', 'null', '-')
if ($ff.ExitCode -ne 0) {
  Write-Host 'FAIL  ffmpeg ssim failed'
  Write-Host ($ff.Text -split "`n" | Select-Object -Last 8 | Out-String).Trim()
  exit 1
}
$m = [regex]::Match($ff.Text, 'SSIM Y:([0-9.]+) \(([^)]+)\) All:([0-9.]+)')
if (-not $m.Success) {
  Write-Host 'FAIL  ffmpeg printed no SSIM line'
  exit 1
}
$ssim = [double]$m.Groups[3].Value
$verdict = if ($ssim -ge $Floor) { 'PASS' } else { 'FAIL' }
Write-Host ("film gate: opening film ssim {0:N4} (floor {1:N2})  {2}" -f $ssim, $Floor, $verdict)
if ($verdict -eq 'FAIL') { exit 1 }
