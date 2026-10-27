<#
.SYNOPSIS
  Repeatable screenshot capture from the real GOG Star Wars Jedi Knight: Dark Forces II
  (JK.EXE) for retail-vs-port parity checks.

.DESCRIPTION
  Drives the retail game the way target/parity/NOTES.md documents it:
    * copies the install to a scratch WorkCopy (the real install is never modified),
    * stages dgVoodoo2 into the copy and sets the HKCU VirtualStore b3DAccel=1 key (never HKLM),
      because the 1997 build's Direct3D path calls the system D3DIM.DLL, which crashes on Windows 11;
      the wrapper's 16-bit renderer works and F12 writes JSHOT##.BMP (24-bit) instead of the
      palettised 8-bit PCX. `-Software` forces the old software path (b3DAccel=0),
    * launches JK.EXE -devMode from the scratch copy and skips the intro movies,
    * mouse-navigates Players -> Ok -> Single Player -> New Game (640x480 fullscreen,
      screen coords == game coords for the menus),
    * waits for the level-parse line in the dev-console ListBox (id 1037),
    * writes the pose with `warp` when a row has an explicit pose,
    * captures a time series of F12 screenshots plus the settled reference screenshot,
    * reads `coords` through the dev-console dialog (best effort; falls back to the known
      walkplayer pose for the level's own spawn row),
    * converts JSHOT*.PCX/BMP raw frames to PNG with ffmpeg, detects the world box
      (letterbox + -devMode corner overlays are stripped) and saves <name>.png plus
      <name>-raw.png and <name>.json.

  Input rows come from tools/parity/shots.tsv (owned by the Lead; never edited here).

.PARAMETER Shots
  TSV with header comments and columns: name, level, x, y, z, pitch, yaw.
  A pose of "-" means the level's own spawn pose (no warp; pose read from `coords`).

.PARAMETER Out
  Output directory (PNG, JSON, retail.log, <name>-series-*.png).

.PARAMETER GamePath
  Retail install root. Default: resolve the GOG install (JK.EXE + Resource) via
  HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\*\path.

.PARAMETER WorkCopy
  Scratch copy of the install that gets launched and modified (player profile,
  JSHOT files). Default: target/parity/retailcopy.

.PARAMETER SeriesFrames
  Number of F12 screenshots at ~1 s intervals right after the level loads (0 disables).

.PARAMETER Software
  Force the old software renderer path (b3DAccel=0, 8-bit JSHOT##.PCX). The default is the
  dgVoodoo wrapper's 16-bit path (b3DAccel=1, 24-bit JSHOT##.BMP).

.EXAMPLE
  pwsh -File tools/parity/retail-capture.ps1
#>
[CmdletBinding()]
param(
  [string]$Shots = 'tools/parity/shots.tsv',
  [string]$Out = 'target/parity/retail',
  [string]$GamePath,
  [string]$WorkCopy = 'target/parity/retailcopy',
  [int]$SeriesFrames = 8,
  [int]$LevelTimeoutSec = 45,
  [switch]$Software
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
function Resolve-FromRepo([string]$Path) {
  if ([System.IO.Path]::IsPathRooted($Path)) { return [System.IO.Path]::GetFullPath($Path) }
  return [System.IO.Path]::GetFullPath((Join-Path $repoRoot $Path))
}

$Shots = Resolve-FromRepo $Shots
$Out = Resolve-FromRepo $Out
$WorkCopy = Resolve-FromRepo $WorkCopy
if ($GamePath) { $GamePath = Resolve-FromRepo $GamePath }

. (Join-Path $PSScriptRoot 'retail-lib.ps1')
. (Join-Path $PSScriptRoot 'display.ps1')

# Pre-run desktop modes. The game restores the monitor itself when it shuts down normally (WM_CLOSE);
# this baseline is the safety net for a crash / force-kill / interrupted run.
$script:DisplayStatePath = Join-Path $repoRoot 'target/parity/display-pre.json'

New-Item -ItemType Directory -Force -Path $Out | Out-Null
$script:LogPath = Join-Path $Out 'retail.log'
function Log([string]$Message) { Write-JkLog -Message $Message -LogPath $script:LogPath }

# Known walkplayer spawn poses used only when `coords` cannot be read (best-effort metadata).
# Level 1 value is from target/parity/NOTES.md (verified manual capture).
# `shots.tsv` gives the **camera** eye position (what the port's `--shot-at` takes), so a `warp`
# (which moves the player *body*) is made at `z - eye offset`. The engine builds the first-person
# view from the body matrix pre-translated by `thing + 0x168` (`sithCamera_Update`, 0x4c56b0), and
# that field is the template's `eyeoffset=(0,0,0.037)`.
$script:JkEyeOffset = 0.037
$script:WalkplayerPoses = @{
  '01narshadda' = [pscustomobject]@{
    Raw = 'fallback: level walkplayer pose (target/parity/NOTES.md)'
    Pos = @(-10.44, -5.46, -1.13)
    Pyr = @(1.04, -59.98, 1.04)
    Sector = 339
  }
}

function Assert-JkInstall([string]$Path, [string]$Label) {
  if (-not (Test-Path -LiteralPath $Path)) { throw "$Label not found: $Path" }
  $exe = Join-Path $Path 'JK.EXE'
  $resources = Join-Path $Path 'Resource'
  if (-not (Test-Path -LiteralPath $exe)) { throw "$Label is not a Jedi Knight install (missing JK.EXE): $Path" }
  if (-not (Test-Path -LiteralPath $resources)) { throw "$Label is not a Jedi Knight install (missing Resource\): $Path" }
}

function Resolve-JkGamePath([string]$Explicit) {
  if ($Explicit) { Assert-JkInstall $Explicit 'retail install (-GamePath)'; return $Explicit }
  $gamesKey = 'HKLM:\SOFTWARE\WOW6432Node\GOG.com\Games'
  if (Test-Path $gamesKey) {
    foreach ($game in Get-ChildItem -Path $gamesKey) {
      $candidate = $null
      try { $candidate = (Get-ItemProperty -Path $game.PSPath -Name 'path' -ErrorAction Stop).path } catch { }
      if (-not $candidate) { continue }
      if ((Test-Path -LiteralPath (Join-Path $candidate 'JK.EXE')) -and (Test-Path -LiteralPath (Join-Path $candidate 'Resource'))) {
        return [System.IO.Path]::GetFullPath($candidate)
      }
    }
  }
  throw "retail install not found: no GOG registry entry under $gamesKey has JK.EXE + Resource. Pass -GamePath '<install dir>'."
}

function Read-ShotList([string]$Path) {
  if (-not (Test-Path -LiteralPath $Path)) { throw "shots file not found: $Path" }
  $rows = New-Object System.Collections.Generic.List[object]
  $lineNo = 0
  foreach ($raw in Get-Content -LiteralPath $Path) {
    $lineNo++
    $line = $raw.TrimEnd()
    if ($line -match '^\s*$' -or $line -match '^\s*#') { continue }
    $cols = $line -split "`t"
    if ($cols.Count -eq 7 -and $cols[0].Trim() -eq 'name' -and $cols[1].Trim() -eq 'level') { continue }  # header row
    if ($cols.Count -ne 7) { throw "shots file line ${lineNo}: expected 7 tab-separated columns, got $($cols.Count)" }
    $pose = @($cols[2], $cols[3], $cols[4], $cols[5], $cols[6])
    if ($pose[0] -eq '-') {
      $pose = $null
    } else {
      foreach ($field in $pose) {
        $parsed = 0.0
        if (-not [double]::TryParse($field, [ref]$parsed)) { throw "shots file line ${lineNo}: pose value '$field' is not a number" }
      }
      $pose = @($pose | ForEach-Object { [double]$_ })
    }
    $rows.Add([pscustomobject]@{ Name = $cols[0]; Level = $cols[1]; Pose = $pose; Line = $lineNo })
  }
  if ($rows.Count -eq 0) { throw "shots file has no capture rows: $Path" }
  return $rows
}

function Invoke-JkStaging([string]$Source, [string]$Destination) {
  if (-not (Test-Path -LiteralPath $Destination)) { New-Item -ItemType Directory -Force -Path $Destination | Out-Null }
  Log "staging scratch copy: robocopy '$Source' -> '$Destination' (incremental)"
  & robocopy $Source $Destination /E /R:1 /W:1 /MT:16 /NFL /NDL /NP /NJH /NJS | Out-Null
  $rc = $LASTEXITCODE
  if ($rc -ge 8) { throw "robocopy failed staging the scratch copy (exit $rc)" }
  Assert-JkInstall $Destination 'scratch copy'
  Log "scratch copy staged (robocopy exit $rc)"

  # Unlock the gated dev-console verbs (`warp` for the pose rows, `cogstatus`, ...) in the copy.
  # `Set-JkDevCommandsEnabled` verifies the unique site and writes the six NOPs; the real install is
  # never touched.
  $null = Set-JkDevCommandsEnabled -ExePath (Join-Path $Destination 'JK.EXE') -LogPath $script:LogPath

  # The 1997 Direct3D path calls the system D3DIM.DLL, which crashes on Windows 11, so the 16-bit
  # renderer needs dgVoodoo2's DDraw/D3DImm in the copy (see Install-Jk3DWrapper).
  if (-not $Software) {
    Install-Jk3DWrapper -Destination $Destination -LogPath $script:LogPath
  }

  # The scratch profile (never the real install) gets cutscenes disabled.
  $plr = Join-Path $Destination 'player\Lee\Lee.plr'
  if (-not (Test-Path -LiteralPath $plr)) { throw "scratch player profile missing: $plr" }
  $text = Get-Content -LiteralPath $plr -Raw
  if ($text -match '(?m)^disablecutscenes\s+\S+') {
    $updated = [regex]::Replace($text, '(?m)^disablecutscenes\s+\S+', 'disablecutscenes 1')
  } else {
    $updated = $text.TrimEnd() + "`r`ndisablecutscenes 1`r`n"
  }
  if ($updated -ne $text) {
    Set-Content -LiteralPath $plr -Value $updated -NoNewline -Encoding ascii
    Log 'set disablecutscenes 1 in scratch player\Lee\Lee.plr'
  } else {
    Log 'scratch player\Lee\Lee.plr already has disablecutscenes 1'
  }

  # Stale screenshots from an earlier run must not be mistaken for new ones.
  Get-ChildItem -Path (Join-Path $Destination 'JSHOT*') -File -ErrorAction SilentlyContinue | Remove-Item -Force
}

function Assert-NoConflictingEngine {
  if (Get-Process -Name 'JK' -ErrorAction SilentlyContinue) {
    throw 'JK.EXE is already running. Close it before running the retail capture harness (only one engine at a time).'
  }
  $port = Get-Process -Name 'bevi-knight' -ErrorAction SilentlyContinue
  if ($port) {
    Log 'port (bevi-knight) is running; waiting up to 20 s for it to exit (only one engine at a time)'
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt 20 -and (Get-Process -Name 'bevi-knight' -ErrorAction SilentlyContinue)) {
      Start-Sleep -Milliseconds 500
    }
    if (Get-Process -Name 'bevi-knight' -ErrorAction SilentlyContinue) {
      throw 'port (bevi-knight) is still running; refusing to launch JK.EXE while another engine is active.'
    }
    Log 'port exited; continuing'
  }
}

# One F12 screenshot: press F12, wait for a new JSHOT*.PCX/BMP, convert it to PNG.
function Invoke-JkF12Shot {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][string]$PngPath,
    [string]$Label = ''
  )
  $known = @(Get-ChildItem -Path (Join-Path $WorkCopy 'JSHOT*') -File -ErrorAction SilentlyContinue | ForEach-Object { $_.Name })
  $found = $null
  foreach ($attempt in 1..2) {
    Send-JkKey 'F12'
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $lastLength = -1
    $stable = 0
    while ($sw.Elapsed.TotalSeconds -lt 6) {
      $candidate = Get-ChildItem -Path (Join-Path $WorkCopy 'JSHOT*') -File -ErrorAction SilentlyContinue |
        Where-Object { $known -notcontains $_.Name } | Sort-Object Name | Select-Object -First 1
      if ($candidate) {
        Start-Sleep -Milliseconds 150
        $candidate.Refresh()
        if ($candidate.Length -gt 0 -and $candidate.Length -eq $lastLength) {
          $stable++
          if ($stable -ge 2) { $found = $candidate; break }
        } else {
          $lastLength = $candidate.Length
          $stable = 0
        }
      }
      Start-Sleep -Milliseconds 150
    }
    if ($found) { break }
    Log "F12 attempt $attempt produced no JSHOT file; activating the window and retrying"
    Send-JkClick -X 10 -Y 10
    Start-Sleep -Milliseconds 300
  }
  if (-not $found) { throw 'F12 produced no JSHOT file within 12 s (screenshot key not reaching the game?)' }
  Convert-JkShotToPng -Source $found.FullName -Destination $PngPath -LogPath $script:LogPath
  $raw = [pscustomobject]@{ RawName = $found.Name; RawBytes = $found.Length }
  Remove-Item -LiteralPath $found.FullName -Force
  Log ("$Label captured $($raw.RawName) ($($raw.RawBytes) bytes) -> $(Split-Path -Leaf $PngPath)")
  return $raw
}

