#requires -Version 7
<#
.SYNOPSIS
  Port-side parity captures: one PNG per row of tools/parity/shots.tsv.

.DESCRIPTION
  Runs target/release/bevi-knight.exe (building it if missing) for every row of
  tools/parity/shots.tsv and writes target/parity/port/<name>.png plus a short
  target/parity/port/port.log.

  This script NEVER launches the retail game. Before every run it checks for a
  running JK.EXE (the retail-capture side owns it) and waits for it to exit;
  if it is still running after the wait, the batch aborts.

  Rows with an explicit pose in shots.tsv also get `--shot-at "x,y,z,yaw,pitch"` for the camera and
  `--warp "x,y,z,yaw"` for the **player**: retail's reference frame is captured with the body at
  that pose, so its scripts, AI and hazards have a body to react to, and a camera-only capture
  leaves the port's player at the level start. The pitch is the port camera pitch in degrees
  (from the row; retail `warp` cannot set pitch — see shots.tsv).

  The port window is rendered at the retail view size: for each row this reads
  target/parity/retail/<name>.json (the world crop inside the retail frame) and
  passes it as `BK_WINDOW="WxH"`, so the pair has one shape. Without a retail
  sidecar the window stays at the port's 1024x768 default.

.EXAMPLE
  pwsh -File tools/parity/port-capture.ps1
  pwsh -File tools/parity/port-capture.ps1 -Only level5_spawn
#>
[CmdletBinding()]
param(
    [string]$Game = 'G:\Games\Star Wars Jedi Knight - Dark Forces 2',
    [int]$TimeoutSec = 120,
    [string[]]$Only = @(),
    # A survey list (the gated pairs live in shots.tsv; a per-level sweep uses its own file).
    [string]$Shots = '',
    [int]$Frames = 600
)

$ErrorActionPreference = 'Stop'

$Root     = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)   # tools\parity -> repo root
if (-not $Shots) { $Shots = Join-Path $Root 'tools\parity\shots.tsv' }
$Exe      = Join-Path $Root 'target\release\bevi-knight.exe'
$OutDir   = Join-Path $Root 'target\parity\port'
$LogPath  = Join-Path $OutDir 'port.log'
# `--frames` default 600 = ~10 s of 60 Hz sim: long enough for scripted `PlayKey` clips at level start
# (THout.key is 61 frames at 15 fps) to finish, which is where the retail capture takes its frame.

. (Join-Path $PSScriptRoot 'display.ps1')

function Write-Log([string]$Line) {
    Add-Content -LiteralPath $LogPath -Value $Line
    Write-Host $Line
}

# Same minimal parser as compare.ps1: '#' comments, a 'name' header, TAB columns.
function Get-ShotRows {
    $rows = @()
    foreach ($line in Get-Content -LiteralPath $Shots) {
        if ($line -match '^\s*(#|$)') { continue }
        $f = $line -split "`t"
        if ($f.Count -lt 7 -or $f[0].Trim() -eq 'name') { continue }
        $rows += [pscustomobject]@{
            Name  = $f[0].Trim()
            Level = $f[1].Trim()
            X     = $f[2].Trim()
            Y     = $f[3].Trim()
            Z     = $f[4].Trim()
            Pitch = $f[5].Trim()
            Yaw   = $f[6].Trim()
        }
    }
    return $rows
}

# Wait for the retail game to finish; $false means it is still running after 120 s.
function Wait-ForNoRetail {
    $deadline = (Get-Date).AddSeconds(120)
    while (Get-Process JK -ErrorAction SilentlyContinue) {
        if ((Get-Date) -gt $deadline) { return $false }
        Write-Host 'JK.EXE is running (retail capture in progress); waiting...'
        Start-Sleep -Seconds 3
    }
    return $true
}

