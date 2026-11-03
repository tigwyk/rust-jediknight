#requires -Version 7
<#
.SYNOPSIS
  Step 10's UI fixture: the retail main menu against the port's, measured.

.DESCRIPTION
  The roadmap asks for "a retail frame pair of a UI screen". Menus are drawn from the shipped
  artwork, so this pair is a direct renderer comparison with no world geometry in it: the retail
  main menu (`retail-capture.ps1 -MainMenuShot`, 640x480, the game's own display mode) and the port
  rendering the same screen at the same size (`--menu singleplayer --shot`, `BK_WINDOW=640x480`).

  Both sides are captured on demand and then scored with ffmpeg's SSIM on grayscale. The floor is
  deliberately high for a UI screen: the layout, fonts and art are the same assets, so the number
  measures the renderer, not the content. -Recapture forces both sides.

.EXAMPLE
  pwsh -File tools/parity/menu-gate.ps1
  pwsh -File tools/parity/menu-gate.ps1 -Recapture -Floor 0.93
#>
[CmdletBinding()]
param(
  [double]$Floor = 0.90,
  [switch]$Recapture,
  [string]$RetailPng = 'target/parity/retail/main-menu.png',
  [string]$PortPng = 'target/parity/port-mainmenu-640.png'
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
  Write-Host ("capturing the retail main menu -> {0}" -f $RetailPng)
  $r = Invoke-Cmd 'pwsh' @('-NoProfile', '-File', 'tools/parity/retail-capture.ps1', '-MainMenuShot',
    '-Shots', 'tools/parity/shots.tsv', '-Only', 'spawn')
  if ($r.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $RetailPng)) {
    Write-Host 'FAIL  the retail main menu could not be captured'
    Write-Host ($r.Text -split "`n" | Select-Object -Last 8 | Out-String).Trim()
    exit 1
  }
}

if ($Recapture -or -not (Test-Path -LiteralPath $PortPng)) {
  Write-Host ("capturing the port's main menu -> {0}" -f $PortPng)
  $env:BK_WINDOW = '640x480'
  try {
    $p = Invoke-Cmd 'target/release/bevi-knight.exe' @('--game', $Game, '--menu', 'singleplayer',
      '--shot', $PortPng)
  } finally {
    Remove-Item Env:BK_WINDOW -ErrorAction SilentlyContinue
  }
  if ($p.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $PortPng)) {
    Write-Host 'FAIL  the port main-menu capture failed'
    Write-Host ($p.Text -split "`n" | Select-Object -Last 8 | Out-String).Trim()
    exit 1
  }
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
Write-Host ("menu gate: main menu ssim {0:N4} (floor {1:N2})  {2}" -f $ssim, $Floor, $verdict)
if ($verdict -eq 'FAIL') { exit 1 }
