[CmdletBinding()]
param(
  [string]$Shots = 'tools/parity/shots.tsv',
  [string]$Out = 'target/parity/retail',
  [string]$GamePath,
  [string]$WorkCopy = 'target/parity/retailcopy',
  [int]$SeriesFrames = 8,
  [int]$LevelTimeoutSec = 45,
  [switch]$Software,
  # Phase 0.5 trace v0, retail half: after the level loads (and any warp) hold the forward key in
  # undisturbed 600 ms segments, reading `coords` in between, and write <name>-walk.tsv into
  # target/parity/trace. 0 keeps the normal screenshot captures.
  [int]$WalkTraceMs = 0,
  # Which key the walk trace holds (default W; UP is the engine's own forward binding).
  [string]$WalkTraceKey = 'UP',
  # Milliseconds to stay idle **before** the hold: a level whose opening is scripted locks the player's
  # input, so a leg that starts after the opening settles for this window first. The trace's `held_ms`
  # starts at 0 after it, and `port-trace.ps1` takes the same value from the row's optional 8th column.
  [int]$WalkTraceSettleMs = 0,
  # Issue #9's door-encounter telemetry, retail half: after the level loads and the row's warp has
  # been applied, sample the engine's own memory (player health, inventory bin 60 shields, the thing
  # array for actors and fired projectiles, and `DAT_00836c14` for the game clock) on a fixed
  # game-time cadence for this many milliseconds, then write <name>.tsv into -EncounterOut. 0 keeps
  # the normal screenshot captures. The dev console is NOT polled during the trace: it pauses the
  # game and would change the encounter.
  [int]$EncounterMs = 0,
  [int]$EncounterCadenceMs = 100,
  [string]$EncounterOut = 'target/parity/encounter',
  # File name (without `.tsv`) for the encounter trace. Empty uses the shots row's name; the gate's
  # fixture is `door_encounter`, while the pose row is `route_door`.
  [string]$EncounterName = '',
  # Capture a walk leg even when the headless preflight says the body cannot advance from the pose
  # (`tools/parity/walk-preflight.ps1`). Not recommended: a blocked leg parks the game against a wall.
  [switch]$ForceWalk,
  # Row names to run (default: every row of the shots file). A trace run uses one row.
  [string[]]$Only = @(),
  # Retained raw frame(s) to run the world-box detector on instead of capturing anything, e.g.
  # `-ProbeBox target/parity/retail/level2_spawn-raw.png`. Prints the box and exits.
  [string[]]$ProbeBox = @(),
  # Dev-console commands to run after the level loads (and after the warp), e.g. `cogstatus 1`.
  # Each row's output is written to target/parity/trace/<name>-console.txt, which is how Phase 0.5
  # step 2 captures one level's cog variables and AI status.
  [string[]]$ConsoleCommands = @(),
  # Extra JK.EXE command-line arguments. Retail has no console command that loads a level (round 60),
  # so this is the harness's only route to boot a level other than the campaign's first - Phase 2's
  # spawn audits need it. `-File` splits `-level,02narshadda` into two arguments.
  [string[]]$JkArgs = @(),
  # Keys to press after the warp and before the console commands (e.g. 'ENTER' for the use action).
  # Phase 1 step 4's interaction probes need this: a switch is used, not walked into.
  [string[]]$Keys = @(),
  # Keys pressed after the console commands (e.g. to dismiss the end-of-level tally and let
  # the next level load, which is how a genuine level-2 save can come out of retail).
  [string[]]$KeysAfter = @(),
  # Hold for an action key in `-KeysAfter`. The retail capture runs under software rendering at a
  # few frames per second; a 40 ms tap can fall between frames and the game never sees the use.
  [int]$ActionHoldMs = 600,
  # How long to leave the game running after `-KeysAfter` before the second console read. The
  # default is the level-settle time; a combat level can kill the player inside it, so a trigger
  # probe can shorten it.
  [int]$AfterWaitMs = 12000,
  # Stop on the Single Player submenu and save a screenshot, so Load Game's coordinates can be read
  # off it (the harness otherwise clicks New Game and never shows the submenu).
  [switch]$StopAtMenu,
  # Capture the opening cutscene instead of skipping it: wait this many ms from the window
  # appearing, grab `intro-cutscene.png`, and return. Step 8's exit is a retail frame pair of a
  # cutscene frame; the port renders the same `.smk` with `--cutscene <file> <frame>`.
  [int]$IntroShotMs = 0,
  # The **main** menu (Players / Cutscenes / Credits / Setup), one screen above `-StopAtMenu`: step
  # 10's UI fixture needs this screen's frame, and the submenu shot cannot stand in for it.
  [switch]$MainMenuShot,
  # Click Load on the submenu and photograph the save list (retail's only remaining route to a
  # level other than the campaign's first, which Phase 2''s spawn audits need).
  [switch]$LoadShot,
  # Boot a single-player level by number from the [Debug] level list (1 = level 1, 2 = The Lost
  # Disk, ...). The level list opens after the first category row and its Ok.
  [string]$StartLevel = '',
  # Extra time the level list boot waits before reading `coords` (levels that open with a
  # scripted sequence need longer for the body to settle where retail puts it).
  [int]$SettleMs = 12000,
  # Milliseconds to wait after the level load before the reference frame, for levels whose
  # opening animation has to finish before the player camera exists (level 2's ship launch).
  [int]$PreShotMs = 0,
  # Click [Debug] on the Single Player submenu and photograph what it offers (devMode may
  # list levels, which would be the clean route to every level).
  [switch]$DebugShot,
  # Load the first savegame instead of starting a new game. With the save's recorded level
  # patched this is how retail boots a level other than the campaign's first (Phase 2).
  [switch]$LoadSave,
  # Level to boot by patching the scratch savegame before launch (e.g. '02narshadda.jkl').
  # The save records the level name twice (offsets 132/1616) and its number (int32 at 1612);
  # patching all three is the only way retail will load a level other than the first.
  [string]$BootSaveLevel = '',
  # Start a different episode from the [Debug] chooser (1 = The Force Within, 2 = Jedi Capture the
  # Flag, 3 = Jedi Training) instead of the campaign's first level, then log coords.
  [string]$StartEpisode = ''
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
$script:TraceOut = Resolve-FromRepo 'target/parity/trace'
New-Item -ItemType Directory -Force -Path $script:TraceOut | Out-Null
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
    # 8th column (walk legs only): milliseconds to settle before the hold, see -WalkTraceSettleMs.
    if ($cols.Count -notin 7, 8) { throw "shots file line ${lineNo}: expected 7 or 8 tab-separated columns, got $($cols.Count)" }
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
    $rows.Add([pscustomobject]@{ Name = $cols[0]; Level = $cols[1]; Pose = $pose; Settle = $(if ($cols.Count -ge 8) { [int]$cols[7] } else { 0 }); Line = $lineNo })
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

  # The Load screen lists `player\<profile>\*.jks` and **skips names starting `_JKAUTO`**
  # (`jkGuiSaveLoad_PopulateList`, 0x41ddd0: `sithGamesave_GetProfilePath`, a find in the profile
  # directory, an `__strnicmp` against `_JKAUTO`, then a header read whose first dword must be 6).
  # Retail's own play therefore leaves only autosaves in that folder, which the list hides — so a save
  # to load has to exist under a normal name. Seed it from the freshest autosave *before* the cleanup
  # below deletes them; a fresh install with no autosave falls back to the shipped `saves\autosave.jks`.
  if ($script:BootSaveLevel) {
    $seed = Join-Path $Destination 'player\Lee\save0000.jks'
    if (-not (Test-Path -LiteralPath $seed)) {
      $auto = Get-ChildItem -Path (Join-Path $Destination 'player') -Recurse -Filter '_JKAUTO_*.jks' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
      if ($auto) {
        Copy-Item -LiteralPath $auto.FullName -Destination $seed -Force
        Log "seeded save0000.jks from the autosave $($auto.Name) so the Load screen can list it"
      } else {
        $fallback = Join-Path $Destination 'saves\autosave.jks'
        if (Test-Path -LiteralPath $fallback) {
          Copy-Item -LiteralPath $fallback -Destination $seed -Force
          Log 'seeded save0000.jks from the shipped saves\autosave.jks'
        } else { Log 'no savegame to seed: no autosave and no shipped saves\autosave.jks' }
      }
    }
  }
  # The scratch profile (never the real install) gets cutscenes disabled.
  # Stale autosaves are a trap: JK writes `_JKAUTO_<level>.jks` during play, and the next run resumes that
# state instead of the level's spawn pose - a warp then arrives at the wrong body and the trace measures
# nothing (which is how the door walk ended up starting in sector 339). The scratch copy is disposable,
# so clear them before every capture.
Get-ChildItem -Path (Join-Path $Destination 'player') -Recurse -Filter '_JKAUTO_*.jks' -ErrorAction SilentlyContinue |
  ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force; Log "removed stale autosave $($_.Name)" }
  if ($script:BootSaveLevel) {
    $savePath = Join-Path $Destination 'player\Lee\save0000.jks'
    if (Test-Path -LiteralPath $savePath) {
      $bytes = [System.IO.File]::ReadAllBytes($savePath)
      $name = $script:BootSaveLevel.ToLowerInvariant()
      # The field is 15 bytes wide. A name shorter than that (`15maw.jkl` is 9) is padded with NULs,
      # which is how a C string ends; padding with spaces instead would leave `15maw.jkl      ` for the
      # engine to parse. Longer names cannot fit and are refused.
      if ($name.Length -gt 15) { throw "BootSaveLevel must fit 15 bytes like 02narshadda.jkl (got $name)" }
      $number = [int]$name.Substring(0, 2)
      $encoded = [System.Text.Encoding]::ASCII.GetBytes($name.PadRight(15, [char]0))
      [Array]::Copy($encoded, 0, $bytes, 132, 15)
      [Array]::Copy($encoded, 0, $bytes, 1616, 15)
      [BitConverter]::GetBytes([int]$number).CopyTo($bytes, 1612)
      [System.IO.File]::WriteAllBytes($savePath, $bytes)
      Log "patched the savegame to boot $name (level number $number)"
    } else { Log "no savegame to patch at $savePath" }
  }
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
  # A previous run of **this harness** can be interrupted (operator Ctrl+C, a tool timeout, a crashed
  # shell) and leave its scratch-copy JK.EXE running — the operator then has to close it by hand, which is
  # exactly what happened in round 250. The scratch copy is disposable, so a leftover process whose
  # executable lives under `$WorkCopy` is ours to close; anything else is a foreground game session and the
  # harness still refuses to fight it.
  $jk = @(Get-Process -Name 'JK' -ErrorAction SilentlyContinue)
  if ($jk.Count -gt 0) {
    $ours = @()
    $foreign = @()
    foreach ($p in $jk) {
      $path = $null
      try { $path = $p.Path } catch { $path = $null }
      if ($path -and $path.StartsWith($WorkCopy, [System.StringComparison]::OrdinalIgnoreCase)) {
        $ours += $p
      } else {
        $foreign += $p
      }
    }
    if ($ours.Count -gt 0) {
      Log ("leftover scratch-copy JK.EXE from an interrupted run: pid={0}; closing it now (the install copy is disposable)" -f (($ours | ForEach-Object { $_.Id }) -join ','))
      foreach ($p in $ours) {
        try {
          $null = $p.CloseMainWindow()
          if (-not $p.WaitForExit(5000)) { $p.Kill(); $null = $p.WaitForExit(5000) }
          Log ("leftover JK.EXE pid={0} closed" -f $p.Id)
        } catch {
          Log ("could not close leftover JK.EXE pid={0}: {1}" -f $p.Id, $_.Exception.Message)
        }
      }
      Start-Sleep -Milliseconds 500
    }
    if ($foreign.Count -gt 0) {
      throw ("JK.EXE is already running (pid={0}) and is not this harness's scratch copy. Close it before running the retail capture harness (only one engine at a time)." -f (($foreign | ForEach-Object { $_.Id }) -join ','))
    }
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
function Invoke-JkConsoleCapture {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][string]$Name,
    [Parameter(Mandatory)][string[]]$Commands
  )
  $consolePath = Join-Path $script:TraceOut ("{0}-console.txt" -f $Name)
  $consoleLines = New-Object System.Collections.Generic.List[string]
  # `-File` passes a comma-joined token as one string, so split it back into commands.
  $split = @($Commands | ForEach-Object { $_ -split ',' } | Where-Object { $_ -ne '' })
  foreach ($command in $split) {
    # `wait:<ms>` pauses instead of issuing a command, so one capture can show a cog's state before and
    # after a trigger (Phase 1 step 4's "cog variable/state trace at each trigger").
    if ($command -like 'wait:*') {
      $milliseconds = [int]($command -replace '^wait:', '')
      $consoleLines.Add("> wait $milliseconds ms")
      Start-Sleep -Milliseconds $milliseconds
      continue
    }
    $consoleLines.Add("> $command")
    foreach ($line in @(Invoke-JkDevConsoleCommand -ProcessId $ProcessId -Command $command)) {
      $consoleLines.Add($line)
    }
  }
  Set-Content -LiteralPath $consolePath -Value $consoleLines -Encoding utf8
  Log "console: wrote $consolePath ($($split.Count) command(s))"
}
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
    * mouse-navigates Players -> Ok -> Single Player, then boots the numbered row's Debug level
      (New Game only for level 1 or an explicit menu/save route),
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
  # The dialog's ListBox is cleared between commands, so a second command's output can be shorter than
  # `$before` and never trip the count test above. Fall back to the whole current box: a state snapshot
  # per command is all the phase-0.5 captures need.
  if ($newLines.Count -eq 0) { $newLines = @(Get-JkConsoleLines -ProcessId $ProcessId) }
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