if (-not (Test-Path -LiteralPath $Shots)) { throw "missing shots list: $Shots" }
if (-not (Test-Path -LiteralPath (Join-Path $Game 'JK.EXE'))) {
    throw "not a Jedi Knight install (no JK.EXE in '$Game')"
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Set-Content -LiteralPath $LogPath -Value "# port parity capture $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"

if (-not (Test-Path -LiteralPath $Exe)) {
    Write-Log 'release build missing; running cargo build --release'
    Push-Location $Root
    try {
        & cargo build --release
        if ($LASTEXITCODE -ne 0) { throw 'cargo build --release failed' }
    } finally { Pop-Location }
    if (-not (Test-Path -LiteralPath $Exe)) { throw 'cargo build --release produced no bevi-knight.exe' }
}

$rows = @(Get-ShotRows)
if ($Only.Count -gt 0) {
    $rows = @($rows | Where-Object { $Only -contains $_.Name })
    if ($rows.Count -eq 0) { throw "no shots matched -Only $($Only -join ',')" }
}

# If a previous retail run was interrupted, the primary monitor can be stuck in the game's 640x480
# mode. The port then opens its 1024x768 window on a 640x480 desktop and looks clipped/wrong, so put
# the desktop back before capturing (uses the retail side's saved baseline, Windows' saved default as
# fallback; never touches an ordinary desktop resolution).
$desktop = Confirm-JkDesktop -StatePath (Join-Path $Root 'target\parity\display-pre.json')
Write-Log "display before port capture: $($desktop.reason)"
Write-Log ("display now: " + [JkDisplay]::Summary())

Write-Log "shots list: $Shots ($($rows.Count) shot(s)); game: $Game"
$failures = 0
$done = 0

foreach ($row in $rows) {
    if (-not (Wait-ForNoRetail)) {
        Write-Log "ABORT $($row.Name): JK.EXE still running after waiting; will not start the port"
        throw 'JK.EXE is still running; aborting so the retail side is not disturbed'
    }

    $shotRel = "target/parity/port/$($row.Name).png"
    $shotAbs = Join-Path $Root $shotRel
    # Delete first so a stale PNG cannot make a failed run look successful.
    Remove-Item -LiteralPath $shotAbs -Force -ErrorAction SilentlyContinue

    $hasPose = ($row.X -ne '' -and $row.X -ne '-')
    # Both sides must have the same shape: the retail sidecar records the 3D view size inside its raw
    # frame (letterbox bars come from the game's `viewSize`), and the port window draws only the view.
    $windowSize = $null
    $retailJson = Join-Path $Root "target/parity/retail/$($row.Name).json"
    if (Test-Path -LiteralPath $retailJson) {
        try {
            $meta = Get-Content -LiteralPath $retailJson -Raw | ConvertFrom-Json
            if ($meta.png_width -and $meta.png_height) {
                $windowSize = '{0}x{1}' -f $meta.png_width, $meta.png_height
            }
        } catch {
            Write-Log "WARN $($row.Name): could not read $retailJson ($($_.Exception.Message)); using the port's default window"
        }
    }
    # The port camera's pitch for the shot, in degrees. `coords` PYR pitch (1.04 at level 1) is not
    # the rendered camera pitch: a 0.25-degree port pitch already drops the spawn pair 0.63 -> 0.59,
    # so the row's own value (0 unless a retail frame says otherwise) is what the port uses.
    $shotPitch = 0.0
    if ($row.Pitch -ne '-' -and $row.Pitch -ne '') {
        $shotPitch = [double]::Parse($row.Pitch, [System.Globalization.CultureInfo]::InvariantCulture)
    }
    $env:BK_WINDOW = $windowSize   # $null removes it again for the next row
    Write-Log ("window: " + $(if ($windowSize) { "$windowSize (retail view size)" } else { 'default 1024x768 (no retail sidecar yet)' }))
    $argLine = '--game "' + $Game + '" --start ' + $row.Level + ' --frames ' + $Frames + ' --shot "' + $shotRel + '" --mute'
    if ($hasPose) {
        $pitchText = $shotPitch.ToString('0.######', [System.Globalization.CultureInfo]::InvariantCulture)
        $argLine += ' --shot-at "' + (@($row.X, $row.Y, $row.Z, $row.Yaw) -join ',') + ',' + $pitchText + '"'
        # The player, not just the camera: retail's reference frame has the body at the pose, so the
        # room's scripts, AI and hazards act on it. Without this the port's body stays at the level
        # start and the state the retail frame shows never happens.
        $argLine += ' --warp "' + (@($row.X, $row.Y, $row.Z, $row.Yaw) -join ',') + '"'
    }

    $outPath = Join-Path $OutDir '_run.out'
    $errPath = Join-Path $OutDir '_run.err'
    Remove-Item -LiteralPath $outPath, $errPath -Force -ErrorAction SilentlyContinue

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $ok = $false
    $why = ''
    try {
        $p = Start-Process -FilePath $Exe -ArgumentList $argLine -WorkingDirectory $Root -PassThru `
            -RedirectStandardOutput $outPath -RedirectStandardError $errPath
        if (-not $p.WaitForExit($TimeoutSec * 1000)) {
            try { $p.Kill($true) } catch { try { $p.Kill() } catch { } }
            $p.WaitForExit(5000) | Out-Null
            $why = "timeout after ${TimeoutSec}s"
        } elseif ($p.ExitCode -ne 0) {
            $why = "exit code $($p.ExitCode)"
        } elseif (-not (Test-Path -LiteralPath $shotAbs)) {
            $why = 'no PNG written'
        } else {
            $ok = $true
        }
    } catch {
        $why = "launch failed: $($_.Exception.Message)"
    }
    $sw.Stop()

    if ($ok) {
        $size = (Get-Item -LiteralPath $shotAbs).Length
        $poseNote = if ($hasPose) {
            "  (pose $($row.X),$($row.Y),$($row.Z),yaw $($row.Yaw),pitch $shotPitch)"
        } else { '' }
        Write-Log ("{0}: OK  {1:N1}s  {2} bytes  {3}{4}" -f $row.Name, $sw.Elapsed.TotalSeconds, $size, $shotRel, $poseNote)
        Write-Log ("  cmd: bevi-knight.exe {0}" -f $argLine)
        $done++
    } else {
        $failures++
        Write-Log ("{0}: FAIL  {1:N1}s  {2}" -f $row.Name, $sw.Elapsed.TotalSeconds, $why)
        Write-Log ("  cmd: bevi-knight.exe {0}" -f $argLine)
        foreach ($f in @($outPath, $errPath)) {
            if (Test-Path -LiteralPath $f) {
                $tail = @(Get-Content -LiteralPath $f -Tail 8 -ErrorAction SilentlyContinue)
                if ($tail.Count -gt 0) {
                    Write-Log ("  {0} tail: {1}" -f (Split-Path -Leaf $f), ($tail -join ' | '))
                }
            }
        }
    }
    Remove-Item -LiteralPath $outPath, $errPath -Force -ErrorAction SilentlyContinue
}

Write-Log ("done: {0}/{1} captured, {2} failed" -f $done, $rows.Count, $failures)
if ($failures -gt 0) { exit 1 }
exit 0