# Execute one console command through the dev-console dialog. The dialog only runs a command
# when its Edit control (id 1024) has focus in the game's own input queue, so cross-process
# focus is forced; the command is submitted as WM_COMMAND/IDOK (verified against JK.EXE).
# Returns the ListBox lines that appeared after the command.
function Invoke-JkDevConsoleCommand {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][string]$Command,
    [int]$TimeoutMs = 2500
  )
  $console = Get-JkConsoleWindow -ProcessId $ProcessId
  if ($console -eq [IntPtr]::Zero) { throw 'dev console dialog not found' }
  $edit = [JkRetail]::GetDlgItem($console, 1024)
  if ($edit -eq [IntPtr]::Zero) { throw 'dev console Edit control (id 1024) not found' }
  $gameWindow = Get-JkWindow -ProcessId $ProcessId
  $before = @(Get-JkConsoleLines -ProcessId $ProcessId).Count
  [JkRetail]::ForceFocus($edit)
  [JkRetail]::SetControlText($edit, $Command)
  [void][JkRetail]::SendMessage($console, 0x0111, [IntPtr]1, $edit)  # WM_COMMAND, IDOK
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $newLines = @()
  while ($sw.Elapsed.TotalMilliseconds -lt $TimeoutMs) {
    Start-Sleep -Milliseconds 150
    $lines = @(Get-JkConsoleLines -ProcessId $ProcessId)
    if ($lines.Count -gt $before) { $newLines = @($lines | Select-Object -Skip $before); break }
  }
  if ($gameWindow -ne [IntPtr]::Zero) { [JkRetail]::ForceFocus($gameWindow) }
  Start-Sleep -Milliseconds 250
  Log "dev console command '$Command' -> $($newLines.Count) new ListBox line(s)"
  foreach ($entry in $newLines) { Log ("  console| " + $entry) }
  return $newLines
}