# The engine's game clock in ms: `sithTime_Advance` (`0x4dd640`) adds each frame's delta, clamped to
# [10, 500] ms and paused time excluded, to `DAT_00836c14`. Retail renders a few frames a second here, so
# this, not the injected hold length, is how long the body actually walked. -1 when it cannot be read.
function Read-JkGameMs {
  param([Parameter(Mandatory)][int]$ProcessId)
  return [JkRetail]::ReadU32([uint32]$ProcessId, 0x00836c14)
}

# `-WalkTraceMs`: hold the forward key in undisturbed segments and read `coords` between them.
#
# This is Phase 0.5 trace v0's retail half. The dev console is a **dialog**: while it has focus the
# game's clock stops advancing, so a naive "hold the key and poll" walk is a sequence of frozen
# instants, not a continuous one. The trace is therefore built from hold segments: each one walks for
# `HoldMs` of real (== game) time with the game window focused and no console traffic, then the pose
# and sector are read while the game is frozen. The `held_ms` column is the sum of the hold
# segments - the game time the body actually walked - not wall time.
#
# The port replays the same thing with `--trace-out` and `BK_WALK_FORWARD=1` (fixed 1/60 s step), and
# `tools/parity/trace-compare.ps1` compares the two at the segment times.
#
# Player bars, the f32s both the walk trace and the encounter telemetry record: health at
# `thing+0x134` (`GetThingHealth`) and inventory bin 60 shields at `inventory + 0x88 + 60*0x18`
# (`sithInventory_SetInventory`). The plausibility guards are the encounter reader's: a wrong
# pointer must abort the capture, not write zeros into a fixture.
function Read-JkPlayerBars {
  param([Parameter(Mandatory)][int]$ProcessId)
  $thing = [JkRetail]::ReadU32($ProcessId, 0x008319A4)
  if ($thing -le 0) { throw "player bars: the player thing pointer (0x008319A4) read $thing" }
  $inv = [JkRetail]::ReadU32($ProcessId, $thing + 0x1C0)
  if ($inv -le 0) { throw "player bars: the inventory pointer (thing+0x1C0) read $inv" }
  $health = [JkRetail]::ReadFloat($ProcessId, $thing + 0x134)
  $shields = [JkRetail]::ReadFloat($ProcessId, $inv + 0x88 + 60 * 0x18)
  if ([double]::IsNaN($health) -or [double]::IsNaN($shields)) {
    throw "player bars: the player bars could not be read (health $health, shields $shields)"
  }
  if ($health -lt 0.0 -or $health -gt 1000.0) { throw "player bars: implausible health $health at thing+0x134" }
  if ($shields -lt -0.01 -or $shields -gt 200.01) { throw "player bars: implausible shields $shields at inventory bin 60" }
  [pscustomobject]@{ Health = $health; Shields = $shields }
}
function Invoke-JkWalkTrace {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][string]$Name,
    [int]$Milliseconds = 3000,
    [int]$HoldMs = 600,
    [string]$Key = 'UP',
    [int]$SettleMs = 0
  )
  $path = Join-Path $script:TraceOut ("{0}-walk.tsv" -f $Name)
  $segments = [math]::Max(1, [int][math]::Floor($Milliseconds / $HoldMs))
  $lines = New-Object System.Collections.Generic.List[string]
  # `game_ms` is appended (not replacing `held_ms`) so older fixtures and readers keep their columns;
  # `health`/`shields` are the route reference's two bar columns (issue #8's required fields).
  $lines.Add((@("held_ms", "x", "y", "z", "sector", "yaw", "game_ms", "health", "shields") -join ([char]9)))
  $start = Read-JkCoords -ProcessId $ProcessId
  if (-not $start) { throw 'walk trace: the start pose could not be read from coords' }
  Log ("walk trace: start " + $start.Raw)
  $clock0 = Read-JkGameMs -ProcessId $ProcessId
  if ($clock0 -lt 0) { throw 'walk trace: the game clock (0x00836c14) could not be read' }
  $bars = Read-JkPlayerBars -ProcessId $ProcessId
  $lines.Add((@(0, $start.Pos[0], $start.Pos[1], $start.Pos[2], $start.Sector, $start.Pyr[1], 0, ('{0:F1}' -f $bars.Health), ('{0:F1}' -f $bars.Shields)) -join ([char]9)))
  $samples = 0
  if ($SettleMs -gt 0) {
    Log ("walk trace: settling {0} ms before the hold" -f $SettleMs)
    Start-Sleep -Milliseconds $SettleMs
  }
  # The key is pressed and released once per segment (below), so the body coasts between the release
  # and the console's pause and each segment restarts from rest; the port's replay holds forward
  # continuously. `game_ms` makes that visible: it counts the coast and the foreground wait too.
  Send-JkKeyDown -Name $Key
  # **Stuck detection.** A walk leg whose forward route runs into geometry does not error: the body
  # stops, the harness keeps holding the key for the rest of the segments, and the operator is left
  # watching a game parked against a wall (round 250: this happened repeatedly and had to be closed by
  # hand). Two consecutive segments with under 0.02 units of horizontal movement is the end of the walk:
  # if the body never moved at all, that is a route into geometry and the leg throws; if it walked and
  # then stopped, the samples are kept (round 293 — the short legs stop against a prop by design, and
  # discarding them lost their whole capture).
  $previous = @([double]$start.Pos[0], [double]$start.Pos[1])
  $still = 0
  try {
    for ($i = 1; $i -le $segments; $i++) {
      # Foreground matters: DirectInput reads the keyboard only while the game window is active, and
      # reading `coords` through the console dialog takes the foreground away. Activate it and wait
      # until it really is foreground, then hold: the segment moves for exactly `HoldMs`.
      $gameWindow = Get-JkWindow -ProcessId $ProcessId
      for ($attempt = 0; $attempt -lt 8; $attempt++) {
        if ($gameWindow -eq [IntPtr]::Zero) { break }
        [void][JkRetail]::ShowWindow($gameWindow, 9)
        [void][JkRetail]::SetForegroundWindow($gameWindow)
        Start-Sleep -Milliseconds 120
        if ([JkRetail]::ForegroundTitle() -eq 'Jedi Knight') { break }
      }
      Send-JkKeyDown -Name $Key
      Start-Sleep -Milliseconds $HoldMs
      Send-JkKeyUp -Name $Key
      $held = $i * $HoldMs
      $sample = Read-JkCoords -ProcessId $ProcessId
      if (-not $sample) {
        Log ("walk trace: segment $i produced no parseable coords line")
        continue
      }
      $clock = Read-JkGameMs -ProcessId $ProcessId
      if ($clock -lt 0) { throw "walk trace: the game clock could not be read after segment $i" }
      $gameMs = $clock - $clock0
      $samples++
      $bars = Read-JkPlayerBars -ProcessId $ProcessId
      $lines.Add((@($held, $sample.Pos[0], $sample.Pos[1], $sample.Pos[2], $sample.Sector, $sample.Pyr[1], $gameMs, ('{0:F1}' -f $bars.Health), ('{0:F1}' -f $bars.Shields)) -join ([char]9)))
      Log ("walk trace: {0} ms held ({5} ms game) at ({1:N3}, {2:N3}, {3:N3}) sector {4}" -f $held, $sample.Pos[0], $sample.Pos[1], $sample.Pos[2], $sample.Sector, $gameMs)
      # Stuck detection (see the comment above the hold loop): a body against a wall has not moved.
      $dx = [double]$sample.Pos[0] - $previous[0]
      $dy = [double]$sample.Pos[1] - $previous[1]
      $previous = @([double]$sample.Pos[0], [double]$sample.Pos[1])
      if ([math]::Sqrt($dx * $dx + $dy * $dy) -lt 0.02) { $still++ } else { $still = 0 }
      if ($still -ge 2) {
        # A leg that **never** moves is a route into geometry (the case the guard exists for). A leg
        # that walks and then stops is a measurement: several shipped legs are short walks that end
        # against a prop or a wall (walk4_spawn walks 0.30 units, walk6_spawn 0.2), and their retail
        # reference is exactly that. Record what was taken instead of discarding the run.
        $walkedFromStart = [math]::Sqrt(($sample.Pos[0] - $start.Pos[0]) * ($sample.Pos[0] - $start.Pos[0]) + ($sample.Pos[1] - $start.Pos[1]) * ($sample.Pos[1] - $start.Pos[1]))
        if ($walkedFromStart -lt 0.02) {
          Log ("walk trace: STUCK at ({0:N3}, {1:N3}, {2:N3}) sector {3} after {4} ms of held forward" -f $sample.Pos[0], $sample.Pos[1], $sample.Pos[2], $sample.Sector, $held)
          throw ("walk leg '$Name' is stuck: two segments of held forward moved it less than 0.02 units, so the route walks into geometry. Aborting this leg and closing the game; run tools/parity/walk-preflight.ps1 before capturing it again.")
        }
        Log ("walk trace: the leg stopped at ({0:N3}, {1:N3}, {2:N3}) sector {3} after {4} ms - it walked {5:N2} units from the start; recording the {6} sample(s) taken" -f $sample.Pos[0], $sample.Pos[1], $sample.Pos[2], $sample.Sector, $held, $walkedFromStart, $samples)
        break
      }
      # Do **not** re-press here: the next iteration activates the window (sleeping up to a second
      # while the engine runs), and a key left down over that sleep moves the body outside the measured
      # segments - which is what made the first traces read 1.5 u/s against a 0.875 u/s rule.
    }
  } finally {
    Send-JkKeyUp -Name $Key
  }
  Set-Content -LiteralPath $path -Value $lines -Encoding utf8
  Log ("walk trace: wrote $path ({0} sample(s) over {1} x {2} ms of held forward)" -f $samples, $segments, $HoldMs)
  # A **leg-quality check**: a walk leg is only a walk while the player is under input control. When a
  # level's script owns the player (a cutscene) the body is pinned and the *camera* is driven instead, so
  # the recorded yaw swings instead of staying put. That is what level 7's opening does (positions flat,
  # yaw 43.6 -> -80.8 -> 161.1 over three windows), and a comparison against such a trace measures
  # nothing. Flag it at capture time so the fixture is not mistaken for a walk.
  $yaws = @($lines | Select-Object -Skip 1 | ForEach-Object { [double](($_ -split "`t")[5]) })
  if ($yaws.Count -ge 2) {
    if ($span -gt 5.0) {
      Log ("walk trace: WARNING camera is scripted (yaw spans {0:N1} degrees over the window); the leg is not a plain walk" -f $span)
    }
    # A pinned body is the other cutscene signature: the camera can hold still while the script keeps the
    # player where it is, so the window records 0.1 units of a "walk" that never happened (level 7's
    # forward trace moves 0.11 units in 3 s with yaw 0 the whole way).
    $pts = @($lines | Select-Object -Skip 1 | ForEach-Object { $c = $_ -split "`t"; ,@([double]$c[1], [double]$c[2]) })
    if ($pts.Count -ge 2) {
      $travel = 0.0
      for ($k = 1; $k -lt $pts.Count; $k++) {
        $dx = $pts[$k][0] - $pts[$k - 1][0]
        $dy = $pts[$k][1] - $pts[$k - 1][1]
        $travel += [math]::Sqrt($dx * $dx + $dy * $dy)
      }
      if ($travel -lt 0.3) {
        Log ("walk trace: WARNING the body travelled {0:N2} units in the window; the leg is not a plain walk" -f $travel)
      }
    }
  }
  return $path
}
# `-EncounterMs`: the retail half of issue #9's door-encounter telemetry.
#
# Everything is read straight out of the engine's memory, because the dev console is a dialog that
# pauses the game: a `coords`/`aistatus` poll on a cadence would stop the encounter between samples
# and measure a different fight. The addresses come from the decompilation (each is a fixed runtime
# address; JK.EXE has no relocations):
#
#   player thing            *(u32*)0x008319A4
#   health                  f32 at thing + 0x134        (`GetThingHealth`, sithPlayer health)
#   inventory block         *(u32*)(thing + 0x1C0)
#   shields (bin 60)        f32 at inventory + 0x88 + 60*0x18
#                           (`sithInventory_SetInventory` 0x4D2000: amount = base + bin*0x18)
#   world                   *(u32*)0x008339C8
#   thing count / base      u32 at world + 0xBC / u32 at world + 0xC4 (`FUN_004da770`),
#                           stride 0x2B0; type at +0x0C (2 actor, 3 weapon/projectile, 10 player),
#                           template u16 at +0x04, angles f32x3 at +0x88, position f32x3 at +0xB8
#   game clock (ms)         *(u32*)0x00836C14           (`sithTime_Advance`)
#
# The row shape is the port's (`t_ms kind thing template x y z yaw health shields detail`), so
# `tools/parity/encounter-gate.ps1` compares the two without a second format. A `shot` row is a
# type-3 thing that appeared since the previous sample (a fired bolt; placed weapons are already in
# the baseline), a `hit` row is a drop in the player's shields or health, and `player`/`actor` rows
# are state at the fixed game-time cadence.
function Invoke-JkEncounterTrace {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][string]$Name,
    [int]$Milliseconds = 10000,
    [int]$CadenceMs = 100,
    [Parameter(Mandatory)][string]$OutDir
  )
  $path = Join-Path $OutDir ("{0}.tsv" -f $Name)
  $playerThing = [JkRetail]::ReadU32($ProcessId, 0x008319A4)
  if ($playerThing -le 0) { throw "encounter trace: the player thing pointer (0x008319A4) read $playerThing" }
  $world = [JkRetail]::ReadU32($ProcessId, 0x008339C8)
  if ($world -le 0) { throw "encounter trace: the world pointer (0x008339C8) read $world" }
  $inv = [JkRetail]::ReadU32($ProcessId, $playerThing + 0x1C0)
  if ($inv -le 0) { throw "encounter trace: the player inventory pointer (thing+0x1C0) read $inv" }
  $health0 = [JkRetail]::ReadFloat($ProcessId, $playerThing + 0x134)
  $shields0 = [JkRetail]::ReadFloat($ProcessId, $inv + 0x88 + 60 * 0x18)
  if ([double]::IsNaN($health0) -or [double]::IsNaN($shields0)) {
    throw "encounter trace: the player bars could not be read (health $health0, shields $shields0)"
  }
  # Plausibility guards: a wrong offset must fail the capture, not write a fixture full of zeros.
  if ($health0 -lt 0.0 -or $health0 -gt 1000.0) { throw "encounter trace: implausible health $health0 at thing+0x134" }
  if ($shields0 -lt -0.01 -or $shields0 -gt 200.01) { throw "encounter trace: implausible shields $shields0 at inventory bin 60" }
  Log ("encounter trace: player thing {0}, health {1}, shields {2}, world {3}" -f $playerThing, $health0, $shields0, $world)

  $clock0 = Read-JkGameMs -ProcessId $ProcessId
  if ($clock0 -lt 0) { throw 'encounter trace: the game clock (0x00836C14) could not be read' }
  $lines = New-Object System.Collections.Generic.List[string]
  $lines.Add((@('t_ms', 'kind', 'thing', 'template', 'x', 'y', 'z', 'yaw', 'health', 'shields', 'detail') -join ([char]9)))
  $previous = $null
  $previousType3 = @{}
  $sampled = 0
  $nextSample = 0
  $shots = 0
  $hits = 0
  $failedReads = 0
  while ($true) {
    $clock = Read-JkGameMs -ProcessId $ProcessId
    if ($clock -lt 0) { throw 'encounter trace: the game clock stopped reading' }
    $elapsed = [double]$clock - [double]$clock0
    if ($elapsed -ge $Milliseconds) { break }
    if ($elapsed -lt $nextSample) { Start-Sleep -Milliseconds 8; continue }

    $count = [JkRetail]::ReadU32($ProcessId, $world + 0xBC)
    $base = [JkRetail]::ReadU32($ProcessId, $world + 0xC4)
    if ($count -le 0 -or $count -gt 5000 -or $base -le 0) {
      $failedReads++
      Start-Sleep -Milliseconds 8
      continue
    }
    $bytes = [JkRetail]::ReadBytes($ProcessId, $base, [int]($count * 0x2B0))
    if ($null -eq $bytes) {
      $failedReads++
      Start-Sleep -Milliseconds 8
      continue
    }
    $health = [JkRetail]::ReadFloat($ProcessId, $playerThing + 0x134)
    $invNow = [JkRetail]::ReadU32($ProcessId, $playerThing + 0x1C0)
    $shields = if ($invNow -gt 0) { [JkRetail]::ReadFloat($ProcessId, $invNow + 0x88 + 60 * 0x18) } else { [double]::NaN }
    if ([double]::IsNaN($health) -or [double]::IsNaN($shields)) {
      $failedReads++
      Start-Sleep -Milliseconds 8
      continue
    }
    $px = [JkRetail]::ReadFloat($ProcessId, $playerThing + 0xB8)
    $py = [JkRetail]::ReadFloat($ProcessId, $playerThing + 0xBC)
    $pz = [JkRetail]::ReadFloat($ProcessId, $playerThing + 0xC0)
    # `thing + 0x88` is `orient.rvec` (rdMatrix): a yaw-only rotation gives
    # (cos yaw, sin yaw, 0), so atan2(y, x) recovers the body yaw. Reading the raw element
    # (the old 0x8C) is sin(yaw), not degrees -- actor facings need this conversion.
    $pyaw = [math]::Atan2([double][JkRetail]::ReadFloat($ProcessId, $playerThing + 0x8C), [double][JkRetail]::ReadFloat($ProcessId, $playerThing + 0x88)) * 180.0 / [math]::PI
    $sampleMs = [math]::Round($nextSample)

    $currentType3 = @{}
    $sampleLines = New-Object System.Collections.Generic.List[string]
    $sampleLines.Add((@($sampleMs, 'player', $playerThing, 'walkplayer', ('{0:F4}' -f $px), ('{0:F4}' -f $py), ('{0:F4}' -f $pz), ('{0:F2}' -f $pyaw), ('{0:F1}' -f $health), ('{0:F1}' -f $shields), '') -join ([char]9)))
    for ($i = 0; $i -lt $count; $i++) {
      $offset = $i * 0x2B0
      $type = [BitConverter]::ToInt32($bytes, $offset + 0x0C)
      if ($type -eq 3) {
        $template = [BitConverter]::ToUInt16($bytes, $offset + 0x04)
        $x = [BitConverter]::ToSingle($bytes, $offset + 0xB8)
        $y = [BitConverter]::ToSingle($bytes, $offset + 0xBC)
        $z = [BitConverter]::ToSingle($bytes, $offset + 0xC0)
        $currentType3[$i] = [pscustomobject]@{ Template = $template; X = $x; Y = $y; Z = $z }
        # A placed weapon is in the baseline sample; anything new afterwards is a fired bolt.
        if ($sampled -gt 0 -and -not $previousType3.ContainsKey($i)) {
          $shots++
          $sampleLines.Add((@($sampleMs, 'shot', $i, $template, ('{0:F4}' -f $x), ('{0:F4}' -f $y), ('{0:F4}' -f $z), '', '', '', '') -join ([char]9)))
          Log ("encounter trace: shot at {0} ms from thing {1} template {2} at ({3:N2}, {4:N2}, {5:N2})" -f $sampleMs, $i, $template, $x, $y, $z)
        }
        continue
      }
      if ($type -ne 2) { continue }
      $actorHealth = [BitConverter]::ToSingle($bytes, $offset + 0x134)
      if ($actorHealth -le 0.0) { continue }
      $x = [BitConverter]::ToSingle($bytes, $offset + 0xB8)
      $y = [BitConverter]::ToSingle($bytes, $offset + 0xBC)
      $z = [BitConverter]::ToSingle($bytes, $offset + 0xC0)
      $dx = $x - $px
      $dy = $y - $py
      if (($dx * $dx + $dy * $dy) -gt (8.0 * 8.0)) { continue }
      # Same conversion as the player: orient.rvec (cos yaw, sin yaw) -> degrees.
      $yaw = [math]::Atan2([double][BitConverter]::ToSingle($bytes, $offset + 0x8C), [double][BitConverter]::ToSingle($bytes, $offset + 0x88)) * 180.0 / [math]::PI
      $template = [BitConverter]::ToUInt16($bytes, $offset + 0x04)
      $sampleLines.Add((@($sampleMs, 'actor', $i, $template, ('{0:F4}' -f $x), ('{0:F4}' -f $y), ('{0:F4}' -f $z), ('{0:F2}' -f $yaw), ('{0:F1}' -f $actorHealth), '', '') -join ([char]9)))
    }
    # A landed hit is a drop in either bar. Shield recharge and respawns raise them; only drops count.
    if ($null -ne $previous) {
      $lost = ($previous.Shields - $shields) + ($previous.Health - $health)
      if (($previous.Shields - $shields) -gt 0.05 -or ($previous.Health - $health) -gt 0.05) {
        $hits++
        $amount = [math]::Max(0.0, $lost)
        $sampleLines.Add((@($sampleMs, 'hit', $playerThing, '', '', '', '', '', ('{0:F1}' -f $health), ('{0:F1}' -f $shields), ('amount {0:F1}' -f $amount)) -join ([char]9)))
        Log ("encounter trace: hit at {0} ms for {1:N1}, {2:N1} shield(s), {3:N1} health" -f $sampleMs, $amount, $shields, $health)
      }
    }
    foreach ($line in $sampleLines) { $lines.Add($line) }
    $previous = [pscustomobject]@{ Health = $health; Shields = $shields }
    $previousType3 = $currentType3
    $sampled++
    # The rows fill the fixed cadence even when the engine renders slower: a grid point between two
    # frames holds the state of the frame before it, so both sides have a row every `CadenceMs`.
    $nextSample += $CadenceMs
  }
  Set-Content -LiteralPath $path -Value $lines -Encoding utf8
  if ($sampled -eq 0) { throw 'encounter trace: no samples were taken' }
  if ($failedReads -gt 0) { Log ("encounter trace: {0} read attempt(s) failed and were retried" -f $failedReads) }
  Log ("encounter trace: wrote {0} ({1} sample(s), {2} shot(s), {3} hit(s) over {4} ms of game time)" -f $path, $sampled, $shots, $hits, $Milliseconds)
  return $path
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
# pixel covers nearly the whole frame. Instead this counts, per row and per column, how many pixels
# carry *detail* (a value differing from the frame's corner value): the world view rows/columns are
# mostly textured (>50%), while the letterbox bars, the HUD corners and the notification are uniform
# islands. Brightness is not the test - level 2's opening plaza is dark but highly dithered, and a
# 'lit' test cropped it to a 1094x355 strip of a 1440x900 frame (round 103). That also works for the
# 8-bit PCX path (whose -devMode overlay text sits in the top corners the same way).
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
    # What separates a bar row from a world row is not brightness but *detail*: the letterbox bars are
    # one uniform value (the frame's own corner, ~16 luma), while a world row is textured. Round 103
    # found this the hard way: level 2's opening plaza is dark, so most of its world pixels sat at or
    # below the 16-luma "lit" threshold and a 1440x900 player-camera frame was cropped to a 1094x355
    # strip. Rows/columns are now scored by how many pixels differ from the corner value by at least
    # `barTolerance`; a dark floor is still full of dithering, so world rows score high and bars stay
    # near zero (the HUD corners and the notification are small islands either way).
    $corner = $gray[0]
    $barTolerance = 2
    $rowHits = New-Object 'int[]' $imageH
    $colHits = New-Object 'int[]' $imageW
    $i = 0
    for ($y = 0; $y -lt $imageH; $y++) {
      $row = 0
      for ($x = 0; $x -lt $imageW; $x++) {
        $delta = [int]$gray[$i] - [int]$corner
        if ($delta -ge $barTolerance -or $delta -le -$barTolerance) { $row++; $colHits[$x]++ }
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
    # The engine centres the view inside the display mode, so the opposite margins are equal by
    # construction. When a frame's top and bottom margins disagree beyond rounding, the detail test
    # swallowed dark world rows on one side (level 2's plaza: top 115 px against the real 84), and a
    # detected margin can only come out *too big*, never too small - so both sides take the smaller.
    $topMargin = $y
    $bottomMargin = $imageH - ($y + $h)
    if ([math]::Abs($topMargin - $bottomMargin) -gt 2) {
      $margin = [math]::Min($topMargin, $bottomMargin)
      $y = $margin
      $h = $imageH - 2 * $margin
    }
    $leftMargin = $x
    $rightMargin = $imageW - ($x + $w)
    if ([math]::Abs($leftMargin - $rightMargin) -gt 2) {
      $margin = [math]::Min($leftMargin, $rightMargin)
      $x = $margin
      $w = $imageW - 2 * $margin
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


# Menus do not answer F12 (that binding is in-game), so a menu screenshot grabs the window itself
# with CopyFromScreen over the window rect.
function Save-JkWindowPng {
  param([Parameter(Mandatory)][int]$ProcessId, [Parameter(Mandatory)][string]$Path)
  Add-Type -AssemblyName System.Drawing
  $rect = [JkRetail]::WindowRectText((Get-JkWindow -ProcessId $ProcessId))
  if ($rect -match '\((-?\d+),(-?\d+)\)-\((-?\d+),(-?\d+)\)') {
    $w = [int]$Matches[3] - [int]$Matches[1]; $h = [int]$Matches[4] - [int]$Matches[2]
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen([int]$Matches[1], [int]$Matches[2], 0, 0, (New-Object System.Drawing.Size($w, $h)))
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
  }
}

function Invoke-JkCaptureRow {
  param([Parameter(Mandatory)]$Row)

  $name = $Row.Name
  $level = $Row.Level
  $requestedPose = $Row.Pose
  # Numbered spawn rows name their Debug entry (`walk17_spawn` -> 17). An explicit -StartLevel wins;
  # an unnumbered non-first-level row fails closed instead of silently capturing New Game.
  $startLevel = $StartLevel
  $otherBootRoute = $StartEpisode -or $LoadSave -or $StopAtMenu -or $DebugShot -or $LoadShot -or $MainMenuShot -or $IntroShotMs
  if (-not $startLevel -and -not $otherBootRoute) {
    if ($name -match '^(?:walk|level)(\d+)_spawn$') { $startLevel = $Matches[1] }
    if (-not $startLevel -and $level -notlike '01narshadda*') {
      throw "row '$name' needs a Debug level index: use -StartLevel or name it walkN_spawn/levelN_spawn"
    }
  }
  $poseText = if ($null -eq $requestedPose) { 'spawn (-)' } else { ($requestedPose -join ',') }
  Log "=== capture '$name' level=$level debugEntry=$startLevel requested pose=$poseText (shots line $($Row.Line)) ==="
  Assert-NoConflictingEngine

  $process = $null
  $gameWindow = [IntPtr]::Zero
  $consoleWindow = [IntPtr]::Zero
  try {
    $exe = Join-Path $WorkCopy 'JK.EXE'
    $launchArgs = @('-devMode') + $JkArgs
$process = Start-Process -FilePath $exe -ArgumentList $launchArgs -WorkingDirectory $WorkCopy -PassThru
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
    # The shell is not the foreground process, so the game can start behind whatever was focused and
    # every synthetic key would go there. Raise it through the foreground thread's input queue first.
    $activated = [JkRetail]::Activate($gameWindow)
    Log "raised the game window (activated=$activated, foreground=$([JkRetail]::ForegroundTitle()))"
    Send-JkClick -X 50 -Y 50
    if ($IntroShotMs -gt 0) {
      # The (50,50) click dismissed the dev console, so the movie has just started: wait on it.
      Start-Sleep -Milliseconds $IntroShotMs
      $introPng = Join-Path $Out 'intro-cutscene.png'
      Add-Type -AssemblyName System.Drawing
      $rect = [JkRetail]::WindowRectText((Get-JkWindow -ProcessId $process.Id))
      if ($rect -match '\((-?\d+),(-?\d+)\)-\((-?\d+),(-?\d+)\)') {
        $w = [int]$Matches[3] - [int]$Matches[1]; $h = [int]$Matches[4] - [int]$Matches[2]
        $bmp = New-Object System.Drawing.Bitmap($w, $h)
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen([int]$Matches[1], [int]$Matches[2], 0, 0, (New-Object System.Drawing.Size($w, $h)))
        $bmp.Save($introPng, [System.Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
      }
      Log "captured the opening cutscene after $IntroShotMs ms; wrote $introPng"
      return
    }
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
    if ($MainMenuShot) {
      $mainPng = Join-Path $Out 'main-menu.png'
      Add-Type -AssemblyName System.Drawing
      $rect = [JkRetail]::WindowRectText((Get-JkWindow -ProcessId $process.Id))
      if ($rect -match '\((-?\d+),(-?\d+)\)-\((-?\d+),(-?\d+)\)') {
        $w = [int]$Matches[3] - [int]$Matches[1]; $h = [int]$Matches[4] - [int]$Matches[2]
        $bmp = New-Object System.Drawing.Bitmap($w, $h)
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen([int]$Matches[1], [int]$Matches[2], 0, 0, (New-Object System.Drawing.Size($w, $h)))
        $bmp.Save($mainPng, [System.Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
      }
      Log "stopped on the main menu; wrote $mainPng"
      return
    }
    Send-JkClick -X 320 -Y 195
    Start-Sleep -Milliseconds 1500
    Log 'clicked Single Player (320,195)'
    if ($startLevel) {
      Send-JkClick -X 105 -Y 102
      Start-Sleep -Milliseconds 1500
      Send-JkClick -X 330 -Y 220
      Start-Sleep -Milliseconds 800
      Send-JkClick -X 505 -Y 443
      Start-Sleep -Milliseconds 1500
      # Click the list's first row to select it, then walk down with the arrow key: this scrolls the
      # list for levels past the visible nine, which a y-coordinate click cannot reach.
      Send-JkClick -X 330 -Y 238
      Start-Sleep -Milliseconds 600
      for ($k = 1; $k -lt [int]$startLevel; $k++) {
        Send-JkKey -Name 'DOWN'
        Start-Sleep -Milliseconds 220
      }
      Start-Sleep -Milliseconds 400
      Send-JkClick -X 505 -Y 443
      Start-Sleep -Milliseconds 1000
      Log "chose level $startLevel through the [Debug] level select"
      # `-SettleMs` was documented for exactly this and never used: a level that opens with a scripted
      # sequence moves the body after the load, so read `coords` only once it has settled. This is the
      # pose retail itself puts the body at -- what Phase 2's walk legs need, with no warp.
      Start-Sleep -Milliseconds $SettleMs
      foreach ($line in @(Invoke-JkDevConsoleCommand -ProcessId $process.Id -Command 'coords')) {
        Log "spawn| $line"
      }
    }

    if ($StartEpisode) {
      Send-JkClick -X 105 -Y 102
      Start-Sleep -Milliseconds 1500
      $rowY = 220 + 18 * ([int]$StartEpisode - 1)
      Send-JkClick -X 330 -Y $rowY
      Start-Sleep -Milliseconds 800
      Send-JkClick -X 505 -Y 443
      # Ok on the chooser returns to the submenu; the level select then needs a second Ok to commit
      # (clicking New Game instead restarts the default episode - the user found this in play).
      Start-Sleep -Milliseconds 1500
      Send-JkClick -X 505 -Y 443
      Start-Sleep -Milliseconds 1500
      # Ok stores the chosen episode and returns to the submenu; New Game then starts its campaign
      # (with the chooser itself already closed).
      Start-Sleep -Milliseconds 1200
      Send-JkClick -X 304 -Y 115
      Start-Sleep -Milliseconds 1000
      Start-Sleep -Milliseconds 1200
      $null = Wait-JkConsoleLine -ProcessId $process.Id -Pattern 'parse section things' -TimeoutSec $LevelTimeoutSec
      Start-Sleep -Milliseconds 12000
      foreach ($line in @(Invoke-JkDevConsoleCommand -ProcessId $process.Id -Command 'coords')) { Log "console| $line" }
      $epPng = Join-Path $Out ('episode-' + $StartEpisode + '-level.png')
      Save-JkWindowPng -ProcessId $process.Id -Path $epPng
      Log "started episode $StartEpisode; wrote $epPng"
      return
    }
    if ($DebugShot) {
      Send-JkClick -X 105 -Y 102
      Start-Sleep -Milliseconds 1800
      $dbgPng = Join-Path $Out 'debug-menu.png'
      Save-JkWindowPng -ProcessId $process.Id -Path $dbgPng
      Log "clicked [Debug] (105,102); wrote $dbgPng"
      return
    }
    if ($LoadShot) {
      Send-JkClick -X 534 -Y 103
      Start-Sleep -Milliseconds 1800
      $loadPng = Join-Path $Out 'load-menu.png'
      Save-JkWindowPng -ProcessId $process.Id -Path $loadPng
      Log "clicked Load (534,103); wrote $loadPng"
      return
    }
    if ($LoadSave) {
      Send-JkClick -X 534 -Y 103
      Start-Sleep -Milliseconds 1800
      Send-JkClick -X 547 -Y 443
      # A failed restore shows a modal message box ("failed to load save"), which freezes the loading
      # screen and eats the console keys: show what is actually on screen, then dismiss it with ACTIVATE.
      Start-Sleep -Milliseconds 8000
      $earlyPng = Join-Path $Out 'load-after-ok.png'
      Save-JkWindowPng -ProcessId $process.Id -Path $earlyPng
      Log "screenshot 8 s after Ok: $earlyPng"
      Send-JkKey -Name 'ENTER'
      Start-Sleep -Milliseconds 1500
      Start-Sleep -Milliseconds 6000
      # The load takes a while and the dev console answers only once the level is up, so poll `coords`
      # until it returns a position instead of guessing one sleep.
      $null = Wait-JkConsoleLine -ProcessId $process.Id -Pattern 'parse section things' -TimeoutSec $LevelTimeoutSec
      # The dev console dialog fights the loading loop for focus, so let the level come up quietly
      # before touching the console at all.
      Start-Sleep -Milliseconds 45000
      $deadline = [Diagnostics.Stopwatch]::StartNew()
      $booted = $false
      while (-not $booted -and $deadline.Elapsed.TotalSeconds -lt 40) {
        foreach ($line in @(Invoke-JkDevConsoleCommand -ProcessId $process.Id -Command 'coords')) {
          Log "console| $line"
          if ($line -match 'Pos:') { $booted = $true }
        }
        if (-not $booted) { Start-Sleep -Milliseconds 2000 }
      }
      Log "level booted: $booted after $([math]::Round($deadline.Elapsed.TotalSeconds, 1)) s"
      foreach ($command in @($ConsoleCommands | ForEach-Object { $_ -split ',' } | Where-Object { $_ -ne '' -and $_ -ne 'coords' })) {
        foreach ($line in @(Invoke-JkDevConsoleCommand -ProcessId $process.Id -Command $command)) { Log "console| $line" }
      }
      $png = Join-Path $Out 'loaded-level.png'
      Save-JkWindowPng -ProcessId $process.Id -Path $png
      Log "loaded the savegame; wrote $png"
      return
    }
    if ($StopAtMenu) {
      $menuPng = Join-Path $Out 'single-player-menu.png'
      # Menus do not answer F12 (that binding is in-game), so grab the window itself.
      Add-Type -AssemblyName System.Drawing
      $rect = [JkRetail]::WindowRectText((Get-JkWindow -ProcessId $process.Id))
      if ($rect -match '\((-?\d+),(-?\d+)\)-\((-?\d+),(-?\d+)\)') {
        $w = [int]$Matches[3] - [int]$Matches[1]; $h = [int]$Matches[4] - [int]$Matches[2]
        $bmp = New-Object System.Drawing.Bitmap($w, $h)
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen([int]$Matches[1], [int]$Matches[2], 0, 0, (New-Object System.Drawing.Size($w, $h)))
        $bmp.Save($menuPng, [System.Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
      }
      Log "stopped on the Single Player submenu; wrote $menuPng"
      return
    }
    if (-not $startLevel) {
      Send-JkClick -X 304 -Y 115
      Log 'clicked New Game (304,115)'
    }
    Start-Sleep -Milliseconds 1000

    # Level-load detection: the dev console ListBox logs the parse lines.
    $parseLine = Wait-JkConsoleLine -ProcessId $process.Id -Pattern 'parse section things' -TimeoutSec $LevelTimeoutSec
    if (-not $parseLine) {
      $tail = @(Get-JkConsoleLines -ProcessId $process.Id)
      Log "dev console ListBox tail at failure ($($tail.Count) lines):"
      foreach ($entry in ($tail | Select-Object -Last 25)) { Log ("  console| " + $entry) }
      throw "level did not load within $LevelTimeoutSec s (no 'parse section things' line in the dev-console ListBox)"
    }
    Log "level loaded: $parseLine"
    # How long the level has been running when the series starts. The series clock alone only says how
    # far apart its frames are; the port must be sampled at the same **game** time, and the engine's
    # clock starts here. Round 308 adds it so `series-gate.ps1` can offset the port's first frame.
    $levelReadyClock = [Diagnostics.Stopwatch]::StartNew()

    # Let the load fade finish, then optionally warp to the requested pose (best effort).
    Start-Sleep -Milliseconds 1200
    if ($PreShotMs -gt 0) { Start-Sleep -Milliseconds $PreShotMs }
    if ($null -ne $requestedPose) {
      # shots.tsv gives the camera eye; `warp` positions the player body, and the camera rides it at
      # `thing + 0x168` (see $script:JkEyeOffset), so the body goes 0.037 below the requested eye.
      $bodyZ = $requestedPose[2] - $script:JkEyeOffset
      $warp = 'warp {0} {1} {2} {3} {4} 0' -f $requestedPose[0], $requestedPose[1], $bodyZ, $requestedPose[3], $requestedPose[4]
      Log "warp command: $warp"
      $warpReply = @(Invoke-JkDevConsoleCommand -ProcessId $process.Id -Command $warp)
      if ($warpReply -match 'Position not in world') {
        throw "warp failed for row '$name' in level '$level': Position not in world"
      }
      if ($EncounterMs -le 0) { Start-Sleep -Milliseconds 900 }
    }

    # Issue #9: a door-encounter run replaces the screenshot series (the `finally` still closes the
    # game and restores the desktop). It runs before any console command or action key so nothing
    # pauses the clock or perturbs the fight.
    if ($EncounterMs -gt 0) {
      $encounterDir = Resolve-FromRepo $EncounterOut
      New-Item -ItemType Directory -Force -Path $encounterDir | Out-Null
      $traceName = if ($EncounterName) { $EncounterName } else { $name }
      $tracePath = Invoke-JkEncounterTrace -ProcessId $process.Id -Name $traceName -Milliseconds $EncounterMs -CadenceMs $EncounterCadenceMs -OutDir $encounterDir
      Log "encounter trace mode: skipped the screenshot series/reference for '$name' (wrote $tracePath)"
      return
    }

    if ($Keys.Count -gt 0) {
      foreach ($key in @($Keys | ForEach-Object { $_ -split ',' } | Where-Object { $_ -ne '' })) {
        if ($key -like 'MOUSE*') {
          # The warp's console dialog took focus and DirectInput reads the mouse only while the
          # game is foreground, so put it back before the press (round 302: a `switch_use` click
          # at 0.12 units from the plate was lost this way and the door still read `doorstatus 0`).
          $keyWindow = Get-JkWindow -ProcessId $process.Id
          if ($keyWindow -ne [IntPtr]::Zero) {
            [void][JkRetail]::SetForegroundWindow($keyWindow)
            Start-Sleep -Milliseconds 150
          }
          # Put the cursor in the game window first: mouse events land on whatever is under it.
          $rect = [JkRetail]::WindowRectText((Get-JkWindow -ProcessId $process.Id))
          if ($rect -match '\((-?\d+),(-?\d+)\)-\((-?\d+),(-?\d+)\)') {
            $cx = ([int]$Matches[1] + [int]$Matches[3]) / 2
            $cy = ([int]$Matches[2] + [int]$Matches[4]) / 2
            [JkRetail]::MoveTo([int]$cx, [int]$cy)
            Start-Sleep -Milliseconds 120
          }
          Log "pressing $key after the warp (cursor $cx,$cy)"
          # A tap can fall between the software renderer's frames; an action needs a hold.
          Send-JkMouseButton -Button ($key -replace '^MOUSE', '') -HoldMs $ActionHoldMs
        } else {
          Log "pressing $key after the warp"
          # Keyboard keys stay a tap here: the cutscene-skip rows pass `SPACE,ENTER,SPACE` and a
          # 600 ms hold would change what they do. A held action key belongs in `-KeysAfter`.
          Send-JkKey -Name $key
        }
        Start-Sleep -Milliseconds 300
      }
    }

    # In walk-trace mode the console runs *after* the walk (opening the dev-console dialog before the
    # walk steals the movement keys, which makes the body drift instead of walking).
    if ($ConsoleCommands.Count -gt 0 -and $WalkTraceMs -le 0) {
      $null = Invoke-JkConsoleCapture -ProcessId $process.Id -Name $name -Commands $ConsoleCommands
    }

    # Phase 0.5 trace v0: a trace run replaces this row's screenshot captures (the `finally` below
    # still closes the game and restores the desktop).
    if ($WalkTraceMs -gt 0) {
      if ($null -ne $requestedPose) {
        $beforeTrace = Read-JkCoords -ProcessId $process.Id
        if (-not $beforeTrace) { throw "walk trace '$name': coords unavailable after warp" }
        $traceDelta = [math]::Max([math]::Abs($beforeTrace.Pos[0] - $requestedPose[0]), [math]::Abs($beforeTrace.Pos[1] - $requestedPose[1]))
        $traceDelta = [math]::Max($traceDelta, [math]::Abs($beforeTrace.Pos[2] + $script:JkEyeOffset - $requestedPose[2]))
        if ($traceDelta -gt 0.25) {
          throw ("walk trace '{0}' is {1:N2} units from its requested pose in level '{2}'; refusing to write a wrong-level fixture" -f $name, $traceDelta, $level)
        }
      }
      $tracePath = Invoke-JkWalkTrace -ProcessId $process.Id -Name $name -Milliseconds $WalkTraceMs -Key $WalkTraceKey -SettleMs $(if ($WalkTraceSettleMs -gt 0) { $WalkTraceSettleMs } else { $Row.Settle })
      Log "walk trace mode: skipped the screenshot series/reference for '$name' (wrote $tracePath)"
      if ($ConsoleCommands.Count -gt 0) {
        # After the walk: this is where a trigger's effect shows up (cog state after the trigger).
        $null = Invoke-JkConsoleCapture -ProcessId $process.Id -Name $name -Commands $ConsoleCommands
      }
      return
    }

    if ($KeysAfter.Count -gt 0) {
      # The console dialog holds focus after the last `cogstatus`. DirectInput reads the keyboard
      # only while the game window is foreground (the walk trace already forces this), so put it
      # back before any action key; otherwise the press goes to the dialog and the player stands.
      $gameWindow = Get-JkWindow -ProcessId $process.Id
      if ($gameWindow -ne [IntPtr]::Zero) {
        [void][JkRetail]::SetForegroundWindow($gameWindow)
        Start-Sleep -Milliseconds 150
        Log ("action focus: foreground='" + [JkRetail]::ForegroundTitle() + "'")
      }
      foreach ($key in @($KeysAfter | ForEach-Object { $_ -split ',' } | Where-Object { $_ -ne '' })) {
        Log "pressing $key after the console commands"
        if ($key -match '^MOUSE') {
          # An action binding is a mouse button (JK default: use = middle); the cursor has to be
          # inside the game window for the event to reach it, the same route `-Keys` takes.
          $win = Get-JkWindow -ProcessId $process.Id
          if ($win -ne [IntPtr]::Zero) {
            $rect = [JkRetail]::WindowRectText($win)
            if ($rect -match '\((-?\d+),(-?\d+)\)-\((-?\d+),(-?\d+)\)') {
              $mx = ([int]$Matches[1] + [int]$Matches[3]) / 2
              $my = ([int]$Matches[2] + [int]$Matches[4]) / 2
              [JkRetail]::MoveTo([int]$mx, [int]$my)
              Start-Sleep -Milliseconds 120
            }
          }
          Send-JkMouseButton -Button ($key -replace '^MOUSE', '') -HoldMs $ActionHoldMs
        } else {
          Send-JkKeyHold -Name $key -HoldMs $ActionHoldMs
        }
        Start-Sleep -Milliseconds 1500
      }
      Start-Sleep -Milliseconds $AfterWaitMs
      # Phase 1 step 4: the trigger has fired (or not) by now, so read the cog state again under a
      # distinct name. The console has been closed for 13.5 s, so the engine has re-acquired its input
      # devices -- which is the point of pressing *after* the first read (round 192's finding).
      if ($ConsoleCommands.Count -gt 0) {
        $null = Invoke-JkConsoleCapture -ProcessId $process.Id -Name ("{0}-after" -f $name) -Commands $ConsoleCommands
      }
    }

    # Time series: capture the first seconds after the level becomes visible. These are kept as
    # full raw frames (the -devMode overlay in the top corners is harmless for the motion analysis).
    $series = New-Object System.Collections.Generic.List[object]
    # The wall clock inside the series. `series_ssim_first_last` alone cannot be aligned with a port run
    # because it says nothing about *when* each frame was taken: retail's renderer runs at a few frames a
    # second under the dgVoodoo path, so the 1 s spacing is a floor, not the real interval. The offsets
    # below let `series-gate.ps1` capture the port over the same **measured** span (and, for a level whose
    # opening is a cutscene, say exactly how far into the level the series starts).
    $seriesSettleMs = [int][math]::Round($levelReadyClock.Elapsed.TotalMilliseconds)
    Log "series settle: $seriesSettleMs ms after the level loaded"
    $seriesClock = [Diagnostics.Stopwatch]::StartNew()
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
          Ms = [int][math]::Round($seriesClock.Elapsed.TotalMilliseconds)
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

    # A level-spawn row must show **that level's** own pose. Round 252 found the level-1 `spawn`
    # reference had been captured with `-StartLevel 3`, so the row named 01narshadda but held level
    # 3's pose and scored 0.18 against the port — a fixture bug, not a port regression. Warn loudly
    # (never fatal: a level's opening script can legitimately move the body after load).
    if ($null -eq $requestedPose -and $coordsA -and $coordsSource -eq 'coords' `
        -and $script:WalkplayerPoses.ContainsKey($level)) {
      $known = $script:WalkplayerPoses[$level]
      $delta = [math]::Sqrt(
        [math]::Pow($coordsA.Pos[0] - $known.Pos[0], 2) +
        [math]::Pow($coordsA.Pos[1] - $known.Pos[1], 2) +
        [math]::Pow($coordsA.Pos[2] - $known.Pos[2], 2))
      if ($delta -gt 2.0) {
        Log ("WARNING: row '{0}' is the {1} spawn, but coords read ({2}, {3}, {4}) — {5:N2} units from the level's walkplayer pose. Did -StartLevel boot another level?" -f `
            $name, $level, $coordsA.Pos[0], $coordsA.Pos[1], $coordsA.Pos[2], $delta)
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
        if (-not $warpVerified) {
          # Round 284: a *large* mismatch is not "the level's opening moved the body" — it is the
          # wrong level or a warp that never happened, and writing the fixture anyway poisons every
          # later comparison (a row named 15maw that photographed level 1's spawn scored as level 14).
          # Retail cannot load an arbitrary level from the console or the command line (`docs/gaps.md`,
          # rounds 60-61), so this will fire whenever a non-first-level row is captured without a
          # savegame route. Fail closed above 5 units; keep the advisory behaviour below it.
          if ([math]::Abs($maxDelta) -gt 5.0) {
            throw ("warp verification failed by {0:N2} units for row '{1}': the pose is not the requested one, so no fixture was written. A non-first-level row needs a savegame (docs/gaps.md, rounds 60-61)." -f $maxDelta, $name)
          }
          Log 'warp verification FAILED (unverified pose) — continuing'
        }
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
      pose_verified = $warpVerified
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
      series = @($series | ForEach-Object { [ordered]@{ index = $_.Index; file = $_.File; md5 = $_.Md5; yavg = $_.Yavg; bytes = $_.Bytes; ms = $_.Ms } })
      # Per-frame offsets from the first series capture, and the span they cover. `series-gate.ps1` reads
      # these to capture the port over the same real interval instead of assuming 1 frame a second.
      series_times_ms = @($series | ForEach-Object { $_.Ms })
      # Wall time from "level loaded" to the first series frame: the port's series must start at the
      # same game time, not at the same frame index (round 308).
      series_settle_ms = $seriesSettleMs
      series_span_ms = if ($series.Count -ge 2) { $series[$series.Count - 1].Ms - $series[0].Ms } else { 0 }
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

# `-ProbeBox <raw.png>`: run only the world-box detector on a retained raw frame and print the box.
# A capture's crop is the one thing this harness cannot re-derive later (the raw frames are kept, the
# reticle of bright pixels is not), so this makes the detector testable on any frame already on disk.
if ($ProbeBox) {
    # `-File` binding hands a list through as one string, so accept semicolon-separated paths too.
    foreach ($raw in @($ProbeBox | ForEach-Object { $_ -split ';' } | Where-Object { $_ })) {
        $box = Get-JkWorldBox -RawPng $raw
        Write-Host ("{0}: {1} (edge fill: rows {2:P0}, columns {3:P0})" -f
            $raw, $box.Text, $box.RowFill, $box.ColFill)
    }
    exit 0
}

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

$selected = if ($Only.Count -gt 0) {
  @($rows | Where-Object { $Only -contains $_.Name })
} else { $rows }
if ($selected.Count -eq 0) { throw ("-Only matched no row; the shots file has: " + (($rows | ForEach-Object { $_.Name }) -join ', ')) }
# One-launch probes. `-DebugShot`, `-LoadShot` and `-StartEpisode` photograph a **menu**, not a level,
# and their branch returns from the row — not from the run — so without this guard they relaunched JK.EXE
# once per row (thirteen launches to read one menu, round 284). A probe is one row by definition.
if (($DebugShot -or $LoadShot -or $StartEpisode) -and $selected.Count -gt 1) {
  Log ("one-launch probe: running 1 of {0} selected row(s), '{1}'" -f $selected.Count, $selected[0].Name)
  $selected = @($selected[0])
}
Log ("selected rows: " + (($selected | ForEach-Object { $_.Name }) -join ', '))

# Walk legs are preflighted first (round 251): a leg whose forward route walks into geometry parks the
# game at the wall and leaves the operator to close it, which is what made the round-250 QA runs useless.
# The preflight is the port's own **headless** walk (`--walk --warp`): no window and no retail launch.
if ($WalkTraceMs -gt 0) {
  $legNames = @($selected | ForEach-Object { $_.Name })
  $onlyList = ($legNames | ForEach-Object { "'$_'" }) -join ','
  Log ("walk preflight for {0} leg(s): {1}" -f $legNames.Count, ($legNames -join ', '))
  $pre = & pwsh -NoProfile -Command "& 'tools/parity/walk-preflight.ps1' -Only @($onlyList)" *>&1
  $blockedLegs = @()
  foreach ($line in $pre) {
    if ("$line" -match '^walk-preflight\t(\S+)\tFAIL') { $blockedLegs += $Matches[1] }
  }
  if ($blockedLegs.Count -gt 0) {
    Log ("walk preflight: blocked leg(s) — the body cannot advance from the pose: {0}" -f ($blockedLegs -join ', '))
    if (-not $ForceWalk) {
      throw ("refusing to capture walk leg(s) {0}: the port's own walk cannot advance from the pose (see tools/parity/walk-preflight.ps1). Fix the pose/heading in walk-shots.tsv, drop the leg, or pass -ForceWalk." -f ($blockedLegs -join ', '))
    }
    Log 'walk preflight: -ForceWalk set; capturing the blocked leg(s) anyway'
  } else {
    Log 'walk preflight: every selected leg moves in the port'
  }
}

foreach ($row in $selected) {
  try {
    Invoke-JkCaptureRow -Row $row
  } catch {
    Log "FATAL while capturing '$($row.Name)': $($_.Exception.Message)"
    throw
  }
}

Log '=== retail-capture done: all rows captured ==='