function ConvertFrom-JkCoordsLine([string]$Line) {
  if ($Line -match 'Pos:\s*\(\s*(-?[0-9.]+)\s*,\s*(-?[0-9.]+)\s*,\s*(-?[0-9.]+)\s*\)\s*PYR:\s*\(\s*(-?[0-9.]+)\s*,\s*(-?[0-9.]+)\s*,\s*(-?[0-9.]+)\s*\)\s*Sector:\s*([0-9]+)') {
    return [pscustomobject]@{
      Raw = $Line
      Pos = @([double]$Matches[1], [double]$Matches[2], [double]$Matches[3])
      Pyr = @([double]$Matches[4], [double]$Matches[5], [double]$Matches[6])
      Sector = [int]$Matches[7]
    }
  }
  return $null
}

# Best-effort pose read: at most 2 attempts, then $null (the caller never fails on this).
function Read-JkCoords {
  param([Parameter(Mandatory)][int]$ProcessId)
  for ($attempt = 1; $attempt -le 2; $attempt++) {
    $newLines = @(Invoke-JkDevConsoleCommand -ProcessId $ProcessId -Command 'coords')
    $hit = @($newLines | Where-Object { $_ -match '\bPos:' } | Select-Object -Last 1)
    if ($hit.Count -gt 0) {
      $coords = ConvertFrom-JkCoordsLine $hit[0]
      if ($coords) { return $coords }
      Log "coords line did not match the expected format: $($hit[0])"
    }
    Log "coords attempt $attempt produced no parseable Pos/PYR line"
    Start-Sleep -Milliseconds 400
  }
  return $null
}

# Wait until FFmpeg reports the game window as bright (the Players screen is a bright static frame;
# the intro movies are dark). Returns the luma average or $null.
function Wait-JkBrightScreen {
  param([int]$TimeoutSec = 20, [double]$MinYavg = 55.0)
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $yavg = $null
  while ($sw.Elapsed.TotalSeconds -lt $TimeoutSec) {
    $yavg = Get-JkWindowYavg -ScratchDir $Out -LogPath $script:LogPath
    if ($null -ne $yavg -and $yavg -gt $MinYavg) { return $yavg }
    Start-Sleep -Milliseconds 300
  }
  return $yavg
}

# Full-frame first-vs-last SSIM (includes animated characters and any dev overlay).
function Get-JkSsimFirstLast {
  param([string]$First, [string]$Last)
  $output = & ffmpeg -hide_banner -nostats -i $First -i $Last -lavfi ssim -f null - 2>&1
  foreach ($line in $output) {
    if ("$line" -match 'All:([0-9.]+)') { return [double]$Matches[1] }
  }
  return $null
}

# SSIM of one crop region shared by two frames, e.g. a static wall area inside the world box.
function Get-JkRegionSsim {
  param([string]$First, [string]$Last, [int]$X, [int]$Y, [int]$W, [int]$H)
  if ($W -le 1 -or $H -le 1) { return $null }
  $crop = "$W`:$H`:$X`:$Y"
  $output = & ffmpeg -hide_banner -nostats -i $First -i $Last -lavfi "[0:v]crop=$crop[a];[1:v]crop=$crop[b];[a][b]ssim" -f null - 2>&1
  foreach ($line in $output) {
    if ("$line" -match 'All:([0-9.]+)') { return [double]$Matches[1] }
  }
  return $null
}

# Detect the world box in a raw retail frame.
#
# The 3D path's JSHOT BMP holds the world view letterboxed inside the display mode, plus HUD corners
# and the "Screenshot Taken" notification in the black margins, so a bounding box of *any* non-black
# pixel covers nearly the whole frame. Instead this counts, per row and per column, how much of it is
# lit: the world view rows/columns are mostly content (>50%), while the HUD and notification are
# small islands in the bars. That also works for the 8-bit PCX path (whose -devMode overlay text sits
# in the top corners the same way).
# Returns @{ X; Y; W; H; Text = "WxH+X+Y"; RowFill; ColFill }.
function Get-JkWorldBox {
  param([Parameter(Mandatory)][string]$RawPng, [string]$LogPath = $null)
  $probe = & ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0 $RawPng 2>$null
  $parts = "$probe".Trim().Split(',')
  if ($parts.Count -lt 2) { throw "ffprobe could not read '$RawPng'" }
  $imageW = [int]$parts[0]
  $imageH = [int]$parts[1]
  $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ('jkworldbox-{0}.gray' -f [guid]::NewGuid().ToString('N'))
  try {
    & ffmpeg -hide_banner -loglevel error -y -i $RawPng -vf 'format=gray' -f rawvideo -pix_fmt gray $tmp 2>$null | Out-Null
    if (-not (Test-Path -LiteralPath $tmp)) { throw "ffmpeg could not dump '$RawPng' to gray" }
    $gray = [System.IO.File]::ReadAllBytes($tmp)
    if ($gray.Length -lt ($imageW * $imageH)) { throw "gray dump of '$RawPng' is short ($($gray.Length) bytes)" }
    $threshold = 16
    $rowHits = New-Object 'int[]' $imageH
    $colHits = New-Object 'int[]' $imageW
    $i = 0
    for ($y = 0; $y -lt $imageH; $y++) {
      $row = 0
      for ($x = 0; $x -lt $imageW; $x++) {
        if ($gray[$i] -gt $threshold) { $row++; $colHits[$x]++ }
        $i++
      }
      $rowHits[$y] = $row
    }
    $minX = -1; $maxX = -1; $minY = -1; $maxY = -1
    for ($x = 0; $x -lt $imageW; $x++) {
      if ($colHits[$x] / [double]$imageH -gt 0.5) { if ($minX -lt 0) { $minX = $x }; $maxX = $x }
    }
    for ($y = 0; $y -lt $imageH; $y++) {
      if ($rowHits[$y] / [double]$imageW -gt 0.5) { if ($minY -lt 0) { $minY = $y }; $maxY = $y }
    }
    $x = $minX; $y = $minY; $w = $maxX - $minX + 1; $h = $maxY - $minY + 1
    if ($minX -lt 0 -or $minY -lt 0 -or $w -lt 64 -or $h -lt 64) {
      throw "could not detect the world view in '$RawPng' (rows/columns above the fill threshold: none)"
    }
    $result = [pscustomobject]@{
      X = $x; Y = $y; W = $w; H = $h
      Text = ('{0}x{1}+{2}+{3}' -f $w, $h, $x, $y)
      ImageWidth = $imageW; ImageHeight = $imageH
      RowFill = (($rowHits[$y] / [double]$imageW) + ($rowHits[$maxY] / [double]$imageW)) / 2.0
      ColFill = (($colHits[$x] / [double]$imageH) + ($colHits[$maxX] / [double]$imageH)) / 2.0
    }
    if ($LogPath) {
      Write-JkLog -Message ("world box detected: {0} of {1}x{2} (edge fill: rows {3:P0}, columns {4:P0})" -f `
          $result.Text, $imageW, $imageH, $result.RowFill, $result.ColFill) -LogPath $LogPath
    }
    return $result
  } finally {
    if (Test-Path -LiteralPath $tmp) { Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue }
  }
}

function Crop-JkPng {
  param(
    [Parameter(Mandatory)][string]$Source,
    [Parameter(Mandatory)][string]$Destination,
    [Parameter(Mandatory)][int]$W,
    [Parameter(Mandatory)][int]$H,
    [Parameter(Mandatory)][int]$X,
    [Parameter(Mandatory)][int]$Y
  )
  $code = Invoke-JkFfmpeg -Arguments @('-loglevel', 'error', '-y', '-i', $Source, '-vf', "crop=${W}:${H}:${X}:${Y}", $Destination) -LogPath $script:LogPath
  if ($code -ne 0 -or -not (Test-Path -LiteralPath $Destination)) {
    throw "ffmpeg failed to crop world box from '$Source'"
  }
}

# YAVG of a thin strip (left/right edge) of the saved image; used to prove the crop removed the bars.
function Get-JkEdgeYavg {
  param([Parameter(Mandatory)][string]$Path, [ValidateSet('left', 'right')][string]$Side)
  $probe = & ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0 $Path 2>$null
  $parts = "$probe".Trim().Split(',')
  $w = [int]$parts[0]; $h = [int]$parts[1]
  $x = if ($Side -eq 'left') { 0 } else { $w - 2 }
  $output = & ffmpeg -hide_banner -nostats -i $Path -vf "crop=2:${h}:${x}:0,signalstats,metadata=print:key=lavfi.signalstats.YAVG" -frames:v 1 -f null - 2>&1
  foreach ($line in $output) {
    if ("$line" -match 'lavfi\.signalstats\.YAVG=([0-9.]+)') { return [double]$Matches[1] }
  }
  return $null
}

function Invoke-JkCaptureRow {
  param([Parameter(Mandatory)]$Row)

  $name = $Row.Name
  $level = $Row.Level
  $requestedPose = $Row.Pose
  $poseText = if ($null -eq $requestedPose) { 'spawn (-)' } else { ($requestedPose -join ',') }
  Log "=== capture '$name' level=$level requested pose=$poseText (shots line $($Row.Line)) ==="
  Assert-NoConflictingEngine

  $process = $null
  $gameWindow = [IntPtr]::Zero
  $consoleWindow = [IntPtr]::Zero
  try {
    $exe = Join-Path $WorkCopy 'JK.EXE'
    $process = Start-Process -FilePath $exe -ArgumentList '-devMode' -WorkingDirectory $WorkCopy -PassThru
    Log "launched JK.EXE -devMode (pid=$($process.Id), cwd=$WorkCopy)"

    # Window + dev console dialog.
    $gameWindow = [IntPtr]::Zero
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt 20) {
      $gameWindow = Get-JkWindow -ProcessId $process.Id
      if ($gameWindow -ne [IntPtr]::Zero) { break }
      Start-Sleep -Milliseconds 250
    }
    if ($gameWindow -eq [IntPtr]::Zero) { throw "game window 'Jedi Knight' did not appear within 20 s" }
    Log "game window hwnd=$gameWindow rect=$([JkRetail]::WindowRectText($gameWindow)) foreground=$([JkRetail]::ForegroundTitle())"

    $consoleWindow = [IntPtr]::Zero
    $sw.Restart()
    while ($sw.Elapsed.TotalSeconds -lt 30) {
      $consoleWindow = Get-JkConsoleWindow -ProcessId $process.Id
      if ($consoleWindow -ne [IntPtr]::Zero) { break }
      Start-Sleep -Milliseconds 250
    }
    if ($consoleWindow -eq [IntPtr]::Zero) { throw "dev console window 'Jedi Console' did not appear; -devMode may not have been honored" }
    Log "dev console dialog hwnd=$consoleWindow (class #32770, ListBox id 1037)"

    # Skip the intro movies: activate the window, ESC, retry until the bright Players screen.
    Start-Sleep -Seconds 2
    Send-JkClick -X 50 -Y 50
    Start-Sleep -Milliseconds 400
    Send-JkKey 'ESC'
    Start-Sleep -Milliseconds 2000
    $bright = Wait-JkBrightScreen -TimeoutSec 3
    $escRetries = 0
    while (($null -eq $bright -or $bright -le 55.0) -and $escRetries -lt 8) {
      $escRetries++
      Log "intro still playing (window yavg=$bright); sending ESC again ($escRetries)"
      Send-JkKey 'ESC'
      Start-Sleep -Milliseconds 1400
      $bright = Wait-JkBrightScreen -TimeoutSec 2
    }
    if ($null -eq $bright -or $bright -le 55.0) { throw "never reached the bright Players screen after $escRetries ESC retries (last window yavg=$bright)" }
    Log "Players screen reached (window yavg=$([math]::Round($bright, 2)))"

    # Menus are mouse-driven (NOTES.md): Players -> Ok, then Single Player -> New Game.
    # Navigation rule (someone clicking the real game found this the hard way): a click on a menu
    # option *commits* to that screen — the only way back to the main menu is that screen's own
    # Ok/Cancel, and clicking a main-menu item while inside a submenu does nothing. So this path
    # always goes one level down, commits, then descends again; never jump sideways between screens.
    Send-JkClick -X 573 -Y 442
    Start-Sleep -Milliseconds 1800
    $menuYavg = Get-JkWindowYavg -ScratchDir $Out -LogPath $script:LogPath
    Log "clicked Players Ok (573,442); window yavg=$menuYavg (main menu expected)"
    Send-JkClick -X 320 -Y 195
    Start-Sleep -Milliseconds 1500
    Log 'clicked Single Player (320,195)'
    Send-JkClick -X 304 -Y 115
    Start-Sleep -Milliseconds 1000
    Log 'clicked New Game (304,115)'

    # Level-load detection: the dev console ListBox logs the parse lines.
    $parseLine = Wait-JkConsoleLine -ProcessId $process.Id -Pattern 'parse section things' -TimeoutSec $LevelTimeoutSec
    if (-not $parseLine) {
      $tail = @(Get-JkConsoleLines -ProcessId $process.Id)
      Log "dev console ListBox tail at failure ($($tail.Count) lines):"
      foreach ($entry in ($tail | Select-Object -Last 25)) { Log ("  console| " + $entry) }
      throw "level did not load within $LevelTimeoutSec s (no 'parse section things' line in the dev-console ListBox)"
    }
    Log "level loaded: $parseLine"

    # Let the load fade finish, then optionally warp to the requested pose (best effort).
    Start-Sleep -Milliseconds 1200
    if ($null -ne $requestedPose) {
      # shots.tsv gives the camera eye; `warp` positions the player body, and the camera rides it at
      # `thing + 0x168` (see $script:JkEyeOffset), so the body goes 0.037 below the requested eye.
      $bodyZ = $requestedPose[2] - $script:JkEyeOffset
      $warp = 'warp {0} {1} {2} {3} {4} 0' -f $requestedPose[0], $requestedPose[1], $bodyZ, $requestedPose[3], $requestedPose[4]
      Log "warp command: $warp"
      $null = Invoke-JkDevConsoleCommand -ProcessId $process.Id -Command $warp
      Start-Sleep -Milliseconds 900
    }

    # Time series: capture the first seconds after the level becomes visible. These are kept as
    # full raw frames (the -devMode overlay in the top corners is harmless for the motion analysis).
    $series = New-Object System.Collections.Generic.List[object]
    for ($i = 0; $i -lt $SeriesFrames; $i++) {
      $frameStart = Get-Date
      $pngPath = Join-Path $Out ("{0}-series-{1}.png" -f $name, $i)
      $shot = Invoke-JkF12Shot -ProcessId $process.Id -PngPath $pngPath -Label ("series[{0}]" -f $i)
      $stats = Get-JkPngStats -Path $pngPath -LogPath $script:LogPath
      $md5 = (Get-FileHash -LiteralPath $pngPath -Algorithm MD5).Hash.ToLowerInvariant()
      $series.Add([pscustomobject]@{
          Index = $i; File = (Split-Path -Leaf $pngPath); Md5 = $md5
          Yavg = if ($null -eq $stats.Yavg) { $null } else { [math]::Round($stats.Yavg, 2) }
          Bytes = $stats.Bytes; Raw = $shot.RawName
        })
      Log ("series[{0}] {1} bytes yavg={2} md5={3}" -f $i, $stats.Bytes, $stats.Yavg, $md5.Substring(0, 8))
      $elapsedMs = ((Get-Date) - $frameStart).TotalMilliseconds
      $remaining = 1000 - $elapsedMs
      if ($remaining -gt 0) { Start-Sleep -Milliseconds ([int]$remaining) }
    }

    # Primary reference screenshot at the settled view.
    Start-Sleep -Milliseconds 900
    $rawPng = Join-Path $Out ("{0}-raw.png" -f $name)
    $spawnPng = Join-Path $Out ("{0}.png" -f $name)
    $spawnShot = Invoke-JkF12Shot -ProcessId $process.Id -PngPath $rawPng -Label 'spawn-raw'
    $rawStats = Get-JkPngStats -Path $rawPng -LogPath $script:LogPath

    # The raw frame has letterbox bars and -devMode overlay blocks in the top corners; save only the
    # detected world box as the comparison reference (robust detection, no hardcoded crop numbers).
    $worldBox = Get-JkWorldBox -RawPng $rawPng -LogPath $script:LogPath
    Crop-JkPng -Source $rawPng -Destination $spawnPng -W $worldBox.W -H $worldBox.H -X $worldBox.X -Y $worldBox.Y
    $spawnStats = Get-JkPngStats -Path $spawnPng -LogPath $script:LogPath
    $leftEdge = Get-JkEdgeYavg -Path $spawnPng -Side 'left'
    $rightEdge = Get-JkEdgeYavg -Path $spawnPng -Side 'right'
    Log ("spawn reference: {0} {1}x{2} {3} bytes sha256={4} yavg={5} world_crop={6} raw={7}" -f `
        (Split-Path -Leaf $spawnPng), $spawnStats.Width, $spawnStats.Height, $spawnStats.Bytes,
        $spawnStats.Sha256, $spawnStats.Yavg, $spawnStats.Crop, (Split-Path -Leaf $rawPng))
    Log ("spawn edge check: left-strip YAVG={0} right-strip YAVG={1} (near-black bars must be gone)" -f $leftEdge, $rightEdge)
    if (($null -ne $leftEdge -and $leftEdge -lt 5.0) -or ($null -ne $rightEdge -and $rightEdge -lt 5.0)) {
      Log 'WARNING: saved reference may still contain near-black bars at an edge; check world-box detection'
    }
    if ($spawnStats.Width -lt 320 -or $spawnStats.Height -lt 240) { throw "spawn reference is unexpectedly small: $($spawnStats.Width)x$($spawnStats.Height)" }
    if ($spawnStats.Bytes -lt 3000) { throw "spawn reference is too small ($($spawnStats.Bytes) bytes) — capture failed" }
    if ($null -eq $spawnStats.Yavg -or $spawnStats.Yavg -lt 12.0) {
      throw "spawn reference is mostly black (YAVG=$($spawnStats.Yavg)); the world did not render — check the renderer (dgVoodoo wrapper staged + b3DAccel=1, or run with -Software)"
    }

    # Best-effort pose from the game console (never fatal; falls back to the walkplayer pose).
    $coordsSource = 'coords'
    $coordsA = Read-JkCoords -ProcessId $process.Id
    $coordsB = $null
    if ($coordsA) {
      Start-Sleep -Milliseconds 700
      $coordsB = Read-JkCoords -ProcessId $process.Id
      Log ("pose from coords: Pos ({0}, {1}, {2}) PYR ({3}, {4}, {5}) Sector {6}" -f `
          $coordsA.Pos[0], $coordsA.Pos[1], $coordsA.Pos[2], $coordsA.Pyr[0], $coordsA.Pyr[1], $coordsA.Pyr[2], $coordsA.Sector)
      if ($coordsB) { Log "coords second read: $($coordsB.Raw)" }
    } else {
      Log 'coords unavailable (pose from walkplayer)'
      $coordsSource = 'walkplayer fallback'
      if ($null -eq $requestedPose -and $script:WalkplayerPoses.ContainsKey($level)) {
        $coordsA = $script:WalkplayerPoses[$level]
        Log ("pose from walkplayer fallback: Pos ({0}, {1}, {2}) PYR ({3}, {4}, {5}) Sector {6}" -f `
            $coordsA.Pos[0], $coordsA.Pos[1], $coordsA.Pos[2], $coordsA.Pyr[0], $coordsA.Pyr[1], $coordsA.Pyr[2], $coordsA.Sector)
      }
    }

    # Warp verification is advisory only: a mismatched pose must not abort the harvest.
    # `shots.tsv` gives the camera eye and the body was warped to `z - JkEyeOffset`, so the position
    # check puts the eye offset back. `warp` sets position and yaw only: pitch/roll keep the level's
    # own view angles (level 1 spawns with PYR pitch 1.04), so pitch is reported but not required.
    $warpVerified = $null
    if ($null -ne $requestedPose) {
      if ($coordsA -and $coordsSource -eq 'coords') {
        $maxDelta = [math]::Max([math]::Abs($coordsA.Pos[0] - $requestedPose[0]), [math]::Abs($coordsA.Pos[1] - $requestedPose[1]))
        $maxDelta = [math]::Max($maxDelta, [math]::Abs($coordsA.Pos[2] + $script:JkEyeOffset - $requestedPose[2]))
        $pitchDelta = [math]::Abs($coordsA.Pyr[0] - $requestedPose[3])
        $yawDelta = [math]::Abs($coordsA.Pyr[1] - $requestedPose[4])
        if ($yawDelta -gt 180.0) { $yawDelta = 360.0 - $yawDelta }
        $warpVerified = ($maxDelta -le 0.05 -and $yawDelta -le 0.1)
        Log ("warp verification: max position delta={0:N3} yaw delta={1:N3} verified={2}; pitch delta={3:N2} (warp keeps the level's own pitch)" -f $maxDelta, $yawDelta, $warpVerified, $pitchDelta)
        if (-not $warpVerified) { Log 'warp verification FAILED (unverified pose) — continuing' }
      } else {
        Log 'unverified pose (coords unavailable) — continuing'
      }
    }

    # Time-series verdict. The camera question needs world geometry, not moving characters:
    # compare static world regions inside the detected world box between the first and last frame.
    $hashes = @($series | ForEach-Object { $_.Md5 } | Sort-Object -Unique)
    $ssim = $null
    $regionSsims = @()
    if ($series.Count -ge 2) {
      $firstPng = Join-Path $Out $series[0].File
      $lastPng = Join-Path $Out $series[$series.Count - 1].File
      $ssim = Get-JkSsimFirstLast -First $firstPng -Last $lastPng
      $rx = $worldBox.X + [int][math]::Floor($worldBox.W * 0.62)
      $ry = $worldBox.Y + [int][math]::Floor($worldBox.H * 0.20)
      $rw = [int][math]::Floor($worldBox.W * 0.30)
      $rh = [int][math]::Floor($worldBox.H * 0.60)
      $regionSsims += Get-JkRegionSsim -First $firstPng -Last $lastPng -X $rx -Y $ry -W $rw -H $rh
      $cy = $worldBox.Y
      $ch = [int][math]::Floor($worldBox.H * 0.25)
      $regionSsims += Get-JkRegionSsim -First $firstPng -Last $lastPng -X $worldBox.X -Y $cy -W $worldBox.W -H $ch
    }
    $validRegions = @($regionSsims | Where-Object { $null -ne $_ })
    $minRegionSsim = if ($validRegions.Count -gt 0) { ($validRegions | Measure-Object -Minimum).Minimum } else { $null }
    $worldStatic = ($null -ne $minRegionSsim -and $minRegionSsim -ge 0.995)
    $seriesStatic = ($hashes.Count -le 1) -or $worldStatic
    $seriesNote = if ($hashes.Count -le 1) {
      'all series frames are byte-identical: the camera is static from the first frame'
    } elseif ($worldStatic) {
      ('camera static from the first frame: static world regions are pixel-identical across the series ' +
       '(min world-region SSIM={0:N4}); the changing pixels are animated characters/NPCs, not camera motion. ' +
       'Retail''s first visible frame is the player camera.') -f $minRegionSsim
    } else {
      ('view changes across the series (world-region SSIM={0}): a scripted intro camera is likely; ' +
       'the settled last frame is the player-camera reference.') -f (($validRegions | ForEach-Object { '{0:N4}' -f $_ }) -join ',')
    }
    Log ("series verdict: static={0} uniqueHashes={1} fullSsimFirstLast={2} worldRegionSsim={3}" -f `
        $seriesStatic, $hashes.Count, $ssim, (($validRegions | ForEach-Object { '{0:N4}' -f $_ }) -join ','))
    Log $seriesNote

    $viewSize = Get-JkEffectiveViewSize
    $viewSizeText = "$viewSize (5 = engine default, 6 = max)"
    Log "effective registry viewSize: $viewSizeText"

    # JSON metadata for the comparison harness.
    $requestedJson = if ($null -eq $requestedPose) { '@spawn' } else {
      [ordered]@{ x = $requestedPose[0]; y = $requestedPose[1]; z = $requestedPose[2]; pitch = $requestedPose[3]; yaw = $requestedPose[4] }
    }
    $rawExtension = [System.IO.Path]::GetExtension($spawnShot.RawName).ToLowerInvariant()
    $document = [ordered]@{
      name = $name
      level = $level
      requested_pose = $requestedJson
      source = 'retail GOG Star Wars Jedi Knight: Dark Forces II (JK.EXE -devMode, 16-bit 3D path via the dgVoodoo wrapper)'
      renderer = $renderer
      display_mode = (Get-JkEffectiveDisplayMode)
      b3d_accel = (Get-JkEffective3DAccel)
      raw_bit_depth = if ($rawExtension -eq '.bmp') { 24 } else { 8 }
      captured_at = (Get-Date).ToString('o')
      raw_shot = $spawnShot.RawName
      raw_png = (Split-Path -Leaf $rawPng)
      raw_width = $rawStats.Width
      raw_height = $rawStats.Height
      png = (Split-Path -Leaf $spawnPng)
      png_width = $spawnStats.Width
      png_height = $spawnStats.Height
      png_bytes = $spawnStats.Bytes
      png_sha256 = $spawnStats.Sha256
      yavg = if ($null -eq $spawnStats.Yavg) { $null } else { [math]::Round($spawnStats.Yavg, 2) }
      world_crop = $worldBox.Text
      world_crop_note = 'detected from the raw frame: x-range from a mid-height band bbox, y-range from full bbox; letterbox bars and -devMode corner overlays are excluded'
      viewsize = $viewSizeText
      view_size = $viewSize
      pos = if ($coordsA) { $coordsA.Pos } else { $null }
      pyr = if ($coordsA) { $coordsA.Pyr } else { $null }
      sector = if ($coordsA) { $coordsA.Sector } else { $null }
      coords = if ($coordsA) { $coordsA.Raw } else { $null }
      coords_second = if ($coordsB) { $coordsB.Raw } else { $null }
      coords_available = ($null -ne $coordsA)
      coords_source = $coordsSource
      warp_verified = $warpVerified
      series = @($series | ForEach-Object { [ordered]@{ index = $_.Index; file = $_.File; md5 = $_.Md5; yavg = $_.Yavg; bytes = $_.Bytes } })
      series_static = $seriesStatic
      first_frame_is_player_camera = $seriesStatic
      series_ssim_first_last = $ssim
      series_world_region_ssim = @($validRegions)
      series_note = $seriesNote
    }
    $jsonPath = Join-Path $Out ("{0}.json" -f $name)
    $document | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $jsonPath -Encoding utf8
    Log "wrote $jsonPath"
  } finally {
    # Close the game the way the engine expects, not with a kill: WM_CLOSE runs its own shutdown,
    # which stops the level and restores the desktop display mode. A force-kill leaves the primary
    # monitor in the 640x480 game mode, which then breaks the next (port) run's window.
    $closedGracefully = $false
    if ($process) {
      $process.Refresh()
      if (-not $process.HasExited) {
        Log 'closing JK.EXE with WM_CLOSE so the engine restores the desktop display mode'
        # The dev console dialog's WM_CLOSE handler runs the engine's own shutdown (Main_Shutdown +
        # exit) and works both in the menus and in gameplay; the game window only honours WM_CLOSE
        # in the menus, so try the dialog first.
        if ($consoleWindow -ne [IntPtr]::Zero -and [JkRetail]::IsWindow($consoleWindow)) { [JkRetail]::CloseWindow($consoleWindow) }
        if ($process.WaitForExit(10000)) {
          $closedGracefully = $true
          Log "JK.EXE closed gracefully through the dev console window (exit code $($process.ExitCode))"
        } else {
          Log 'JK.EXE ignored WM_CLOSE on the console window; trying the game window'
          if ($gameWindow -ne [IntPtr]::Zero -and [JkRetail]::IsWindow($gameWindow)) { [JkRetail]::CloseWindow($gameWindow) }
          if ($process.WaitForExit(5000)) {
            $closedGracefully = $true
            Log "JK.EXE closed gracefully through the game window (exit code $($process.ExitCode))"
          }
        }
      } else {
        $closedGracefully = $true
        Log "JK.EXE exited by itself (code $($process.ExitCode))"
      }
      if (-not $closedGracefully) {
        $process.Kill()
        $null = $process.WaitForExit(10000)
        Log "WARNING: force-killed JK.EXE pid=$($process.Id); the display mode may be left changed (restored below if so)"
      }
    }

    # Display safety net: leave the desktop in the mode it was in before this run.
    Start-Sleep -Milliseconds 500
    if (Test-Path -LiteralPath $script:DisplayStatePath) {
      $saved = (Get-Content -LiteralPath $script:DisplayStatePath -Raw | ConvertFrom-Json).modes
      if (Test-JkDisplayMatches -Saved $saved) {
        Log 'display after capture: desktop mode matches the pre-run baseline'
      } else {
        Log 'display after capture: desktop mode changed; restoring the pre-run baseline'
        $restore = Restore-JkDisplayState -Saved $saved
        Log ("display restore: " + ($restore.applied -join ', '))
        if (Test-JkDisplayMatches -Saved $saved) {
          Log 'display restored to the pre-run baseline'
        } else {
          Log 'WARNING: could not restore the pre-run display mode; run tools/parity/restore-display.ps1 -RegistryDefaults'
        }
      }
    } else {
      $fix = Restore-JkRegistryDefaults
      Log "display: no pre-run baseline; restored Windows display defaults: $($fix.applied)"
    }
    Get-ChildItem -Path (Join-Path $WorkCopy 'JSHOT*') -File -ErrorAction SilentlyContinue | Remove-Item -Force
    Start-Sleep -Milliseconds 400
  }
}

# --- main -------------------------------------------------------------------

Log "=== retail-capture run $(Get-Date -Format o) ==="
Log "repo=$repoRoot"
Log "shots=$Shots"
Log "out=$Out"
Log "workcopy=$WorkCopy seriesFrames=$SeriesFrames"

$rows = Read-ShotList $Shots
Log ("rows: " + (($rows | ForEach-Object { "$($_.Name)/$($_.Level)" }) -join ', '))

$GamePath = Resolve-JkGamePath $GamePath
Assert-JkInstall $GamePath 'retail install'
Log "retail install resolved: $GamePath (read-only; never modified)"

if ($Software) {
  Set-JkVirtualStore3DOff -LogPath $script:LogPath
  $renderer = 'software-8bit'
} else {
  Set-JkVirtualStore3DOn -LogPath $script:LogPath
  $renderer = 'd3d16-dgvoodoo'
}
Log "renderer: $renderer"
$viewSize = Get-JkEffectiveViewSize
Log "effective viewSize before capture: $viewSize (5 = engine default; 6 = max)"
$displayMode = Get-JkEffectiveDisplayMode
Log "effective displayMode before capture: $displayMode (index into the enumerated mode list)"

# Start from a sane desktop (a previous interrupted run may have left a 640x480 game mode behind),
# then remember the mode this run must leave behind.
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $script:DisplayStatePath) | Out-Null
$desktop = Confirm-JkDesktop -StatePath $script:DisplayStatePath
Log "display before capture: $($desktop.reason)"
Log ("display now: " + [JkDisplay]::Summary())
Save-JkDisplayState -Path $script:DisplayStatePath | Out-Null
Log "saved desktop mode baseline: $script:DisplayStatePath"

Invoke-JkStaging -Source $GamePath -Destination $WorkCopy

foreach ($row in $rows) {
  try {
    Invoke-JkCaptureRow -Row $row
  } catch {
    Log "FATAL while capturing '$($row.Name)': $($_.Exception.Message)"
    throw
  }
}

Log '=== retail-capture done: all rows captured ==='
